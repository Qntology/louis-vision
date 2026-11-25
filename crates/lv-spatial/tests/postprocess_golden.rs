use image::GrayImage;
use lv_spatial::mask_ops::postprocess::{
    apply, apply_border_fix, fill_small_holes, grow_shrink, remove_small_dots, PostprocessParams,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct GoldenFile {
    cases: Vec<GoldenCase>,
}

#[derive(Deserialize)]
struct GoldenCase {
    holes: u32,
    dots: u32,
    border_fix: u32,
    grow: i32,
    input: Vec<String>,
    expected: ExpectedStages,
}

#[derive(Deserialize)]
struct ExpectedStages {
    holes: Vec<String>,
    dots: Vec<String>,
    border_fix: Vec<String>,
    grow: Vec<String>,
    pipeline: Vec<String>,
}

fn decode_mask(rows: &[String]) -> GrayImage {
    let h = rows.len() as u32;
    let w = rows[0].len() as u32;
    let mut buf = Vec::with_capacity((w * h) as usize);
    for row in rows {
        for b in row.bytes() {
            buf.push(if b == b'1' { 255 } else { 0 });
        }
    }
    GrayImage::from_raw(w, h, buf).expect("golden mask dimensions match")
}

#[test]
fn opencv_4_14_postprocess_golden_parity() {
    let raw = include_str!("fixtures/postprocess_golden.json");
    let golden: GoldenFile = serde_json::from_str(raw).expect("valid golden json");
    assert!(!golden.cases.is_empty());

    for (idx, case) in golden.cases.iter().enumerate() {
        let input = decode_mask(&case.input);
        let params = PostprocessParams {
            holes: case.holes,
            dots: case.dots,
            border_fix: case.border_fix,
            grow: case.grow,
        };
        if case.holes > 0 {
            let got = fill_small_holes(&input, case.holes);
            assert_eq!(got, decode_mask(&case.expected.holes), "case {idx} holes");
        }
        if case.dots > 0 {
            let got = remove_small_dots(&input, case.dots);
            assert_eq!(got, decode_mask(&case.expected.dots), "case {idx} dots");
        }
        if case.border_fix > 0 {
            let got = apply_border_fix(&input, case.border_fix).expect("border_fix");
            assert_eq!(got, decode_mask(&case.expected.border_fix), "case {idx} border_fix");
        }
        if case.grow != 0 {
            let got = grow_shrink(&input, case.grow);
            assert_eq!(got, decode_mask(&case.expected.grow), "case {idx} grow");
        }
        let pipeline = apply(&input, &params).expect("pipeline");
        assert_eq!(pipeline, decode_mask(&case.expected.pipeline), "case {idx} pipeline");
    }
}
