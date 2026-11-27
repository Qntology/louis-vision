// scripts/record_all_scenarios_video.mjs
// Records real user scenarios across ALL 3 louis-vision apps (Routing Overlay, Docs Studio,
// Field-3D Lens, LAN Mobile 4-Skin, Vision Sim Sandbox, Vision XR Stage + HaringDancerRig,
// FOH Web Console, and Cross-App Navigation) via local 127.0.0.1 static server + Headless Chromium CDP.

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, '..');
const APPS_ROOT = path.join(REPO_ROOT, 'apps');
const OUT_DIR = path.join(REPO_ROOT, 'artifacts', 'video_capture');
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

async function launchChromium() {
  const chromeBin = findChromeBin();
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lv-video-cdp-'));
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
    async openPage(url) {
      const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank' });
      const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
      await cdp.send('Page.enable', {}, sessionId);
      await cdp.send('Runtime.enable', {}, sessionId);
      await cdp.send('Target.activateTarget', { targetId });
      await cdp.send('Page.bringToFront', {}, sessionId);
      await cdp.send('Emulation.setDeviceMetricsOverride', {
        width: 1280,
        height: 720,
        deviceScaleFactor: 1,
        mobile: false,
      }, sessionId);
      await cdp.send('Page.navigate', { url }, sessionId);
      await waitLoaded(cdp, sessionId);
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
      if (!proc.killed) {
        try { proc.kill('SIGTERM'); } catch { /* ignore */ }
      }
      await sleep(200);
      try { fs.rmSync(userDataDir, { recursive: true, force: true }); } catch { /* ignore */ }
    },
  };
}

async function waitLoaded(cdp, sid) {
  for (let i = 0; i < 80; i++) {
    const ready = await cdp.evalExpr(sid, 'document.readyState === "complete"');
    if (ready) break;
    await sleep(50);
  }
  await sleep(180);
}

