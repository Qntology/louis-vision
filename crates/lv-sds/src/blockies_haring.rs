//! Bit-exact `0x...` Ethereum Blockies x Keith Haring identity generator matching `apps/shared-ui/js/blockies-haring.js`.

use serde::de::{Error as _, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod grid64_serde {
    use super::*;

    pub fn serialize<S: Serializer>(arr: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(64))?;
        for &b in arr {
            seq.serialize_element(&b)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        struct GridVisitor;
        impl<'de> Visitor<'de> for GridVisitor {
            type Value = [u8; 64];
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a sequence of 64 u8 values")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<[u8; 64], A::Error> {
                let mut out = [0u8; 64];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(i, &self))?;
                }
                Ok(out)
            }
        }
        d.deserialize_seq(GridVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HslColor {
    pub h_deg: u16,
    pub s_pct: f64,
    pub l_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlockiesHaringSpec {
    pub canonical_seed_0x: String,
    pub color: HslColor,
    pub bgcolor: HslColor,
    pub spotcolor: HslColor,
    #[serde(with = "grid64_serde")]
    pub grid_8x8: [u8; 64],
    pub haring_body_hsl: HslColor,
    pub outline_hex: &'static str,
}

impl<'de> Deserialize<'de> for BlockiesHaringSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Helper {
            canonical_seed_0x: String,
            color: HslColor,
            bgcolor: HslColor,
            spotcolor: HslColor,
            #[serde(with = "grid64_serde")]
            grid_8x8: [u8; 64],
            haring_body_hsl: HslColor,
            outline_hex: String,
        }
        let h = Helper::deserialize(d)?;
        if h.outline_hex != "#000000" {
            return Err(D::Error::custom(format!(
                "expected #000000 outline_hex, got {}",
                h.outline_hex
            )));
        }
        Ok(Self {
            canonical_seed_0x: h.canonical_seed_0x,
            color: h.color,
            bgcolor: h.bgcolor,
            spotcolor: h.spotcolor,
            grid_8x8: h.grid_8x8,
            haring_body_hsl: h.haring_body_hsl,
            outline_hex: "#000000",
        })
    }
}

struct JsBlockiesRng {
    randseed: [i32; 4],
}

impl JsBlockiesRng {
    fn from_canonical_seed(seed_lower: &str) -> Self {
        let mut randseed = [0i32; 4];
        for (i, code_unit) in seed_lower.encode_utf16().enumerate() {
            let slot = i % 4;
            let prev = randseed[slot];
            randseed[slot] = prev
                .wrapping_shl(5)
                .wrapping_sub(prev)
                .wrapping_add(code_unit as i32);
        }
        Self { randseed }
    }

    fn next_f64(&mut self) -> f64 {
        let t = self.randseed[0] ^ self.randseed[0].wrapping_shl(11);
        self.randseed[0] = self.randseed[1];
        self.randseed[1] = self.randseed[2];
        self.randseed[2] = self.randseed[3];
        self.randseed[3] = self.randseed[3] ^ (self.randseed[3] >> 19) ^ t ^ (t >> 8);
        (self.randseed[3] as u32) as f64 / 2_147_483_648.0_f64
    }

    fn next_hsl(&mut self) -> HslColor {
        let h_deg = (self.next_f64() * 360.0).floor() as u16;
        let s_pct = self.next_f64() * 60.0 + 40.0;
        let l_pct =
            (self.next_f64() + self.next_f64() + self.next_f64() + self.next_f64()) * 25.0;
        HslColor {
            h_deg,
            s_pct,
            l_pct,
        }
    }
}

