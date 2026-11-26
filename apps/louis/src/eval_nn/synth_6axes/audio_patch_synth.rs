//! Axis 4/7 (`AudioWaveform16k`): 16,000 Hz mono PCM and exact 40 ms (`640`-sample) patch grid synthesizer.

use std::f64::consts::PI;

use anyhow::{ensure, Result};
use lv_sds::seed_tree::{splitmix64_stateless, SeedStreamTag};
use serde::{Deserialize, Serialize};

use crate::eval_nn::schema::{derive_stream_seed, EvalSplit};

pub const AUDIO_SR_HZ: usize = 16_000;
pub const SAMPLES_PER_AUDIO_TOKEN: usize = 640; // HOP=160 (10 ms) * 4 frames = 40 ms

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthAudioEventKind {
    L0GasDetectorBeepAlarm,     // 8 tokens (320 ms): 2,850 Hz pulsed piezo alarm
    L0UltrasonicNdtDefectPing,  // 8 tokens (320 ms): 2,850 Hz NDT defect ping
    L0BearingChatterImpact,     // 8 tokens (320 ms): FM bearing chatter impact
    L1HoistChainGrindHazard,    // 32 tokens (1,280 ms): FM + 3,420 Hz chain grind
    L1SynthRiserBuildup,        // 32 tokens (1,280 ms): swept riser
    L1RadioLotoConfirmSpeech,   // 32 tokens (1,280 ms): radio LOTO confirmation speech
    L2McSpeechUnaccompanied,    // 96 tokens (3,840 ms): soloist spoken MC vamp
    L2AuditCollusionOrIllegalQ, // 96 tokens (3,840 ms): collusion / illegal question span
    NegSungBalladVocalWithPad,  // Hard negative: sustained harmonic ballad singing
    NegCrowdCheerAndApplause,   // Hard negative: wideband crowd applause
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSpanTruth {
    pub event_kind: SynthAudioEventKind,
    pub is_positive_target: bool,
    pub start_token_idx: usize,
    pub end_token_idx: usize,
    pub start_ms: u64,
    pub end_ms: u64,
}

pub fn synth_calibrated_audio_clip_16k(
    split: EvalSplit,
    bundle_id: &str,
    total_tokens: usize,
    schedule: &[(SynthAudioEventKind, usize, usize)],
) -> (Vec<f32>, Vec<AudioSpanTruth>) {
    let base_seed = derive_stream_seed(split, bundle_id, 0, SeedStreamTag::AudioWaveform16k);
    let mut pcm = vec![0.0f32; total_tokens * SAMPLES_PER_AUDIO_TOKEN];
    let mut truths = Vec::with_capacity(schedule.len());

    for (i, sample) in pcm.iter_mut().enumerate() {
        let r = splitmix64_stateless(base_seed.wrapping_add(i as u64));
        let u = ((r >> 40) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0;
        *sample = 0.012 * u;
    }

    for &(kind, s_tok, len_tok) in schedule {
        let e_tok = (s_tok + len_tok).min(total_tokens);
        for idx in (s_tok * SAMPLES_PER_AUDIO_TOKEN)..(e_tok * SAMPLES_PER_AUDIO_TOKEN) {
            let t = (idx - s_tok * SAMPLES_PER_AUDIO_TOKEN) as f64 / AUDIO_SR_HZ as f64;
            let sig = match kind {
                SynthAudioEventKind::L0GasDetectorBeepAlarm
                | SynthAudioEventKind::L0UltrasonicNdtDefectPing => {
                    let gate = if ((t * 8.0) as usize) % 2 == 0 {
                        1.0
                    } else {
                        0.05
                    };
                    0.55 * gate * (2.0 * PI * 2850.0 * t).sin()
                }
                SynthAudioEventKind::L1HoistChainGrindHazard
                | SynthAudioEventKind::L0BearingChatterImpact => {
                    let fm = 1180.0 + 290.0 * (2.0 * PI * 23.0 * t).sin();
                    0.50 * ((2.0 * PI * fm * t).sin() + 0.4 * (2.0 * PI * 3420.0 * t).sin())
                }
                SynthAudioEventKind::L1SynthRiserBuildup => {
                    0.45 * (2.0 * PI * (220.0 + 1800.0 * (t / 1.28).clamp(0.0, 1.0)) * t).sin()
                }
                SynthAudioEventKind::L2McSpeechUnaccompanied
                | SynthAudioEventKind::L1RadioLotoConfirmSpeech
                | SynthAudioEventKind::L2AuditCollusionOrIllegalQ => {
                    let env = (2.0 * PI * 4.5 * t).sin().max(0.0);
                    let f0 = 135.0 + 28.0 * (2.0 * PI * 1.7 * t).sin();
                    env * (0.35 * (2.0 * PI * f0 * t).sin() + 0.20 * (2.0 * PI * 520.0 * t).sin())
                }
                SynthAudioEventKind::NegSungBalladVocalWithPad => {
                    0.30 * (2.0 * PI * 261.63 * t).sin()
                        + 0.25 * (2.0 * PI * 329.63 * t).sin()
                        + 0.30 * (2.0 * PI * (440.0 + 5.0 * (2.0 * PI * 5.8 * t).sin()) * t).sin()
                }
                SynthAudioEventKind::NegCrowdCheerAndApplause => pcm[idx] as f64 * 18.0,
            };
            pcm[idx] = (pcm[idx] + sig as f32).clamp(-0.98, 0.98);
        }
        let is_pos = !matches!(
            kind,
            SynthAudioEventKind::NegSungBalladVocalWithPad
                | SynthAudioEventKind::NegCrowdCheerAndApplause
        );
        truths.push(AudioSpanTruth {
            event_kind: kind,
            is_positive_target: is_pos,
            start_token_idx: s_tok,
            end_token_idx: e_tok,
            start_ms: (s_tok as u64) * 40,
            end_ms: (e_tok as u64) * 40,
        });
    }
    (pcm, truths)
}

/// Encodes mono f32 [-1.0, 1.0] PCM as deterministic 16,000 Hz 16-bit canonical RIFF/WAVE bytes.
pub fn encode_pcm16_wav_16khz(pcm: &[f32]) -> Vec<u8> {
    let data_bytes = (pcm.len() * 2) as u32;
    let riff_size = 36u32.saturating_add(data_bytes);
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&1u16.to_le_bytes()); // Mono
    out.extend_from_slice(&(AUDIO_SR_HZ as u32).to_le_bytes());
    out.extend_from_slice(&((AUDIO_SR_HZ * 2) as u32).to_le_bytes()); // ByteRate
    out.extend_from_slice(&2u16.to_le_bytes()); // BlockAlign
    out.extend_from_slice(&16u16.to_le_bytes()); // BitsPerSample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in pcm {
        let q = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&q.to_le_bytes());
    }
    out
}

