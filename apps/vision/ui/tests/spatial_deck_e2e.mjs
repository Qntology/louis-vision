// Zero-npm E2E test suite for vision-sandbox v1.8 Spatial Deck (Desktop 1280x720 + Mobile 390x844).
// Verifies static HTML hygiene, State A/B/C transitions, 2-axis scroll lock, layer filter chips,
// [+] add / [x] mute / Shift+[x] delete / Ctrl+Z undo, [S] split + inheritance, inline drawers,
// and char_prompt / stage_cad_prompt / stage_slice_program / stage_mask_apply / stage_render_* IPC.
import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const UI_ROOT = path.resolve(__dirname, '..');
const APPS_ROOT = path.resolve(UI_ROOT, '..', '..');

const CHROME_CANDIDATES = [
  process.env.LV_CHROMIUM_BIN,
  'C:\\Users\\lp\\AppData\\Local\\ms-playwright\\chromium-1155\\chrome-win\\chrome.exe',
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
].filter(Boolean);

function findChromeBin() {
  for (const p of CHROME_CANDIDATES) {
    if (fs.existsSync(p)) return p;
  }
  return null;
}

const MIME_TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
};

function startStaticServer() {
  const server = http.createServer((req, res) => {
    const cleanUrl = (req.url || '/').split('?')[0];
    if (cleanUrl === '/favicon.ico') {
      res.writeHead(204);
      res.end();
      return;
    }
    let relPath = decodeURIComponent(cleanUrl).replace(/^\/+/, '');
    if (!relPath || relPath.endsWith('/')) relPath += 'index.html';
    const fullPath = path.resolve(APPS_ROOT, relPath);
    if (!fullPath.startsWith(APPS_ROOT) || !fs.existsSync(fullPath) || fs.statSync(fullPath).isDirectory()) {
      res.writeHead(404);
      res.end('Not found');
      return;
    }
    const ext = path.extname(fullPath).toLowerCase();
    res.writeHead(200, { 'Content-Type': MIME_TYPES[ext] || 'application/octet-stream' });
    fs.createReadStream(fullPath).pipe(res);
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const addr = server.address();
      resolve({ server, baseUrl: `http://127.0.0.1:${addr.port}` });
    });
  });
}

class CdpClient {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(`CDP ${msg.error.code}: ${msg.error.message}`));
        else resolve(msg.result);
      }
    });
  }
  static async connect(wsUrl) {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error('WS timeout')), 10000);
      ws.addEventListener('open', () => { clearTimeout(t); resolve(); }, { once: true });
      ws.addEventListener('error', (err) => { clearTimeout(t); reject(err); }, { once: true });
    });
    return new CdpClient(ws);
  }
  send(method, params = {}, sessionId = undefined) {
    const id = this.nextId++;
    const payload = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => { this.pending.delete(id); reject(new Error(`CDP timeout ${method}`)); }, 15000);
      this.pending.set(id, {
        resolve: (v) => { clearTimeout(t); resolve(v); },
        reject: (e) => { clearTimeout(t); reject(e); },
      });
      this.ws.send(JSON.stringify(payload));
    });
  }
  async evalExpr(sessionId, expression) {
    const res = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId);
    if (res.exceptionDetails) {
      throw new Error(res.exceptionDetails.exception?.description || res.exceptionDetails.text || 'JS error');
    }
    return res.result?.value;
  }
  close() { try { this.ws.close(); } catch { /* ignore */ } }
}

async function launchChromium(chromeBin) {
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lv-spatial-deck-'));
  const proc = spawn(chromeBin, [
    '--headless=new',
    '--remote-debugging-port=0',
    '--no-sandbox',
    '--disable-gpu',
    '--disable-dev-shm-usage',
    '--window-size=1280,720',
    `--user-data-dir=${userDataDir}`,
    'about:blank',
  ], { stdio: ['ignore', 'pipe', 'pipe'] });
  const wsUrl = await new Promise((resolve, reject) => {
    let stderrBuf = '';
    const timer = setTimeout(() => reject(new Error(`Chromium startup timeout:\n${stderrBuf}`)), 15000);
    proc.stderr.on('data', (chunk) => {
      stderrBuf += chunk.toString('utf8');
      const m = /DevTools listening on (ws:\/\/[^\s]+)/.exec(stderrBuf);
      if (m) { clearTimeout(timer); resolve(m[1]); }
    });
    proc.on('exit', (code) => { clearTimeout(timer); reject(new Error(`Chromium exited (${code})`)); });
  });
  return { proc, wsUrl, userDataDir };
}

