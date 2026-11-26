//! Air-gapped local LAN vault pairing (`X25519` + `ChaCha20-Poly1305` 8-shard RAM key),
//! offline BIP-39 12-word recovery, `CounterKiosk` hard reception-only lock, and SHA-256 `AuditLog`.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;

use anyhow::{bail, ensure, Result};
use lv_router::vault::{
    derive_x25519_shared, generate_recovery_phrase, recover_from_phrase, DeviceRole, ProtectedKey,
    VaultController, VaultError, VaultState, RECOVERY_WORDS,
};
use premium_discount_sds::audit::{verify_file, AuditLog, Level};
use rand::rngs::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

pub const DEFAULT_SIGNALING_PORT: u16 = 9999;

/// Verifies an address stays strictly inside loopback, link-local, or RFC1918/ULA LAN subnets.
pub fn ensure_local_lan_addr(addr: SocketAddr) -> Result<SocketAddr> {
    let ok = match addr.ip() {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || ((v6.segments()[0] & 0xfe00) == 0xfc00),
    };
    ensure!(ok, "WAN address rejected in air-gapped vault signaling: {addr}");
    Ok(addr)
}

pub struct LouisVaultSession {
    controller: VaultController,
    audit: AuditLog,
}

impl LouisVaultSession {
    pub fn new(role: DeviceRole, audit_dir: Option<&Path>) -> Result<Self> {
        let audit = match audit_dir {
            Some(dir) => {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                AuditLog::open(dir, &format!("louis_docs_{ts}"), 512)?
            }
            None => AuditLog::memory(512),
        };
        Ok(Self {
            controller: VaultController::new(role),
            audit,
        })
    }

    pub fn state(&self) -> VaultState {
        self.controller.state()
    }

    pub fn role(&self) -> DeviceRole {
        self.controller.role()
    }

    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// Pairs two local LAN TCP endpoints via ephemeral X25519 exchange without any WAN STUN/TURN.
    pub fn pair_over_local_tcp(
        &mut self,
        bind_addr: SocketAddr,
        role: DeviceRole,
        context: &[u8],
    ) -> Result<[u8; 32]> {
        ensure_local_lan_addr(bind_addr)?;
        let listener = TcpListener::bind(bind_addr)?;
        let local_addr = listener.local_addr()?;
        ensure_local_lan_addr(local_addr)?;

        let peer_secret = StaticSecret::random_from_rng(OsRng);
        let peer_pub = PublicKey::from(&peer_secret);
        let ctx_peer = context.to_vec();

        let peer_thread = std::thread::spawn(move || -> Result<[u8; 32]> {
            let mut stream = TcpStream::connect(local_addr)?;
            let mut host_pub_bytes = [0u8; 32];
            stream.read_exact(&mut host_pub_bytes)?;
            stream.write_all(peer_pub.as_bytes())?;
            stream.flush()?;
            let key = derive_x25519_shared(&peer_secret, &PublicKey::from(host_pub_bytes), &ctx_peer);
            Ok(*key.reconstruct())
        });

        let (mut host_stream, remote) = listener.accept()?;
        ensure_local_lan_addr(remote)?;
        let host_secret = StaticSecret::random_from_rng(OsRng);
        let host_pub = PublicKey::from(&host_secret);
        host_stream.write_all(host_pub.as_bytes())?;
        host_stream.flush()?;
        let mut peer_pub_bytes = [0u8; 32];
        host_stream.read_exact(&mut peer_pub_bytes)?;

        let peer_key_bytes = peer_thread
            .join()
            .map_err(|_| anyhow::anyhow!("peer LAN handshake thread panicked"))??;
        let host_key = derive_x25519_shared(&host_secret, &PublicKey::from(peer_pub_bytes), context);
        if *host_key.reconstruct() != peer_key_bytes {
            bail!("LAN X25519 derived key mismatch");
        }
        self.unlock(role, host_key)?;
        Ok(peer_key_bytes)
    }

