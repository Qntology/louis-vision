//! Layer 3 offline cryptographic vault and model residency router for `louis-vision`.

pub mod cross_app_coord;
pub mod residency_router;
pub mod vault;

pub use cross_app_coord::{
    CrossAppCoordinator, CrossAppPrivacySync, GpuLeaseState, MobileSkinKind, DESKTOP_ANCHOR_LOCK,
    STAGE_PINNED_LOUIS_CAP_MB,
};

pub use residency_router::{CacheAccessMode, ResidencyRouter, RouteTarget, RouterError};
pub use vault::{
    DeviceRole, ProtectedKey, VaultController, VaultError, VaultState, RECOVERY_WORDS,
};