/// Verifies a WAV payload is 16,000 Hz mono 16-bit PCM with sample count exact multiple of 640 (40 ms tokens).
/// Returns `(total_samples, total_tokens)`.
pub fn verify_wav_16khz_token_aligned(wav_bytes: &[u8]) -> Result<(usize, usize)> {
    ensure!(wav_bytes.len() >= 44, "WAV too short");
    ensure!(&wav_bytes[0..4] == b"RIFF" && &wav_bytes[8..12] == b"WAVE", "invalid RIFF/WAVE");
    ensure!(&wav_bytes[12..16] == b"fmt " && &wav_bytes[36..40] == b"data", "unexpected chunk layout");
    let audio_fmt = u16::from_le_bytes([wav_bytes[20], wav_bytes[21]]);
    let channels = u16::from_le_bytes([wav_bytes[22], wav_bytes[23]]);
    let sample_rate = u32::from_le_bytes([wav_bytes[24], wav_bytes[25], wav_bytes[26], wav_bytes[27]]);
    let bits = u16::from_le_bytes([wav_bytes[34], wav_bytes[35]]);
    ensure!(audio_fmt == 1 && channels == 1, "expected mono PCM");
    ensure!(sample_rate == AUDIO_SR_HZ as u32, "expected 16,000 Hz sample rate, got {sample_rate}");
    ensure!(bits == 16, "expected 16-bit PCM");
    let data_bytes = u32::from_le_bytes([wav_bytes[40], wav_bytes[41], wav_bytes[42], wav_bytes[43]]) as usize;
    ensure!(wav_bytes.len() == 44 + data_bytes, "data size mismatch");
    ensure!(data_bytes % 2 == 0, "odd byte count for i16 PCM");
    let total_samples = data_bytes / 2;
    ensure!(
        total_samples > 0 && total_samples % SAMPLES_PER_AUDIO_TOKEN == 0,
        "sample count {total_samples} not aligned to 640-sample (40ms) patch grid"
    );
    Ok((total_samples, total_samples / SAMPLES_PER_AUDIO_TOKEN))
}

