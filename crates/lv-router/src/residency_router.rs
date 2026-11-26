//! CacheAccessMode-aware residency router preventing read-only search/overlay queries
//! from evicting or thrashing resident vision/language model slots.

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use lv_core::residency::{Group, Loaded, ModelId, Phase, QwenTier, ResidencyMode, SharedResidency};
use lv_core::vocab_trim::canonical_policy_key;

use crate::vault::{DeviceRole, VaultError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheAccessMode {
    #[default]
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteTarget {
    Planar2D,
    Spatial3D,
}

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("ReadOnly cache access cannot evict or load model {model:?}/{phase}; already-resident slot required")]
    ReadOnlyMiss { model: ModelId, phase: String },
    #[error("CounterKiosk terminal forbidden from loading higher-grade model {model:?}")]
    KioskForbidden { model: ModelId },
    #[error("retired or blocked model rejected: {reason}")]
    ModelPolicyBlocked { reason: String },
    #[error(transparent)]
    Inner(#[from] anyhow::Error),
}

/// Validates N2SF device-role clearance for `model`.
/// `CounterKiosk` permits ONLY `Siglip2` and `Qwen35(Nano08B)`.
pub fn ensure_role_allows_model(role: DeviceRole, model: ModelId) -> std::result::Result<(), VaultError> {
    if role == DeviceRole::CounterKiosk
        && !matches!(model, ModelId::Siglip2 | ModelId::Qwen35(QwenTier::Nano08B))
    {
        return Err(VaultError::KioskForbidden);
    }
    Ok(())
}

/// Validates both N2SF device-role clearance and retired-model policy (`canonical_policy_key`) prior to acquisition.
pub fn validate_model_access(role: DeviceRole, model: ModelId, canonical_name: Option<&str>) -> std::result::Result<(), RouterError> {
    if ensure_role_allows_model(role, model).is_err() {
        return Err(RouterError::KioskForbidden { model });
    }
    if let Some(name) = canonical_name {
        if let Err(e) = canonical_policy_key(name) {
            return Err(RouterError::ModelPolicyBlocked { reason: e.to_string() });
        }
    }
    Ok(())
}

pub struct ResidencyRouter {
    residency: SharedResidency,
}

impl ResidencyRouter {
    pub fn new(residency: SharedResidency) -> Self {
        Self { residency }
    }

    pub fn mode(&self) -> ResidencyMode {
        self.residency.lock().unwrap_or_else(|p| p.into_inner()).mode()
    }

    pub fn resident_bytes(&self) -> u64 {
        self.residency.lock().unwrap_or_else(|p| p.into_inner()).resident_bytes()
    }

    /// Acquires model group leases for `role` after enforcing `CounterKiosk` & `canonical_policy_key` clearance.
    pub fn acquire_for_role<F>(
        &self,
        role: DeviceRole,
        access: CacheAccessMode,
        model: ModelId,
        phase: &Phase,
        free_bytes_override: Option<u64>,
        loader: F,
    ) -> std::result::Result<Vec<Arc<Loaded>>, RouterError>
    where
        F: FnMut(Group) -> Result<Arc<Loaded>>,
    {
        if ensure_role_allows_model(role, model).is_err() {
            return Err(RouterError::KioskForbidden { model });
        }
        self.acquire(access, model, phase, free_bytes_override, loader)
    }

    /// Acquires model group leases for `phase`, honoring `CacheAccessMode::ReadOnly` so overlay/search
    /// lookups never thrash or evict another active phase.
    pub fn acquire<F>(
        &self,
        access: CacheAccessMode,
        model: ModelId,
        phase: &Phase,
        free_bytes_override: Option<u64>,
        loader: F,
    ) -> std::result::Result<Vec<Arc<Loaded>>, RouterError>
    where
        F: FnMut(Group) -> Result<Arc<Loaded>>,
    {
        let mut guard = self.residency.lock().unwrap_or_else(|p| p.into_inner());
        if access == CacheAccessMode::ReadOnly {
            // ReadOnly lookups must resolve purely from Pinned or currently loaded slots without swapping.
            return guard
                .lookup_resident(model, phase)
                .ok_or_else(|| RouterError::ReadOnlyMiss {
                    model,
                    phase: phase.name.clone(),
                });
        }
        Ok(guard.enter_phase(model, phase, free_bytes_override, loader)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{
        derive_x25519_shared, generate_recovery_phrase, recover_from_phrase, DeviceRole,
        ProtectedKey, VaultController, VaultError, VaultState,
    };
    use candle_core::Device;
    use x25519_dalek::{PublicKey, StaticSecret};

    #[test]
    fn vault_x25519_eight_shards_kiosk_lock_and_bip39_roundtrip() {
        let alice = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let bob = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let bob_pub = PublicKey::from(&bob);
        let alice_pub = PublicKey::from(&alice);

        let ka = derive_x25519_shared(&alice, &bob_pub, b"session-1");
        let kb = derive_x25519_shared(&bob, &alice_pub, b"session-1");
        assert_eq!(ka.shard_count(), 8);
        assert_eq!(*ka.reconstruct(), *kb.reconstruct());

        let mut vc = VaultController::new(DeviceRole::PersonalPhone);
        assert_eq!(vc.state(), VaultState::Sealed);
        assert_eq!(vc.seal_bytes(b"aad", b"secret"), Err(VaultError::Sealed));

        // CounterKiosk cannot unlock internal vaults
        assert_eq!(
            vc.unlock_with_key(DeviceRole::CounterKiosk, ProtectedKey::generate()),
            Err(VaultError::KioskForbidden)
        );
        assert_eq!(vc.state(), VaultState::Sealed);

        // PersonalPhone unlocks & encrypts/decrypts AEAD payload
        vc.unlock_with_key(DeviceRole::PersonalPhone, ka).unwrap();
        let ct = vc.seal_bytes(b"case-1", b"ledger payload").unwrap();
        assert_eq!(vc.open_bytes(b"case-1", &ct).unwrap(), b"ledger payload");
        assert_eq!(vc.open_bytes(b"wrong-aad", &ct), Err(VaultError::CryptoFailed));

        vc.begin_grace_blur();
        assert_eq!(vc.state(), VaultState::GraceBlurred);
        vc.seal_and_zeroize();
        assert_eq!(vc.state(), VaultState::Sealed);

        // Offline 12-word recovery phrase with checksum verification
        let (mut words, rkey) = generate_recovery_phrase();
        assert_eq!(words.len(), 12);
        let recovered = recover_from_phrase(&words).unwrap();
        assert_eq!(*recovered.reconstruct(), *rkey.reconstruct());
        words[11] = if words[11] == "amber-arch" { "amber-beacon".into() } else { "amber-arch".into() };
        assert_eq!(recover_from_phrase(&words).err(), Some(VaultError::InvalidRecoveryPhrase));
    }

    #[test]
    fn residency_router_blocks_read_only_cache_thrashing() {
        let res = lv_core::residency::Residency::shared(Device::Cpu, 2048, ResidencyMode::Coexist { margin_bytes: 0 });
        {
            let mut g = res.lock().unwrap();
            g.pin(ModelId::Siglip2, Arc::new(Loaded::empty(Group::Vision, 512)));
        }
        let router = ResidencyRouter::new(res);
        let vis = Phase::new("vis", [Group::Vision]);
        let hit = router.acquire(CacheAccessMode::ReadOnly, ModelId::Siglip2, &vis, None, |_| unreachable!()).unwrap();
        assert_eq!(hit[0].device_bytes, 512);

        let miss = Phase::new("qwen35_4b", [Group::blocks(0..2)]);
        assert!(matches!(
            router.acquire(CacheAccessMode::ReadOnly, ModelId::Qwen35(QwenTier::Heavy4B), &miss, None, |g| Ok(Arc::new(Loaded::empty(g, 200)))),
            Err(RouterError::ReadOnlyMiss { .. })
        ));
    }

    #[test]
    fn counter_kiosk_only_permits_siglip2_and_nano08b() {
        assert_eq!(ensure_role_allows_model(DeviceRole::CounterKiosk, ModelId::Siglip2), Ok(()));
        assert_eq!(
            ensure_role_allows_model(DeviceRole::CounterKiosk, ModelId::Qwen35(QwenTier::Nano08B)),
            Ok(())
        );
        assert_eq!(
            ensure_role_allows_model(DeviceRole::CounterKiosk, ModelId::Qwen35(QwenTier::Base2B)),
            Err(VaultError::KioskForbidden)
        );
        assert_eq!(
            ensure_role_allows_model(DeviceRole::CounterKiosk, ModelId::Qwen35(QwenTier::Heavy4B)),
            Err(VaultError::KioskForbidden)
        );
        assert_eq!(
            ensure_role_allows_model(DeviceRole::CounterKiosk, ModelId::Gemma4),
            Err(VaultError::KioskForbidden)
        );

        let res = lv_core::residency::Residency::shared(Device::Cpu, 2048, ResidencyMode::StrictSwap);
        let router = ResidencyRouter::new(res);
        let ph = Phase::new("lang", [Group::blocks(0..1)]);
        assert!(matches!(
            router.acquire_for_role(
                DeviceRole::CounterKiosk,
                CacheAccessMode::ReadWrite,
                ModelId::Qwen35(QwenTier::Heavy4B),
                &ph,
                None,
                |g| Ok(Arc::new(Loaded::empty(g, 100))),
            ),
            Err(RouterError::KioskForbidden { .. })
        ));
        assert!(router
            .acquire_for_role(
                DeviceRole::CounterKiosk,
                CacheAccessMode::ReadWrite,
                ModelId::Qwen35(QwenTier::Nano08B),
                &ph,
                None,
                |g| Ok(Arc::new(Loaded::empty(g, 100))),
            )
            .is_ok());
    }

    #[test]
    fn readonly_miss_never_evicts_resident_slot() {
        let res = lv_core::residency::Residency::shared(Device::Cpu, 2048, ResidencyMode::StrictSwap);
        let router = ResidencyRouter::new(Arc::clone(&res));
        let vis = Phase::new("ov", [Group::Vision]);
        let h1 = router
            .acquire(CacheAccessMode::ReadWrite, ModelId::Siglip2, &vis, None, |g| {
                Ok(Arc::new(Loaded::empty(g, 512)))
            })
            .unwrap();
        assert_eq!(h1.len(), 1);
        drop(h1);

        let heavy = Phase::new("heavy", [Group::Text]);
        assert!(matches!(
            router.acquire(
                CacheAccessMode::ReadOnly,
                ModelId::Qwen35(QwenTier::Heavy4B),
                &heavy,
                None,
                |_| unreachable!(),
            ),
            Err(RouterError::ReadOnlyMiss { .. })
        ));
        assert!(router
            .acquire(CacheAccessMode::ReadOnly, ModelId::Siglip2, &vis, None, |_| unreachable!())
            .is_ok());
        let rel_count = res
            .lock()
            .unwrap()
            .transitions()
            .iter()
            .filter(|t| t.action == lv_core::residency::Action::Release)
            .count();
        assert_eq!(rel_count, 0);
    }
}
