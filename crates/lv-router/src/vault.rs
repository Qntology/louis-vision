//! Offline air-gapped Vault state machine:
//! - `VaultState::{Sealed, Unlocked, GraceBlurred}`
//! - `ProtectedKey`: 8-shard XOR RAM split + `ZeroizeOnDrop` (clears plain key buffers immediately)
//! - X25519 ECDH + ChaCha20-Poly1305 AEAD LAN pairing
//! - Offline 12-word BIP-39 style checksummed mnemonic recovery phrase generation & derivation
//! - `DeviceRole::CounterKiosk` hard reception-only lock (no internal archives, no vault unlock)

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const SHARDS: usize = 8;
pub const RECOVERY_WORDS: usize = 12;
const NONCE_LEN: usize = 12;

/// Offline 256-word deterministic syllable lexicon (256 = 8 bits/word -> 11 payload words + 1 SHA-256 checksum word = 12 words).
/// Avoids network lookups while guaranteeing strict 8-bit checksum validation per 12-word phrase.
const MNEMONIC_SALT: &[u8] = b"lv-router-vault-mnemonic-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultState {
    #[default]
    Sealed,
    Unlocked,
    GraceBlurred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    PersonalPhone,
    CounterKiosk,
}

impl DeviceRole {
    pub fn can_unlock_vault(self) -> bool {
        matches!(self, Self::PersonalPhone)
    }

    pub fn can_view_internal(self) -> bool {
        matches!(self, Self::PersonalPhone)
    }

    pub fn can_scan_intake(self) -> bool {
        true
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("vault is sealed or blurred")]
    Sealed,
    #[error("CounterKiosk is hard-locked for reception only and cannot unlock internal vaults")]
    KioskForbidden,
    #[error("invalid recovery phrase length or checksum")]
    InvalidRecoveryPhrase,
    #[error("AEAD encryption/decryption failure")]
    CryptoFailed,
}

/// 256-bit key material stored exclusively as 8 random XOR shards in RAM (`ZeroizeOnDrop`).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ProtectedKey {
    shards: [[u8; 32]; SHARDS],
}

impl ProtectedKey {
    pub fn from_raw(mut key: [u8; 32]) -> Self {
        let mut shards = [[0u8; 32]; SHARDS];
        let mut acc = [0u8; 32];
        for s in shards.iter_mut().take(SHARDS - 1) {
            OsRng.fill_bytes(s);
            for (a, &b) in acc.iter_mut().zip(s.iter()) {
                *a ^= b;
            }
        }
        for i in 0..32 {
            shards[SHARDS - 1][i] = key[i] ^ acc[i];
        }
        key.zeroize();
        acc.zeroize();
        Self { shards }
    }

    pub fn generate() -> Self {
        let mut raw = [0u8; 32];
        OsRng.fill_bytes(&mut raw);
        Self::from_raw(raw)
    }

    pub fn shard_count(&self) -> usize {
        SHARDS
    }

    pub fn shard(&self, idx: usize) -> Option<&[u8; 32]> {
        self.shards.get(idx)
    }

    pub fn reconstruct(&self) -> Zeroizing<[u8; 32]> {
        let mut out = Zeroizing::new([0u8; 32]);
        for s in &self.shards {
            for (o, &b) in out.iter_mut().zip(s.iter()) {
                *o ^= b;
            }
        }
        out
    }

    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
        let raw = self.reconstruct();
        let key = Key::from(*raw);
        let cipher = ChaCha20Poly1305::new(&key);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);
        let ct = cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad })
            .map_err(|_| VaultError::CryptoFailed)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, VaultError> {
        if sealed.len() < NONCE_LEN + 16 {
            return Err(VaultError::CryptoFailed);
        }
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&sealed[..NONCE_LEN]);
        let raw = self.reconstruct();
        let key = Key::from(*raw);
        let cipher = ChaCha20Poly1305::new(&key);
        let nonce = Nonce::from(nonce_bytes);
        cipher
            .decrypt(&nonce, Payload { msg: &sealed[NONCE_LEN..], aad })
            .map_err(|_| VaultError::CryptoFailed)
    }
}

/// Offline X25519 Diffie-Hellman key pair + HKDF-style SHA-256 session key derivation.
pub fn derive_x25519_shared(
    my_secret: &StaticSecret,
    peer_public: &PublicKey,
    context: &[u8],
) -> ProtectedKey {
    let shared = my_secret.diffie_hellman(peer_public);
    let mut hasher = Sha256::new();
    hasher.update(b"lv-vault-x25519-v1:");
    hasher.update(shared.as_bytes());
    hasher.update(context);
    let mut derived = [0u8; 32];
    derived.copy_from_slice(&hasher.finalize());
    ProtectedKey::from_raw(derived)
}

