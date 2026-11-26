//! Axis 3 (`Scan2DDegraded`): deterministic in-memory scans, borderless `Luma=180` tables, `TitleBlock` 5/5, `ZZD1` deltas, `ZZS1` legend few-shot, and A0 `4x4` tiling.

use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use lv_doc::vision::{
    delta::{chromatic_stroke_count, ink_changes, DeltaParams, Master},
    ink::extract_ink_page,
    plan::{classify_protected_role, Baseline, ProtectedRole},
    symbols::{SymbolDict, SymbolFeature, NONE_CLASS},
    table_grid_reverse::{inject_virtual_gridlines, VIRTUAL_GRIDLINE_LUMA},
    tiling::{a0_tiles, covering, ledger_boxes, merge, GlobalHit},
    DocPatchGrid,
};

#[derive(Debug, Clone)]
pub struct Scan2DReport {
    pub virtual_cols: usize,
    pub virtual_rows: usize,
    pub virtual_luma_ok: bool,
    pub titleblock_protected: bool,
    pub chroma_strokes: usize,
    pub zzd1_changed_blocks: usize,
    pub legend_recall_ok: bool,
    pub legend_none_ok: bool,
    pub a0_tile_count: usize,
    pub a0_boundary_hits: usize,
    pub a0_merged_hits: usize,
}

fn fill_rect_gray(img: &mut GrayImage, x0: u32, y0: u32, x1: u32, y1: u32, val: u8) {
    let (w, h) = img.dimensions();
    for y in y0.min(h)..y1.min(h) {
        for x in x0.min(w)..x1.min(w) {
            img.put_pixel(x, y, Luma([val]));
        }
    }
}

fn fill_rect_rgb(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, val: [u8; 3]) {
    let (w, h) = img.dimensions();
    for y in y0.min(h)..y1.min(h) {
        for x in x0.min(w)..x1.min(w) {
            img.put_pixel(x, y, Rgb(val));
        }
    }
}

pub fn run_scan_2d_degraded_probe() -> Result<Scan2DReport> {
    // 1. Borderless 4-column table -> inject_virtual_gridlines (Luma = 180)
    let mut table_gray = GrayImage::from_pixel(320, 160, Luma([255]));
    for row in 0..4u32 {
        let y0 = 20 + row * 28;
        for col in 0..4u32 {
            let x0 = 16 + col * 72;
            fill_rect_gray(&mut table_gray, x0, y0, x0 + 36, y0 + 14, 40);
        }
    }
    let (grid_img, vx, vy) =
        inject_virtual_gridlines(&table_gray, 2).expect("borderless gridlines");
    let luma_ok = vx
        .first()
        .is_some_and(|&x| grid_img.get_pixel(x, 40)[0] == VIRTUAL_GRIDLINE_LUMA);
    let _ink = extract_ink_page(&grid_img);

    // 2. ZZD1 red-line chromatic stroke + block delta + bottom-right TitleBlock 5/5 protection
    let mut prev = RgbImage::from_pixel(240, 180, Rgb([255, 255, 255]));
    fill_rect_rgb(&mut prev, 156, 140, 224, 170, [30, 30, 30]);
    let mut curr = prev.clone();
    fill_rect_rgb(&mut curr, 24, 24, 96, 60, [220, 28, 28]);

    let prev_gray = DynamicImage::ImageRgb8(prev).to_luma8();
    let curr_gray = DynamicImage::ImageRgb8(curr.clone()).to_luma8();
    let prev_ink = extract_ink_page(&prev_gray);
    let scan_ink = extract_ink_page(&curr_gray);
    let dummy_grid = DocPatchGrid {
        grid_rows: 4,
        grid_cols: 4,
        orig_width: 240,
        orig_height: 180,
        patches: vec![vec![0.1f32; 16]; 16],
    };
    let master = Master::new("MEP-103-CDU", &dummy_grid, prev_ink, 3);
    let diff = ink_changes(&master, &scan_ink, &DeltaParams::default())?;
    let changed = diff.changed(0.03);
    let chroma_strokes = chromatic_stroke_count(curr.as_raw(), &scan_ink);
    let tb_protected =
        classify_protected_role((156, 140, 224, 170), 240, 180, Baseline::Best)
            == ProtectedRole::TitleBlock;
    anyhow::ensure!(changed >= 1 && chroma_strokes > 0 && tb_protected);

    // 3. ZZS1 legend few-shot without retraining + NONE_CLASS rejection
    let make_vec = |hot_lo: usize, hot_hi: usize| {
        let mut v = vec![0.0f32; 32];
        for i in hot_lo..hot_hi {
            v[i] = 1.0;
        }
        v
    };
    let dict = SymbolDict::from_legend_samples(
        SymbolFeature::Pooled,
        vec![
            NONE_CLASS.to_string(),
            "cryo_valve".to_string(),
            "immersion_tank".to_string(),
        ],
        &[
            (0, make_vec(24, 32)),
            (1, make_vec(0, 8)),
            (2, make_vec(12, 20)),
        ],
    )?;
    let bytes_v2 = dict.to_bytes_v2(SymbolFeature::Pooled, 0.72, 0.05);
    let roundtrip = SymbolDict::from_bytes(&bytes_v2)?;
    let hit_cryo = roundtrip.classify_with(&make_vec(0, 8), 0.72, 0.05);
    let hit_none = roundtrip.classify_with(&make_vec(24, 32), 0.72, 0.05);

    // 4. A0 4x4 grid tiling + boundary-straddling dedup merge
    let page = (7016, 4961);
    let ts = a0_tiles(page);
    let boxes = ledger_boxes(
        &[
            ("10LAB20AA102", [0.48, 0.25, 0.54, 0.30]),
            ("DC1CDU20AA02", [0.10, 0.10, 0.14, 0.14]),
        ],
        page,
        None,
    );
    let cov = covering(&ts, [0.48, 0.25, 0.54, 0.30], page);
    let merged = merge(vec![
        GlobalHit {
            code: "10LAB20AA102".into(),
            bbox: [0.48, 0.25, 0.54, 0.30],
            tile: "B2".into(),
            corrected: false,
        },
        GlobalHit {
            code: "10LAB20AA102".into(),
            bbox: [0.482, 0.251, 0.54, 0.30],
            tile: "B3".into(),
            corrected: false,
        },
    ]);
    anyhow::ensure!(boxes.len() == 2 && !cov.is_empty());

    Ok(Scan2DReport {
        virtual_cols: vx.len(),
        virtual_rows: vy.len(),
        virtual_luma_ok: luma_ok,
        titleblock_protected: tb_protected,
        chroma_strokes,
        zzd1_changed_blocks: changed,
        legend_recall_ok: hit_cryo.class.as_deref() == Some("cryo_valve"),
        legend_none_ok: hit_none.class.is_none(),
        a0_tile_count: ts.len(),
        a0_boundary_hits: cov.len(),
        a0_merged_hits: merged.len(),
    })
}
