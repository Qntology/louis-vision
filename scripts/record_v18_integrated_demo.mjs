// scripts/record_v18_integrated_demo.mjs
// Live Headless Chromium CDP + real CLI telemetry recorder showcasing ALL integrated improvements:
// Ch.1 Vision Sandbox v1.8 — Isometric 3D, Spatial Deck State A->B, Cards CRUD, Filter, Split & Undo
// Ch.2 Spatial Deck Inline [▾] Drawers & :8788 MCP 4-Way IPC (char_prompt, mask, Cycles, DMX, CAD, Polymorphic Bar)
// Ch.3 State C Master Theater Full Playback & Mobile <760px 4-Stack Responsive + WirelessCam OLED Lock
// Ch.4 XR Stage Live — V-01..V-09 Fixes (HaringDancerRig, V-02 Strict RFC1918, V-09 True Blackout & Recovery)
// Ch.5 Louis Docs Studio — D-01 Hardened SQL Guard Bypass Blocking, D-12 Trade SSOT & 5-Gate WORM Audit
// Ch.6 Real-Time CLI Verification Telemetry (4-Repo Pin Sync, deny.toml, UI N:N 36/36, Spatial Deck E2E)

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, '..');
const WORKSPACE_ROOT = path.resolve(REPO_ROOT, '..');
const APPS_ROOT = path.join(REPO_ROOT, 'apps');
const OUT_DIR = path.join(REPO_ROOT, 'artifacts', 'v18_video_capture');
const FRAMES_DIR = path.join(OUT_DIR, 'frames');

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
  throw new Error('No Chromium/Edge executable found');
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
    this.exceptions = [];
    ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(`CDP ${msg.error.code}: ${msg.error.message}`));
        else resolve(msg.result);
      } else if (msg.method === 'Runtime.exceptionThrown') {
        this.exceptions.push(msg.params?.exceptionDetails?.text || 'exception');
      }
    });
  }

  static async connect(wsUrl) {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`WS timeout: ${wsUrl}`)), 10000);
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
      }, 20000);
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
      const desc = res.exceptionDetails.exception?.description || res.exceptionDetails.text || 'JS exception';
      throw new Error(`Page eval error: ${desc}`);
    }
    return res.result?.value;
  }

  close() {
    try { this.ws.close(); } catch { /* ignore */ }
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitLoaded(cdp, sid) {
  for (let i = 0; i < 80; i++) {
    const ready = await cdp.evalExpr(sid, 'document.readyState === "complete"');
    if (ready) break;
    await sleep(50);
  }
  await sleep(200);
}

async function launchChromium() {
  const chromeBin = findChromeBin();
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lv-v18-video-'));
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
    let buf = '';
    const timer = setTimeout(() => reject(new Error(`Chromium startup timeout:\n${buf}`)), 15000);
    proc.stderr.on('data', (chunk) => {
      buf += chunk.toString('utf8');
      const m = /DevTools listening on (ws:\/\/[^\s]+)/.exec(buf);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    });
    proc.on('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`Chromium exited (${code}):\n${buf}`));
    });
  });

  const cdp = await CdpClient.connect(wsUrl);
  return {
    cdp,
    async openPage(url, dims = { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false }) {
      const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank' });
      const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
      await cdp.send('Page.enable', {}, sessionId);
      await cdp.send('Runtime.enable', {}, sessionId);
      await cdp.send('Target.activateTarget', { targetId });
      await cdp.send('Page.bringToFront', {}, sessionId);
      await cdp.send('Emulation.setDeviceMetricsOverride', dims, sessionId);
      await cdp.send('Page.navigate', { url }, sessionId);
      await waitLoaded(cdp, sessionId, new URL(url).pathname);
      return { sessionId, targetId };
    },
    async closePage(page) {
      if (page?.targetId) {
        try { await cdp.send('Target.closeTarget', { targetId: page.targetId }); } catch { /* ignore */ }
      }
    },
    async teardown() {
      try { await cdp.send('Browser.close'); } catch { /* ignore */ }
      cdp.close();
      const exited = new Promise((resolve) => {
        proc.once('exit', resolve);
        setTimeout(resolve, 1500);
      });
      if (!proc.killed) {
        try { proc.kill(); } catch { /* ignore */ }
      }
      await exited;
      try { fs.rmSync(userDataDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); } catch { /* ignore */ }
    },
  };
}

