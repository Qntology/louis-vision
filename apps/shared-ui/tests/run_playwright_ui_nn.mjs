// Zero-npm Layer 2 Headless Chromium CDP Evaluator for UI N:N (36 diag cells + 12 release-seal holdout cells).
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const SHARED_UI_ROOT = path.resolve(__dirname, '..');
const APPS_ROOT = path.resolve(SHARED_UI_ROOT, '..');
const SCENARIOS_PATH = path.join(SHARED_UI_ROOT, 'data', 'ui_nn_scenarios.json');

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
  throw new Error(`Chromium executable not found in candidates: ${CHROME_CANDIDATES.join(', ')}`);
}

function parseCliArgs(argv) {
  let split = 'diag';
  let releaseSeal = false;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--split' && argv[i + 1]) {
      split = argv[++i];
    } else if (argv[i] === '--release-seal') {
      releaseSeal = true;
    }
  }
  if (!['diag', 'holdout'].includes(split)) {
    console.error(`Invalid --split '${split}'. Use diag or holdout.`);
    process.exit(2);
  }
  if (split === 'holdout' && !releaseSeal) {
    console.error('REFUSING --split holdout without --release-seal (run diag until final gate).');
    process.exit(2);
  }
  return { split, releaseSeal };
}

function fnv1aUiDefsRev(rawBytes) {
  let h = 0xcbf29ce484222325n;
  const prime = 0x00000100000001b3n;
  const mask64 = 0xffffffffffffffffn;
  for (const b of rawBytes) {
    h ^= BigInt(b);
    h = (h * prime) & mask64;
  }
  const folded = Number((h ^ (h >> 32n)) & 0xffffffffn) >>> 0;
  return folded.toString(16).padStart(8, '0');
}

const MIME_TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
};

function startStaticServer() {
  const httpErrors = [];
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
    if (!fullPath.startsWith(APPS_ROOT)) {
      res.writeHead(403);
      res.end('Forbidden');
      return;
    }
    if (!fs.existsSync(fullPath) || fs.statSync(fullPath).isDirectory()) {
      httpErrors.push(`404 ${cleanUrl}`);
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
      resolve({ server, baseUrl: `http://127.0.0.1:${addr.port}`, httpErrors });
    });
  });
}

