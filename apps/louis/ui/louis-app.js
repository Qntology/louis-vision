// apps/louis/ui/louis-app.js
// Zero-npm vanilla ES entry mounting StarWidget + Omni-Bar + Single-Anchor Router + Blockies QR + Bridge fallback.
import { invokeDual } from '../../shared-ui/js/bridge-client.js';
import { classifyIntentRoute, parseOmniQuery } from '../../shared-ui/js/omni-bar.js';
import { StarWidgetController } from '../../shared-ui/js/star-widget.js';
import { deriveBlockiesHaringSpec } from '../../shared-ui/js/blockies-haring.js';

const controller = new StarWidgetController({ surfaceOwner: 'louis_desktop_overlay' });
let msgSeq = 1;
const SAMPLE_DROPS = [
  { file: 'contract_2026_settlement.hwpx', prompt: 'DOM-01 기성검사원 포렌식 검증' },
  { file: 'concert_truss_stage.urdf', prompt: '리허설 최대 오차 확인' },
  { file: 'forklift_warehouse_rig.glb', prompt: '/sim 지게차 적치 협업 검증' },
];
let dropIdx = 0;

function drawPairIdenticon(seedHex = '0xAbCDef012345678901234567890123456789AbCd') {
  const canvas = document.getElementById('pair-qr-canvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  const spec = deriveBlockiesHaringSpec(seedHex);
  const { h_deg, s_pct, l_pct } = spec.bgcolor;
  ctx.fillStyle = `hsl(${h_deg}, ${s_pct}%, ${Math.max(82, l_pct)}%)`;
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

async function refreshRouterTelemetry(lastNote = null) {
  const st = await invokeDual('router_state');
  const ownerEl = document.getElementById('rt-owner');
  const leaseEl = document.getElementById('rt-lease');
  const privEl = document.getElementById('rt-privacy');
  const starsEl = document.getElementById('rt-stars');
  const noteEl = document.getElementById('rt-last-route');
  if (ownerEl) ownerEl.textContent = st.surfaceOwner;
  if (leaseEl) leaseEl.textContent = `${st.gpuLeaseTier} (${st.louisResidentMb} MB, ${st.louisIngestMode})`;
  if (privEl) privEl.textContent = `${st.n2sfGrade} · epoch=${st.privacyEpoch} ${st.privacyLocked ? 'LOCKED' : 'clear'}`;
  if (starsEl) starsEl.textContent = `Active ( ✦ ): ${st.activeStarWidgets} / Anchor: ${st.anchorLock}`;
  document.body.dataset.shield = st.privacyLocked ? 'lockdown' : 'clear';
  document.getElementById('lv-star-root')?.setAttribute('data-shield', st.privacyLocked ? 'lockdown' : 'clear');
  if (noteEl && lastNote) noteEl.textContent = lastNote;
}

function syncDom() {
  const root = document.getElementById('lv-star-root');
  const popover = document.getElementById('lv-chat-popover');
  const feedEl = document.getElementById('lv-mini-feed');
  const streamEl = document.getElementById('lv-chat-stream');
  const badgeEl = document.getElementById('lv-star-badge');
  const omniEl = document.getElementById('lv-omni-backdrop');
  if (root) root.dataset.stage = controller.stage;
  if (popover) popover.hidden = controller.stage !== 'popover';
  if (omniEl) omniEl.hidden = !controller.omniOpen;
  if (badgeEl) {
    badgeEl.hidden = controller.rows.length === 0;
    badgeEl.textContent = String(controller.rows.length);
  }
  if (feedEl) {
    feedEl.hidden = controller.stage === 'popover';
    feedEl.replaceChildren();
    for (const row of controller.getMiniFeed()) {
      const li = document.createElement('li');
      li.className = row.isFeedTop ? 'lv-feed-bubble feed-top' : 'lv-feed-bubble';
      li.style.opacity = String(row.opacity);
      li.textContent = row.text;
      li.addEventListener('click', () => {
        controller.stage = 'popover';
        syncDom();
      });
      feedEl.appendChild(li);
    }
  }
  if (streamEl) {
    streamEl.replaceChildren();
    for (const row of controller.rows) {
      const card = document.createElement('div');
      card.className = row.kind === 'user' ? 'lv-chat-msg user' : 'lv-chat-msg assistant';
      card.textContent = row.text;
      streamEl.appendChild(card);
    }
    const scrollEl = document.getElementById('lv-chat-scroll');
    if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
  }
}

async function handleRouteSubmission(rawText, files = []) {
  const text = String(rawText || '').trim();
  if (!text && files.length === 0) return;
  const parsed = parseOmniQuery(text, files);
  const intent = classifyIntentRoute(parsed.slashCmd, parsed.tags, parsed.freeText, files);
  const res = await invokeDual('router_route_input', { text, files });
  const owner = res.surfaceOwner || res.owner || intent.route;
  const lease = res.gpuLeaseTier || intent.warmLeaseKind;
  const summary = files.length
    ? `[파일 라우팅: ${files.join(', ')}] -> ${owner} (${lease})`
    : `[라우팅 완료] ${parsed.slashCmd || parsed.tags.dom || 'query'} -> owner=${owner}, lease=${lease}${res.louisResidentMb === 0 ? ', louisVRAM=0MB' : ''}`;

  const now = Date.now();
  controller.mergeIncomingRows([
    { id: `u-${msgSeq++}`, created_at: now, kind: 'user', text: files.length ? `${files[0]} · ${text}` : text },
    { id: `a-${msgSeq++}`, created_at: now + 1, kind: 'reply', text: summary },
  ]);
  await refreshRouterTelemetry(summary);
  syncDom();
}

window.addEventListener('DOMContentLoaded', async () => {
  const btn = document.getElementById('lv-star-btn');
  const collapseBtn = document.getElementById('lv-btn-collapse');
  const studioBtn = document.getElementById('lv-btn-studio');
  const attachBtn = document.getElementById('lv-btn-attach');
  const form = document.getElementById('lv-chat-form');
  const input = document.getElementById('lv-chat-input');
  const omniInput = document.getElementById('lv-omni-input');
  const pairModal = document.getElementById('lv-pair-modal');

  btn?.addEventListener('click', () => {
    controller.stage = controller.stage === 'popover' ? 'mini' : 'popover';
    syncDom();
  });
  collapseBtn?.addEventListener('click', () => {
    controller.stage = 'mini';
    syncDom();
  });
  studioBtn?.addEventListener('click', () => {
    window.location.href = './studio.html';
  });
  attachBtn?.addEventListener('click', async () => {
    const sample = SAMPLE_DROPS[dropIdx % SAMPLE_DROPS.length];
    dropIdx += 1;
    await handleRouteSubmission(sample.prompt, [sample.file]);
  });

  input?.addEventListener('input', (e) => {
    controller.setDraft(e.target.value);
  });
  form?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const text = input?.value?.trim() || '';
    if (!text) return;
    await handleRouteSubmission(text);
    controller.setDraft('');
    if (input) input.value = '';
    syncDom();
  });

  omniInput?.addEventListener('keydown', async (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'Enter') {
      const parsed = parseOmniQuery(omniInput.value);
      const status = document.getElementById('lv-omni-status');
      if (status) {
        status.textContent = `Route: ${parsed.route} | Lease: ${parsed.warmLeaseKind}${parsed.timeKey ? ` | time_guide=${parsed.timeKey}` : ''}`;
      }
      await handleRouteSubmission(omniInput.value);
    }
  });

  document.getElementById('btn-open-omni')?.addEventListener('click', () => {
    controller.omniOpen = !controller.omniOpen;
    syncDom();
    if (controller.omniOpen) omniInput?.focus();
  });

  drawPairIdenticon();
  document.getElementById('btn-open-pair')?.addEventListener('click', () => {
    if (pairModal && typeof pairModal.showModal === 'function') pairModal.showModal();
    else if (pairModal) pairModal.setAttribute('open', '');
  });
  document.getElementById('btn-pair-close')?.addEventListener('click', () => {
    if (pairModal && typeof pairModal.close === 'function') pairModal.close();
    else pairModal?.removeAttribute('open');
  });
  document.getElementById('btn-lan-scan')?.addEventListener('click', async () => {
    const addr = document.getElementById('pair-addr-input')?.value?.trim() || '192.168.45.12:9999';
    const res = await invokeDual('lan_pair_check', { addr });
    const msg = document.getElementById('pair-status');
    if (msg) msg.textContent = res.allowed ? `PASS: RFC1918 LAN 허용 (${addr}) · X25519 키교환 준비` : `BLOCKED: ${res.error} (${addr})`;
  });
  document.getElementById('btn-wan-reject')?.addEventListener('click', async () => {
    const res = await invokeDual('lan_pair_check', { addr: '8.8.8.8:9999' });
    const msg = document.getElementById('pair-status');
    if (msg) msg.textContent = res.allowed ? 'UNEXPECTED' : `BLOCKED: ${res.error} (8.8.8.8:9999 공인 WAN 차단)`;
  });

  let privLocked = false;
  document.getElementById('btn-toggle-privacy')?.addEventListener('click', async () => {
    privLocked = !privLocked;
    const res = await invokeDual('broadcast_privacy_epoch', { locked: privLocked, reason: 'bank.exe' });
    await refreshRouterTelemetry(`broadcast_privacy_epoch: epoch=${res.epoch}, locked=${res.locked}, 3D_nonzero=${res.stage3dScreenNonzeroBytes}B`);
  });

  window.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if ((e.ctrlKey && e.key.toLowerCase() === 'k') || (e.altKey && e.code === 'Space')) {
      e.preventDefault();
      controller.omniOpen = !controller.omniOpen;
      syncDom();
      if (controller.omniOpen) omniInput?.focus();
      return;
    }
    const verdict = controller.handleKeyEvent(e);
    if (verdict.action !== 'noop') {
      syncDom();
    }
  });

  await refreshRouterTelemetry();
  syncDom();
});