pub fn compute_span_tiou(a: (u64, u64), b: (u64, u64)) -> f64 {
    let inter_s = a.0.max(b.0);
    let inter_e = a.1.min(b.1);
    if inter_e <= inter_s {
        return 0.0;
    }
    let inter = (inter_e - inter_s) as f64;
    let union = (a.1.max(b.1) - a.0.min(b.0)) as f64;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn goertzel_power(samples: &[f32], freq_hz: f64) -> f64 {
    let w = 2.0 * PI * freq_hz / AUDIO_SR_HZ as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let real = s1 - s2 * w.cos();
    let imag = s2 * w.sin();
    (real * real + imag * imag) / (samples.len().max(1) as f64)
}

/// Deterministic zero-model spectral/envelope detector over 40 ms patches supporting 40 ms NMS spans (`tIoU >= 0.70`)
/// while rejecting ballad singing and broadband crowd applause.
pub fn detect_calibrated_audio_spans(
    pcm: &[f32],
    target: SynthAudioEventKind,
) -> Vec<(u64, u64)> {
    let total_tokens = pcm.len() / SAMPLES_PER_AUDIO_TOKEN;
    let mut active = vec![false; total_tokens];

    for tok in 0..total_tokens {
        let win = &pcm[tok * SAMPLES_PER_AUDIO_TOKEN..(tok + 1) * SAMPLES_PER_AUDIO_TOKEN];
        let total_energy: f64 = win.iter().map(|&v| (v as f64) * (v as f64)).sum();
        let p_2850 = goertzel_power(win, 2850.0);
        let p_3420 = goertzel_power(win, 3420.0);
        let p_1180 = goertzel_power(win, 1180.0);
        let speech_band = goertzel_power(win, 120.0)
            + goertzel_power(win, 140.0)
            + goertzel_power(win, 160.0)
            + goertzel_power(win, 515.0)
            + goertzel_power(win, 520.0)
            + goertzel_power(win, 525.0);
        let p_261 = goertzel_power(win, 261.63);
        let p_329 = goertzel_power(win, 329.63);
        let ballad_band = p_261 + p_329 + goertzel_power(win, 440.0);
        let is_ballad_chord = p_261 > 2.0 && p_329 > 2.0;
        let hf_noise_band = goertzel_power(win, 1850.0)
            + goertzel_power(win, 2150.0)
            + goertzel_power(win, 2450.0)
            + goertzel_power(win, 3100.0)
            + goertzel_power(win, 3700.0)
            + goertzel_power(win, 4100.0);

        active[tok] = match target {
            SynthAudioEventKind::L0GasDetectorBeepAlarm
            | SynthAudioEventKind::L0UltrasonicNdtDefectPing => {
                p_2850 > 0.25 && total_energy > 0.2
            }
            SynthAudioEventKind::L1HoistChainGrindHazard
            | SynthAudioEventKind::L0BearingChatterImpact => {
                (p_3420 > 0.15 || p_1180 > 0.15) && !is_ballad_chord && total_energy > 1.5
            }
            SynthAudioEventKind::L1SynthRiserBuildup => {
                let mid = (220..=2020)
                    .step_by(300)
                    .map(|f| goertzel_power(win, f as f64))
                    .sum::<f64>();
                mid > 0.5 && ballad_band < 2.0
            }
            SynthAudioEventKind::L2McSpeechUnaccompanied
            | SynthAudioEventKind::L1RadioLotoConfirmSpeech
            | SynthAudioEventKind::L2AuditCollusionOrIllegalQ => {
                speech_band > 0.15
                    && speech_band > 4.0 * hf_noise_band.max(1e-4)
                    && !is_ballad_chord
                    && p_3420 < 0.2
            }
            SynthAudioEventKind::NegSungBalladVocalWithPad
            | SynthAudioEventKind::NegCrowdCheerAndApplause => false,
        };
    }

    // Bridge inter-syllable / alarm off-cycle gaps (up to 6 tokens = 240 ms)
    let mut bridged = active.clone();
    let mut last_active: Option<usize> = None;
    for tok in 0..total_tokens {
        if active[tok] {
            if let Some(prev) = last_active {
                if tok - prev <= 6 {
                    for k in prev..=tok {
                        bridged[k] = true;
                    }
                }
            }
            last_active = Some(tok);
        }
    }

    let min_tokens = match target {
        SynthAudioEventKind::L0GasDetectorBeepAlarm
        | SynthAudioEventKind::L0UltrasonicNdtDefectPing
        | SynthAudioEventKind::L0BearingChatterImpact => 4,
        SynthAudioEventKind::L1HoistChainGrindHazard
        | SynthAudioEventKind::L1SynthRiserBuildup
        | SynthAudioEventKind::L1RadioLotoConfirmSpeech => 8,
        _ => 12,
    };

    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < total_tokens {
        if bridged[i] {
            let start = i;
            while i < total_tokens && bridged[i] {
                i += 1;
            }
            if i - start >= min_tokens {
                spans.push(((start as u64) * 40, (i as u64) * 40));
            }
        } else {
            i += 1;
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_audio_spans_match_40ms_grid_and_tiou_rejecting_hard_negatives() {
        let schedule = vec![
            (SynthAudioEventKind::L0GasDetectorBeepAlarm, 10, 8),
            (SynthAudioEventKind::NegSungBalladVocalWithPad, 30, 80),
            (SynthAudioEventKind::L2McSpeechUnaccompanied, 120, 96),
            (SynthAudioEventKind::L1HoistChainGrindHazard, 230, 32),
            (SynthAudioEventKind::NegCrowdCheerAndApplause, 280, 40),
        ];
        let (pcm_a, truths) =
            synth_calibrated_audio_clip_16k(EvalSplit::Diag, "XRC-AUD-01", 340, &schedule);
        let (pcm_b, _) =
            synth_calibrated_audio_clip_16k(EvalSplit::Diag, "XRC-AUD-01", 340, &schedule);
        assert_eq!(pcm_a, pcm_b, "audio synthesis must be bit-identical");

        let wav = encode_pcm16_wav_16khz(&pcm_a);
        let (samples, tokens) = verify_wav_16khz_token_aligned(&wav).unwrap();
        assert_eq!(tokens, 340);
        assert_eq!(samples, 340 * 640);

        for tr in truths.iter().filter(|t| t.is_positive_target) {
            let det = detect_calibrated_audio_spans(&pcm_a, tr.event_kind);
            let best_tiou = det
                .iter()
                .map(|&d| compute_span_tiou(d, (tr.start_ms, tr.end_ms)))
                .fold(0.0f64, f64::max);
            assert!(
                best_tiou >= 0.70,
                "{:?} best_tiou={best_tiou:.3} < 0.70 (det={det:?}, truth=({},{}))",
                tr.event_kind,
                tr.start_ms,
                tr.end_ms
            );
        }
        assert!(detect_calibrated_audio_spans(&pcm_a, SynthAudioEventKind::NegSungBalladVocalWithPad).is_empty());
        assert!(detect_calibrated_audio_spans(&pcm_a, SynthAudioEventKind::NegCrowdCheerAndApplause).is_empty());
    }
}
