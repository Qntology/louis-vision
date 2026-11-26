//! Click-through overlay controller wiring `premium-discount-window` (`WDA_EXCLUDEFROMCAPTURE`),
//! `lv-screen` (`LogLumSensor`, `EpochWatch`, NaFlex redaction, `REPRIME_THRESHOLD_MILLI`), and
//! `premium-discount-nav` (`evaluate_privacy`, `PrivacyStateMachine`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lv_screen::{
    redact_rgb_for_naflex, EpochWatch, EventDiffState, LogLumSensor, Preprocess, PurgableSensor,
    REPRIME_THRESHOLD_MILLI,
};
use premium_discount_nav::{
    evaluate_privacy, PrivacyPhase, PrivacyProbeInput, PrivacyShield, PrivacyStateMachine, Rect,
    Region, ResumeStepOutcome, SurfaceSignature, ZeroPixelProbe,
};
use premium_discount_window::{
    assess_surfaces, ex_style, LayerFlags, PxRect, VisibleWindowEntry, HOVER_ALPHA,
    WDA_EXCLUDEFROMCAPTURE,
};
use zeroize::Zeroize;

/// Result of evaluating one desktop capture tick for the overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayTick {
    pub capture_allowed: bool,
    pub shield: PrivacyShield,
    pub masked_cells: Vec<u32>,
    pub event_count: u32,
    pub threshold_milli: u32,
    pub ex_style_bits: u32,
    pub alpha: u8,
    pub epoch: u64,
    pub outcome: ResumeStepOutcome,
}

pub struct OverlayController {
    flags: LayerFlags,
    user_alpha: u8,
    sensor: LogLumSensor,
    diff_state: EventDiffState,
    machine: PrivacyStateMachine,
    privacy_epoch: Arc<AtomicU64>,
    pre: Preprocess,
    max_patches: usize,
}

impl OverlayController {
    pub fn new(width: u32, height: u32, threshold: f32) -> Self {
        Self {
            flags: LayerFlags { click_through: true, topmost: true, typing: false },
            user_alpha: 209,
            sensor: LogLumSensor::new(width as usize, height as usize, threshold),
            diff_state: EventDiffState::new(width, height, threshold),
            machine: PrivacyStateMachine::new(),
            privacy_epoch: Arc::new(AtomicU64::new(0)),
            pre: Preprocess::SIGLIP2,
            max_patches: 64,
        }
    }

    pub fn self_display_affinity(&self) -> u32 {
        WDA_EXCLUDEFROMCAPTURE
    }

    pub fn window_ex_style(&self) -> u32 {
        ex_style(self.flags)
    }

    pub fn effective_alpha(&self, cursor_over: bool) -> u8 {
        if self.flags.click_through && cursor_over && !self.flags.typing {
            self.user_alpha.min(HOVER_ALPHA)
        } else {
            self.user_alpha
        }
    }

    pub fn set_typing(&mut self, typing: bool) {
        self.flags.typing = typing;
    }

    pub fn privacy_epoch(&self) -> u64 {
        self.privacy_epoch.load(Ordering::Acquire)
    }

    pub fn phase(&self) -> &PrivacyPhase {
        &self.machine.phase
    }

