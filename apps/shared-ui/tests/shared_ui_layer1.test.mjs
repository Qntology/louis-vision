// apps/shared-ui/tests/shared_ui_layer1.test.mjs
// Gate UI-P0, UI-P1, UI-P2 Layer 1 headless verification.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { parseOmniQuery, resolveTimeGuideWindow, decodeRrfRowsSafely, validateSqlPreflight, buildMediaDeepLink } from '../js/omni-bar.js';
import { StarWidgetController } from '../js/star-widget.js';
import { BUILTIN_18_PACK_TERMS, SUPPORTED_LANGS, normalizeLangCode } from '../js/skin-terms.js';
import { filterCatalogForRole, isModelAllowedForRole, verifyStaticReductionBytes, validateAccelProbeOutcome } from '../js/model-vault.js';
import { MockDualBackend, CITIZEN_FORBIDDEN_TERMS_16 } from '../js/mock-dual-backend.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const sharedUiRoot = path.resolve(__dirname, '..');

test('Omni-Bar tag+free-text lossless split, BUG-DB-3 tolerance, deterministic time_guide, and SQL 5-gate guard', () => {
  const q = parseOmniQuery('type:BL dom:DOM-08 지난달 shanghai 차액 합계');
  assert.equal(q.tags.type, 'BL');
  assert.equal(q.tags.dom, 'DOM-08');
  assert.ok(q.freeText.includes('shanghai'), 'free text lost (`BUG-DB-2`)');
  assert.equal(q.timeKey, 'last_month');
  assert.equal(q.requiresSql, true);

  const win = resolveTimeGuideWindow('last_month', Date.UTC(2026, 10, 26));
  assert.equal(win.llmGeneratedDates, false);
  assert.equal(new Date(win.startMs).getUTCMonth(), 9); // October

  const decoded = decodeRrfRowsSafely([JSON.stringify({ id: 'ok-1' }), 'null', '{bad-json']);
  assert.equal(decoded.valid.length, 1);
  assert.equal(decoded.droppedCorrupted, 2);

  // 20-style SQL injection probes must fail AST preflight 100%
  const attacks = [
    'DROP TABLE gate_ledger',
    'DELETE FROM docs_entities WHERE 1=1',
    'SELECT * FROM docs_entities; DROP TABLE gate_ledger',
    'ATTACH DATABASE "/tmp/pwn" AS pwn',
    'SELECT * FROM read_file("/etc/passwd")',
    'SELECT * FROM unknown_secret_table',
    'SELECT * FROM docs_entities CROSS JOIN gate_ledger',
    'INSERT INTO gate_ledger VALUES (1)',
    'UPDATE docs_entities SET amt = 0',
    'COPY docs_entities TO "/tmp/out.csv"',
  ];
  for (const sql of attacks) {
    assert.equal(validateSqlPreflight(sql).ok, false, `attack slipped through: ${sql}`);
  }
  assert.equal(validateSqlPreflight('SELECT * FROM docs_entities', { role: 'counter_kiosk' }).ok, false);
  const cleanSql = validateSqlPreflight('SELECT doc_id, amt FROM docs_entities');
  assert.equal(cleanSql.ok, true);
  assert.ok(cleanSql.wrappedSql.includes('vault_id =') && cleanSql.wrappedSql.includes('n2sf_grade <=') && cleanSql.wrappedSql.includes('LIMIT 200'));
  assert.equal(buildMediaDeepLink('cam01', 12450), 'louis://media/cam01?ts_ms=12450&bbox=0,0,1,1');
});

