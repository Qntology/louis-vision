// apps/louis/ui/louis-app.js
// Zero-npm vanilla ES entry mounting StarWidget + Omni-Bar + Bridge fallback.
import { invokeDual } from '../../shared-ui/js/bridge-client.js';
import { parseOmniQuery } from '../../shared-ui/js/omni-bar.js';
import { StarWidgetController } from '../../shared-ui/js/star-widget.js';

const controller = new StarWidgetController({ surfaceOwner: 'louis_desktop_overlay' });

function syncDom() {
  const popover = document.getElementById('lv-chat-popover');
  const feedEl = document.getElementById('lv-mini-feed');
  const omniEl = document.getElementById('lv-omni-backdrop');
  if (popover) popover.hidden = controller.stage !== 'popover';
  if (omniEl) omniEl.hidden = !controller.omniOpen;
  if (feedEl) {
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
}

window.addEventListener('DOMContentLoaded', async () => {
  const btn = document.getElementById('lv-star-btn');
  const collapseBtn = document.getElementById('lv-btn-collapse');
  const studioBtn = document.getElementById('lv-btn-studio');
  const form = document.getElementById('lv-chat-form');
  const input = document.getElementById('lv-chat-input');
  const omniInput = document.getElementById('lv-omni-input');

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
  input?.addEventListener('input', (e) => {
    controller.setDraft(e.target.value);
  });
  form?.addEventListener('submit', async (e) => {
    e.preventDefault();
    const text = input?.value?.trim() || '';
    if (!text) return;
    const res = await invokeDual('router_route_input', { text });
    controller.mergeIncomingRows([
      { id: `msg-${Date.now()}`, created_at: Date.now(), kind: 'reply', text: JSON.stringify(res) },
    ]);
    controller.setDraft('');
    if (input) input.value = '';
    syncDom();
  });
  omniInput?.addEventListener('keydown', async (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'Enter') {
      const parsed = parseOmniQuery(omniInput.value);
      const status = document.getElementById('lv-omni-status');
      if (status) status.textContent = `${parsed.route} (${parsed.warmLeaseKind})`;
      await invokeDual('router_route_input', { text: omniInput.value });
    }
  });
  window.addEventListener('keydown', (e) => {
    if (e.isComposing || e.keyCode === 229) return;
    if ((e.ctrlKey && e.key.toLowerCase() === 'k') || (e.altKey && e.code === 'Space')) {
      e.preventDefault();
      controller.omniOpen = !controller.omniOpen;
      syncDom();
      return;
    }
    const verdict = controller.handleKeyEvent(e);
    if (verdict.action !== 'noop') {
      syncDom();
    }
  });
  await invokeDual('router_state');
  syncDom();
});
