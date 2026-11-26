//! Pure Tier-0/1 deterministic seed tree (`SplitMix64` + `FNV-1a`), strictly thread-order
//! and OS-independent with zero evaluation/holdout constant literals.

pub const FIXED_EVAL_NOW_MS: i64 = 1_795_737_600_000;
pub const DEFAULT_DIAG_ROOT_SEED: u64 = 2026_0501;
pub const SEALED_PARTITION_XOR_MASK: u64 = 0xA5A5_5A5A_F0F0_0F0F;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EvalSplit {
    Diag = 0x10,
    Holdout = 0xEE,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SeedStreamTag {
    DocAstAndAmounts = 0x01,
    HostileMutation = 0x02,
    StereoCameraPose = 0x03,
    AudioWaveform16k = 0x04,
    WalletSecp256k1 = 0x05,
    QueryPermutation = 0x06,
    CoupledTwinStage = 0x07,
}

impl SeedStreamTag {
    pub const ALL: [Self; 7] = [
        Self::DocAstAndAmounts,
        Self::HostileMutation,
        Self::StereoCameraPose,
        Self::AudioWaveform16k,
        Self::WalletSecp256k1,
        Self::QueryPermutation,
        Self::CoupledTwinStage,
    ];
}

#[inline]
pub const fn splitmix64_stateless(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
pub const fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

pub fn derive_stream_seed_with_root(
    root_seed: u64,
    split: EvalSplit,
    domain_or_track: &str,
    bundle_idx: u32,
    tag: SeedStreamTag,
) -> u64 {
    let root = match split {
        EvalSplit::Diag => root_seed,
        EvalSplit::Holdout => root_seed ^ SEALED_PARTITION_XOR_MASK,
    };
    let dom_hash = fnv1a_64(domain_or_track.as_bytes());
    let packed = ((split as u64) << 56) | ((tag as u64) << 48) | (bundle_idx as u64);
    splitmix64_stateless(root ^ dom_hash.rotate_left(17) ^ splitmix64_stateless(packed))
}

pub fn derive_deterministic_wallet_key32_with_root(
    root_seed: u64,
    split: EvalSplit,
    actor_slot: &str,
) -> [u8; 32] {
    let base = derive_stream_seed_with_root(root_seed, split, actor_slot, 0, SeedStreamTag::WalletSecp256k1);
    let mut out = [0u8; 32];
    for chunk_idx in 0..4u64 {
        let word = splitmix64_stateless(base ^ chunk_idx.wrapping_mul(0x517C_C1B7_2722_0A95));
        out[(chunk_idx as usize) * 8..(chunk_idx as usize + 1) * 8]
            .copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn one_million_stream_seeds_have_zero_diag_vs_sealed_intersection() {
        let domains = ["DOM-01", "DOM-08", "DOM-11", "DOM-15", "XRC-TWIN"];
        let mut diag_set = HashSet::with_capacity(500_000);
        let alt_root = 0x2026_0502_u64;

        for idx in 0..500_000u32 {
            let dom = domains[(idx as usize) % domains.len()];
            let tag = SeedStreamTag::ALL[(idx as usize) % SeedStreamTag::ALL.len()];
            let s = derive_stream_seed_with_root(DEFAULT_DIAG_ROOT_SEED, EvalSplit::Diag, dom, idx, tag);
            assert!(diag_set.insert(s), "collision inside Diag partition at {idx}");
        }
        for idx in 0..500_000u32 {
            let dom = domains[(idx as usize) % domains.len()];
            let tag = SeedStreamTag::ALL[(idx as usize) % SeedStreamTag::ALL.len()];
            let s = derive_stream_seed_with_root(alt_root, EvalSplit::Holdout, dom, idx, tag);
            assert!(
                !diag_set.contains(&s),
                "Diag vs Sealed seed intersection found at {idx}"
            );
        }
        assert_eq!(diag_set.len(), 500_000);
    }

    #[test]
    fn multithreaded_reverse_order_derivation_is_bit_identical() {
        let expected: Vec<u64> = (0..512u32)
            .map(|i| derive_stream_seed_with_root(DEFAULT_DIAG_ROOT_SEED, EvalSplit::Diag, "DOM-09", i, SeedStreamTag::StereoCameraPose))
            .collect();

        let handles: Vec<_> = (0..4usize)
            .map(|tid| {
                let exp_clone = expected.clone();
                std::thread::spawn(move || {
                    for i in (0..512u32).rev() {
                        if (i as usize) % 4 == tid {
                            let got = derive_stream_seed_with_root(
                                DEFAULT_DIAG_ROOT_SEED,
                                EvalSplit::Diag,
                                "DOM-09",
                                i,
                                SeedStreamTag::StereoCameraPose,
                            );
                            assert_eq!(got, exp_clone[i as usize]);
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let k1 = derive_deterministic_wallet_key32_with_root(DEFAULT_DIAG_ROOT_SEED, EvalSplit::Diag, "officer");
        let k2 = derive_deterministic_wallet_key32_with_root(DEFAULT_DIAG_ROOT_SEED, EvalSplit::Diag, "officer");
        let k3 = derive_deterministic_wallet_key32_with_root(DEFAULT_DIAG_ROOT_SEED, EvalSplit::Diag, "submitter");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }
}