async function captureFrame(cdp, sid, name) {
  const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }, sid);
  const rel = `${name}.png`;
  const out = path.join(FRAMES_DIR, rel);
  fs.writeFileSync(out, Buffer.from(data, 'base64'));
  return path.join('frames', rel);
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
    // CHAPTER 1: Routing App & Single ( ✦ ) Anchor (louis/ui/index.html)
    // =========================================================================
    {
      const page = await browser.openPage(`${baseUrl}/louis/ui/index.html`);
      const sid = page.sessionId;
      const shots = [];

      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_01_deck'),
        duration_s: 2.2,
        scenario_id: 'UI-01 / UI-02',
        caption: '단일 앵커 라우터 기동 (127.0.0.1:8787 · Active Star=1 · louis_exclusive)',
      });

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('lv-star-btn').click();
        const inp = document.getElementById('lv-chat-input');
        inp.value = '/docs dom:DOM-08 어제 정산 내역';
        document.getElementById('lv-chat-form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_02_route_docs'),
        duration_s: 2.4,
        scenario_id: 'UI-07 / UI-14',
        caption: '( ✦ ) 팝오버 내 /docs dom:DOM-08 입력 -> louis_docs_studio 의도·태그 라우팅',
      });

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('lv-btn-attach').click();
      })()`);
      await sleep(140);
      await cdp.evalExpr(sid, `(() => {
        document.getElementById('lv-btn-attach').click();
      })()`);
      await sleep(140);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_03_attach_urdf'),
        duration_s: 2.2,
        scenario_id: 'UI-17 / UI-31',
        caption: '파일 첨부(.hwpx -> Docs, .urdf -> Speculative 45s 예열 전환)',
      });

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('lv-btn-collapse').click();
        document.getElementById('btn-open-omni').click();
        const oi = document.getElementById('lv-omni-input');
        oi.value = '/stage 본공연';
        oi.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
      })()`);
      await sleep(160);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_04_omnibar_stage'),
        duration_s: 2.4,
        scenario_id: 'UI-19 / UI-36',
        caption: 'Omni-Bar (Ctrl+K) /stage 본공연 -> StagePinnedLock & Louis VRAM 0 MB 전환',
      });

      await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 }, sid);
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 }, sid);
      await cdp.evalExpr(sid, `(() => {
        document.getElementById('btn-open-pair').click();
        document.getElementById('btn-wan-reject').click();
      })()`);
      await sleep(120);
      await cdp.evalExpr(sid, `document.getElementById('btn-lan-scan').click()`);
      await sleep(120);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_05_lan_qr_pair'),
        duration_s: 2.4,
        scenario_id: 'Rule 5 / Blockies QR',
        caption: 'LAN QR 페어링 모달 (8.8.8.8 WAN 차단 -> RFC1918 192.168.45.12 키교환 허용)',
      });

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('btn-pair-close').click();
        document.getElementById('btn-toggle-privacy').click();
      })()`);
      await sleep(140);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch01_06_privacy_lock'),
        duration_s: 2.0,
        scenario_id: 'UI-13 / UI-25',
        caption: '3줄 상단 마스크 피드 + 전역 Privacy Epoch (epoch>=1) 화면 즉시 블러 차폐',
      });
      await cdp.evalExpr(sid, `document.getElementById('btn-toggle-privacy').click()`);
      await browser.closePage(page);

      chapters.push({
        id: 'ch01_router',
        title: 'Chapter 1. 단일 앵커 라우터 (apps/louis/ui/index.html)',
        subtitle: '127.0.0.1:8787 루프백 · ( ✦ ) 3줄 미니피드 · Omni-Bar · GPU 임대 · Blockies QR · Privacy Epoch',
        app: 'Louis Routing Overlay',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 2: Louis Docs Studio (louis/ui/studio.html)
    // =========================================================================
    {
      const studioPage = await browser.openPage(`${baseUrl}/louis/ui/studio.html`);
      const sid = studioPage.sessionId;
      const shots = [];

      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_01_initial'),
        duration_s: 2.0,
        scenario_id: 'UI-03 / UI-15',
        caption: 'Docs Studio 3-Pane 유리 무대 · DOM-01 정산 & A0 4x4 (ZZD1 Δ) 미니맵',
      });

      await cdp.evalExpr(sid, `(() => {
        const sel = document.getElementById('studio-pack-select');
        sel.value = 'DOM-08';
        sel.dispatchEvent(new Event('change'));
        document.getElementById('btn-opc-jump').click();
        document.querySelectorAll('.legend-sample').forEach((b, i) => { if (i < 2) b.click(); });
        document.getElementById('btn-toggle-ambiguous').click();
        document.getElementById('btn-toggle-ambiguous').click();
      })()`);
      await sleep(140);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_02_dom08_opc_legend'),
        duration_s: 2.5,
        scenario_id: 'UI-15 / UI-21',
        caption: 'DOM-08 무역 스킨 전환 · 2-홉 OPC 역추적 (Tile B2) · 0-LLM 범례 퓨샷 & L3 모호 기권',
      });

      await cdp.evalExpr(sid, `document.getElementById('btn-sql-attack').click()`);
      await sleep(120);
      await cdp.evalExpr(sid, `(() => {
        const inp = document.getElementById('sql-guard-input');
        inp.value = 'SELECT doc_id, amt_krw FROM docs_entities';
        document.getElementById('btn-run-sql').click();
      })()`);
      await sleep(120);
      await cdp.evalExpr(sid, `(() => {
        document.getElementById('output-form-select').value = 'ApprovalNotice';
        const vSel = document.getElementById('gate-verdict-select');
        vSel.value = 'G2RejectContradictionLock';
        vSel.dispatchEvent(new Event('change'));
        document.getElementById('manual-amount-input').value = '85000001';
        document.getElementById('btn-check-amount').click();
        document.getElementById('g3-kind-select').value = 'None';
        document.getElementById('btn-g3-approve').click();
        document.getElementById('n2sf-src').value = 'Classified';
        document.getElementById('n2sf-dst').value = 'Open';
        document.getElementById('btn-test-reverse-cdr').click();
        document.getElementById('btn-worm-tamper').click();
        document.getElementById('g3-cosign').checked = true;
        document.getElementById('btn-unmask-pii').click();
      })()`);
      await sleep(140);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_03_sql_5gate_worm'),
        duration_s: 2.8,
        scenario_id: 'UI-09 / UI-27 / UI-33',
        caption: '5-Gate 심사(양식강제스냅 · +1원 차단 · Reverse-CDR 0B) & WORM seq=200 변조 적발 · SQL 가드 통과',
      });

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('btn-run-cascade').click();
        document.getElementById('lv-star-btn').click();
      })()`);
      await sleep(140);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch02_04_cascade_star'),
        duration_s: 2.2,
        scenario_id: 'Cascade & Unified ( ✦ )',
        caption: '캐스케이드 검증 완료 (DSLI_18 0.99 PASS) 및 스튜디오 통합 ( ✦ ) 팝오버 동기화',
      });

      await browser.closePage(studioPage);

      chapters.push({
        id: 'ch02_louis_studio',
        title: 'Chapter 2. Louis Docs Studio (apps/louis/ui/studio.html)',
        subtitle: '18대 도메인 스킨 · A0 ZZD1 · 5중 DataFusion SQL · 5-Gate · Reverse-CDR · 5년 WORM 이중 해시',
        app: 'Louis Docs Studio',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 3: Louis Field-3D AR Lens & Sequential LOTO (louis/ui/field-lens.html)
    // =========================================================================
    {
      const fieldPage = await browser.openPage(`${baseUrl}/louis/ui/field-lens.html`);
      const sid = fieldPage.sessionId;
      const shots = [];

      shots.push({
        file: await captureFrame(cdp, sid, 'ch03_01_low_dpi'),
        duration_s: 1.8,
        scenario_id: 'UI-04 / UI-22',
        caption: 'Field-3D 초기 해상도 부족 (1.55 px/mm) · 0-VLM 한글 유도 & 3D 모니터 0 B zeroize 사각형',
      });

      for (const [reason, idx, msg] of [
        ['SteepViewingAngle', '02a', '시선각 71° 비가독 배너 (0-VLM) — AR 명판 투영 각도 경보'],
        ['SpecularGlareHoles', '02b', '유광 빛반사 NaN 홀 48% 배너 — 반사 회피 측면 이동 유도'],
      ]) {
        await cdp.evalExpr(sid, `(() => {
          const sel = document.getElementById('reposition-reason-select');
          sel.value = '${reason}';
          sel.dispatchEvent(new Event('change'));
        })()`);
        await sleep(100);
        shots.push({
          file: await captureFrame(cdp, sid, `ch03_${idx}_${reason}`),
          duration_s: 1.3,
          scenario_id: 'UI-22 / UI-34',
          caption: msg,
        });
      }

      // Trigger out-of-order LOTO Step 3 BEFORE Step 2
      await cdp.evalExpr(sid, `document.getElementById('btn-loto-step3').click()`);
      await cdp.evalExpr(sid, `(() => {
        const sel = document.getElementById('reposition-reason-select');
        sel.value = 'Pass';
        sel.dispatchEvent(new Event('change'));
      })()`);
      await sleep(450);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch03_03_pass_outoforder'),
        duration_s: 2.2,
        scenario_id: 'UI-22 / UI-28',
        caption: '가독 Pass (6.4 px/mm) 350ms Auto-Freeze 잠금 + LOTO 역순(Step 3 선행) OutOfOrder 차단',
      });

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('btn-glove-next').click();
        document.getElementById('loto-step2-val').value = '60';
        document.getElementById('btn-loto-step2').click();
        document.getElementById('btn-loto-step3').click();
        document.getElementById('btn-glove-commit').click();
      })()`);
      await sleep(140);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch03_04_loto_commit'),
        duration_s: 2.2,
        scenario_id: 'UI-16 / UI-34',
        caption: '#rectified-pip 확장 + Step 2(60%) -> Step 3(SUP-01) -> < 2,000 us 원장 커밋 완료',
      });

      await cdp.evalExpr(sid, `(() => {
        const role = document.getElementById('field-role-select');
        role.value = 'CounterKiosk';
        role.dispatchEvent(new Event('change'));
      })()`);
      await sleep(100);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch03_05_kiosk_block'),
        duration_s: 2.0,
        scenario_id: 'UI-08 / UI-ZH-07',
        caption: 'CounterKiosk 단말 전환 즉시 전면 Fail-Closed 차폐 (VaultError::KioskForbidden)',
      });
      await cdp.evalExpr(sid, `document.getElementById('btn-kiosk-reset').click()`);
      await browser.closePage(fieldPage);

      chapters.push({
        id: 'ch03_louis_field3d',
        title: 'Chapter 3. Louis Field-3D AR 렌즈 (apps/louis/ui/field-lens.html)',
        subtitle: '>= 56x56px 안전장갑 도크 · 0-VLM 재촬영 유도 · 3D 모니터 zeroize · 순차 LOTO · Kiosk 감옥',
        app: 'Louis Field-3D Lens',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 4: Louis LAN WebRTC Mobile 4-Skins (louis/ui/mobile.html)
    // =========================================================================
    {
      const mobilePage = await browser.openPage(`${baseUrl}/louis/ui/mobile.html`);
      const sid = mobilePage.sessionId;
      const shots = [];

      await cdp.evalExpr(sid, `document.getElementById('btn-lan-probe').click()`);
      await sleep(120);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch04_01_skin1_lan_qr'),
        duration_s: 2.0,
        scenario_id: 'UI-26 / Skin 1',
        caption: 'Skin 1 현장 3D 점검 + Blockies-Haring QR + RFC1918 LAN (192.168.45.12:9999) 페어링 허용',
      });

      await cdp.evalExpr(sid, `(() => {
        const s = document.getElementById('mobile-skin-select');
        s.value = 'kiosk_intake';
        s.dispatchEvent(new Event('change'));
        document.getElementById('btn-wan-probe').click();
      })()`);
      await sleep(120);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch04_02_kiosk_wan_block'),
        duration_s: 2.2,
        scenario_id: 'UI-26 / UI-ZH-09',
        caption: 'Skin 3 CounterKiosk Open 등급 스킨 전환 및 8.8.8.8:9999 공인 WAN 영구 차단',
      });

      await cdp.evalExpr(sid, `(() => {
        const s = document.getElementById('mobile-skin-select');
        s.value = 'executive_audit';
        s.dispatchEvent(new Event('change'));
        document.getElementById('btn-trigger-grace').click();
      })()`);
      await sleep(120);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch04_03_grace_blurred'),
        duration_s: 2.2,
        scenario_id: 'CX-06 / GraceBlurred',
        caption: 'Wi-Fi 순단 유예(GraceBlurred) 화면 블러 보호 오버레이 -> QR/12단어 재인증 및 zeroize 대기',
      });

      await browser.closePage(mobilePage);

      chapters.push({
        id: 'ch04_louis_mobile',
        title: 'Chapter 4. LAN 사설망 WebRTC 모바일 4-스킨 (apps/louis/ui/mobile.html)',
        subtitle: '4종 전용 스킨 · 공인 WAN 영구 차단 · Blockies QR 핸드셰이크 · GraceBlurred 순단 유예',
        app: 'Louis LAN Mobile',
        shots,
      });
    }

    // =========================================================================
    // CHAPTER 5: Vision Sandbox Sim Mode (vision/ui/index.html)
    // =========================================================================
    {
      const visionPage = await browser.openPage(`${baseUrl}/vision/ui/index.html`);
      const sid = visionPage.sessionId;
      // Wait for mock world frame generation
      for (let i = 0; i < 40; i++) {
        const ok = await cdp.evalExpr(sid, `(() => {
          const empty = document.getElementById('world-empty');
          return !!empty && empty.hidden && document.getElementById('sb-seed').textContent !== '-';
        })()`);
        if (ok) break;
        await sleep(100);
      }
      const shots = [];

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('crew-add-humanoid').click();
        document.getElementById('crew-add-forklift').click();
        const m = document.getElementById('crew-mode');
        m.value = 'team';
        m.dispatchEvent(new Event('change'));
      })()`);
      await sleep(250);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch05_01_crew_team'),
        duration_s: 2.0,
        scenario_id: 'UI-35 / Sim Crew',
        caption: 'Sim Sandbox 3D/탑뷰 월드 기동 · 휴머노이드+지게차 소환 & 🤝 협업트랙 설정',
      });

      await cdp.evalExpr(sid, `(() => {
        const ci = document.getElementById('command-input');
        ci.value = '철광석 3개 캐서 베이스캠프로 가져와';
        document.getElementById('command-bar').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
      })()`);
      await sleep(260);
      // Record 10 live simulation ticks smoothly!
      for (let f = 0; f < 10; f++) {
        await sleep(170);
        shots.push({
          file: await captureFrame(cdp, sid, `ch05_02_sim_t${String(f).padStart(2, '0')}`),
          duration_s: 0.28,
          scenario_id: 'UI-05 / Live Sim',
          caption: '우하단 ( ✦ ) 명령창 자연어 임베딩 실행 -> 협업 로봇 실시간 틱 기동 및 경로·로그 추종',
        });
      }

      // Pause sim & switch viewpoints + toggle Oracle provenance
      await cdp.evalExpr(sid, `(() => {
        const playBtn = document.getElementById('sim-play');
        if (playBtn && playBtn.getAttribute('aria-pressed') === 'true') playBtn.click();
        const vs = document.getElementById('view-select');
        vs.value = 'chase';
        vs.dispatchEvent(new Event('change'));
        const or = document.getElementById('tg-oracle');
        or.click();
      })()`);
      await sleep(320);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch05_03_chase_oracle'),
        duration_s: 2.0,
        scenario_id: 'UI-23 / Provenance',
        caption: '3인칭 관절 물리 카메라 전환 + 정답 ID(오라클) 활성 시 Truth | Reference 배지 분리',
      });

      // Open Quest drop-up & Analysis modal (Time-Series / SDS tab)
      await cdp.evalExpr(sid, `document.getElementById('quest-bar').click()`);
      await sleep(220);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch05_04_quest_dropup'),
        duration_s: 1.8,
        scenario_id: 'Quest & Inventory Dropup',
        caption: '협업 퀘스트·인벤토리·거점 보관 더미 통합 관리 Drop-up 패널',
      });

      await cdp.evalExpr(sid, `(() => {
        document.getElementById('tl-live').click();
      })()`);
      await sleep(220);
      await cdp.evalExpr(sid, `document.getElementById('atab-ts').click()`);
      await sleep(240);
      shots.push({
        file: await captureFrame(cdp, sid, 'ch05_05_ts_sds_modal'),
        duration_s: 2.0,
        scenario_id: 'TTM & SDS Ledger',
        caption: '분석 도크: Granite TTM 시계열 예측(P10/P50/P90) 및 SDS 탐색기·원장 감시',
      });
      await cdp.evalExpr(sid, `(() => {
        document.querySelector('#analysis-modal [data-close]')?.click();
        const qd = document.getElementById('quest-dropup');
        if (qd && !qd.hidden) {
          const qc = qd.querySelector('[data-close]');
          if (qc) qc.click();
          else document.getElementById('quest-bar')?.click();
        }
        const vs = document.getElementById('view-select');
        if (vs) {
          vs.value = 'top';
          vs.dispatchEvent(new Event('change'));
        }
        const orBtn = document.getElementById('tg-oracle');
        if (orBtn && orBtn.getAttribute('aria-pressed') !== 'false') {
          orBtn.click();
        }
      })()`);
      await sleep(120);

      chapters.push({
        id: 'ch05_vision_sim',
        title: 'Chapter 5. Vision Sandbox — Sim 모드 (apps/vision/ui/index.html)',
        subtitle: 'CoexistSim · 휴머노이드+지게차 협업트랙 · 우하단 ( ✦ ) 통합 명령 · 3인칭 관절 · TTM/SDS 분석',
        app: 'Vision Sandbox (Sim)',
        shots,
      });

      // =======================================================================
      // CHAPTER 6: Vision Sandbox XR Stage Live + HaringDancerRig + Web Console
      // =======================================================================
      {
        const sShots = [];
        await cdp.evalExpr(sid, `window.__visionStage.switchVisionMode('stage')`);
        await sleep(180);

        // Animate HaringDancerRig & step SMPTE cues across 8 crisp frames
        for (let f = 0; f < 8; f++) {
          if (f === 3 || f === 6) {
            await cdp.evalExpr(sid, `document.getElementById('btn-cue-go').click()`);
          }
          await cdp.evalExpr(sid, `window.__visionStage.renderHaringDancerFrame(${0.35 + f * 0.45})`);
          await sleep(110);
          sShots.push({
            file: await captureFrame(cdp, sid, `ch06_01_stage_haring_${f}`),
            duration_s: 0.30,
            scenario_id: 'UI-06 / Gate UI-HARING',
            caption: 'XR Stage Live 무리로드 OLED Dark 전환 + HaringDancerRig 정사영(#000000·6 ticks) & SMPTE 큐 발사',
          });
        }

        // Open Z_bg calibration popover -> capture 30/30 -> Lock Z-Keying
        await cdp.evalExpr(sid, `(() => {
          document.getElementById('btn-stage-calib').click();
          document.getElementById('btn-capture-zbg').click();
          document.getElementById('btn-lock-zkey').click();
        })()`);
        await sleep(140);
        sShots.push({
          file: await captureFrame(cdp, sid, 'ch06_02_zbg_calib'),
          duration_s: 2.2,
          scenario_id: 'UI-24 / SE(3) Align',
          caption: '3점 영점(s=1.000, RMSE 0.008 m) · 빈 무대 Z_bg 30/30 수집 후 Z-Keying Pinned 잠금 해금',
        });

        // Open Show-IO popover -> reject WAN 8.8.8.8 -> accept RFC1918
        await cdp.evalExpr(sid, `(() => {
          document.querySelector('#stage-calib-popover [data-close]').click();
          document.getElementById('btn-show-io').click();
          const inp = document.getElementById('show-io-osc-addr');
          inp.value = '8.8.8.8:9000';
          document.getElementById('btn-validate-show-io').click();
        })()`);
        await sleep(140);
        sShots.push({
          file: await captureFrame(cdp, sid, 'ch06_03_show_io_wan_block'),
          duration_s: 2.2,
          scenario_id: 'UI-29 / Rule 5 Show-IO',
          caption: 'Spout2 · NDI · OSC 사설망 가드 — 8.8.8.8:9000 공인 WAN 주소 즉시 거부',
        });

        // Apply valid RFC1918, replay stage_cues, trigger F12 Blackout!
        await cdp.evalExpr(sid, `(() => {
          const inp = document.getElementById('show-io-osc-addr');
          inp.value = '192.168.10.20:9000';
          document.getElementById('btn-validate-show-io').click();
          document.querySelector('#show-io-popover [data-close]').click();
          document.getElementById('btn-cue-replay').click();
          document.getElementById('btn-hazard-obb').click();
        })()`);
        await sleep(140);
        sShots.push({
          file: await captureFrame(cdp, sid, 'ch06_04_interlock_halt'),
          duration_s: 2.0,
          scenario_id: 'UI-12 / UI-30',
          caption: 'stage_cues 복기 + 위험구역 OBB 침범 감지 -> MetricZoneClear HALT · [GO] 큐 잠금',
        });

        await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'F12', code: 'F12', windowsVirtualKeyCode: 123 }, sid);
        await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'F12', code: 'F12', windowsVirtualKeyCode: 123 }, sid);
        await sleep(160);
        sShots.push({
          file: await captureFrame(cdp, sid, 'ch06_05_blackout'),
          duration_s: 2.2,
          scenario_id: 'UI-18 / P0 Blackout',
          caption: 'F12 비상 Blackout (<= 16.6 ms) 발령 중에도 3중 인터록 HUD 상시 고정 유지',
        });
        await cdp.evalExpr(sid, `document.getElementById('btn-stage-reset').click()`);

        // Capture Rule-6 remediated Web Console (apps/vision/web-console/index.html)
        const wcPage = await browser.openPage(`${baseUrl}/vision/web-console/index.html`);
        const wcSid = wcPage.sessionId;
        await cdp.evalExpr(wcSid, `(() => {
          document.getElementById('cueGoBtn').click();
          document.getElementById('toggleHaltBtn').click();
        })()`);
        await sleep(140);
        sShots.push({
          file: await captureFrame(cdp, wcSid, 'ch06_06_web_console'),
          duration_s: 2.2,
          scenario_id: 'Rule 6 FOH Web Console',
          caption: 'FOH Web Console (외부 ES 모듈 console.js 분리 완료 · 인라인 스크립트 0줄 · Interlock Halt)',
        });
        await browser.closePage(wcPage);

        chapters.push({
          id: 'ch06_vision_stage',
          title: 'Chapter 6. Vision XR Stage Live & FOH Console',
          subtitle: 'StagePinnedLock · HaringDancerRig · Z_bg 30/30 · WAN 차단 · F12 Blackout · Rule 6 Web Console',
          app: 'Vision Sandbox (XR Stage)',
          shots: sShots,
        });

        // =====================================================================
        // CHAPTER 7: Cross-App Real Navigation (Vision -> Studio -> Router)
        // =====================================================================
        await cdp.send('Target.activateTarget', { targetId: visionPage.targetId });
        await cdp.send('Page.bringToFront', {}, sid);
        await sleep(140);
        const xShots = [];
        await cdp.evalExpr(sid, `window.__visionStage.switchVisionMode('sim')`);
        await sleep(120);
        await cdp.evalExpr(sid, `document.getElementById('link-louis-studio').click()`);
        await waitLoaded(cdp, sid);
        const curStudioUrl = await cdp.evalExpr(sid, `location.pathname`);
        xShots.push({
          file: await captureFrame(cdp, sid, 'ch07_01_nav_to_studio'),
          duration_s: 2.0,
          scenario_id: 'Cross-App Nav -> Studio',
          caption: `Vision -> Louis Docs Studio 실라우팅 전환 완료 (${curStudioUrl} · Runtime Errors: 0)`,
        });

        await cdp.evalExpr(sid, `document.querySelector('a[href="./index.html"]').click()`);
        await waitLoaded(cdp, sid);
        const curRouterInfo = await cdp.evalExpr(sid, `({
          path: location.pathname,
          stars: document.querySelectorAll('#lv-star-root').length
        })`);
        xShots.push({
          file: await captureFrame(cdp, sid, 'ch07_02_nav_to_router'),
          duration_s: 2.2,
          scenario_id: 'CX-01..08 Complete',
          caption: `Studio -> 단일 앵커 라우터 복귀 (${curRouterInfo.path} · Active Star=${curRouterInfo.stars})`,
        });
        await browser.closePage(visionPage);

        chapters.push({
          id: 'ch07_cross_app_routing',
          title: 'Chapter 7. 전 앱 크로스 라우팅 완주 및 무오류 검증',
          subtitle: 'Vision Sandbox <-> Docs Studio <-> 단일 ( ✦ ) 프로세스 라우터 왕복 · JS 예외 0 · HTTP 404 0',
          app: 'Cross-App E2E Routing',
          shots: xShots,
        });
      }
    }

    if (httpErrors.length > 0) {
      throw new Error(`HTTP asset errors detected: ${httpErrors.join(', ')}`);
    }
    const runtimeExceptions = cdp.events.filter((e) => e.method === 'Runtime.exceptionThrown');
    if (runtimeExceptions.length > 0) {
      throw new Error(`Runtime JS exception captured: ${JSON.stringify(runtimeExceptions[0])}`);
    }

    const totalShots = chapters.reduce((acc, c) => acc + c.shots.length, 0);
    const estVideoDurationS = chapters.reduce((acc, c) => acc + c.shots.reduce((s, sh) => s + sh.duration_s, 0), 0);
    const manifest = {
      schema: 'louis-vision-video-manifest/v1',
      captured_at_iso: new Date().toISOString(),
      viewport: '1280x720',
      http_errors: httpErrors.length,
      runtime_exceptions: runtimeExceptions.length,
      total_chapters: chapters.length,
      total_shots: totalShots,
      estimated_clip_seconds: Number(estVideoDurationS.toFixed(2)),
      chapters,
    };
    const manifestPath = path.join(OUT_DIR, 'storyboard_manifest.json');
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), 'utf8');
    console.log(JSON.stringify({
      status: 'CAPTURE_OK',
      manifest: manifestPath,
      chapters: chapters.length,
      total_shots: totalShots,
      http_errors: 0,
      runtime_exceptions: 0,
    }, null, 2));
  } finally {
    await browser.teardown();
    server.close();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
