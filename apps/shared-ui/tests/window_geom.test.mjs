// apps/shared-ui/tests/window_geom.test.mjs
// Verifies 100% 0px parity against premium-discount-window stage_cases.json (32 + 30 + 18 + 9 cases).
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  classifyPointer,
  growsTo,
  nextStage,
  placeWindow,
  feedRows,
  StickyNumberer,
  computeReadingExemptMs,
} from '../js/window-geom.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const stageCasesPath = path.resolve(__dirname, '../../../../premium-discount/crates/premium-discount-window/tests/stage_cases.json');
const cases = JSON.parse(fs.readFileSync(stageCasesPath, 'utf8'));

test('placeWindow matches all 32 stage_cases.json coordinates to 0 px', () => {
  assert.ok(cases.place_window.length >= 32);
  for (const c of cases.place_window) {
    const got = placeWindow(c.size, c.area, c.scale, c.anchor, c.margin);
    assert.deepEqual(got, c.expected, `placeWindow failed on: ${c.name}`);
  }
});

test('nextStage, classifyPointer, and growsTo match stage_cases.json 100%', () => {
  for (const c of cases.next_stage) {
    assert.equal(nextStage(c.current, c.event, c.ready), c.expected, JSON.stringify(c));
  }
  assert.equal(cases.classify_pointer.length, 18);
  for (const c of cases.classify_pointer) {
    assert.equal(classifyPointer(c.dx, c.dy, c.threshold), c.expected, JSON.stringify(c));
  }
  assert.equal(cases.grows_to.length, 9);
  for (const c of cases.grows_to) {
    assert.equal(growsTo(c.from, c.to), c.expected, JSON.stringify(c));
  }
});

test('feedRows produces 3 items ordered ascending with bottom-heavy opacities [0.55, 0.82, 1.0]', () => {
  const rows = [
    { id: '1', created_at: 10, text: 'a' },
    { id: '2', created_at: 20, text: 'b' },
    { id: '3', created_at: 30, text: 'c' },
    { id: '4', created_at: 40, text: 'd' },
  ];
  const fed = feedRows(rows, 3);
  assert.deepEqual(fed.map((r) => r.id), ['2', '3', '4']);
  assert.deepEqual(fed.map((r) => r.opacity), [0.55, 0.82, 1.0]);
  assert.equal(fed[0].isFeedTop, true);
  assert.equal(fed[2].isFeedTop, false);
});

test('StickyNumberer keeps stable 1~9 slots under minor jitter and clears immediately on privacy', () => {
  const sn = new StickyNumberer();
  const base = [
    { id: 'btn-a', source: 'dom', x: 100, y: 100, w: 80, h: 40 },
    { id: 'btn-a-vis', source: 'vision', x: 102, y: 101, w: 78, h: 39 }, // absorbed into btn-a
    { id: 'btn-b', source: 'dom', x: 400, y: 100, w: 80, h: 40 },
    { id: 'off', source: 'dom', x: 3000, y: 100, w: 80, h: 40 }, // out of viewport -> dropped
  ];
  const r1 = sn.update(base, { nowMs: 1000, viewport: { w: 1920, h: 1080 } });
  assert.equal(r1.badges.length, 2);
  assert.equal(r1.badges[0].num, 1);
  assert.equal(r1.badges[1].num, 2);
  assert.equal(r1.dropped, 1);

  // Micro jitter does not reallocate numbers
  const jitter = [
    { id: 'btn-a', source: 'dom', x: 103, y: 102, w: 80, h: 40 },
    { id: 'btn-b', source: 'dom', x: 398, y: 101, w: 80, h: 40 },
  ];
  const r2 = sn.update(jitter, { nowMs: 1200, viewport: { w: 1920, h: 1080 } });
  assert.deepEqual(r2.badges.map((b) => [b.num, b.id]), [[1, 'btn-a'], [2, 'btn-b']]);

  // Sensitive window immediately destroys badges and sets record=false
  const r3 = sn.update(jitter, { nowMs: 1300, sensitive: true });
  assert.equal(r3.badges.length, 0);
  assert.equal(r3.record, false);
  assert.equal(computeReadingExemptMs(5000), 120000);
});
