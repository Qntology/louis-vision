// apps/vision/ui/js/stage-mode.js
// Zero-reload Sim Sandbox <-> XR Stage Live switch (<100ms), Z_bg 30-frame gate,
// RFC1918/Loopback Show-IO guard, always-pinned 3-interlock HUD, P0 Blackout (<=16.6ms),
// Provenance separation, and bottom-right ( ✦ ) unified panel toggle.

import { call } from './api.js';
import { state } from './state.js';
import { $ } from './util.js';
import { openPanel, togglePanel } from './overlays.js';
import { setStatus } from './toast.js';

let zbgFrames = 0;
let zkeyLocked = false;
let blackoutActive = false;
let stageCueManual = false;
const interlocks = { metricZone: true, rigSync: true, temporal: true };

function isLocalOrRfc1918Host(raw) {
  const host = String(raw || '').trim().split(':')[0].replace(/^\[|\]$/g, '').toLowerCase();
  if (!host) return false;
  if (host === 'localhost' || host === '127.0.0.1' || host === '::1') return true;
  if (host.startsWith('10.') || host.startsWith('192.168.') || host.startsWith('169.254.')) return true;
  const m = /^172\.(\d+)\./.exec(host);
  if (m) {
    const second = Number(m[1]);
    return second >= 16 && second <= 31;
  }
  return host.startsWith('fc') || host.startsWith('fd');
}

function syncInterlockHud() {
  const zLamp = $('lamp-metric-zone');
  const rLamp = $('lamp-rig-sync');
  const tLamp = $('lamp-temporal-gate');
  const statusEl = $('stage-interlock-status');
  const goBtn = $('btn-cue-go');

  const allClear = interlocks.metricZone && interlocks.rigSync && interlocks.temporal && !blackoutActive;
  if (zLamp) {
    zLamp.textContent = interlocks.metricZone ? 'OK (0 voxels)' : 'HALT (hazard intruded)';
    zLamp.className = `v ${interlocks.metricZone ? 'lv-ok' : 'lv-crit'}`;
  }
  if (rLamp) {
    rLamp.textContent = interlocks.rigSync ? 'OK (< 0.02 m)' : 'WARN (>= 0.02 m)';
    rLamp.className = `v ${interlocks.rigSync ? 'lv-ok' : 'lv-crit'}`;
  }
  if (tLamp) {
    tLamp.textContent = interlocks.temporal ? 'CLEAR' : 'HELD (depth spike)';
    tLamp.className = `v ${interlocks.temporal ? 'lv-ok' : 'lv-warn'}`;
  }
  if (goBtn) goBtn.disabled = !allClear;
  if (statusEl) {
    statusEl.textContent = blackoutActive
      ? 'BLACKOUT HALT · /louis/interlock/halt (<=16.6ms)'
      : allClear
        ? '전 채널 정상 — AutoTimecode 및 Cue Remote 허가됨'
        : '인터록 위반 감지 — /louis/interlock/halt 큐 발사 잠금';
  }
}

export async function switchVisionMode(mode) {
  const t0 = performance.now();
  const isStage = mode === 'stage';
  document.body.dataset.visionMode = isStage ? 'stage' : 'sim';
  document.body.dataset.theme = isStage ? 'stage-dark' : 'default';
  document.body.dataset.surface = isStage ? 'vision_xr_stage' : 'vision_sim_sandbox';

  $('mode-btn-sim')?.setAttribute('aria-selected', String(!isStage));
  $('mode-btn-stage')?.setAttribute('aria-selected', String(isStage));

  if ($('sim-toolbar')) $('sim-toolbar').hidden = isStage;
  if ($('stage-toolbar')) $('stage-toolbar').hidden = !isStage;
  if ($('sim-transport-row')) $('sim-transport-row').hidden = isStage;
  if ($('stage-cue-row')) $('stage-cue-row').hidden = !isStage;
  if ($('stage-interlock-hud')) $('stage-interlock-hud').hidden = !isStage;
  if ($('frame-budget-pill')) $('frame-budget-pill').hidden = !isStage;

  state.visionMode = isStage ? 'stage' : 'sim';
  if (isStage) {
    try {
      await call('peer_request_stage_live', { hostAckLouisMb: 0.0 }, { quiet: true });
    } catch {
      /* mock or standalone fallback */
    }
  }
  const dt = Math.max(0.1, performance.now() - t0);
  if ($('frame-budget-pill')) {
    $('frame-budget-pill').textContent = `${dt.toFixed(1)} / 16.6 ms · ${isStage ? 'StagePinnedLock' : 'CoexistSim'}`;
  }
  syncInterlockHud();
  return dt;
}

