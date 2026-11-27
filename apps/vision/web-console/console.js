// apps/vision/web-console/console.js
// Zero-npm external ES module replacing inline <script> in web-console/index.html (Rule 6 compliance).
function isAllowedLoopbackOrLan(raw) {
  const host = String(raw || '').trim().split(':')[0].replace(/^\[|\]$/g, '').toLowerCase();
  return (
    host === '127.0.0.1' ||
    host === 'localhost' ||
    host === '::1' ||
    host.startsWith('10.') ||
    host.startsWith('192.168.') ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(host)
  );
}

window.addEventListener('DOMContentLoaded', () => {
  let halted = false;
  let cueSeq = 1;
  const haltBanner = document.getElementById('haltBanner');
  const cueBtn = document.getElementById('cueGoBtn');
  const toggleBtn = document.getElementById('toggleHaltBtn');
  const feedback = document.getElementById('cueFeedback');
  const targetInput = document.getElementById('oscTargetInput');

  toggleBtn?.addEventListener('click', () => {
    halted = !halted;
    haltBanner?.classList.toggle('active', halted);
    if (cueBtn) cueBtn.disabled = halted;
    if (toggleBtn) toggleBtn.textContent = halted ? 'Clear Interlock' : 'Simulate Interlock Halt';
    if (feedback) {
      feedback.textContent = halted
        ? 'BLOCKED by SandboxInterlock (/louis/interlock/halt)'
        : 'Ready — All 3 interlock channels cleared';
    }
  });

  cueBtn?.addEventListener('click', () => {
    if (halted) return;
    const addr = targetInput?.value?.trim() || '127.0.0.1:9000';
    if (!isAllowedLoopbackOrLan(addr)) {
      if (feedback) {
        feedback.textContent = `BLOCKED WAN target (${addr}): Loopback / RFC1918 LAN only`;
      }
      return;
    }
    if (feedback) {
      feedback.textContent = `Dispatched /louis/cue/main #${cueSeq++} to ${addr} (<= 16.6ms budget)`;
    }
  });
});