    pub fn watch(&self) -> EpochWatch<'_> {
        EpochWatch::capture(&self.privacy_epoch)
    }

    /// Returns a cloned handle so external capture threads can hold `EpochWatch` across ticks.
    pub fn epoch_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.privacy_epoch)
    }

    /// Processes a capture tick, zeroizing `rgb_buf` and aborting encoding on any lockdown trigger.
    #[allow(clippy::too_many_arguments)]
    pub fn step_frame(
        &mut self,
        now_ms: i64,
        active_cc: &str,
        surface: SurfaceSignature,
        windows: &[VisibleWindowEntry],
        regions: &[Region],
        vision_privacy_hit: bool,
        auth_or_payment_context: bool,
        rgb_buf: &mut [u8],
        bgra_buf: &[u8],
        cursor_over: bool,
    ) -> OverlayTick {
        let capture = PxRect::new(0, 0, self.sensor.width as i32, self.sensor.height as i32);
        let os_verdict = assess_surfaces(windows, capture);
        let toast_zones: Vec<Rect> = os_verdict
            .redact_zones
            .iter()
            .filter_map(|&(x, y, w, h)| Rect::new(x, y, w, h))
            .collect();
        let shield = evaluate_privacy(&PrivacyProbeInput {
            regions,
            os_capture_protected: os_verdict.capture_protected(),
            os_overlay_payment_detected: os_verdict.overlay_payment(),
            os_notification_zones: &toast_zones,
            vision_privacy_hit,
            seed_phrase_detected: false,
            auth_or_payment_context,
        });

        let probe = ZeroPixelProbe {
            now_ms,
            active_cc: active_cc.to_string(),
            surface,
            unoccluded_trigger: shield.trigger(),
            in_payment_flow: auth_or_payment_context,
            receipt_finish_page_hash: None,
        };
        let outcome = self.machine.step(&probe);
        if outcome.privacy_epoch_started || matches!(shield, PrivacyShield::FullLockdown(_)) {
            self.privacy_epoch.fetch_add(1, Ordering::Release);
            self.sensor.purge_privacy_residue();
            self.diff_state.purge_privacy_residue();
            rgb_buf.zeroize();
        }
        if outcome.trigger_sensor_reprime {
            self.diff_state.purge_privacy_residue();
        }

        let params = self.diff_state.next_params();
        let (masked_cells, event_count) = if outcome.capture_and_record_allowed {
            let masked = redact_rgb_for_naflex(
                rgb_buf,
                self.sensor.width,
                self.sensor.height,
                &os_verdict.redact_zones,
                self.max_patches,
                &self.pre,
            );
            let tile_stats = self.sensor.process_bgra(bgra_buf);
            let total_events = if params.is_reprime() {
                0
            } else {
                tile_stats.count.iter().sum()
            };
            (masked, total_events)
        } else {
            rgb_buf.zeroize();
            (Vec::new(), 0)
        };

        OverlayTick {
            capture_allowed: outcome.capture_and_record_allowed,
            shield,
            masked_cells,
            event_count,
            threshold_milli: params.threshold_milli,
            ex_style_bits: self.window_ex_style(),
            alpha: self.effective_alpha(cursor_over),
            epoch: self.privacy_epoch(),
            outcome,
        }
    }

    pub fn notify_external_privacy_hit(&mut self) -> u64 {
        let next = self.privacy_epoch.fetch_add(1, Ordering::Release) + 1;
        self.sensor.purge_privacy_residue();
        self.diff_state.purge_privacy_residue();
        next
    }

    pub fn reprime_sentinel() -> u32 {
        REPRIME_THRESHOLD_MILLI
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lv_screen::VisionAbort;
    use premium_discount_nav::{GRACE_MS, GRACE_TICKS};
    use premium_discount_window::WindowInfo;

    fn sample_window(hwnd: isize, exe: &str, title: &str, affinity: u32) -> VisibleWindowEntry {
        VisibleWindowEntry {
            info: WindowInfo {
                hwnd,
                pid: 99999,
                exe_path: None,
                exe_name: Some(exe.into()),
                title: title.into(),
            },
            class_name: "Chrome_WidgetWin_1".into(),
            rect: PxRect::new(0, 0, 128, 64),
            ex_style: 0,
            display_affinity: affinity,
        }
    }

    #[test]
    fn overlay_constants_hover_alpha_epoch_zeroize_and_grace_reprime() {
        let mut ctrl = OverlayController::new(64, 64, 0.15);
        assert_eq!(ctrl.self_display_affinity(), 0x11);
        assert_eq!(ctrl.window_ex_style(), 0x0808_00A8);
        assert_eq!(ctrl.effective_alpha(true), 76);
        assert_eq!(ctrl.effective_alpha(false), 209);

        let surface_normal = SurfaceSignature::new(10, "hwp.exe");
        let surface_bank = SurfaceSignature::new(20, "bank.exe/auth/login");
        let bgra = vec![120u8; 64 * 64 * 4];
        let mut rgb = vec![120u8; 64 * 64 * 3];

        // Tick 0: initial normal prime.
        let t0 = ctrl.step_frame(1_000, "hwp.exe", surface_normal, &[], &[], false, false, &mut rgb, &bgra, true);
        assert!(t0.capture_allowed);
        assert_eq!(t0.threshold_milli, REPRIME_THRESHOLD_MILLI);
        assert_eq!(t0.event_count, 0);

        // Tick 1: banking / WDA window appears -> immediate epoch abort, buffer zeroize, lockdown.
        let epoch_arc = ctrl.epoch_handle();
        let watch = EpochWatch::capture(&epoch_arc);
        let bank_win = sample_window(20, "ahn.exe", "본인인증 보안키패드", WDA_EXCLUDEFROMCAPTURE);
        let mut rgb_secret = vec![200u8; 64 * 64 * 3];
        let t1 = ctrl.step_frame(1_500, "ahn.exe", surface_bank, &[bank_win], &[], false, true, &mut rgb_secret, &bgra, false);
        assert!(!t1.capture_allowed);
        assert!(matches!(t1.shield, PrivacyShield::FullLockdown(_)));
        assert!(rgb_secret.iter().all(|&b| b == 0), "RGB frame must be zeroized on lockdown");
        assert_eq!(watch.check().unwrap_err(), VisionAbort::PrivacyLocked);

        // Ticks 2..: bank closes; GraceProbe ticks through GRACE_MS + GRACE_TICKS then reprimes cleanly.
        let mut t_cur = 2_000i64;
        let mut unlocked = None;
        for _ in 0..=(GRACE_TICKS as usize + 2) {
            t_cur += (GRACE_MS / GRACE_TICKS as i64) + 50;
            let mut rgb_clean = vec![120u8; 64 * 64 * 3];
            let res = ctrl.step_frame(t_cur, "hwp.exe", surface_normal, &[], &[], false, false, &mut rgb_clean, &bgra, false);
            if res.capture_allowed {
                unlocked = Some(res);
                break;
            }
        }
        let resumed = unlocked.expect("overlay must exit GraceProbe back to Normal");
        assert!(resumed.outcome.trigger_sensor_reprime);
        assert_eq!(resumed.threshold_milli, REPRIME_THRESHOLD_MILLI);
        assert_eq!(resumed.event_count, 0, "reprime frame emits zero spurious events");
    }
}