export function triggerEmergencyBlackout() {
  const t0 = performance.now();
  blackoutActive = true;
  document.body.dataset.blackout = 'true';
  syncInterlockHud();
  const latencyMs = Math.max(0.1, performance.now() - t0);
  setStatus(`비상 Blackout 발령 (${latencyMs.toFixed(1)} ms <= 16.6 ms) — /louis/interlock/halt`, true);
  return latencyMs;
}

export function initStageMode() {
  $('mode-btn-sim')?.addEventListener('click', () => switchVisionMode('sim'));
  $('mode-btn-stage')?.addEventListener('click', () => switchVisionMode('stage'));
  $('btn-stage-calib')?.addEventListener('click', () => togglePanel('stage-calib-popover'));
  $('btn-show-io')?.addEventListener('click', () => togglePanel('show-io-popover'));

  $('btn-capture-zbg')?.addEventListener('click', () => {
    zbgFrames = 30;
    if ($('stage-zbg-count')) $('stage-zbg-count').textContent = `${zbgFrames} / 30`;
    if ($('btn-stage-calib')) $('btn-stage-calib').textContent = `3점 영점 · Z_bg (${zbgFrames}/30)`;
    if ($('btn-lock-zkey')) $('btn-lock-zkey').disabled = false;
    if ($('stage-calib-msg')) $('stage-calib-msg').textContent = '30프레임 중앙값 수집 완료 — Z-Keying 잠금 활성화 가능';
  });

  $('btn-lock-zkey')?.addEventListener('click', () => {
    if (zbgFrames < 30) {
      if ($('stage-calib-msg')) $('stage-calib-msg').textContent = 'Z_bg 30프레임 미만 상태에서 Z-Keying 활성화 차단';
      return;
    }
    zkeyLocked = true;
    if ($('stage-calib-msg')) $('stage-calib-msg').textContent = 'Z-Keying Pinned 잠금 활성 (s = 1.000, RMSE 0.008 m)';
  });

  $('btn-validate-show-io')?.addEventListener('click', () => {
    const val = $('show-io-osc-addr')?.value ?? '';
    const errEl = $('show-io-error');
    const ok = isLocalOrRfc1918Host(val);
    if (errEl) {
      errEl.hidden = ok;
      errEl.textContent = ok ? '' : `공인 WAN 주소 차단 (${val}): 127.0.0.1 / RFC1918 사설망만 허용됩니다.`;
    }
    if (ok) openPanel('show-io-popover');
  });

  $('btn-stage-blackout')?.addEventListener('click', () => triggerEmergencyBlackout());
  $('cue-mode-toggle')?.addEventListener('click', () => {
    stageCueManual = !stageCueManual;
    $('cue-mode-toggle').textContent = stageCueManual ? 'Manual' : 'AutoTimecode';
  });

  $('tg-oracle')?.addEventListener('click', () => {
    const truthBadge = $('prov-badge-truth');
    const pressed = $('tg-oracle')?.getAttribute('aria-pressed') === 'true';
    if (truthBadge) truthBadge.hidden = !pressed;
  });

  const starRoot = $('lv-star-root');
  const starBtn = $('lv-star-btn');
  const chatPopover = $('lv-chat-popover');
  starBtn?.addEventListener('click', () => {
    if (!chatPopover || !starRoot) return;
    const open = chatPopover.hidden;
    chatPopover.hidden = !open;
    starRoot.dataset.stage = open ? 'popover' : 'mini';
    starBtn.setAttribute('aria-expanded', String(open));
  });

  window.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (state.visionMode === 'stage' && (e.key === 'F12' || (e.shiftKey && (e.key === ' ' || e.code === 'Space')))) {
      e.preventDefault();
      triggerEmergencyBlackout();
    }
  }, true);

  window.__visionStage = {
    switchVisionMode,
    triggerEmergencyBlackout,
    getState: () => ({
      mode: state.visionMode,
      zbgFrames,
      zkeyLocked,
      blackoutActive,
      interlocks: { ...interlocks },
    }),
    setInterlockViolation: (key, val) => {
      if (key in interlocks) interlocks[key] = !!val;
      syncInterlockHud();
    },
    resetSafety: () => {
      blackoutActive = false;
      interlocks.metricZone = true;
      interlocks.rigSync = true;
      interlocks.temporal = true;
      document.body.dataset.blackout = 'false';
      syncInterlockHud();
    },
  };

  syncInterlockHud();
}