class CdpClient {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.events = [];
    ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(`CDP ${msg.error.code}: ${msg.error.message}`));
        else resolve(msg.result);
      } else if (msg.method) {
        this.events.push(msg);
      }
    });
  }

  static async connect(wsUrl) {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`WebSocket connect timeout: ${wsUrl}`)), 10000);
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
      const t = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP timeout on ${method}`));
      }, 15000);
      this.pending.set(id, {
        resolve: (v) => { clearTimeout(t); resolve(v); },
        reject: (e) => { clearTimeout(t); reject(e); },
      });
      this.ws.send(JSON.stringify(payload));
    });
  }

  async evalExpr(sessionId, expression) {
    const res = await this.send('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true,
    }, sessionId);
    if (res.exceptionDetails) {
      const desc = res.exceptionDetails.exception?.description || res.exceptionDetails.text || 'unknown JS exception';
      throw new Error(`Page eval error: ${desc}`);
    }
    return res.result?.value;
  }

  close() {
    try { this.ws.close(); } catch { /* ignore */ }
  }
}

async function launchChromium() {
  const chromeBin = findChromeBin();
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lv-ui-nn-cdp-'));
  const proc = spawn(chromeBin, [
    '--headless=new',
    '--remote-debugging-port=0',
    '--no-sandbox',
    '--disable-gpu',
    '--disable-dev-shm-usage',
    '--window-size=1280,820',
    `--user-data-dir=${userDataDir}`,
    'about:blank',
  ], { stdio: ['ignore', 'pipe', 'pipe'] });

  const wsUrl = await new Promise((resolve, reject) => {
    let stderrBuf = '';
    const timer = setTimeout(() => reject(new Error(`Chromium DevTools startup timeout. stderr:\n${stderrBuf}`)), 15000);
    proc.stderr.on('data', (chunk) => {
      stderrBuf += chunk.toString('utf8');
      const m = /DevTools listening on (ws:\/\/[^\s]+)/.exec(stderrBuf);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    });
    proc.on('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`Chromium exited prematurely (${code}). stderr:\n${stderrBuf}`));
    });
  });

  const cdp = await CdpClient.connect(wsUrl);
  return {
    cdp,
    async openPage(url) {
      const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank' });
      const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
      await cdp.send('Page.enable', {}, sessionId);
      await cdp.send('Runtime.enable', {}, sessionId);
      await cdp.send('Page.navigate', { url }, sessionId);
      for (let i = 0; i < 60; i++) {
        const ready = await cdp.evalExpr(sessionId, 'document.readyState === "complete"');
        if (ready) break;
        await new Promise((r) => setTimeout(r, 50));
      }
      await new Promise((r) => setTimeout(r, 150));
      return sessionId;
    },
    async teardown() {
      try { await cdp.send('Browser.close'); } catch { /* ignore */ }
      cdp.close();
      if (!proc.killed) {
        try { proc.kill('SIGTERM'); } catch { /* ignore */ }
      }
      await new Promise((r) => setTimeout(r, 200));
      try { fs.rmSync(userDataDir, { recursive: true, force: true }); } catch { /* ignore */ }
    },
  };
}

async function setupBrowserSurfaces(browser, baseUrl) {
  const indexSid = await browser.openPage(`${baseUrl}/louis/ui/index.html`);
  const studioSid = await browser.openPage(`${baseUrl}/louis/ui/studio.html`);
  const fieldSid = await browser.openPage(`${baseUrl}/louis/ui/field-lens.html`);
  const mobileSid = await browser.openPage(`${baseUrl}/louis/ui/mobile.html`);
  const visionSid = await browser.openPage(`${baseUrl}/vision/ui/index.html`);
  return { indexSid, studioSid, fieldSid, mobileSid, visionSid };
}

function assertCleanStaticHtml() {
  const htmlFiles = [
    path.join(APPS_ROOT, 'shared-ui', 'star-widget.html'),
    path.join(APPS_ROOT, 'shared-ui', 'pairing-modal.html'),
    path.join(APPS_ROOT, 'louis', 'ui', 'index.html'),
    path.join(APPS_ROOT, 'louis', 'ui', 'studio.html'),
    path.join(APPS_ROOT, 'louis', 'ui', 'field-lens.html'),
    path.join(APPS_ROOT, 'louis', 'ui', 'mobile.html'),
    path.join(APPS_ROOT, 'vision', 'ui', 'index.html'),
  ];
  for (const f of htmlFiles) {
    const raw = fs.readFileSync(f, 'utf8');
    const inlineScripts = [...raw.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)].filter((m) => m[2].trim().length > 0);
    assert.equal(inlineScripts.length, 0, `inline script body found in ${f}`);
    assert.equal(/\son[a-z]+\s*=/i.test(raw), false, `inline event handler found in ${f}`);
  }
}

async function runCellCheck(id, sids, cdp, baseUrl) {
  const { indexSid, studioSid, fieldSid, mobileSid, visionSid } = sids;
  switch (id) {
    case 'UI-01': {
      const st = await cdp.evalExpr(indexSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const r0 = await m.invoke('router_state');
        const r1 = await m.invoke('router_set_startup_order', { order: ['vision', 'louis'] });
        const domCount = document.querySelectorAll('#lv-star-root').length;
        return { r0Count: r0.activeStarWidgets, r1Count: r1.activeStarWidgets, owner: r1.owner, domCount };
      })()`);
      assert.equal(st.r0Count, 1);
      assert.equal(st.r1Count, 1);
      assert.equal(st.owner, 'vision_sim_sandbox');
      assert.equal(st.domCount, 1);
      break;
    }
    case 'UI-02': {
      const geom = await cdp.evalExpr(indexSid, `(async () => {
        const { placeWindow } = await import('/shared-ui/js/window-geom.js');
        const p1 = placeWindow({ width: 360, height: 56 }, { x: 0, y: 0, width: 1920, height: 1080 }, 1.5);
        const p2 = placeWindow({ width: 400, height: 600 }, { x: -1920, y: 0, width: 1920, height: 1080 }, 1.0);
        return { p1, p2 };
      })()`);
      assert.ok(geom.p1.x >= 0 && geom.p1.x + 540 <= 1920 && geom.p1.y >= 0 && geom.p1.y + 84 <= 1080);
      assert.ok(geom.p2.x >= -1920 && geom.p2.x + 400 <= 0 && geom.p2.y >= 0 && geom.p2.y + 600 <= 1080);
      break;
    }
    case 'UI-03': {
      const res = await cdp.evalExpr(studioSid, `(async () => {
        const { growsTo } = await import('/shared-ui/js/window-geom.js');
        const toFull = growsTo('pill', 'popover');
        const toPill = growsTo('popover', 'pill');
        return { toFull, toPill, hasGrid: !!document.querySelector('.studio-grid') };
      })()`);
      assert.equal(res.hasGrid, true);
      assert.equal(res.toFull, true);
      assert.equal(res.toPill, false);
      break;
    }
    case 'UI-04': {
      const rects = await cdp.evalExpr(fieldSid, `(() => {
        const btns = [...document.querySelectorAll('#glove-action-dock button')];
        return btns.map((b) => {
          const r = b.getBoundingClientRect();
          return { id: b.id, w: r.width, h: r.height };
        });
      })()`);
      assert.ok(rects.length >= 3);
      for (const b of rects) {
        assert.ok(b.w >= 56 && b.h >= 56, `glove button ${b.id} smaller than 56x56: ${b.w}x${b.h}`);
      }
      break;
    }
    case 'UI-05': {
      const res = await cdp.evalExpr(visionSid, `(() => {
        const stars = document.querySelectorAll('#lv-star-root').length;
        const btn = document.getElementById('lv-star-btn');
        const pop = document.getElementById('lv-chat-popover');
        const wasOpen = !pop.hidden;
        btn.click();
        const after1 = !pop.hidden;
        btn.click();
        const after2 = !pop.hidden;
        if (!after2) btn.click();
        return { stars, wasOpen, after1, after2 };
      })()`);
      assert.equal(res.stars, 1);
      assert.notEqual(res.after1, res.wasOpen);
      assert.equal(res.after2, res.wasOpen);
      break;
    }
    case 'UI-06': {
      const sw = await cdp.evalExpr(visionSid, `(async () => {
        const dtStage = await window.__visionStage.switchVisionMode('stage');
        const theme = document.body.dataset.theme;
        const mode = document.body.dataset.visionMode;
        const dtSim = await window.__visionStage.switchVisionMode('sim');
        return { dtStage, dtSim, theme, mode, restored: document.body.dataset.visionMode };
      })()`);
      assert.ok(sw.dtStage < 100 && sw.dtSim < 100, `switch took too long: ${JSON.stringify(sw)}`);
      assert.equal(sw.theme, 'stage-dark');
      assert.equal(sw.mode, 'stage');
      assert.equal(sw.restored, 'sim');
      break;
    }
    case 'UI-07': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { parseOmniQuery } = await import('/shared-ui/js/omni-bar.js');
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const q = parseOmniQuery('/docs dom:DOM-01 어제 정산 내역');
        const d1 = await m.invoke('router_route_input', { text: '/docs' });
        const d2 = await m.invoke('router_route_input', { text: '/lens' });
        const d3 = await m.invoke('router_route_input', { text: '/sim' });
        const d4 = await m.invoke('router_route_input', { text: '/stage' });
        return { q, d1, d2, d3, d4 };
      })()`);
      assert.ok(r.q.freeText.includes('정산 내역'));
      assert.equal(r.d1.owner, 'louis_docs_studio');
      assert.equal(r.d2.owner, 'louis_field_3d_lens');
      assert.equal(r.d3.mode, 'sim');
      assert.equal(r.d3.gpuLeaseTier, 'coexist_sim');
      assert.equal(r.d4.mode, 'stage');
      assert.equal(r.d4.gpuLeaseTier, 'stage_pinned_lock');
      break;
    }
    case 'UI-08': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        await m.invoke('set_device_role', { role: 'counter_kiosk' });
        const kioskLens = await m.invoke('router_route_input', { text: '/lens' });
        const kioskSql = await m.invoke('sql_execute', { sql: 'SELECT doc_id FROM docs_entities' });
        return { kioskLens, kioskSql };
      })()`);
      assert.equal(r.kioskLens.blocked, true);
      assert.equal(r.kioskLens.error, 'VaultError::KioskForbidden');
      assert.equal(r.kioskSql.ok, false);
      assert.equal(r.kioskSql.error, 'VaultError::KioskForbidden');
      break;
    }
    case 'UI-09': {
      const r = await cdp.evalExpr(studioSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const bad1 = await m.invoke('sql_execute', { sql: 'DROP TABLE docs_entities' });
        const bad2 = await m.invoke('sql_execute', { sql: 'SELECT secret_field FROM forbidden_tbl' });
        const good = await m.invoke('sql_execute', { sql: 'SELECT doc_id, amt_krw FROM docs_entities' });
        return { bad1, bad2, good };
      })()`);
      assert.equal(r.bad1.ok, false);
      assert.equal(r.bad2.ok, false);
      assert.equal(r.good.ok, true);
      assert.ok(r.good.wrappedSql.includes('vault_id'));
      assert.ok(r.good.wrappedSql.includes('LIMIT 200'));
      break;
    }
    case 'UI-10': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        return await m.invoke('router_route_input', { text: '명판 촬영 /lens' });
      })()`);
      assert.equal(r.owner, 'louis_field_3d_lens');
      break;
    }
    case 'UI-11': {
      const ok = await cdp.evalExpr(visionSid, `(() => {
        const pop = document.getElementById('lv-chat-popover');
        return !!pop.querySelector('#command-bar') && !!pop.querySelector('#unit-tag') && !!pop.querySelector('#command-input');
      })()`);
      assert.equal(ok, true);
      break;
    }
    case 'UI-12': {
      const r = await cdp.evalExpr(visionSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const q = await m.invoke('sql_execute', { sql: 'SELECT cue_id, delta_m, smpte FROM stage_cues' });
        return { q, hasReplayBtn: !!document.getElementById('btn-cue-replay') };
      })()`);
      assert.equal(r.q.ok, true);
      assert.equal(r.hasReplayBtn, true);
      break;
    }
    case 'UI-13': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { StarWidgetController } = await import('/shared-ui/js/star-widget.js');
        const ctl = new StarWidgetController();
        const items = ctl.mergeIncomingRows([
          { id: '1', created_at: 1, text: 'line 1' },
          { id: '2', created_at: 2, text: 'line 2' },
          { id: '3', created_at: 3, text: 'line 3' },
          { id: '4', created_at: 4, text: 'line 4' },
        ]);
        return items;
      })()`);
      assert.equal(r.length, 3);
      assert.equal(r[0].opacity, 0.55);
      assert.equal(r[1].opacity, 0.82);
      assert.equal(r[2].opacity, 1.0);
      assert.equal(r[0].isFeedTop, true);
      assert.equal(r[2].text, 'line 4');
      break;
    }
    case 'UI-14': {
      const r = await cdp.evalExpr(indexSid, `(() => {
        const star = document.getElementById('lv-star-btn');
        const pop = document.getElementById('lv-chat-popover');
        star.click();
        const openAfterClick = !pop.hidden;
        return { hasStar: !!star, openAfterClick, seqOk: typeof window.__louisSeq === 'undefined' || window.__louisSeq >= 0 };
      })()`);
      assert.equal(r.hasStar, true);
      assert.equal(r.openAfterClick, true);
      break;
    }
    case 'UI-15': {
      const r = await cdp.evalExpr(studioSid, `(() => {
        const sel = document.getElementById('studio-pack-select');
        const count = sel.options.length;
        sel.value = 'DOM-14';
        sel.dispatchEvent(new Event('change'));
        const badgeText = document.getElementById('studio-domain-badge').textContent;
        const stage = document.getElementById('crop-stage');
        // Toggle each off then back on
        ['tog-del', 'tog-strike', 'tog-luma180', 'tog-retry15'].forEach((id) => document.getElementById(id).click());
        const toggledOff = {
          luma: stage.classList.contains('forensic-gridlines'),
          retry: stage.classList.contains('retry-15'),
          del: !document.getElementById('crop-del-layer').hidden,
          strike: !document.getElementById('crop-strike-layer').hidden,
        };
        ['tog-del', 'tog-strike', 'tog-luma180', 'tog-retry15'].forEach((id) => document.getElementById(id).click());
        return {
          count,
          badgeText,
          toggledOff,
          lumaOn: stage.classList.contains('forensic-gridlines'),
          retryOn: stage.classList.contains('retry-15'),
          delVis: !document.getElementById('crop-del-layer').hidden,
          strikeVis: !document.getElementById('crop-strike-layer').hidden,
        };
      })()`);
      assert.equal(r.count, 18);
      assert.ok(r.badgeText.includes('DOM-14'));
      assert.deepEqual(r.toggledOff, { luma: false, retry: false, del: false, strike: false });
      assert.equal(r.lumaOn, true);
      assert.equal(r.retryOn, true);
      assert.equal(r.delVis, true);
      assert.equal(r.strikeVis, true);
      break;
    }
    case 'UI-16': {
      const r = await cdp.evalExpr(fieldSid, `(() => {
        const pip = document.getElementById('rectified-pip');
        document.getElementById('btn-glove-next').click();
        const w1 = pip.style.width;
        document.getElementById('btn-glove-next').click();
        const w2 = pip.style.width;
        return { w1, w2 };
      })()`);
      assert.equal(r.w1, '360px');
      assert.equal(r.w2, '220px');
      break;
    }
    case 'UI-17': {
      const r = await cdp.evalExpr(visionSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const route = await m.invoke('router_route_input', { text: '방금 리허설 최대 오차 알려줘' });
        const pop = document.getElementById('lv-chat-popover');
        return { route, hasLogFeed: !!pop.querySelector('#log-feed') };
      })()`);
      assert.equal(r.route.warmLeaseKind, 'speculative');
      assert.equal(r.route.ttlSec, 45);
      assert.equal(r.route.surfaceUnchanged, true);
      assert.equal(r.hasLogFeed, true);
      break;
    }
    case 'UI-18': {
      const r = await cdp.evalExpr(visionSid, `(async () => {
        await window.__visionStage.switchVisionMode('stage');
        window.__visionStage.triggerEmergencyBlackout();
        const st = window.__visionStage.getState();
        const hudVisible = !document.getElementById('stage-interlock-hud').hidden;
        window.__visionStage.resetSafety();
        return { st, hudVisible, datasetBlackout: st.blackoutActive };
      })()`);
      assert.equal(r.hudVisible, true);
      assert.equal(r.datasetBlackout, true);
      break;
    }
    case 'UI-19': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        await m.invoke('router_route_input', { text: '/docs' });
        const a = await m.invoke('router_state');
        await m.invoke('router_route_input', { text: '/stage' });
        const b = await m.invoke('router_state');
        return { aStar: a.activeStarWidgets, bStar: b.activeStarWidgets, bOwner: b.surfaceOwner };
      })()`);
      assert.equal(r.aStar, 1);
      assert.equal(r.bStar, 1);
      assert.equal(r.bOwner, 'vision_xr_stage');
      break;
    }
    case 'UI-20': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { StickyNumberer } = await import('/shared-ui/js/window-geom.js');
        const sn = new StickyNumberer();
        const f1 = sn.update([
          { id: 'a', source: 'dom', x: 100, y: 100, w: 80, h: 30 },
          { id: 'b', source: 'dom', x: 300, y: 100, w: 80, h: 30 },
        ], { nowMs: 1000 });
        const f2 = sn.update([
          { id: 'a', source: 'dom', x: 102, y: 101, w: 80, h: 30 },
          { id: 'b', source: 'dom', x: 301, y: 100, w: 80, h: 30 },
        ], { nowMs: 1100 });
        const fPriv = sn.update([{ id: 'a', source: 'dom', x: 100, y: 100, w: 80, h: 30 }], { nowMs: 1200, sensitive: true });
        return {
          s1: f1.badges.map((b) => [b.num, b.id]),
          s2: f2.badges.map((b) => [b.num, b.id]),
          privCount: fPriv.badges.length,
          privRecord: fPriv.record,
        };
      })()`);
      assert.deepEqual(r.s1, r.s2);
      assert.equal(r.s1.length, 2);
      assert.equal(r.privCount, 0);
      assert.equal(r.privRecord, false);
      break;
    }
    case 'UI-21': {
      const r = await cdp.evalExpr(studioSid, `(() => {
        const tiles = document.querySelectorAll('#a0-minimap .a0-tile').length;
        document.getElementById('btn-opc-jump').click();
        const opc = document.getElementById('opc-result').textContent;
        document.querySelector('.legend-sample').click();
        const leg = document.getElementById('legend-status').textContent;
        const dec1 = document.getElementById('l1-l3-decision').textContent;
        document.getElementById('btn-toggle-ambiguous').click();
        const dec2 = document.getElementById('l1-l3-decision').textContent;
        return { tiles, opc, leg, dec1, dec2 };
      })()`);
      assert.equal(r.tiles, 16);
      assert.ok(r.opc.includes('DC1CDU20AA02'));
      assert.ok(r.leg.includes('LLM 호출 0회'));
      assert.ok(r.dec1.includes('Identified') || r.dec2.includes('Identified'));
      assert.ok(r.dec1.includes('PendingOfficer') || r.dec2.includes('PendingOfficer'));
      break;
    }
    case 'UI-22': {
      const r = await cdp.evalExpr(fieldSid, `(async () => {
        const sel = document.getElementById('reposition-reason-select');
        const banners = [];
        for (const v of ['LowEffectiveDpi', 'SteepViewingAngle', 'SpecularGlareHoles', 'ClippedByFrameEdge']) {
          sel.value = v;
          sel.dispatchEvent(new Event('change'));
          banners.push(document.getElementById('reposition-banner').textContent);
        }
        sel.value = 'Pass';
        sel.dispatchEvent(new Event('change'));
        await new Promise((res) => setTimeout(res, 420));
        return { banners, freeze: document.getElementById('freeze-status').textContent };
      })()`);
      assert.equal(r.banners.length, 4);
      for (const b of r.banners) assert.ok(b.includes('VLM 호출 0회'));
      assert.ok(r.freeze.includes('AUTO-FREEZE LOCKED'));
      break;
    }
    case 'UI-23': {
      const r = await cdp.evalExpr(visionSid, `(() => {
        const oracle = document.getElementById('tg-oracle');
        const truth = document.getElementById('prov-badge-truth');
        const spec = document.getElementById('prov-badge-spec');
        const initTruthHidden = truth.hidden;
        const specVisible = !spec.hidden;
        oracle.setAttribute('aria-pressed', 'true');
        oracle.click();
        return { initTruthHidden, specVisible, afterClickExists: !!truth };
      })()`);
      assert.equal(r.initTruthHidden, true);
      assert.equal(r.specVisible, true);
      assert.equal(r.afterClickExists, true);
      break;
    }
    case 'UI-24': {
      const r = await cdp.evalExpr(visionSid, `(() => {
        document.getElementById('btn-stage-calib').click();
        const beforeDisabled = document.getElementById('btn-lock-zkey').disabled;
        document.getElementById('btn-capture-zbg').click();
        const afterCount = document.getElementById('stage-zbg-count').textContent;
        const afterDisabled = document.getElementById('btn-lock-zkey').disabled;
        document.getElementById('btn-lock-zkey').click();
        const msg = document.getElementById('stage-calib-msg').textContent;
        return { beforeDisabled, afterCount, afterDisabled, msg, scale: document.getElementById('stage-scale-val').textContent };
      })()`);
      assert.equal(r.beforeDisabled, true);
      assert.equal(r.afterCount, '30 / 30');
      assert.equal(r.afterDisabled, false);
      assert.ok(r.msg.includes('s = 1.000'));
      assert.ok(r.scale.includes('s = 1.000'));
      break;
    }
    case 'UI-25': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const b1 = await m.invoke('broadcast_privacy_epoch', { locked: true, reason: 'bank.exe' });
        const b2 = await m.invoke('router_state');
        return { b1, b2 };
      })()`);
      assert.ok(r.b1.epoch >= 1 && r.b1.locked === true && r.b1.stage3dScreenNonzeroBytes === 0);
      assert.ok(r.b2.privacyEpoch >= 1 && r.b2.privacyLocked === true);
      break;
    }
    case 'UI-26': {
      const r = await cdp.evalExpr(mobileSid, `(() => {
        document.getElementById('btn-wan-probe').click();
        const wanMsg = document.getElementById('mobile-wan-msg').textContent;
        document.getElementById('btn-trigger-grace').click();
        const graceOpen = !document.getElementById('grace-blurred').hidden;
        document.getElementById('btn-mobile-reconnect').click();
        const graceClosed = document.getElementById('grace-blurred').hidden;
        return { wanMsg, graceOpen, graceClosed };
      })()`);
      assert.ok(r.wanMsg.length > 0);
      assert.equal(r.graceOpen, true);
      assert.equal(r.graceClosed, true);
      break;
    }
    case 'UI-27': {
      const r = await cdp.evalExpr(studioSid, `(() => {
        const vSel = document.getElementById('gate-verdict-select');
        const fSel = document.getElementById('output-form-select');
        fSel.value = 'ApprovalNotice';
        vSel.value = 'G2RejectContradictionLock';
        vSel.dispatchEvent(new Event('change'));
        const snappedForm = fSel.value;
        const badgeShown = !document.getElementById('form-override-badge').hidden;

        document.getElementById('manual-amount-input').value = '85000001';
        document.getElementById('btn-check-amount').click();
        const amtMsg = document.getElementById('amount-guard-msg').textContent;

        document.getElementById('g3-kind-select').value = 'None';
        document.getElementById('btn-g3-approve').click();
        const g3Msg = document.getElementById('g3-approve-msg').textContent;

        document.getElementById('n2sf-src').value = 'Classified';
        document.getElementById('n2sf-dst').value = 'Open';
        document.getElementById('btn-test-reverse-cdr').click();
        const cdrMsg = document.getElementById('reverse-cdr-msg').textContent;
        return { snappedForm, badgeShown, amtMsg, g3Msg, cdrMsg };
      })()`);
      assert.equal(r.snappedForm, 'RejectNotice');
      assert.equal(r.badgeShown, true);
      assert.ok(r.amtMsg.includes('G2BlockUnprovenEdit'));
      assert.ok(r.g3Msg.includes('G3InterceptRubberStamp'));
      assert.ok(r.cdrMsg.includes('0 B'));
      break;
    }
    case 'UI-28': {
      const r = await cdp.evalExpr(fieldSid, `(() => {
        const hasStamp = !!document.getElementById('privacy-quad-stamp');
        const roleSel = document.getElementById('field-role-select');
        roleSel.value = 'CounterKiosk';
        roleSel.dispatchEvent(new Event('change'));
        const shieldOpen = !document.getElementById('kiosk-shield').hidden;
        document.getElementById('btn-kiosk-reset').click();

        document.getElementById('btn-loto-step3').click();
        const outOfOrder = document.getElementById('interlock-alert').textContent;
        document.getElementById('loto-step2-val').value = '60';
        document.getElementById('btn-loto-step2').click();
        document.getElementById('btn-loto-step3').click();
        const okMsg = document.getElementById('interlock-alert').textContent;
        return { hasStamp, shieldOpen, outOfOrder, okMsg };
      })()`);
      assert.equal(r.hasStamp, true);
      assert.equal(r.shieldOpen, true);
      assert.ok(r.outOfOrder.includes('OutOfOrder'));
      assert.ok(r.okMsg.includes('LOTO 완결'));
      break;
    }
    case 'UI-29': {
      const r = await cdp.evalExpr(visionSid, `(() => {
        document.getElementById('btn-show-io').click();
        const inp = document.getElementById('show-io-osc-addr');
        const err = document.getElementById('show-io-error');
        inp.value = '8.8.8.8:9000';
        document.getElementById('btn-validate-show-io').click();
        const badHidden = err.hidden;
        inp.value = '192.168.10.20:9000';
        document.getElementById('btn-validate-show-io').click();
        const okHidden = err.hidden;
        return { badHidden, okHidden };
      })()`);
      assert.equal(r.badHidden, false);
      assert.equal(r.okHidden, true);
      break;
    }
    case 'UI-30': {
      await cdp.evalExpr(visionSid, `window.__visionStage.switchVisionMode('stage')`);
      const goBlocked = await cdp.evalExpr(visionSid, `(() => {
        window.__visionStage.setInterlockViolation('metricZone', false);
        const dis = document.getElementById('btn-cue-go').disabled;
        window.__visionStage.resetSafety();
        return dis;
      })()`);
      assert.equal(goBlocked, true);
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'F12', code: 'F12', windowsVirtualKeyCode: 123 }, visionSid);
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'F12', code: 'F12', windowsVirtualKeyCode: 123 }, visionSid);
      const bo = await cdp.evalExpr(visionSid, `(() => {
        const s = window.__visionStage.getState();
        window.__visionStage.resetSafety();
        return s.blackoutActive;
      })()`);
      assert.equal(bo, true);
      break;
    }
    case 'UI-31': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const a = await m.invoke('router_route_input', { text: '리허설 최대 오차' });
        const st = await m.invoke('router_state');
        return { a, stars: st.activeStarWidgets };
      })()`);
      assert.equal(r.a.warmLeaseKind, 'speculative');
      assert.equal(r.a.ttlSec, 45);
      assert.equal(r.stars, 1);
      break;
    }
    case 'UI-32': {
      const r = await cdp.evalExpr(indexSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        await m.invoke('vision_switch_mode', { mode: 'sim' });
        return await m.invoke('router_state');
      })()`);
      assert.equal(r.surfaceOwner, 'vision_sim_sandbox');
      assert.equal(r.activeStarWidgets, 1);
      break;
    }
    case 'UI-33': {
      const r = await cdp.evalExpr(studioSid, `(() => {
        document.getElementById('btn-worm-verify').click();
        const okTxt = document.getElementById('worm-status').textContent;
        document.getElementById('btn-worm-tamper').click();
        const badTxt = document.getElementById('worm-status').textContent;
        document.getElementById('g3-cosign').checked = false;
        document.getElementById('btn-unmask-pii').click();
        const deniedTxt = document.getElementById('worm-status').textContent;
        document.getElementById('g3-cosign').checked = true;
        document.getElementById('btn-unmask-pii').click();
        const allowedTxt = document.getElementById('worm-status').textContent;
        return { okTxt, badTxt, deniedTxt, allowedTxt };
      })()`);
      assert.ok(r.okTxt.includes('1,825일'));
      assert.ok(r.badTxt.includes('broken_at seq=200'));
      assert.ok(r.deniedTxt.includes('거부'));
      assert.ok(r.allowedTxt.includes('PrivacyPermission3Y'));
      break;
    }
    case 'UI-34': {
      const r = await cdp.evalExpr(fieldSid, `(async () => {
        document.getElementById('loto-step2-val').value = '75';
        document.getElementById('btn-loto-step2').click();
        document.getElementById('btn-loto-step3').click();
        document.getElementById('btn-glove-commit').click();
        await new Promise((res) => setTimeout(res, 100));
        return document.getElementById('field-lock-us').textContent;
      })()`);
      assert.ok(r.includes('원장 커밋 완료'));
      break;
    }
    case 'UI-35': {
      const r = await cdp.evalExpr(visionSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        return await m.invoke('router_route_input', { text: '/sim' });
      })()`);
      assert.equal(r.gpuLeaseTier, 'coexist_sim');
      assert.ok(r.louisResidentMb <= 1150.0);
      break;
    }
    case 'UI-36': {
      await cdp.evalExpr(visionSid, `window.__visionStage.switchVisionMode('stage')`);
      const r = await cdp.evalExpr(visionSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        await m.invoke('router_route_input', { text: '/stage' });
        const st = await m.invoke('router_state');
        const pill = document.getElementById('frame-budget-pill').textContent;
        const bodyText = document.body.innerHTML;
        const hasSam = /\bSAM\b|MobileSAM|4종/.test(bodyText);
        return { st, pill, hasSam, starCount: document.querySelectorAll('#lv-star-root').length };
      })()`);
      assert.ok(r.st.louisResidentMb <= 32.0);
      assert.equal(r.st.louisIngestMode, 'disk_queue_only');
      assert.equal(r.st.visionVramSwaps, 0);
      assert.equal(r.st.activeStarWidgets, 1);
      assert.ok(r.pill.includes('StagePinnedLock'));
      assert.equal(r.hasSam, false);
      assert.equal(r.starCount, 1);
      break;
    }

    // 12 Sealed Holdout Scenarios (only executed with --split holdout --release-seal)
    case 'UI-ZH-00': {
      await runCellCheck('UI-01', sids, cdp, baseUrl);
      await runCellCheck('UI-32', sids, cdp, baseUrl);
      break;
    }
    case 'UI-ZH-01': {
      const r = await cdp.evalExpr(indexSid, `(() => {
        const star = document.getElementById('lv-star-btn');
        const pop = document.getElementById('lv-chat-popover');
        const inp = document.getElementById('lv-chat-input');
        if (pop.hidden) star.click();
        let composing = true;
        inp.value = 'ㄱ';
        inp.dispatchEvent(new CompositionEvent('compositionstart', { data: 'ㄱ' }));
        const ev = new KeyboardEvent('keydown', { key: 'Escape', isComposing: true, bubbles: true, cancelable: true });
        Object.defineProperty(ev, 'isComposing', { get: () => composing });
        inp.dispatchEvent(ev);
        const stillOpen = !pop.hidden;
        composing = false;
        inp.dispatchEvent(new CompositionEvent('compositionend', { data: '가' }));
        return { stillOpen };
      })()`);
      assert.equal(r.stillOpen, true);
      break;
    }
    case 'UI-ZH-02': {
      const r = await cdp.evalExpr(studioSid, `(async () => {
        const { MockDualBackend } = await import('/shared-ui/js/mock-dual-backend.js');
        const m = new MockDualBackend();
        const probes = [
          "SELECT read_file('/etc/passwd')",
          'SELECT a.doc_id FROM docs_entities a CROSS JOIN gate_ledger b',
          'SELECT col FROM unknown_table',
          'DELETE FROM docs_entities',
        ];
        const results = [];
        for (const sql of probes) results.push(await m.invoke('sql_execute', { sql }));
        return results;
      })()`);
      for (const x of r) assert.equal(x.ok, false);
      break;
    }
    case 'UI-ZH-03': {
      const r = await cdp.evalExpr(studioSid, `(() => {
        const vSel = document.getElementById('gate-verdict-select');
        const fSel = document.getElementById('output-form-select');
        const adverse = [
          ['G1ExpiredMandatoryReject', 'RejectNotice'],
          ['G1SignInvalidRetry', 'SupplementRequest'],
          ['G2RejectContradictionLock', 'RejectNotice'],
          ['G2RequireRepairReceipt', 'SupplementRequest'],
          ['G3HoldSimpleReceiptMissing', 'SupplementRequest'],
          ['G3ViolationRelatedVendorSplit', 'RejectNotice'],
          ['G3ViolationHighRisk', 'RejectNotice'],
          ['G3InterceptInterlockOrder', 'RejectNotice'],
        ];
        return adverse.map(([v, exp]) => {
          fSel.value = 'ApprovalNotice';
          vSel.value = v;
          vSel.dispatchEvent(new Event('change'));
          return { v, exp, got: fSel.value, badge: !document.getElementById('form-override-badge').hidden };
        });
      })()`);
      for (const row of r) {
        assert.equal(row.got, row.exp);
        assert.equal(row.badge, true);
      }
      break;
    }
    case 'UI-ZH-04': {
      const r = await cdp.evalExpr(studioSid, `(() => {
        const inp = document.getElementById('manual-amount-input');
        const btn = document.getElementById('btn-check-amount');
        const msg = document.getElementById('amount-guard-msg');
        inp.value = '85000001';
        btn.click();
        const over = msg.textContent;
        inp.value = '85000000';
        btn.click();
        const ok = msg.textContent;
        return { over, ok };
      })()`);
      assert.ok(r.over.includes('G2BlockUnprovenEdit'));
      assert.ok(r.ok.includes('적합'));
      break;
    }
    case 'UI-ZH-05': {
      const r = await cdp.evalExpr(studioSid, `(() => {
        const src = document.getElementById('n2sf-src');
        const dst = document.getElementById('n2sf-dst');
        const decl = document.getElementById('n2sf-declass-ref');
        const btn = document.getElementById('btn-test-reverse-cdr');
        const msg = document.getElementById('reverse-cdr-msg');

        src.value = 'Classified'; dst.value = 'Open'; decl.value = 'FORCE-BYPASS';
        btn.click();
        const c2o = msg.textContent;

        src.value = 'Sensitive'; dst.value = 'Open'; decl.value = '';
        btn.click();
        const s2oNoRef = msg.textContent;

        decl.value = 'N2SF-2026-0812';
        btn.click();
        const s2oOk = msg.textContent;
        return { c2o, s2oNoRef, s2oOk };
      })()`);
      assert.ok(r.c2o.includes('0 B'));
      assert.ok(r.s2oNoRef.includes('0 B'));
      assert.ok(r.s2oOk.includes('통과'));
      break;
    }
    case 'UI-ZH-06': {
      await runCellCheck('UI-33', sids, cdp, baseUrl);
      break;
    }
    case 'UI-ZH-07': {
      await runCellCheck('UI-08', sids, cdp, baseUrl);
      const shield = await cdp.evalExpr(fieldSid, `(() => {
        const sel = document.getElementById('field-role-select');
        sel.value = 'CounterKiosk';
        sel.dispatchEvent(new Event('change'));
        const active = !document.getElementById('kiosk-shield').hidden;
        document.getElementById('btn-kiosk-reset').click();
        return active;
      })()`);
      assert.equal(shield, true);
      break;
    }
    case 'UI-ZH-08': {
      await runCellCheck('UI-04', sids, cdp, baseUrl);
      const r = await cdp.evalExpr(fieldSid, `(() => {
        document.getElementById('loto-step2-val').value = '49';
        document.getElementById('btn-loto-step2').click();
        const step2Err = document.getElementById('interlock-alert').textContent;
        document.getElementById('btn-loto-step3').click();
        const step3Err = document.getElementById('interlock-alert').textContent;
        return { step2Err, step3Err };
      })()`);
      assert.ok(r.step2Err.includes('NotMet'));
      assert.ok(r.step3Err.includes('OutOfOrder'));
      break;
    }
    case 'UI-ZH-09': {
      await runCellCheck('UI-26', sids, cdp, baseUrl);
      const skins = await cdp.evalExpr(mobileSid, `(() => {
        const sel = document.getElementById('mobile-skin-select');
        return [...sel.options].map((o) => o.value);
      })()`);
      assert.ok(skins.length >= 4);
      break;
    }
    case 'UI-ZH-10': {
      await runCellCheck('UI-23', sids, cdp, baseUrl);
      await runCellCheck('UI-29', sids, cdp, baseUrl);
      break;
    }
    case 'UI-ZH-11': {
      await runCellCheck('UI-30', sids, cdp, baseUrl);
      await runCellCheck('UI-36', sids, cdp, baseUrl);
      break;
    }
    default:
      throw new Error(`No handler registered for UI scenario ${id}`);
  }
}

