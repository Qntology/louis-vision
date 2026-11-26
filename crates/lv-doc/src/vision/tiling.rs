//! A0 large-format blueprint 4x4 grid tiling (`overlap = 0.10`), partial `covering` rescan, `LedgerBox`, and NMS `merge`.

use serde::Serialize;

pub const A0_DEFAULT_ROWS: u32 = 4;
pub const A0_DEFAULT_COLS: u32 = 4;
pub const A0_DEFAULT_OVERLAP: f32 = 0.10;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tile {
    pub id: String,
    pub row: u32,
    pub col: u32,
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Tile {
    #[inline]
    pub fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }

    /// Maps normalized coordinates inside this tile `[x0, y0, x1, y1]` to full A0 normalized coordinates.
    pub fn to_global(&self, local: [f32; 4], page: (u32, u32)) -> [f32; 4] {
        let (w, h) = (self.width() as f32, self.height() as f32);
        let (pw, ph) = (page.0.max(1) as f32, page.1.max(1) as f32);
        [
            (self.x0 as f32 + local[0] * w) / pw,
            (self.y0 as f32 + local[1] * h) / ph,
            (self.x0 as f32 + local[2] * w) / pw,
            (self.y0 as f32 + local[3] * h) / ph,
        ]
    }

    pub fn contains(&self, global: [f32; 4], page: (u32, u32)) -> bool {
        let (pw, ph) = (page.0 as f32, page.1 as f32);
        global[0] * pw >= self.x0 as f32
            && global[2] * pw <= self.x1 as f32
            && global[1] * ph >= self.y0 as f32
            && global[3] * ph <= self.y1 as f32
    }

    pub fn touches(&self, global: [f32; 4], page: (u32, u32)) -> bool {
        let (pw, ph) = (page.0 as f32, page.1 as f32);
        global[0] * pw < self.x1 as f32
            && global[2] * pw > self.x0 as f32
            && global[1] * ph < self.y1 as f32
            && global[3] * ph > self.y0 as f32
    }
}

/// Partitions `page = (width, height)` into `rows x cols` overlapping tiles.
pub fn tiles(page: (u32, u32), rows: u32, cols: u32, overlap: f32) -> Vec<Tile> {
    let rows = rows.max(1);
    let cols = cols.max(1);
    let overlap = overlap.clamp(0.0, 0.45);
    let (w, h) = (page.0 as f32, page.1 as f32);
    let tw = w / (cols as f32 * (1.0 - overlap) + overlap);
    let th = h / (rows as f32 * (1.0 - overlap) + overlap);
    let mut out = Vec::with_capacity((rows * cols) as usize);
    for r in 0..rows {
        for c in 0..cols {
            let x0 = (c as f32 * tw * (1.0 - overlap)).round() as u32;
            let y0 = (r as f32 * th * (1.0 - overlap)).round() as u32;
            let x1 = if c + 1 == cols {
                page.0
            } else {
                ((x0 as f32) + tw).round().min(w) as u32
            };
            let y1 = if r + 1 == rows {
                page.1
            } else {
                ((y0 as f32) + th).round().min(h) as u32
            };
            let id = format!("{}{}", (b'A' + (r.min(25)) as u8) as char, c + 1);
            out.push(Tile { id, row: r, col: c, x0, y0, x1, y1 });
        }
    }
    out
}

/// Default A0 `4 x 4` grid with `overlap = 0.10`.
pub fn a0_tiles(page: (u32, u32)) -> Vec<Tile> {
    tiles(page, A0_DEFAULT_ROWS, A0_DEFAULT_COLS, A0_DEFAULT_OVERLAP)
}

/// Finds the tile whose center lies closest to the center of `global`.
pub fn tile_of<'a>(tiles: &'a [Tile], global: [f32; 4], page: (u32, u32)) -> Option<&'a Tile> {
    let cx = (global[0] + global[2]) * 0.5 * page.0 as f32;
    let cy = (global[1] + global[3]) * 0.5 * page.1 as f32;
    tiles
        .iter()
        .filter(|t| cx >= t.x0 as f32 && cx < t.x1 as f32 && cy >= t.y0 as f32 && cy < t.y1 as f32)
        .min_by(|a, b| {
            let d = |t: &Tile| {
                ((t.x0 + t.x1) as f32 * 0.5 - cx).powi(2)
                    + ((t.y0 + t.y1) as f32 * 0.5 - cy).powi(2)
            };
            d(a).total_cmp(&d(b))
        })
}

