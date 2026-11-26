// apps/louis/ui/js/mobile-skins.js — LAN QR WebRTC 4 skins, GraceBlurred & 8-tier ESC/IME stack.
const SKIN_DESCRIPTIONS = {
  field_inspection: 'field_inspection — 안전장갑 56px 조작 · 3D 명판 스탠드오프 유도',
  docs_review: 'docs_review — 5-Gate 심사 요약 · 양식 교정 · 10% 사후감사 큐',
  kiosk_intake: 'kiosk_intake — Open 등급(CounterKiosk): Nano08B 작성 안내 전용 (복호화/3D 영구 차단)',
  executive_audit: 'executive_audit — DSLI_18 · 5년 WORM 이중 해시체인 감사 대시보드',
};

let graceBlurred = false;

window.addEventListener('DOMContentLoaded', () => {
  const skinSel = document.getElementById('mobile-skin-select');
  const descEl = document.getElementById('mobile-skin-desc');
  const graceOverlay = document.getElementById('grace-blurred');
  const escLog = document.getElementById('esc-ime-log');

  skinSel?.addEventListener('change', () => {
    const val = skinSel.value;
    document.body.dataset.skin = val;
    document.body.dataset.n2sf = val === 'kiosk_intake' ? 'open' : 'sensitive';
    if (descEl) descEl.textContent = `활성 스킨: ${SKIN_DESCRIPTIONS[val] || val}`;
  });

  document.getElementById('btn-trigger-grace')?.addEventListener('click', () => {
    graceBlurred = true;
    if (graceOverlay) graceOverlay.hidden = false;
  });

  document.getElementById('btn-mobile-reconnect')?.addEventListener('click', () => {
    graceBlurred = false;
    if (graceOverlay) graceOverlay.hidden = true;
  });

  document.getElementById('btn-wan-probe')?.addEventListener('click', () => {
    const msg = document.getElementById('mobile-wan-msg');
    if (msg) msg.textContent = 'Err(VaultError::PublicWanForbidden) — 8.8.8.8:9999 차단 완료';
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
