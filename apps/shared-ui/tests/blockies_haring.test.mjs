// apps/shared-ui/tests/blockies_haring.test.mjs
// Verifies EIP-55 case insensitivity, 8x8 horizontal mirror symmetry, and Keith Haring color invariants across 1,000 addresses.
import test from 'node:test';
import assert from 'node:assert/strict';

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deriveBlockiesHaringSpec } from '../js/blockies-haring.js';

const __dirname = dirname(fileURLToPath(import.meta.url));

function synthEip55Addresses(count) {
  const out = [];
  for (let idx = 0; idx < count; idx++) {
    let z = (BigInt(idx) + 0x9e3779b97f4a7c15n) & 0xffffffffffffffffn;
    let addr = '0x';
    for (let i = 0; i < 40; i++) {
      z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & 0xffffffffffffffffn;
      z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & 0xffffffffffffffffn;
      z = (z ^ (z >> 31n)) & 0xffffffffffffffffn;
      const n = Number((z >> BigInt((i % 8) * 4)) & 0xfn);
      let ch = n.toString(16);
      if (n >= 10 && ((z >> 3n) & 1n) === 1n) ch = ch.toUpperCase();
      addr += ch;
    }
    out.push(addr);
  }
  return out;
}

test('Blockies-Haring derives mirrored 8x8 grids and clamped #000000 specs across 1,000 EIP-55 addresses', () => {
  const addrs = synthEip55Addresses(1000);
  for (const addr of addrs) {
    const mixed = deriveBlockiesHaringSpec(addr);
    const lower = deriveBlockiesHaringSpec(addr.toLowerCase());
    assert.deepEqual(mixed, lower);
    assert.equal(mixed.canonical_seed_0x, addr.toLowerCase());
    assert.equal(mixed.outline_hex, '#000000');
    assert.equal(mixed.grid_8x8.length, 64);
    assert.ok(mixed.haring_body_hsl.s_pct >= 65.0);
    assert.ok(mixed.haring_body_hsl.l_pct >= 42.0 && mixed.haring_body_hsl.l_pct <= 72.0);
    for (let y = 0; y < 8; y++) {
      for (let x = 0; x < 4; x++) {
        assert.equal(mixed.grid_8x8[y * 8 + x], mixed.grid_8x8[y * 8 + (7 - x)]);
      }
    }
  }
});

test('vision frontendDist copy of blockies-haring.js is byte-identical to shared-ui SSOT (V-01)', () => {
  const ssotPath = resolve(__dirname, '../js/blockies-haring.js');
  const visionCopyPath = resolve(__dirname, '../../vision/ui/js/blockies-haring.js');
  const ssotBytes = readFileSync(ssotPath);
  const visionBytes = readFileSync(visionCopyPath);
  assert.deepEqual(visionBytes, ssotBytes, 'apps/vision/ui/js/blockies-haring.js must match shared-ui SSOT');
});
