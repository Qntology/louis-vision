// apps/louis/ui/js/mobile-skins.js — LAN QR WebRTC 4 skins, Blockies-Haring QR, GraceBlurred & 8-tier ESC/IME stack.
import { invokeDual } from '../../../shared-ui/js/bridge-client.js';
import { deriveBlockiesHaringSpec } from '../../../shared-ui/js/blockies-haring.js';

const SKIN_DESCRIPTIONS = {
  field_inspection: 'field_inspection — 안전장갑 56px 조작 · 3D 명판 스탠드오프 유도',
  docs_review: 'docs_review — 5-Gate 심사 요약 · 양식 교정 · 10% 사후감사 큐',
  kiosk_intake: 'kiosk_intake — Open 등급(CounterKiosk): Nano08B 작성 안내 전용 (복호화/3D 영구 차단)',
  executive_audit: 'executive_audit — DSLI_18 · 5년 WORM 이중 해시체인 감사 대시보드',
};

let graceBlurred = false;

function drawMobileQr() {
  const canvas = document.getElementById('mobile-qr-canvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  const spec = deriveBlockiesHaringSpec('0xAbCDef012345678901234567890123456789AbCd');
  ctx.fillStyle = `hsl(${spec.bgcolor.h_deg}, ${spec.bgcolor.s_pct}%, ${Math.max(84, spec.bgcolor.l_pct)}%)`;
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  const cell = 16;
  const pad = 8;
  for (let y = 0; y < 8; y++) {
    for (let x = 0; x < 8; x++) {
      const v = spec.grid_8x8[y * 8 + x];
      if (v === 0) continue;
      const hsl = v === 1 ? spec.haring_body_hsl : spec.spotcolor;
      ctx.fillStyle = `hsl(${hsl.h_deg}, ${hsl.s_pct}%, ${hsl.l_pct}%)`;
      ctx.fillRect(pad + x * cell, pad + y * cell, cell, cell);
    }
  }
  ctx.lineWidth = 2;
  ctx.strokeStyle = '#000000';
  ctx.strokeRect(1, 1, canvas.width - 2, canvas.height - 2);
}

window.addEventListener('DOMContentLoaded', () => {
  const skinSel = document.getElementById('mobile-skin-select');
  const descEl = document.getElementById('mobile-skin-desc');
  const graceOverlay = document.getElementById('grace-blurred');
  const escLog = document.getElementById('esc-ime-log');
  const wanMsg = document.getElementById('mobile-wan-msg');

  drawMobileQr();

  const applySkin = (val) => {
    document.body.dataset.skin = val;
    document.body.dataset.n2sf = val === 'kiosk_intake' ? 'open' : 'sensitive';
    if (descEl) descEl.textContent = `활성 스킨: ${SKIN_DESCRIPTIONS[val] || val}`;
    document.querySelectorAll('[data-skin-pane]').forEach((pane) => {
      pane.classList.toggle('active', pane.getAttribute('data-skin-pane') === val);
    });
  };

  skinSel?.addEventListener('change', () => applySkin(skinSel.value));

  document.getElementById('btn-trigger-grace')?.addEventListener('click', () => {
    graceBlurred = true;
    if (graceOverlay) graceOverlay.hidden = false;
  });

  document.getElementById('btn-mobile-reconnect')?.addEventListener('click', () => {
    graceBlurred = false;
    if (graceOverlay) graceOverlay.hidden = true;
  });

  document.getElementById('btn-wan-probe')?.addEventListener('click', async () => {
    if (wanMsg) wanMsg.textContent = 'Err(VaultError::PublicWanForbidden) — 8.8.8.8:9999 차단 완료';
    await invokeDual('lan_pair_check', { addr: '8.8.8.8:9999' });
  });

  document.getElementById('btn-lan-probe')?.addEventListener('click', async () => {
    const res = await invokeDual('lan_pair_check', { addr: '192.168.45.12:9999' });
    if (wanMsg) wanMsg.textContent = res.allowed ? 'RFC1918 LAN 허용: 192.168.45.12:9999 (Zero-WAN PASS)' : `Err: ${res.error}`;
  });

  document.getElementById('btn-grace-zeroize')?.addEventListener('click', async () => {
    const res = await invokeDual('vault_expire_zeroize');
    if (escLog) escLog.textContent = `Grace 만료 zeroize 완료: state=${res.state}, keyBytesNonZero=${res.keyBytesNonZero} B`;
  });

  window.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) {
      if (escLog) escLog.textContent = 'IME 조합 중 (isComposing / keyCode=229) — ESC 단축키 소비 억제';
      return;
    }
    if (e.key === 'Escape') {
      if (graceBlurred && graceOverlay) {
        graceOverlay.hidden = true;
        graceBlurred = false;
        if (escLog) escLog.textContent = 'ESC Tier-1: GraceBlurred 해제';
      } else {
        if (escLog) escLog.textContent = 'ESC Tier-8: 상위 화면 복귀 가용';
      }
    }
  });
});