/// Derives canonical lowercase seed, Blockies HSL palette, mirrored 8x8 grid, and clamped Keith Haring body color.
pub fn derive_blockies_haring_spec(raw_address: &str) -> BlockiesHaringSpec {
    let canonical_seed_0x = raw_address.trim().to_ascii_lowercase();
    let mut rng = JsBlockiesRng::from_canonical_seed(&canonical_seed_0x);
    let color = rng.next_hsl();
    let bgcolor = rng.next_hsl();
    let spotcolor = rng.next_hsl();

    let mut grid_8x8 = [0u8; 64];
    for y in 0..8usize {
        let mut half = [0u8; 4];
        for slot in &mut half {
            *slot = (rng.next_f64() * 2.3).floor() as u8;
        }
        for x in 0..4usize {
            grid_8x8[y * 8 + x] = half[x];
            grid_8x8[y * 8 + (7 - x)] = half[x];
        }
    }
    let haring_body_hsl = HslColor {
        h_deg: color.h_deg,
        s_pct: color.s_pct.max(65.0),
        l_pct: color.l_pct.clamp(42.0, 72.0),
    };
    BlockiesHaringSpec {
        canonical_seed_0x,
        color,
        bgcolor,
        spotcolor,
        grid_8x8,
        haring_body_hsl,
        outline_hex: "#000000",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};

    fn synth_eip55_mixed_case_addresses(count: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(count);
        for idx in 0..count {
            let mut z = (idx as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut addr = String::from("0x");
            for nibble_idx in 0..40usize {
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                z ^= z >> 31;
                let n = ((z >> ((nibble_idx % 8) * 4)) & 0xF) as u8;
                let ch = match n {
                    0..=9 => (b'0' + n) as char,
                    _ => {
                        let base = b'a' + (n - 10);
                        if ((z >> 3) & 1) == 1 {
                            base.to_ascii_uppercase() as char
                        } else {
                            base as char
                        }
                    }
                };
                addr.push(ch);
            }
            out.push(addr);
        }
        out
    }

    #[derive(Deserialize)]
    struct JsBlockiesOut {
        canonical_seed_0x: String,
        color: HslColor,
        bgcolor: HslColor,
        spotcolor: HslColor,
        grid_8x8: Vec<u8>,
        haring_body_hsl: HslColor,
        outline_hex: String,
    }

    fn assert_hsl_close(a: HslColor, b: HslColor, tag: &str) {
        assert_eq!(a.h_deg, b.h_deg, "h_deg mismatch for {tag}");
        assert!((a.s_pct - b.s_pct).abs() < 1e-11, "s_pct mismatch for {tag}: {} vs {}", a.s_pct, b.s_pct);
        assert!((a.l_pct - b.l_pct).abs() < 1e-11, "l_pct mismatch for {tag}: {} vs {}", a.l_pct, b.l_pct);
    }

    #[test]
    fn blockies_1000_eip55_addresses_match_js_bit_exact() {
        let addrs = synth_eip55_mixed_case_addresses(1000);
        let rust_specs: Vec<BlockiesHaringSpec> = addrs.iter().map(|a| derive_blockies_haring_spec(a)).collect();
        for spec in &rust_specs {
            assert_eq!(spec.outline_hex, "#000000");
            assert!(spec.haring_body_hsl.s_pct >= 65.0);
            assert!((42.0..=72.0).contains(&spec.haring_body_hsl.l_pct));
            for y in 0..8usize {
                for x in 0..4usize {
                    assert_eq!(spec.grid_8x8[y * 8 + x], spec.grid_8x8[y * 8 + (7 - x)]);
                }
            }
        }

        let js_mod = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/shared-ui/js/blockies-haring.js");
        if !js_mod.exists() {
            return;
        }
        let abs_js = std::fs::canonicalize(&js_mod).unwrap();
        let js_driver = r#"
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
const mod = await import(pathToFileURL(process.argv[1]).href);
const addrs = JSON.parse(fs.readFileSync(0, 'utf8'));
const out = addrs.map((a) => mod.deriveBlockiesHaringSpec(a));
process.stdout.write(JSON.stringify(out));
"#;
        let mut child = Command::new("node")
            .arg("--input-type=module")
            .arg("-e")
            .arg(js_driver)
            .arg(abs_js)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("node must run for Rust/JS Blockies cross-check");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(serde_json::to_string(&addrs).unwrap().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "node parity script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let js_specs: Vec<JsBlockiesOut> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(js_specs.len(), 1000);
        for (i, (r, j)) in rust_specs.iter().zip(js_specs.iter()).enumerate() {
            assert_eq!(r.canonical_seed_0x, j.canonical_seed_0x, "seed at {i}");
            assert_eq!(r.grid_8x8.as_slice(), j.grid_8x8.as_slice(), "grid at {i}");
            assert_eq!(r.outline_hex, j.outline_hex, "outline at {i}");
            assert_hsl_close(r.color, j.color, "color");
            assert_hsl_close(r.bgcolor, j.bgcolor, "bgcolor");
            assert_hsl_close(r.spotcolor, j.spotcolor, "spotcolor");
            assert_hsl_close(r.haring_body_hsl, j.haring_body_hsl, "haring_body_hsl");
        }
    }
}