test('StarWidget compose protection, Tombstone delete sync, Stage blackout filter, and 8-tier ESC/IME stack', () => {
  const sw = new StarWidgetController();
  sw.stage = 'popover';
  sw.setDraft('계약서 검토');
  assert.equal(sw.handleOutsideBlur().collapsed, false);
  sw.setDraft('');
  assert.equal(sw.handleOutsideBlur().collapsed, true);

  sw.mergeIncomingRows([
    { id: 'm1', created_at: 100, kind: 'doc_done', text: '일반 문서 완료' },
    { id: 'm2', created_at: 200, kind: 'interlock_violation', text: '인터록 경고' },
  ]);
  sw.deleteRow('m1');
  const after = sw.mergeIncomingRows([{ id: 'm1', created_at: 100, kind: 'doc_done', text: 'resurrected' }]);
  assert.equal(after.some((r) => r.id === 'm1'), false);

  // Stage blackout suppresses regular toasts, passes only 3 alert kinds
  sw.surfaceOwner = 'vision_xr_stage';
  sw.mergeIncomingRows([{ id: 'm3', created_at: 300, kind: 'doc_done', text: 'toast' }]);
  const stageFeed = sw.getMiniFeed();
  assert.deepEqual(stageFeed.map((r) => r.kind), ['interlock_violation']);

  // P1 IME composing protects Escape
  assert.equal(sw.handleKeyEvent({ key: 'Escape', isComposing: true }).tier, 'P1');
  // P0 blackout overrides everything in Stage Live
  let blackedOut = false;
  sw.onBlackout = () => { blackedOut = true; };
  assert.equal(sw.handleKeyEvent({ key: 'F12' }).tier, 'P0');
  assert.equal(blackedOut, true);
});

test('18 domain packs terms, 15 locales including zh-tw, air-gap model gate & mock dual router parity', async () => {
  assert.equal(Object.keys(BUILTIN_18_PACK_TERMS).length, 18);
  for (const [id, t] of Object.entries(BUILTIN_18_PACK_TERMS)) {
    for (const k of ['code', 'procedure', 'change', 'drawing', 'supervisor', 'permit', 'domain_badge']) {
      assert.ok(t[k] && t[k].length > 0, `empty term ${k} in ${id}`);
    }
  }
  assert.equal(SUPPORTED_LANGS.length, 15);
  assert.equal(normalizeLangCode('zh_TW'), 'zh-tw');
  assert.equal(normalizeLangCode('zh_CN'), 'zh');
  for (const lang of SUPPORTED_LANGS) {
    const p = path.join(sharedUiRoot, 'i18n', `${lang}.json`);
    assert.ok(fs.existsSync(p), `missing locale file ${p}`);
  }

  // Model gate: CounterKiosk lockdown & fake trim rejection
  const kioskModels = filterCatalogForRole('counter_kiosk').map((m) => m.id);
  assert.deepEqual(kioskModels, ['siglip2', 'qwen35_08b']);
  assert.equal(isModelAllowedForRole('counter_kiosk', 'qwen35_4b').code, 'VaultError::KioskForbidden');
  assert.equal(verifyStaticReductionBytes({ actualDeltaBytes: 1024 }).passed, false);
  assert.equal(verifyStaticReductionBytes({ actualDeltaBytes: 497_172_480 }).passed, true);
  assert.equal(validateAccelProbeOutcome({ hardware_verified: true, adapter_name: 'llvmpipe', accel_fallbacks: [] }).ok, false);

  // Mock Dual Backend checks
  const mock = new MockDualBackend();
  const s1 = await mock.invoke('router_route_input', { text: '/stage' });
  assert.equal(s1.louisResidentMb, 0);
  assert.equal((await mock.invoke('router_state')).activeStarWidgets, 1);
  assert.equal((await mock.invoke('lan_pair_check', { addr: '8.8.8.8:9999' })).allowed, false);
  assert.equal((await mock.invoke('lan_pair_check', { addr: '192.168.45.12:9999' })).allowed, true);
  assert.equal(CITIZEN_FORBIDDEN_TERMS_16.length, 16);

  // Zero inline scripts or handlers across shared-ui templates
  for (const f of ['star-widget.html', 'pairing-modal.html']) {
    const html = fs.readFileSync(path.join(sharedUiRoot, f), 'utf8');
    assert.equal(/<script\b/i.test(html), false, `inline <script> in ${f}`);
    assert.equal(/\son[a-z]+\s*=/i.test(html), false, `inline event handler in ${f}`);
  }
});