    pub fn unlock(&mut self, role: DeviceRole, key: ProtectedKey) -> std::result::Result<(), VaultError> {
        let res = self.controller.unlock_with_key(role, key);
        match &res {
            Ok(()) => {
                self.audit.log("vault", Level::Info, "vault unlocked via local LAN", None);
            }
            Err(err) => {
                self.audit.log("vault", Level::Warn, format!("unlock denied: {err}"), None);
            }
        }
        res
    }

    pub fn recover_with_phrase(&mut self, words: &[String]) -> Result<()> {
        ensure!(words.len() == RECOVERY_WORDS, "recovery phrase requires {RECOVERY_WORDS} words");
        let key = recover_from_phrase(words)?;
        self.unlock(DeviceRole::PersonalPhone, key)?;
        Ok(())
    }

    pub fn generate_backup_phrase() -> (Vec<String>, ProtectedKey) {
        generate_recovery_phrase()
    }

    pub fn seal_record(&self, aad: &[u8], payload: &[u8]) -> std::result::Result<Vec<u8>, VaultError> {
        self.controller.seal_bytes(aad, payload)
    }

    pub fn open_record(&self, aad: &[u8], sealed: &[u8]) -> std::result::Result<Vec<u8>, VaultError> {
        self.controller.open_bytes(aad, sealed)
    }

    pub fn disconnect_grace_blur(&mut self) {
        self.controller.begin_grace_blur();
        self.audit.log("vault", Level::Warn, "peer disconnected -> GraceBlurred", None);
    }

    pub fn expire_grace_and_zeroize(&mut self) {
        self.controller.seal_and_zeroize();
        self.audit.log("vault", Level::Info, "vault sealed and RAM zeroized", None);
    }

    pub fn verify_audit_on_disk(&self) -> Result<Option<usize>> {
        match self.audit.path() {
            Some(p) => Ok(Some(verify_file(p).map_err(|e| anyhow::anyhow!("{}", e.reason))?.lines)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_lan_pairing_kiosk_lock_wan_rejection_and_zeroize() {
        let wan: SocketAddr = "8.8.8.8:9999".parse().unwrap();
        assert!(ensure_local_lan_addr(wan).is_err());

        // Sealed startup + CounterKiosk hard lock.
        let mut kiosk = LouisVaultSession::new(DeviceRole::CounterKiosk, None).unwrap();
        assert_eq!(kiosk.state(), VaultState::Sealed);
        assert_eq!(
            kiosk.unlock(DeviceRole::CounterKiosk, ProtectedKey::generate()),
            Err(VaultError::KioskForbidden)
        );
        assert_eq!(kiosk.state(), VaultState::Sealed);

        // Localhost X25519 LAN handshake unlocks PersonalPhone & encrypts department ledger.
        let tmp = std::env::temp_dir().join(format!("louis-docs-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut session = LouisVaultSession::new(DeviceRole::PersonalPhone, Some(&tmp)).unwrap();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        session.pair_over_local_tcp(addr, DeviceRole::PersonalPhone, b"dept-ledger").unwrap();
        assert_eq!(session.state(), VaultState::Unlocked);
        let blob = session.seal_record(b"doc-1", b"classified budget").unwrap();
        assert_eq!(session.open_record(b"doc-1", &blob).unwrap(), b"classified budget");

        // GraceBlurred -> Sealed + Zeroize + offline 12-word recovery.
        session.disconnect_grace_blur();
        assert_eq!(session.state(), VaultState::GraceBlurred);
        assert_eq!(session.open_record(b"doc-1", &blob), Err(VaultError::Sealed));
        session.expire_grace_and_zeroize();
        assert_eq!(session.state(), VaultState::Sealed);

        let (words, backup_key) = LouisVaultSession::generate_backup_phrase();
        assert_eq!(words.len(), 12);
        let mut recovered = LouisVaultSession::new(DeviceRole::PersonalPhone, None).unwrap();
        recovered.recover_with_phrase(&words).unwrap();
        let sealed2 = recovered.seal_record(b"a", b"ok").unwrap();
        assert_eq!(backup_key.open(b"a", &sealed2).unwrap(), b"ok");
        assert!(session.verify_audit_on_disk().unwrap().is_some_and(|n| n >= 3));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
