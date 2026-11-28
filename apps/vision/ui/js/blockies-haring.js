// apps/shared-ui/js/blockies-haring.js
// Pure ES module bit-equivalent to crates/lv-sds/src/blockies_haring.rs (UTF-16, i32 shifts, / 2147483648.0).

class JsBlockiesRng {
  constructor(seedLower) {
    this.randseed = new Int32Array(4);
    for (let i = 0; i < seedLower.length; i++) {
      const slot = i % 4;
      const prev = this.randseed[slot];
      this.randseed[slot] = ((prev << 5) - prev + seedLower.charCodeAt(i)) | 0;
    }
  }

  nextF64() {
    const t = (this.randseed[0] ^ (this.randseed[0] << 11)) | 0;
    this.randseed[0] = this.randseed[1];
    this.randseed[1] = this.randseed[2];
    this.randseed[2] = this.randseed[3];
    this.randseed[3] = (this.randseed[3] ^ (this.randseed[3] >> 19) ^ t ^ (t >> 8)) | 0;
    return (this.randseed[3] >>> 0) / 2147483648.0;
  }

  nextHsl() {
    const h_deg = Math.floor(this.nextF64() * 360.0);
    const s_pct = this.nextF64() * 60.0 + 40.0;
    const l_pct = (this.nextF64() + this.nextF64() + this.nextF64() + this.nextF64()) * 25.0;
    return { h_deg, s_pct, l_pct };
  }
}

export function deriveBlockiesHaringSpec(rawAddress) {
  const canonical_seed_0x = String(rawAddress ?? '').trim().toLowerCase();
  const rng = new JsBlockiesRng(canonical_seed_0x);
  const color = rng.nextHsl();
  const bgcolor = rng.nextHsl();
  const spotcolor = rng.nextHsl();

  const grid_8x8 = new Array(64).fill(0);
  for (let y = 0; y < 8; y++) {
    const half = [0, 0, 0, 0];
    for (let x = 0; x < 4; x++) {
      half[x] = Math.floor(rng.nextF64() * 2.3);
    }
    for (let x = 0; x < 4; x++) {
      grid_8x8[y * 8 + x] = half[x];
      grid_8x8[y * 8 + (7 - x)] = half[x];
    }
  }
  const haring_body_hsl = {
    h_deg: color.h_deg,
    s_pct: Math.max(color.s_pct, 65.0),
    l_pct: Math.min(72.0, Math.max(42.0, color.l_pct)),
  };
  return {
    canonical_seed_0x,
    color,
    bgcolor,
    spotcolor,
    grid_8x8,
    haring_body_hsl,
    outline_hex: '#000000',
  };
}