/// Partial rescan: returns either the single tile fully containing `global`, or every touching tile.
pub fn covering<'a>(tiles: &'a [Tile], global: [f32; 4], page: (u32, u32)) -> Vec<&'a Tile> {
    if let Some(t) = tiles.iter().find(|t| t.contains(global, page)) {
        return vec![t];
    }
    tiles.iter().filter(|t| t.touches(global, page)).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LedgerBox {
    pub kks: String,
    pub bbox: [f32; 4],
    pub px: [u32; 4],
    pub tile: Option<String>,
}

/// Computes pixel boxes and home tiles for normalized ledger equipment boxes on `page`.
pub fn ledger_boxes(
    items: &[(&str, [f32; 4])],
    page: (u32, u32),
    only: Option<&[&str]>,
) -> Vec<LedgerBox> {
    let grid = a0_tiles(page);
    let (w, h) = (page.0 as f32, page.1 as f32);
    items
        .iter()
        .filter(|(kks, _)| only.is_none_or(|o| o.contains(kks)))
        .map(|&(kks, g)| {
            let px = [
                (g[0] * w).floor().max(0.0) as u32,
                (g[1] * h).floor().max(0.0) as u32,
                ((g[2] * w).ceil().max(0.0) as u32).min(page.0),
                ((g[3] * h).ceil().max(0.0) as u32).min(page.1),
            ];
            LedgerBox {
                kks: kks.to_string(),
                bbox: g,
                px,
                tile: tile_of(&grid, g, page).map(|t| t.id.clone()),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GlobalHit {
    pub code: String,
    pub bbox: [f32; 4],
    pub tile: String,
    pub corrected: bool,
}

pub fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let area = |r: [f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let union = area(a) + area(b) - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn contained(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let area = |r: [f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let small = area(a).min(area(b));
    if small <= 0.0 {
        0.0
    } else {
        ix * iy / small
    }
}

/// Deduplicates identical equipment codes detected inside overlapping A0 tile bands (NMS).
pub fn merge(mut hits: Vec<GlobalHit>) -> Vec<GlobalHit> {
    let area = |r: [f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    hits.sort_by(|a, b| area(b.bbox).total_cmp(&area(a.bbox)).then(a.code.cmp(&b.code)));
    let mut kept: Vec<GlobalHit> = Vec::new();
    for h in hits {
        let dup = kept
            .iter()
            .any(|k| k.code == h.code && (iou(k.bbox, h.bbox) >= 0.5 || contained(k.bbox, h.bbox) >= 0.8));
        if !dup {
            kept.push(h);
        }
    }
    kept.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]).then(a.bbox[0].total_cmp(&b.bbox[0])));
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a0_4x4_grid_and_overlap_nms_work() {
        let page = (7016, 4961);
        let ts = a0_tiles(page);
        assert_eq!(ts.len(), 16);
        assert_eq!(ts[0].id, "A1");
        assert_eq!(ts[6].id, "B3");
        assert_eq!(ts[15].id, "D4");
        assert_eq!((ts[15].x1, ts[15].y1), page);

        let b3 = ts.iter().find(|t| t.id == "B3").unwrap();
        let mid = b3.to_global([0.5, 0.5, 0.5, 0.5], page);
        assert_eq!(tile_of(&ts, mid, page).unwrap().id, "B3");

        let a = GlobalHit { code: "10LAB20AA102".into(), bbox: [0.70, 0.30, 0.75, 0.33], tile: "B3".into(), corrected: false };
        let b = GlobalHit { code: "10LAB20AA102".into(), bbox: [0.705, 0.301, 0.75, 0.33], tile: "B4".into(), corrected: false };
        let far = GlobalHit { code: "10LAB20AA102".into(), bbox: [0.10, 0.80, 0.15, 0.83], tile: "D1".into(), corrected: false };
        assert_eq!(merge(vec![b, a, far]).len(), 2);
    }
}