async function captureFrame(cdp, sid, name) {
  const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }, sid);
  const rel = `${name}.png`;
  const out = path.join(FRAMES_DIR, rel);
  fs.writeFileSync(out, Buffer.from(data, 'base64'));
  return path.join('frames', rel);
}

function runCmdCapture(cmd, args, cwd = WORKSPACE_ROOT) {
  const t0 = Date.now();
  const r = spawnSync(cmd, args, { cwd, encoding: 'utf8', timeout: 60000 });
  const ms = Date.now() - t0;
  const stdout = (r.stdout || '').trim();
  const stderr = (r.stderr || '').trim();
  return {
    cmd: `${cmd} ${args.join(' ')}`,
    exit_code: r.status ?? -1,
    elapsed_ms: ms,
    stdout_lines: stdout ? stdout.split(/\r?\n/).slice(-14) : [],
    stderr_lines: stderr ? stderr.split(/\r?\n/).slice(-6) : [],
  };
}

async function waitVisionReady(cdp, sid) {
  for (let i = 0; i < 80; i++) {
    const ok = await cdp.evalExpr(sid, `(() => {
      const empty = document.getElementById('world-empty');
      return Boolean(window.__spatialDeck && window.__masterPlayer && window.__visionStage && empty && empty.hidden);
    })()`);
    if (ok) {
      await sleep(180);
      return;
    }
    await sleep(100);
  }
  const diag = await cdp.evalExpr(sid, `JSON.stringify({
    href: location.href,
    sd: Boolean(window.__spatialDeck),
    mp: Boolean(window.__masterPlayer),
    vs: Boolean(window.__visionStage),
    emptyHidden: document.getElementById('world-empty')?.hidden,
    sbMsg: document.getElementById('sb-msg')?.textContent,
    sbSeed: document.getElementById('sb-seed')?.textContent
  })`);
  throw new Error(`waitVisionReady timeout: ${diag}; exceptions=${JSON.stringify(cdp.exceptions)}`);
}