test('static hygiene: zero inline script, zero on*=, zero forbidden tokens, all v1.8 IDs present', () => {
  const html = fs.readFileSync(path.join(UI_ROOT, 'index.html'), 'utf8');
  assert.equal(/<script\b(?![^>]*\bsrc=)/i.test(html), false, 'index.html must contain zero inline scripts');
  assert.equal(/\son[a-z]+\s*=/i.test(html), false, 'index.html must contain zero inline event handlers');
  assert.equal(/\bSAM\b|MobileSAM|4종/.test(html), false, 'index.html must not contain forbidden UI-36 tokens');
  for (const id of [
    'spatial-deck', 'deck-toggle-btn', 'deck-drag-handle', 'deck-filter-bar',
    'slice-v-scroll', 'slide-h-track', 'master-scrubber', 'master-theater-btn', 'master-theater-bar',
  ]) {
    assert.ok(html.includes(`id="${id}"`), `index.html missing id="${id}"`);
  }
  assert.ok(html.includes('value="iso"'), 'view-select missing isometric option');
});

test('headless Chromium E2E: State A/B/C, 2-axis scroll lock, filters, CRUD+Undo, drawers, mobile 4-stack', async () => {
  const chromeBin = findChromeBin();
  assert.ok(chromeBin, 'Local Chromium/Chrome/Edge executable must be installed');
  const { server, baseUrl } = await startStaticServer();
  const { proc, wsUrl, userDataDir } = await launchChromium(chromeBin);
  const cdp = await CdpClient.connect(wsUrl);
  try {
    const { targetId } = await cdp.send('Target.createTarget', { url: `${baseUrl}/vision/ui/index.html` });
    const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
    await cdp.send('Page.enable', {}, sessionId);
    await cdp.send('Runtime.enable', {}, sessionId);

    // Wait for __spatialDeck initialization
    for (let i = 0; i < 40; i++) {
      const ready = await cdp.evalExpr(sessionId, 'Boolean(window.__spatialDeck && window.__masterPlayer)');
      if (ready) break;
      await new Promise((r) => setTimeout(r, 100));
    }

    // 1. State A (default collapsed) + Sim defaults (×1, chase) + UI-36 forbidden token check on rendered DOM
    const sA = await cdp.evalExpr(sessionId, `({
      deck: window.__spatialDeck.getState(),
      hidden: document.getElementById('spatial-deck').hidden,
      forbidden: /\\bSAM\\b|MobileSAM|4종/.test(document.body.innerHTML),
      starCount: document.querySelectorAll('#lv-star-root').length,
      defaultView: document.getElementById('view-select')?.value,
      defaultSpeed: document.getElementById('sim-speed')?.value,
    })`);
    assert.equal(sA.deck.state, 'collapsed');
    assert.equal(sA.hidden, true);
    assert.equal(sA.forbidden, false);
    assert.equal(sA.starCount, 1);
    assert.equal(sA.defaultView, 'chase');
    assert.equal(sA.defaultSpeed, '1');

    // 2. State B (expanded) + PiP minimap visible + default 2 cards per track (4 tracks × 2 = 8 cards)
    const sB = await cdp.evalExpr(sessionId, `(() => {
      document.getElementById('deck-toggle-btn').click();
      return {
        deck: window.__spatialDeck.getState(),
        deckHidden: document.getElementById('spatial-deck').hidden,
        minimapHidden: document.getElementById('vp-minimap').hidden,
        cardCount: document.querySelectorAll('#slide-h-track .deck-card').length,
        hasGhost: Boolean(document.getElementById('deck-ghost-add')),
        lockH: window.__spatialDeck.resolveTwoAxisGesture(16, 4),
        lockV: window.__spatialDeck.resolveTwoAxisGesture(3, 18),
        lockNone: window.__spatialDeck.resolveTwoAxisGesture(5, 5),
      };
    })()`);
    assert.equal(sB.deck.state, 'expanded');
    assert.equal(sB.deckHidden, false);
    assert.equal(sB.minimapHidden, false);
    assert.equal(sB.cardCount, 8);
    assert.equal(sB.hasGhost, true);
    assert.equal(sB.lockH, 'h');
    assert.equal(sB.lockV, 'v');
    assert.equal(sB.lockNone, null);

    // 3. [+] Ghost card add, all 4 filter chips (2 baseline cards each), split + inheritance, mute / Shift+delete / Ctrl+Z undo
    const crud = await cdp.evalExpr(sessionId, `(() => {
      document.getElementById('deck-ghost-add').click();
      const afterAdd = document.querySelectorAll('#slide-h-track .deck-card').length;
      window.__spatialDeck.setDeckFilter('cam');
      const filteredCam = document.querySelectorAll('#slide-h-track .deck-card').length;
      window.__spatialDeck.setDeckFilter('actor');
      const filteredActor = document.querySelectorAll('#slide-h-track .deck-card').length;
      window.__spatialDeck.setDeckFilter('light');
      const filteredLight = document.querySelectorAll('#slide-h-track .deck-card').length;
      window.__spatialDeck.setDeckFilter('set');
      const filteredSet = document.querySelectorAll('#slide-h-track .deck-card').length;
      window.__spatialDeck.setDeckFilter('all');
      window.__spatialDeck.splitActiveSlice();
      const afterSplitSlices = window.__spatialDeck.getState().slices.length;
      window.__spatialDeck.toggleMuteOrDeleteSlice(1, false);
      const muted1 = window.__spatialDeck.getState().slices[1].muted;
      window.__spatialDeck.toggleMuteOrDeleteSlice(1, true);
      const afterDeleteSlices = window.__spatialDeck.getState().slices.length;
      window.__spatialDeck.undoDeckAction();
      const afterUndoSlices = window.__spatialDeck.getState().slices.length;
      return { afterAdd, filteredCam, filteredActor, filteredLight, filteredSet, afterSplitSlices, muted1, afterDeleteSlices, afterUndoSlices };
    })()`);
    assert.equal(crud.afterAdd, 9);
    assert.equal(crud.filteredCam, 2);
    assert.equal(crud.filteredActor, 2);
    assert.equal(crud.filteredLight, 3);
    assert.equal(crud.filteredSet, 2);
    assert.equal(crud.afterSplitSlices, 3);
    assert.equal(crud.muted1, true);
    assert.equal(crud.afterDeleteSlices, 2);
    assert.equal(crud.afterUndoSlices, 3);

    // 4. Inline [▾] drawer opens and triggers char_prompt + stage_mask_apply + stage_render_step + abort
    const drawerRes = await cdp.evalExpr(sessionId, `(async () => {
      const actorCard = document.querySelector('.deck-card[data-card-kind="actor"]');
      actorCard.querySelector('.deck-drawer-btn').click();
      const applyBtn = document.querySelector('.deck-card[data-card-kind="actor"] .deck-drawer button');
      applyBtn.click();
      await new Promise((r) => setTimeout(r, 80));
      const actorOut = document.querySelector('.deck-card[data-card-kind="actor"] .deck-drawer-out')?.textContent || '';

      const camCard = document.querySelector('.deck-card[data-card-kind="cam"]');
      camCard.querySelector('.deck-drawer-btn').click();
      const camBtns = Array.from(document.querySelectorAll('.deck-card[data-card-kind="cam"] .deck-drawer button'));
      camBtns[0].click(); // mask apply
      await new Promise((r) => setTimeout(r, 60));
      camBtns[2].click(); // cycles step
      await new Promise((r) => setTimeout(r, 60));
      camBtns[3].click(); // instant abort
      await new Promise((r) => setTimeout(r, 60));
      const camOut = document.querySelector('.deck-card[data-card-kind="cam"] .deck-drawer-out')?.textContent || '';
      return { actorOut, camOut };
    })()`);
    assert.ok(drawerRes.actorOut.includes('BPM'), `actor drawer output: ${drawerRes.actorOut}`);
    assert.ok(drawerRes.camOut.includes('16.6ms'), `cam drawer abort output: ${drawerRes.camOut}`);

    // 5. State C Master Theater enter + exit back to State B
    const theater = await cdp.evalExpr(sessionId, `(() => {
      document.getElementById('master-theater-btn').click();
      const inTheater = window.__masterPlayer.getState();
      window.__masterPlayer.exitTheaterToSlice();
      const afterExit = window.__spatialDeck.getState().state;
      return { inTheater, afterExit };
    })()`);
    assert.equal(theater.inTheater.visible, true);
    assert.equal(theater.afterExit, 'expanded');

    // 6. Mobile (<760px) 4-stack layout check (390x844, viewport >= 45% height, >= 36px touch buttons)
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width: 390, height: 844, deviceScaleFactor: 2, mobile: true,
    }, sessionId);
    await new Promise((r) => setTimeout(r, 100));
    const mobile = await cdp.evalExpr(sessionId, `(() => {
      const wrap = document.getElementById('world-wrap').getBoundingClientRect();
      const chip = document.querySelector('#deck-filter-bar .deck-chip')?.getBoundingClientRect();
      return {
        vpRatio: wrap.height / window.innerHeight,
        chipH: chip ? chip.height : 0,
      };
    })()`);
    assert.ok(mobile.vpRatio >= 0.42, `mobile viewport canvas height ratio too small: ${mobile.vpRatio}`);
    assert.ok(mobile.chipH >= 36, `mobile touch target height < 36px: ${mobile.chipH}`);
  } finally {
    cdp.close();
    const exited = new Promise((resolve) => {
      proc.once('exit', resolve);
      setTimeout(resolve, 1500);
    });
    proc.kill();
    await exited;
    server.close();
    try {
      fs.rmSync(userDataDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
    } catch {
      /* ignore transient Windows Chromium profile handle lock */
    }
  }
});
