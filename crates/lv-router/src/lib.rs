//! Layer 3 offline cryptographic vault and model residency router for `louis-vision`.

pub mod residency_router;
pub mod vault;

pub use residency_router::{CacheAccessMode, ResidencyRouter, RouteTarget, RouterError};
pub use vault::{
    DeviceRole, ProtectedKey, VaultController, VaultError, VaultState, RECOVERY_WORDS,
};