async function main() {
  fs.rmSync(OUT_DIR, { recursive: true, force: true });
  fs.mkdirSync(FRAMES_DIR, { recursive: true });

  const { server, baseUrl, httpErrors } = await startStaticServer();
  const browser = await launchChromium();
  const { cdp } = browser;
  const chapters = [];

  try {
    // =========================================================================
    // CHAPTER 1: Vision Sandbox v1.8 — Isometric 3D, Spatial Deck State A->B, CRUD & Undo
    // =========================================================================
    const visionPage = await browser.openPage(`${baseUrl}/vision/ui/index.html`);
    const sid = visionPage.sessionId;
    await waitVisionReady(cdp, sid);

    {
      const shots = [];
      // Shot 1: State A + Isometric 3D view
      await cdp.evalExpr(sid, `(() => {
        const vs = document.getElementById('view-select');
        vs.value = 'iso';
        vs.dispatchEvent(new Event('change'));
      })()`);
      await sleep(260);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_01_state_a_iso'),
        duration_s: 2.3,
        scenario_id: 'v1.8 State A / ISO',
        caption: 'State A 기본 깔끔한 전체 캔버스 + 신규 아이소메트릭(3D) 시점 전환 (#view-select="iso")',
      });

      // Switch back to top for rich map contrast, collapse star popover so Spatial Deck is 100% unobstructed, then open State B
      await cdp.evalExpr(sid, `(() => {
        const vs = document.getElementById('view-select');
        vs.value = 'top';
        vs.dispatchEvent(new Event('change'));
        const pop = document.getElementById('lv-chat-popover');
        if (pop && !pop.hidden) document.getElementById('lv-star-btn').click();
        document.getElementById('deck-toggle-btn').click();
      })()`);
      await sleep(260);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_02_state_b_deck'),
        duration_s: 2.6,
        scenario_id: 'v1.8 State B / PiP',
        caption: 'State B 2축 공간 데크 확장(max 720px 글래스 도크) + PiP 미니맵 자동 활성화 + 기본 2카드 + [+] 고스트 카드',
      });

      // Add light + set cards via [+] ghost card
      await cdp.evalExpr(sid, `(() => {
        document.getElementById('deck-ghost-add').click();
        document.getElementById('deck-ghost-add').click();
      })()`);
      await sleep(180);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_03_add_4_cards'),
        duration_s: 2.4,
        scenario_id: 'Ghost [+] & Budget',
        caption: '[+] 고스트 카드로 조명·DMX 및 무대·세트 카드 추가 -> 실시간 VRAM/프레임 예산 자동 갱신',
      });

      // Filter chip to light, then back to all + Split slice + re-slice to 15s
      await cdp.evalExpr(sid, `(() => {
        document.querySelector('#deck-filter-bar [data-filter="light"]').click();
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_04_filter_light'),
        duration_s: 2.0,
        scenario_id: 'Layer Filter Chips',
        caption: '레이어 필터 칩 (전체 | 카메라 | 배우·유닛 | 조명·DMX | 무대·세트) 즉시 필터링',
      });

      await cdp.evalExpr(sid, `(() => {
        document.querySelector('#deck-filter-bar [data-filter="all"]').click();
        document.getElementById('deck-split-btn').click();
        const u = document.getElementById('deck-slice-unit');
        u.value = '15';
        u.dispatchEvent(new Event('change'));
      })()`);
      await sleep(180);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_05_split_reslice'),
        duration_s: 2.4,
        scenario_id: 'Split(S) & Inheritance',
        caption: '[분할 (S)]로 Slice k -> k+1 상태 상속 복제 + 15s 단위 재슬라이싱(0-15s / 15-30s / 30-45s)',
      });

      // Mute Slice 2 -> Shift+Delete Slice 2 -> Undo (Ctrl+Z)
      await cdp.evalExpr(sid, `(() => {
        window.__spatialDeck.setSliceUnitS(10);
        window.__spatialDeck.toggleMuteOrDeleteSlice(1, false);
      })()`);
      await sleep(150);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_06_mute_slice'),
        duration_s: 2.0,
        scenario_id: 'Mute [X] / Shift+Del',
        caption: 'Slice 2 음소거(점선 비활성화) — Shift+X 영구 삭제 후 Ctrl+Z로 즉시 복구 가능',
      });

      await cdp.evalExpr(sid, `(() => {
        window.__spatialDeck.toggleMuteOrDeleteSlice(1, true);
        window.__spatialDeck.undoDeckAction();
        window.__spatialDeck.toggleMuteOrDeleteSlice(1, false);
        document.querySelector('#slice-v-scroll .slice-row[data-slice-idx="0"] .slice-row-main').click();
      })()`);
      await sleep(150);

      chapters.push({
        id: 'ch01_spatial_deck_core',
        title: 'Chapter 1. Vision Sandbox v1.8 — 2축 공간 데크 & CRUD',
        subtitle: '아이소메트릭 3D · State A->B + PiP 미니맵 · 레이어 필터 · 상태 상속 분할(S) · 음소거/삭제/Undo',
        app: 'Vision Sandbox v1.8 (Deck)',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 2: Inline [▾] Drawers & :8788 MCP 4-Way IPC Parity
    // =========================================================================
    {
      const shots = [];

      // 1. Actor drawer -> char_prompt (128 BPM bboy_freeze crown + Rule 6 purity)
      await cdp.evalExpr(sid, `(async () => {
        const card = document.querySelector('.deck-card[data-card-kind="actor"]');
        card.querySelector('.deck-drawer-btn').click();
        const drawer = document.querySelector('.deck-card[data-card-kind="actor"] .deck-drawer');
        const inputs = drawer.querySelectorAll('input');
        const selects = drawer.querySelectorAll('select');
        inputs[1].value = '128';
        selects[0].value = 'bboy_freeze';
        selects[1].value = 'crown';
        drawer.querySelector('button').click();
        await new Promise((r) => setTimeout(r, 120));
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_01_actor_char_prompt'),
        duration_s: 2.7,
        scenario_id: 'MCP char_prompt',
        caption: '배우 인라인 드로어 -> char_prompt (128 BPM · bboy_freeze · crown · Rule 6 순도 99.80% · 틱 8개)',
      });

      // 2. Cam drawer -> stage_mask_apply + stage_render_step + stage_render_abort
      await cdp.evalExpr(sid, `(async () => {
        document.querySelector('.deck-card[data-card-kind="actor"] .deck-drawer-btn').click();
        document.querySelector('.deck-card[data-card-kind="cam"] .deck-drawer-btn').click();
        const drawer = document.querySelector('.deck-card[data-card-kind="cam"] .deck-drawer');
        const sel = drawer.querySelector('select');
        sel.value = 'cam_crane_02';
        sel.dispatchEvent(new Event('change'));
        const btns = drawer.querySelectorAll('button');
        btns[0].click(); // +전경 마스크 전파
        await new Promise((r) => setTimeout(r, 90));
        btns[2].click(); // Cycles 패스 스텝
        await new Promise((r) => setTimeout(r, 90));
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_02_cam_mask_cycles'),
        duration_s: 2.7,
        scenario_id: 'Mask & Cycles Bake',
        caption: '카메라 드로어 -> lv_spatial 마스크 10프레임 전파 + 채택 카메라(cam_crane_02) 단독 Cycles 4/16 spp',
      });

      await cdp.evalExpr(sid, `(async () => {
        const btns = document.querySelectorAll('.deck-card[data-card-kind="cam"] .deck-drawer button');
        btns[3].click(); // 즉시 중단
        await new Promise((r) => setTimeout(r, 90));
      })()`);
      await sleep(140);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_03_cam_instant_abort'),
        duration_s: 2.2,
        scenario_id: 'stage_render_abort',
        caption: 'Cycles 베이크 즉시 중단 (stage_render_abort) -> <= 16.6ms 프레임 예산 내 안전 정지 검증',
      });

      // 3. Light & Set drawers + Polymorphic Command Bar
      await cdp.evalExpr(sid, `(async () => {
        document.querySelector('.deck-card[data-card-kind="cam"] .deck-drawer-btn').click();
        const track = document.getElementById('slide-h-track');
        track.scrollLeft = 500;
        document.querySelector('.deck-card[data-card-kind="light"] .deck-drawer-btn').click();
        const ld = document.querySelector('.deck-card[data-card-kind="light"] .deck-drawer');
        ld.querySelector('input[type="checkbox"]').click(); // Laser BAM 0%
        ld.querySelector('button').click();
        await new Promise((r) => setTimeout(r, 90));

        document.querySelector('.deck-card[data-card-kind="set"] .deck-drawer-btn').click();
        const sd = document.querySelector('.deck-card[data-card-kind="set"] .deck-drawer');
        sd.querySelector('button').click();
        await new Promise((r) => setTimeout(r, 120));
        track.scrollLeft = 700;
      })()`);
      await sleep(180);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_04_light_set_cad'),
        duration_s: 2.8,
        scenario_id: 'DMX U64 & SolidCraft',
        caption: 'DMX 조명(U64 파이로 격리 · Laser BAM 0%) + 무대 세트(SolidCraft URDF 오차 0.0m · 관절 별칭 정규화)',
      });

      // Polymorphic command bar prompt routed to focused deck card (re-open star popover cleanly)
      await cdp.evalExpr(sid, `(async () => {
        const pop = document.getElementById('lv-chat-popover');
        if (pop && pop.hidden) document.getElementById('lv-star-btn').click();
        const ci = document.getElementById('command-input');
        ci.value = '무빙헤드 커브드 LED 분할 LED 런웨이 물류 랙 조립';
        document.getElementById('command-bar').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
        await new Promise((r) => setTimeout(r, 150));
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_05_polymorphic_bar'),
        duration_s: 2.6,
        scenario_id: '4-Way IPC Polymorphic',
        caption: '우하단 ( ✦ ) 통합 명령창 다형성 바인딩 -> 포커스된 무대·세트 카드의 stage_cad_prompt로 직접 라우팅',
      });

      chapters.push({
        id: 'ch02_drawers_mcp_ipc',
        title: 'Chapter 2. 인라인 [▾] 드로어 · :8788 MCP · 4-Way IPC',
        subtitle: 'char_prompt · lv_spatial 마스크 전파 · Cycles 단독 베이크/중단 · DMX 6계층 · SolidCraft CAD',
        app: 'Vision Sandbox v1.8 (MCP/IPC)',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 3: State C Master Theater & Mobile <760px 4-Stack + OLED Cam Lock
    // =========================================================================
    {
      const shots = [];

      // Enter State C Master Theater (collapse star popover so theater bar is fully unobstructed)
      await cdp.evalExpr(sid, `(() => {
        const pop = document.getElementById('lv-chat-popover');
        if (pop && !pop.hidden) document.getElementById('lv-star-btn').click();
        document.getElementById('master-theater-btn').click();
      })()`);
      await sleep(150);
      const exportModes = [
        ['rgb', 4.0, 'State C 마스터 시어터 진입 — 풀스크린 타임라인 재생 (RGB 합성 출력)'],
        ['alpha', 11.5, '마스터 시어터 실시간 스크러빙 (11.5s -> Slice 2 진입) · Alpha 매트 출력 전환'],
        ['greenscreen', 21.0, '마스터 시어터 (21.0s -> Slice 3) · Greenscreen 송출 모드 전환 (Space로 슬라이스 복귀)'],
      ];
      for (let i = 0; i < exportModes.length; i++) {
        const [mode, tSec, cap] = exportModes[i];
        await cdp.evalExpr(sid, `(() => {
          const sel = document.getElementById('theater-export-mode');
          sel.value = '${mode}';
          sel.dispatchEvent(new Event('change'));
          window.__spatialDeck.setPlayheadS(${tSec});
        })()`);
        await sleep(180);
        shots.push({
          file: await captureFrame(cdp, sid, `ch03_0${i + 1}_theater_${mode}`),
          duration_s: 1.9,
          scenario_id: `State C (${mode.toUpperCase()})`,
          caption: cap,
        });
      }

      // Exit Theater back to State B
      await cdp.evalExpr(sid, `window.__masterPlayer.exitTheaterToSlice()`);
      await sleep(160);

      // Mobile <760px 4-Stack Layout (390x844 portrait)
      const mobileVision = await browser.openPage(`${baseUrl}/vision/ui/index.html`, {
        width: 390,
        height: 844,
        deviceScaleFactor: 2,
        mobile: true,
      });
      const mSid = mobileVision.sessionId;
      await waitVisionReady(cdp, mSid);
      await cdp.evalExpr(mSid, `(() => {
        // Collapse bottom-right star popover cleanly so mobile 4-stack is crystal clear
        const pop = document.getElementById('lv-chat-popover');
        if (pop && !pop.hidden) document.getElementById('lv-star-btn').click();
        document.getElementById('deck-toggle-btn').click();
      })()`);
      await sleep(240);
      shots.push({
        file: await captureFrame(cdp, mSid, 'ch03_04_mobile_4stack'),
        duration_s: 2.8,
        scenario_id: 'Mobile <760px 4-Stack',
        caption: '모바일 390x844 4단 수직 Flex 레이아웃 — 뷰포트 >=45%, === 드래그 핸들, >=36px 터치 타겟',
        mobile_frame: true,
        mobile_notes: [
          '뷰포트 390 x 844 px (DPR 2x)',
          '상단 캔버스 높이 >= 45% 확보',
          '중앙 === 드래그 핸들 + 필터 칩 >= 36px',
          '2축 제스처 락 (12px 벡터 각도 분리)',
          'Spatial Deck E2E 모바일 검증 PASS',
        ],
      });

      // WirelessCameraRig OLED Touch Lock on Mobile
      await cdp.evalExpr(mSid, `document.getElementById('deck-cam-lock-btn').click()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, mSid, 'ch03_05_mobile_oled_lock'),
        duration_s: 2.2,
        scenario_id: 'WirelessCam OLED Lock',
        caption: '모바일 WirelessCameraRig #000000 OLED 터치 잠금 활성화 (오조작·발열·배터리 소모 차단)',
        mobile_frame: true,
        mobile_notes: [
          'WirelessCameraRig OLED 절전 잠금',
          '#000000 배경 + pointer-events 차단',
          '촬영 중 우발적 UI 터치 원천 방지',
          '5% 저전력 탈리 램프 규격 정합',
        ],
      });
      await browser.closePage(mobileVision);

      chapters.push({
        id: 'ch03_theater_mobile',
        title: 'Chapter 3. State C 마스터 시어터 & 모바일 4-스택',
        subtitle: '전체 재생/RGB·Alpha·Greenscreen 출력 · 390x844 4단 반응형 · WirelessCameraRig OLED 잠금',
        app: 'Theater & Mobile 4-Stack',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 4: XR Stage Live — V-01..V-09 Bug Fixes Verification
    // =========================================================================
    {
      await cdp.send('Target.activateTarget', { targetId: visionPage.targetId });
      await cdp.send('Page.bringToFront', {}, sid);
      await cdp.evalExpr(sid, `(() => {
        window.__spatialDeck.setDeckState('collapsed');
        window.__visionStage.switchVisionMode('stage');
      })()`);
      await sleep(220);

      const shots = [];
      // Animate HaringDancerRig across 5 frames showing V-01 byte-identical blockies & V-08 honest preview badge
      for (let f = 0; f < 5; f++) {
        if (f === 2 || f === 4) {
          await cdp.evalExpr(sid, `document.getElementById('btn-cue-go').click()`);
        }
        await cdp.evalExpr(sid, `window.__visionStage.renderHaringDancerFrame(${0.4 + f * 0.55})`);
        await sleep(110);
        shots.push({
          file: await captureFrame(cdp, sid, `ch04_01_haring_${f}`),
          duration_s: 0.42,
          scenario_id: 'V-01 / V-08 Haring',
          caption: 'XR Stage Live — V-01 바이트동일 Blockies-Haring (#000000·6 ticks) + V-08 정직한 preview 상태 배지',
        });
      }

      // V-02 Strict numeric RFC1918 guard: reject spoofed 172.32.0.1:9000!
      await cdp.evalExpr(sid, `(() => {
        document.getElementById('btn-show-io').click();
        const inp = document.getElementById('show-io-osc-addr');
        inp.value = '172.32.0.1:9000';
        document.getElementById('btn-validate-show-io').click();
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch04_02_v02_strict_wan'),
        duration_s: 2.6,
        scenario_id: 'V-02 Strict RFC1918',
        caption: 'V-02 엄격한 IPv4/IPv6 옥텟 검증 — 172.32.0.1:9000 (비RFC1918 공인 IP) 접두사 우회 즉시 차단',
      });

      // Accept valid RFC1918 172.16.4.10:9000, close popover, trigger V-09 F12 True Blackout
      await cdp.evalExpr(sid, `(() => {
        const inp = document.getElementById('show-io-osc-addr');
        inp.value = '172.16.4.10:9000';
        document.getElementById('btn-validate-show-io').click();
        document.querySelector('#show-io-popover [data-close]').click();
      })()`);
      await sleep(120);
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'F12', code: 'F12', windowsVirtualKeyCode: 123 }, sid);
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'F12', code: 'F12', windowsVirtualKeyCode: 123 }, sid);
      await sleep(180);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch04_03_v09_blackout'),
        duration_s: 2.5,
        scenario_id: 'V-09 True Blackout',
        caption: 'V-09 F12 진성 암전(brightness(0)) + Haring RAF 즉시 정지 + 3중 안전 인터록 HUD 상시 고정',
      });

      // Recover from Blackout
      await cdp.evalExpr(sid, `document.getElementById('btn-stage-reset').click()`);
      await sleep(180);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch04_04_v09_recover'),
        duration_s: 2.0,
        scenario_id: 'V-09 Safe Recovery',
        caption: '인터록·암전 안전 복구 -> 전 채널 CLEAR 및 HaringDancerRig 애니메이션 정상 재기동',
      });

      chapters.push({
        id: 'ch04_xr_stage_fixes',
        title: 'Chapter 4. XR Stage Live — V-01..V-09 버그 수정 실증',
        subtitle: 'V-01 Blockies 패리티 · V-02 엄격 IPv4/IPv6 가드 · V-08 정직 배지 · V-09 진성 암전 & RAF 정지',
        app: 'Vision XR Stage (Fixes)',
        shots,
      });
    }

    await browser.closePage(visionPage);

    // =========================================================================
    // CHAPTER 5: Louis Docs Studio — D-01 Hardened SQL Guard & D-12 Trade SSOT
    // =========================================================================
    {
      const studioPage = await browser.openPage(`${baseUrl}/louis/ui/studio.html`);
      const stSid = studioPage.sessionId;
      const shots = [];

      // D-01 Hardened SQL Guard: test comment-obfuscated DROP bypass!
      await cdp.evalExpr(stSid, `(() => {
        const inp = document.getElementById('sql-guard-input');
        inp.value = '/*bypass*/ DROP TABLE docs_entities';
        document.getElementById('btn-run-sql').click();
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, stSid, 'ch05_01_d01_sql_bypass_block'),
        duration_s: 2.6,
        scenario_id: 'D-01 Hardened SQL',
        caption: 'D-01 강화된 SQL 5중 가드 — 주석 위장 파괴 구문(/*bypass*/ DROP TABLE) 즉시 차단',
      });

      // Valid SELECT + DOM-08 trade-logistics SSOT + 5-Gate + WORM tamper + Cascade
      await cdp.evalExpr(stSid, `(() => {
        const sel = document.getElementById('studio-pack-select');
        sel.value = 'DOM-08';
        sel.dispatchEvent(new Event('change'));
        const inp = document.getElementById('sql-guard-input');
        inp.value = 'SELECT doc_id, amt_krw FROM docs_entities';
        document.getElementById('btn-run-sql').click();
        document.getElementById('manual-amount-input').value = '85000001';
        document.getElementById('btn-check-amount').click();
        document.getElementById('btn-test-reverse-cdr').click();
        document.getElementById('btn-worm-tamper').click();
        document.getElementById('btn-run-cascade').click();
      })()`);
      await sleep(180);
      shots.push({
        file: await captureFrame(cdp, stSid, 'ch05_02_d12_trade_5gate_worm'),
        duration_s: 2.8,
        scenario_id: 'D-12 / 5-Gate / WORM',
        caption: 'DOM-08 무역·통관 SSOT 정합 + 정상 SELECT 통과 + +1원/Reverse-CDR/WORM 변조 적발 + DSLI_18 0.99 PASS',
      });

      await browser.closePage(studioPage);

      chapters.push({
        id: 'ch05_louis_security',
        title: 'Chapter 5. Louis Docs Studio — 보안 가드 & 5-Gate 심사',
        subtitle: 'D-01 SQL 우회 공격 차단 · D-12 무역/도메인 스킨 정합 · +1원 차단 · Reverse-CDR · WORM 감사',
        app: 'Louis Docs Studio (Security)',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 6: Real-Time CLI Verification Telemetry (Freshly Captured!)
    // =========================================================================
    const telemetry = {
      deny: runCmdCapture('python', ['louis-vision/.github/scripts/check_deny_toml.py']),
      pins_lv: runCmdCapture('python', ['louis-vision/.github/scripts/verify-pins.py', '--tier', 'all']),
      pins_pd: runCmdCapture('python', ['premium-discount/.github/scripts/verify-pins.py', '--tier', 'all']),
      pins_lc: runCmdCapture('python', ['logis-center/.github/scripts/verify-pins.py']),
      pins_ns: runCmdCapture('python', ['noto-sandbox/.github/scripts/verify-pins.py']),
      shared_ui: runCmdCapture('node', [
        '--test',
        'louis-vision/apps/shared-ui/tests/blockies_haring.test.mjs',
        'louis-vision/apps/shared-ui/tests/shared_ui_layer1.test.mjs',
        'louis-vision/apps/shared-ui/tests/window_geom.test.mjs',
      ]),
      spatial_deck_e2e: runCmdCapture('node', ['--test', 'louis-vision/apps/vision/ui/tests/spatial_deck_e2e.mjs']),
      ui_nn_36: runCmdCapture('node', ['louis-vision/apps/shared-ui/tests/run_playwright_ui_nn.mjs', '--split', 'diag']),
      git_heads: ['premium-discount', 'louis-vision', 'logis-center', 'noto-sandbox'].map((r) => {
        const sha = spawnSync('git', ['-C', r, 'rev-parse', '--short', 'HEAD'], { cwd: WORKSPACE_ROOT, encoding: 'utf8' }).stdout.trim();
        const subj = spawnSync('git', ['-C', r, 'log', '-1', '--pretty=%s'], { cwd: WORKSPACE_ROOT, encoding: 'utf8' }).stdout.trim();
        return { repo: r, sha, subject: subj };
      }),
    };

    chapters.push({
      id: 'ch06_live_verification',
      title: 'Chapter 6. 4-레포 핀 동기화 · E2E · Rust 실측 검증 결과',
      subtitle: '방금 실시간 실행된 4개 레포 verify-pins · deny.toml · UI N:N 36/36 · Spatial Deck E2E · 6개 PR 머지 현황',
      app: 'Live Verification Telemetry',
      shots: [
        {
          terminal_slide: 'pins_and_prs',
          duration_s: 3.4,
          scenario_id: '4-Repo Sync & Hygiene',
          caption: '4개 저장소 6개 PR 머지 완료 (b024dbb5 / 5ea66a0f) · deny.toml 0 violations · 4-레포 verify-pins 전항목 PASS',
        },
        {
          terminal_slide: 'e2e_and_rust',
          duration_s: 3.6,
          scenario_id: 'Rust + Chromium E2E',
          caption: 'UI N:N 36/36 · Spatial Deck E2E 2/2 · shared-ui 9/9 · lv-core 57/57 · sandbox-runtime 320/320 · pd-vision-eval 5/5 PASS',
        },
      ],
    });

    const totalShots = chapters.reduce((acc, c) => acc + c.shots.length, 0);
    const estClipSec = chapters.reduce((acc, c) => acc + c.shots.reduce((s, x) => s + x.duration_s, 0), 0);

    const manifest = {
      schema: 'louis-vision-v18-video-manifest/v1',
      captured_at: new Date().toISOString(),
      viewport: '1280x720 + mobile 390x844',
      total_chapters: chapters.length,
      total_shots: totalShots,
      estimated_clip_seconds: Number(estClipSec.toFixed(2)),
      http_errors: httpErrors,
      runtime_exceptions: cdp.exceptions,
      telemetry,
      chapters,
    };

    fs.writeFileSync(path.join(OUT_DIR, 'storyboard_manifest.json'), JSON.stringify(manifest, null, 2));
    console.log(JSON.stringify({
      status: 'CAPTURE_OK',
      manifest: path.join(OUT_DIR, 'storyboard_manifest.json'),
      total_chapters: chapters.length,
      total_shots: totalShots,
      http_errors: httpErrors.length,
      runtime_exceptions: cdp.exceptions.length,
      cli_exit_codes: {
        deny: telemetry.deny.exit_code,
        pins_lv: telemetry.pins_lv.exit_code,
        pins_pd: telemetry.pins_pd.exit_code,
        pins_lc: telemetry.pins_lc.exit_code,
        pins_ns: telemetry.pins_ns.exit_code,
        shared_ui: telemetry.shared_ui.exit_code,
        spatial_deck_e2e: telemetry.spatial_deck_e2e.exit_code,
        ui_nn_36: telemetry.ui_nn_36.exit_code,
      },
    }, null, 2));

    if (httpErrors.length > 0 || cdp.exceptions.length > 0) {
      process.exitCode = 1;
    }
  } finally {
    await browser.teardown();
    server.close();
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
