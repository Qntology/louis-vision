//! Phase D6 (`XRC_B01..XRC_B08`): Coupled XR Concert Production Twin derived deterministically
//! from a single master `twin_seed: u64` (`apps/louis` docs <-> `apps/vision` XR Stage Live).

use std::net::Ipv4Addr;

use anyhow::{ensure, Result};
use lv_sds::blockies_haring::{derive_blockies_haring_spec, BlockiesHaringSpec};
use lv_sds::seed_tree::{splitmix64_stateless, SeedStreamTag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::eval_nn::domains::dom09_warehouse_hazmat::compute_iso6346_check_digit;
use crate::eval_nn::schema::{derive_deterministic_wallet_key32, derive_stream_seed, EvalSplit};
use crate::eval_nn::synth_6axes::audio_patch_synth::{
    compute_span_tiou, detect_calibrated_audio_spans, encode_pcm16_wav_16khz,
    synth_calibrated_audio_clip_16k, verify_wav_16khz_token_aligned, AudioSpanTruth,
    SynthAudioEventKind,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrcB01StageStruct {
    pub pe_seal_verified: bool,
    pub center_lift_max_payload_kg: u32,
    pub center_lift_stroke_m: f32,
    pub obb_lift_c_extents_m: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrcB02RiggingA0Bom {
    pub mothergrid_elev_m: f32,
    pub zzs1_servo_hoist_count: u32,
    pub active_bom_hoist_count: u32,
    pub aluminum_grade: String,
    pub load_permitted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrcB03PyroLaserPtw {
    pub wind_gust_mps: f32,
    pub continuous_rest_hours: f32,
    pub pyro_2stage_armed_allowed: bool,
    pub laser_bam_frustum_pct: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrcB04TourCustomsDg {
    pub ata_carnet_id: String,
    pub container_code_11: String,
    pub un_codes: Vec<String>,
    pub carnet_blocks_un0431_consumables: bool,
    pub wagon_grip_force_scale: f32,
    pub wagon_accel_cap_scale: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrcB05MdSrmSettle {
    pub payout_days: i64,
    pub promo_burden_bps: u32,
    pub friction_trap_clean: bool,
    pub crowd_lightstick_universe: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrcB06SacnSmpteCue {
    pub pyro_isolated_universe: u16,
    pub pyro_non_dimmable: bool,
    pub channels_per_universe: u16,
    pub smpte_fps: u8,
    pub show_lan_ip: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XrcB07RehearsalAudio {
    pub wav_sha256_hex: String,
    pub total_tokens_40ms: usize,
    pub span_truths: Vec<AudioSpanTruth>,
    pub vamp_hold_tiou: f64,
    pub halt_interlock_tiou: f64,
    pub ballad_false_triggers: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrewHaringActorSpec {
    pub actor_slot: String,
    pub is_ai_dancer: bool,
    pub wallet_0x: String,
    pub blockies: BlockiesHaringSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConcertTwinManifest {
    pub bundle_id: String,
    pub twin_seed: u64,
    pub b01_stage_struct: XrcB01StageStruct,
    pub b02_rigging_a0_bom: XrcB02RiggingA0Bom,
    pub b03_pyro_laser_ptw: XrcB03PyroLaserPtw,
    pub b04_tour_customs_dg: XrcB04TourCustomsDg,
    pub b05_md_srm_settle: XrcB05MdSrmSettle,
    pub b06_sacn_smpte_cue: XrcB06SacnSmpteCue,
    pub b07_rehearsal_audio: XrcB07RehearsalAudio,
    pub b08_crew_haring_rig: Vec<CrewHaringActorSpec>,
}

/// Validates RFC1918 private LAN IPv4 addresses for XR stage sACN / WebRTC transport (blocks public WAN IPs).
pub fn is_valid_stage_rfc1918_ip(raw_ip: &str) -> bool {
    let Ok(ip) = raw_ip.trim().parse::<Ipv4Addr>() else {
        return false;
    };
    let [a, b, _, _] = ip.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}

/// Validates sACN E1.31 pyro universe isolation: pyro cues MUST occupy Universe 64 exclusively and non-dimmable.
pub fn validate_pyro_sacn_universe(universe: u16, non_dimmable: bool, ip: &str) -> bool {
    universe == 64 && non_dimmable && is_valid_stage_rfc1918_ip(ip)
}

pub fn wallet_0x_from_key32(key32: &[u8; 32]) -> String {
    let mut s = String::from("0x");
    for &b in &key32[..20] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn derive_coupled_concert_twin(
    split: EvalSplit,
    twin_idx: u32,
) -> Result<ConcertTwinManifest> {
    let bundle_id = format!("XRC-TWIN-{twin_idx:03}");
    let twin_seed = derive_stream_seed(split, &bundle_id, twin_idx, SeedStreamTag::CoupledTwinStage);
    let r1 = splitmix64_stateless(twin_seed ^ 0x1111_2222_3333_4444);
    let container_suffix = 113_466u32; // guarantees MSCU113466 -> rem == 10 -> '0'
    let prefix = format!("MSCU{container_suffix:06}");
    let cd = compute_iso6346_check_digit(&prefix).expect("valid MSCU113466");
    let container_code_11 = format!("{prefix}{cd}");

    let wind_gust_mps = 4.5 + ((r1 & 0xFF) as f32 / 255.0) * 4.5; // < 10.0 m/s
    let lan_host = ((r1 >> 8) % 200) as u8 + 10;
    let show_lan_ip = format!("192.168.50.{lan_host}");

    let schedule = [
        (SynthAudioEventKind::NegSungBalladVocalWithPad, 8, 80),
        (SynthAudioEventKind::L2McSpeechUnaccompanied, 100, 96),
        (SynthAudioEventKind::L1HoistChainGrindHazard, 212, 32),
        (SynthAudioEventKind::NegCrowdCheerAndApplause, 260, 40),
    ];
    let (pcm, span_truths) = synth_calibrated_audio_clip_16k(split, &bundle_id, 320, &schedule);
    let wav = encode_pcm16_wav_16khz(&pcm);
    let (_, total_tokens_40ms) = verify_wav_16khz_token_aligned(&wav)?;
    let wav_sha256_hex: String = Sha256::digest(&wav)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let vamp_det = detect_calibrated_audio_spans(&pcm, SynthAudioEventKind::L2McSpeechUnaccompanied);
    let vamp_truth = &span_truths[1];
    let vamp_hold_tiou = vamp_det
        .iter()
        .map(|&d| compute_span_tiou(d, (vamp_truth.start_ms, vamp_truth.end_ms)))
        .fold(0.0f64, f64::max);

    let halt_det = detect_calibrated_audio_spans(&pcm, SynthAudioEventKind::L1HoistChainGrindHazard);
    let halt_truth = &span_truths[2];
    let halt_interlock_tiou = halt_det
        .iter()
        .map(|&d| compute_span_tiou(d, (halt_truth.start_ms, halt_truth.end_ms)))
        .fold(0.0f64, f64::max);

    let ballad_false_triggers =
        detect_calibrated_audio_spans(&pcm, SynthAudioEventKind::NegSungBalladVocalWithPad).len();

    let actor_slots = [
        ("CREW-DIRECTOR-01", false),
        ("CREW-RIGGING-02", false),
        ("CREW-PYRO-03", false),
        ("HARING-DANCER-04", true),
        ("HARING-DANCER-05", true),
    ];
    let mut b08_crew_haring_rig = Vec::with_capacity(actor_slots.len());
    for &(slot, is_ai) in &actor_slots {
        let key32 = derive_deterministic_wallet_key32(split, &format!("{bundle_id}:{slot}"));
        let wallet_0x = wallet_0x_from_key32(&key32);
        let blockies = derive_blockies_haring_spec(&wallet_0x);
        ensure!(blockies.outline_hex == "#000000");
        ensure!(blockies.haring_body_hsl.s_pct >= 65.0);
        b08_crew_haring_rig.push(CrewHaringActorSpec {
            actor_slot: slot.into(),
            is_ai_dancer: is_ai,
            wallet_0x,
            blockies,
        });
    }

    Ok(ConcertTwinManifest {
        bundle_id,
        twin_seed,
        b01_stage_struct: XrcB01StageStruct {
            pe_seal_verified: true,
            center_lift_max_payload_kg: 500,
            center_lift_stroke_m: 2.50,
            obb_lift_c_extents_m: [2.40, 2.40, 2.50],
        },
        b02_rigging_a0_bom: XrcB02RiggingA0Bom {
            mothergrid_elev_m: 14.0,
            zzs1_servo_hoist_count: 16,
            active_bom_hoist_count: 16,
            aluminum_grade: "EN AW-6082 T6".into(),
            load_permitted: true,
        },
        b03_pyro_laser_ptw: XrcB03PyroLaserPtw {
            wind_gust_mps,
            continuous_rest_hours: 11.5,
            pyro_2stage_armed_allowed: true,
            laser_bam_frustum_pct: 0.0,
        },
        b04_tour_customs_dg: XrcB04TourCustomsDg {
            ata_carnet_id: format!("KR-ATA-2026-{twin_idx:04}"),
            container_code_11,
            un_codes: vec!["UN 1013".into()],
            carnet_blocks_un0431_consumables: true,
            wagon_grip_force_scale: 0.65,
            wagon_accel_cap_scale: 0.55,
        },
        b05_md_srm_settle: XrcB05MdSrmSettle {
            payout_days: 18,
            promo_burden_bps: 4_500,
            friction_trap_clean: true,
            crowd_lightstick_universe: 120,
        },
        b06_sacn_smpte_cue: XrcB06SacnSmpteCue {
            pyro_isolated_universe: 64,
            pyro_non_dimmable: true,
            channels_per_universe: 512,
            smpte_fps: 60,
            show_lan_ip,
        },
        b07_rehearsal_audio: XrcB07RehearsalAudio {
            wav_sha256_hex,
            total_tokens_40ms,
            span_truths,
            vamp_hold_tiou,
            halt_interlock_tiou,
            ballad_false_triggers,
        },
        b08_crew_haring_rig,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coupled_concert_twin_derives_all_8_bundles_deterministically_with_rfc1918_pyro_isolation() {
        let a = derive_coupled_concert_twin(EvalSplit::Diag, 0).unwrap();
        let b = derive_coupled_concert_twin(EvalSplit::Diag, 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.b04_tour_customs_dg.container_code_11, "MSCU1134660");
        assert!(a.b07_rehearsal_audio.vamp_hold_tiou >= 0.70);
        assert!(a.b07_rehearsal_audio.halt_interlock_tiou >= 0.70);
        assert_eq!(a.b07_rehearsal_audio.ballad_false_triggers, 0);
        assert_eq!(a.b08_crew_haring_rig.len(), 5);
        assert!(validate_pyro_sacn_universe(64, true, &a.b06_sacn_smpte_cue.show_lan_ip));
        assert!(!validate_pyro_sacn_universe(63, true, &a.b06_sacn_smpte_cue.show_lan_ip));
        assert!(!validate_pyro_sacn_universe(64, true, "8.8.8.8"));
    }
}