fn byte_to_word(b: u8) -> String {
    const PREFIX: [&str; 16] = [
        "amber", "basalt", "cedar", "delta", "ember", "flint", "granite", "harbor",
        "indigo", "jasper", "kelvin", "lumen", "marble", "nebula", "ochre", "quartz",
    ];
    const NOUN: [&str; 16] = [
        "arch", "beacon", "canyon", "drift", "estuary", "forge", "glacier", "haven",
        "island", "kernel", "lagoon", "meadow", "needle", "orbit", "prism", "ridge",
    ];
    format!("{}-{}", PREFIX[(b >> 4) as usize], NOUN[(b & 0x0F) as usize])
}

fn word_to_byte(w: &str) -> Option<u8> {
    let (p, n) = w.split_once('-')?;
    const PREFIX: [&str; 16] = [
        "amber", "basalt", "cedar", "delta", "ember", "flint", "granite", "harbor",
        "indigo", "jasper", "kelvin", "lumen", "marble", "nebula", "ochre", "quartz",
    ];
    const NOUN: [&str; 16] = [
        "arch", "beacon", "canyon", "drift", "estuary", "forge", "glacier", "haven",
        "island", "kernel", "lagoon", "meadow", "needle", "orbit", "prism", "ridge",
    ];
    let hi = PREFIX.iter().position(|&x| x == p)? as u8;
    let lo = NOUN.iter().position(|&x| x == n)? as u8;
    Some((hi << 4) | lo)
}

/// Generates an offline 12-word checksummed recovery phrase and derives its `ProtectedKey`.
pub fn generate_recovery_phrase() -> (Vec<String>, ProtectedKey) {
    let mut payload = [0u8; RECOVERY_WORDS - 1];
    OsRng.fill_bytes(&mut payload);
    let chk = Sha256::digest(payload)[0];
    let mut words = Vec::with_capacity(RECOVERY_WORDS);
    for &b in &payload {
        words.push(byte_to_word(b));
    }
    words.push(byte_to_word(chk));
    let key = recover_from_phrase(&words).expect("generated phrase is always valid");
    (words, key)
}

/// Validates a 12-word offline recovery phrase checksum and derives the `ProtectedKey`.
pub fn recover_from_phrase(words: &[String]) -> Result<ProtectedKey, VaultError> {
    if words.len() != RECOVERY_WORDS {
        return Err(VaultError::InvalidRecoveryPhrase);
    }
    let mut bytes = [0u8; RECOVERY_WORDS];
    for (i, w) in words.iter().enumerate() {
        bytes[i] = word_to_byte(w.trim()).ok_or(VaultError::InvalidRecoveryPhrase)?;
    }
    let expected_chk = Sha256::digest(&bytes[..RECOVERY_WORDS - 1])[0];
    if bytes[RECOVERY_WORDS - 1] != expected_chk {
        bytes.zeroize();
        return Err(VaultError::InvalidRecoveryPhrase);
    }
    let mut h = Sha256::new();
    h.update(MNEMONIC_SALT);
    h.update(&bytes[..RECOVERY_WORDS - 1]);
    let mut key = [0u8; 32];
    key.copy_from_slice(&h.finalize());
    bytes.zeroize();
    Ok(ProtectedKey::from_raw(key))
}

/// Stateful controller enforcing `Sealed` startup, LAN X25519 pairing, `GraceBlurred` disconnect handling,
/// and `CounterKiosk` hard reception-only access restrictions.
pub struct VaultController {
    state: VaultState,
    key: Option<ProtectedKey>,
    role: DeviceRole,
}

impl Default for VaultController {
    fn default() -> Self {
        Self::new(DeviceRole::PersonalPhone)
    }
}

impl VaultController {
    pub fn new(role: DeviceRole) -> Self {
        Self { state: VaultState::Sealed, key: None, role }
    }

    pub fn state(&self) -> VaultState {
        self.state
    }

    pub fn role(&self) -> DeviceRole {
        self.role
    }

    pub fn unlock_with_key(&mut self, role: DeviceRole, key: ProtectedKey) -> Result<(), VaultError> {
        if !role.can_unlock_vault() {
            return Err(VaultError::KioskForbidden);
        }
        self.role = role;
        self.key = Some(key);
        self.state = VaultState::Unlocked;
        Ok(())
    }

    pub fn begin_grace_blur(&mut self) {
        if self.state == VaultState::Unlocked {
            self.state = VaultState::GraceBlurred;
        }
    }

    pub fn seal_and_zeroize(&mut self) {
        self.key.take();
        self.state = VaultState::Sealed;
    }

    pub fn seal_bytes(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
        if self.state != VaultState::Unlocked {
            return Err(VaultError::Sealed);
        }
        if !self.role.can_view_internal() {
            return Err(VaultError::KioskForbidden);
        }
        let Some(k) = &self.key else {
            return Err(VaultError::Sealed);
        };
        k.seal(aad, plaintext)
    }

    pub fn open_bytes(&self, aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, VaultError> {
        if self.state != VaultState::Unlocked {
            return Err(VaultError::Sealed);
        }
        if !self.role.can_view_internal() {
            return Err(VaultError::KioskForbidden);
        }
        let Some(k) = &self.key else {
            return Err(VaultError::Sealed);
        };
        k.open(aad, ciphertext)
    }
}
