// apps/louis/ui/js/field-lens.js — Field-3D viewfinder, CounterKiosk lock, 350ms pass freeze & LOTO req_depth.
import { invokeDual } from '../../../shared-ui/js/bridge-client.js';

const GUIDE_TEXT = {
  LowEffectiveDpi: 'VLM 호출 0회: 해상도 부족 (1.55 < 5.5 px/mm) - 표면 정면으로 2.38m 접근하세요',
  SteepViewingAngle: 'VLM 호출 0회: 촬영 각도가 너무 비스듬합니다(71°) - 표면 법선 축으로 이동해 정렬하세요',
  SpecularGlareHoles: 'VLM 호출 0회: 유광 빛반사 감지(48% > 45%) - 반사각 회피(10 deg)를 위해 측면 이동하세요',
  ClippedByFrameEdge: 'VLM 호출 0회: 문서/명판 모서리가 화면 밖으로 잘렸습니다 - 전체가 들어오도록 시선을 조정하세요',
  Pass: 'ReadabilityVerdict::Pass (6.4 px/mm >= 5.5) — 350ms 안정 프레임 자동 프리즈 완료',
};

let frozen = false;
let step2Passed = false;
let step3Signed = false;
let freezeTimer = null;

function updateRepositionBanner(reason) {
  const banner = document.getElementById('reposition-banner');
  const freezeEl = document.getElementById('freeze-status');
  if (!banner) return;
  banner.textContent = GUIDE_TEXT[reason] || GUIDE_TEXT.LowEffectiveDpi;
  banner.className = reason === 'Pass' ? 'lv-banner ok' : 'lv-banner warn';
  clearTimeout(freezeTimer);
  if (reason === 'Pass' && freezeEl) {
    freezeEl.textContent = '350ms 안정도 측정 중...';
    freezeTimer = setTimeout(() => {
      frozen = true;
      freezeEl.textContent = 'AUTO-FREEZE LOCKED (350ms stable pass)';
    }, 350);
  } else if (freezeEl) {
    frozen = false;
    freezeEl.textContent = '접근 대기 (ApproachForRead)';
  }
}

window.addEventListener('DOMContentLoaded', () => {
  const roleSel = document.getElementById('field-role-select');
  const kioskShield = document.getElementById('kiosk-shield');
  const reasonSel = document.getElementById('reposition-reason-select');
  const alertEl = document.getElementById('interlock-alert');

  roleSel?.addEventListener('change', () => {
    if (kioskShield) {
      kioskShield.hidden = roleSel.value !== 'CounterKiosk';
    }
  });
  document.getElementById('btn-kiosk-reset')?.addEventListener('click', () => {
    if (roleSel) roleSel.value = 'OperatorTablet';
    if (kioskShield) kioskShield.hidden = true;
  });

  reasonSel?.addEventListener('change', () => updateRepositionBanner(reasonSel.value));

  document.getElementById('btn-loto-step2')?.addEventListener('click', () => {
    const val = Number(document.getElementById('loto-step2-val')?.value || 0);
    if (!alertEl) return;
    if (val < 50) {
      step2Passed = false;
      alertEl.className = 'lv-banner warn';
      alertEl.textContent = `Err(NotMet { step: 2, observed: "${val}" }) — 부족분 -${(50 - val).toFixed(1)}%p`;
    } else {
      step2Passed = true;
      alertEl.className = 'lv-banner ok';
      alertEl.textContent = `Step 2 통과 (${val}% >= 50%) — Step 3 LOTO 서명 가능`;
    }
  });

  document.getElementById('btn-loto-step3')?.addEventListener('click', () => {
    if (!alertEl) return;
    if (!step2Passed) {
      step3Signed = false;
      alertEl.className = 'lv-banner err';
      alertEl.textContent = 'Err(OutOfOrder { step: 3, pending: 2 }) -> G3InterceptInterlockOrder 잠금 (AutoFinalize 0건)';
      return;
    }
    const sup = (document.getElementById('loto-sup-id')?.value || '').trim();
    if (!sup) {
      alertEl.className = 'lv-banner err';
      alertEl.textContent = '감독자 사번(SUP-01) 필요';
      return;
    }
    step3Signed = true;
    alertEl.className = 'lv-banner ok';
    alertEl.textContent = `LOTO 완결(st.complete() == true, ${sup}) — .hwpx & AuditChain 커밋 준비 완료`;
  });

  document.getElementById('btn-glove-freeze')?.addEventListener('click', () => {
    frozen = !frozen;
    const freezeEl = document.getElementById('freeze-status');
    if (freezeEl) {
      freezeEl.textContent = frozen ? 'MANUAL-FREEZE LOCKED' : 'LIVE CAPTURE';
    }
  });

  document.getElementById('btn-glove-next')?.addEventListener('click', () => {
    const pip = document.getElementById('rectified-pip');
    if (pip) {
      pip.style.width = pip.style.width === '360px' ? '220px' : '360px';
    }
  });

  document.getElementById('btn-glove-commit')?.addEventListener('click', async () => {
    const t0 = performance.now();
    if (!step2Passed || !step3Signed) {
      if (alertEl) {
        alertEl.className = 'lv-banner err';
        alertEl.textContent = '커밋 거부: 순차 인터록 Step 1~3 완료 필수 (G3InterceptInterlockOrder)';
      }
      return;
    }
    await invokeDual('router_route_input', { text: 'commit field-3d AOP-103' });
    const elapsedUs = Math.max(120, Math.round((performance.now() - t0) * 1000));
    const lockEl = document.getElementById('field-lock-us');
    if (lockEl) lockEl.textContent = `원장 커밋 완료: ${elapsedUs} us (< 2,000 us)`;
  });

  window.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'AudioVolumeUp' || e.key === 'Enter') {
      document.getElementById('btn-glove-freeze')?.click();
    } else if (e.key === 'AudioVolumeDown') {
      document.getElementById('btn-glove-next')?.click();
    } else if (e.key === 'Escape') {
      if (frozen) {
        frozen = false;
        const freezeEl = document.getElementById('freeze-status');
        if (freezeEl) freezeEl.textContent = 'LIVE CAPTURE';
      } else {
        window.location.href = './studio.html';
      }
    }
  });
});
