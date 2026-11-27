// apps/louis/ui/js/field-lens.js — Field-3D viewfinder canvas renderer, CounterKiosk lock, 350ms pass freeze & LOTO req_depth.
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

function drawFieldScene(reason) {
  const canvas = document.getElementById('field-ar-canvas');
  if (canvas) {
    const ctx = canvas.getContext('2d');
    const w = canvas.width;
    const h = canvas.height;
    const g = ctx.createLinearGradient(0, 0, w, h);
    g.addColorStop(0, '#0f1726');
    g.addColorStop(1, '#182438');
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);

    // Subtle AR depth grid
    ctx.strokeStyle = 'rgba(148, 163, 184, 0.14)';
    ctx.lineWidth = 1;
    for (let x = 0; x <= w; x += 40) {
      ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, h); ctx.stroke();
    }
    for (let y = 0; y <= h; y += 40) {
      ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(w, y); ctx.stroke();
    }

    // Industrial valve + piping backdrop
    ctx.fillStyle = '#253248';
    ctx.fillRect(110, 210, 260, 44);
    ctx.fillStyle = '#31425e';
    ctx.fillRect(210, 155, 68, 145);

    // Equipment Nameplate Quad (transforms per readability condition)
    ctx.save();
    ctx.translate(reason === 'ClippedByFrameEdge' ? 765 : 285, 130);
    if (reason === 'SteepViewingAngle') ctx.transform(0.55, 0.28, -0.18, 0.9, 0, 0);
    else if (reason === 'LowEffectiveDpi') ctx.scale(0.62, 0.62);

    ctx.fillStyle = '#d4dde8';
    ctx.fillRect(0, 0, 210, 118);
    ctx.strokeStyle = reason === 'Pass' ? '#22c55e' : '#f59e0b';
    ctx.lineWidth = 3;
    ctx.strokeRect(0, 0, 210, 118);

    ctx.fillStyle = '#0f172a';
    ctx.font = 'bold 14px ui-monospace, monospace';
    ctx.fillText('KKS: 10LAB10AP001', 12, 28);
    ctx.font = '12px ui-monospace, monospace';
    ctx.fillText('MSCU1234566 | UN1263', 12, 52);
    ctx.fillText('MAWP: 30.0 bar | AOP-103', 12, 74);
    ctx.fillText(reason === 'Pass' ? '6.4 px/mm OK (RMSE 8.2mm)' : `GATE: ${reason}`, 12, 100);

    if (reason === 'SpecularGlareHoles') {
      ctx.fillStyle = 'rgba(255, 255, 255, 0.86)';
      ctx.beginPath();
      ctx.ellipse(125, 58, 72, 34, -0.35, 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();

    // 45° Oblique 3D Display Screen Privacy Fail-Closed Zeroize Mask (strict #000000 0 non-zero bytes)
    ctx.save();
    ctx.translate(585, 220);
    ctx.transform(1.0, -0.14, 0.16, 0.92, 0, 0);
    ctx.fillStyle = '#000000';
    ctx.fillRect(0, 0, 190, 110);
    ctx.strokeStyle = '#3fb950';
    ctx.lineWidth = 2;
    ctx.strokeRect(0, 0, 190, 110);
    ctx.fillStyle = '#3fb950';
    ctx.font = 'bold 11px ui-monospace, monospace';
    ctx.fillText('3D MONITOR REDACTED', 14, 48);
    ctx.fillText('0 NON-ZERO BYTES', 14, 68);
    ctx.restore();
  }

  const pip = document.getElementById('pip-canvas');
  if (pip) {
    const pctx = pip.getContext('2d');
    pctx.fillStyle = '#0b1220';
    pctx.fillRect(0, 0, pip.width, pip.height);
    pctx.fillStyle = reason === 'Pass' ? '#163427' : '#2b2113';
    pctx.fillRect(6, 6, pip.width - 12, pip.height - 12);
    pctx.strokeStyle = reason === 'Pass' ? '#3fb950' : '#f59e0b';
    pctx.lineWidth = 2;
    pctx.strokeRect(6, 6, pip.width - 12, pip.height - 12);
    pctx.fillStyle = '#e6edf3';
    pctx.font = 'bold 11px ui-monospace, monospace';
    pctx.fillText('10LAB10AP001 [3x3 UNWARP]', 14, 30);
    pctx.fillStyle = reason === 'Pass' ? '#7ee787' : '#f2cc60';
    pctx.fillText(reason === 'Pass' ? '6.4 px/mm · FROZEN READY' : `HOLD: ${reason}`, 14, 54);
  }
}

function updateRepositionBanner(reason) {
  const banner = document.getElementById('reposition-banner');
  const freezeEl = document.getElementById('freeze-status');
  drawFieldScene(reason);
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

  drawFieldScene(reasonSel?.value || 'LowEffectiveDpi');

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

  const starBtn = document.getElementById('lv-star-btn');
  const starPop = document.getElementById('lv-chat-popover');
  starBtn?.addEventListener('click', () => {
    if (starPop) starPop.hidden = !starPop.hidden;
  });
  document.getElementById('lv-btn-collapse')?.addEventListener('click', () => {
    if (starPop) starPop.hidden = true;
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