async function main() {
  const { split, releaseSeal } = parseCliArgs(process.argv.slice(2));
  const raw = fs.readFileSync(SCENARIOS_PATH);
  const defs = JSON.parse(raw.toString('utf8'));
  const uiDefsRev = fnv1aUiDefsRev(raw);

  assertCleanStaticHtml();
  assert.equal(defs.schema, 'louis-ui-nn/v1');

  const cases = split === 'diag' ? defs.scenarios : defs.holdout_scenarios;
  const expectedCount = split === 'diag' ? 36 : 12;
  assert.equal(cases.length, expectedCount, `Expected ${expectedCount} ${split} cases, found ${cases.length}`);

  const { server, baseUrl, httpErrors } = await startStaticServer();
  const browser = await launchChromium();
  try {
    const sids = await setupBrowserSurfaces(browser, baseUrl);
    const passedIds = [];
    const failures = [];
    for (const sc of cases) {
      try {
        await runCellCheck(sc.id, sids, browser.cdp, baseUrl);
        passedIds.push(sc.id);
      } catch (err) {
        failures.push({ id: sc.id, error: String(err?.stack || err) });
      }
    }
    if (httpErrors.length > 0) {
      failures.push({ id: 'HTTP_ASSETS', error: `404/asset errors: ${httpErrors.join(', ')}` });
    }
    const pageExceptions = browser.cdp.events.filter((e) => e.method === 'Runtime.exceptionThrown');
    if (pageExceptions.length > 0) {
      failures.push({ id: 'RUNTIME_EXCEPTIONS', error: JSON.stringify(pageExceptions[0]) });
    }

    const report = {
      schema: defs.schema,
      ui_defs_rev: uiDefsRev,
      split,
      release_seal: releaseSeal,
      passed: passedIds.length,
      total: cases.length,
      hard_gate_hits: failures.length,
      failures,
    };
    console.log(JSON.stringify(report, null, 2));
    if (failures.length > 0 || passedIds.length !== expectedCount) {
      process.exitCode = 1;
    }
  } finally {
    await browser.teardown();
    server.close();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
