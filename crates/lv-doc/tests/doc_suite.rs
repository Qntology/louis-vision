//! Integration tests for `lv-doc` (`vision` + `docs-io` + `writer` round-trip gates).

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use image::{GrayImage, Luma, Rgb, RgbImage};
use lv_doc::docs::{
    hwp_native::{parse_hwp5, parse_hwpx},
    parse_docs_bytes,
    sheet_pdf::{parse_csv, repair_hangul_false_kerning},
    DocBlock, TrackChange, VisionFallbackNeed,
};
use lv_doc::vision::{
    delta::{
        evaluate_delta_gate_ab, frame_offset, ink_changes, trim_ignored_with_baseline, DeltaParams,
        Master, MasterSet, MatchMethod,
    },
    grounding::{
        bank_neutral_with_titleblock_immunity, padded_retry_bbox, strip_name_residue,
        trim_name_residue, verify_claims_v2, GroundingClaim, StandardDocLexicon, VerdictGate,
        BLANK_SURGE_SIGMA,
    },
    ink::InkPage,
    lattice_verify::{verify_ledger_with_shadow_top3, LatticeVerifyDecision, ShadowToken},
    layout_cache::LayoutCache,
    legibility::{LegibilityMap, PatchLegibility},
    phrase_cache::PhraseCache,
    plan::{classify_protected_role, Baseline, CropKind, CropPlan, PagePlan, ProtectedRole},
    symbols::{SymbolDict, SymbolEntry},
    table_grid_reverse::{
        inject_virtual_gridlines, plan_solid_table_overlap, preprocess_single_text_crop,
        PIPE_ATTENUATE_LUMA, VIRTUAL_GRIDLINE_LUMA,
    },
    DocPatchGrid,
};
use lv_doc::writer::{build_docx, build_hwpx, fill_docx_template_text, NoticeSpec, WriterGuard};
use premium_discount_eval::gate_ab::{ArmRunSummary, DetectionCounters};
use serde_json::json;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn make_grid(rows: usize, cols: usize, dim: usize, base: f32, hi: Option<(usize, f32)>) -> DocPatchGrid {
    let mut patches = vec![vec![base; dim]; rows * cols];
    if let Some((idx, val)) = hi {
        if idx < patches.len() {
            patches[idx][0] = val;
        }
    }
    DocPatchGrid { grid_rows: rows, grid_cols: cols, orig_width: 2000, orig_height: 1400, patches }
}

fn synthetic_ink(w: u32, h: u32, rects: &[(u32, u32, u32, u32)]) -> InkPage {
    let mut g = GrayImage::from_pixel(w, h, Luma([252]));
    for &(x0, y0, x1, y1) in rects {
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                g.put_pixel(x, y, Luma([20]));
            }
        }
    }
    InkPage::new(&g)
}

#[test]
fn gate_titleblock_five_of_five_preserved_on_best_and_a0_each() {
    // 5 titleblock locations (bottom-right & top band) under Baseline::Best
    let tb_boxes = [
        (1200, 1050, 1950, 1360),
        (1350, 1100, 1980, 1380),
        (1400, 1180, 1990, 1390),
        (60, 28, 1940, 133),
        (1500, 1200, 1960, 1350),
    ];
    let mut hits = 0usize;
    for b in tb_boxes {
        assert_eq!(classify_protected_role(b, 2000, 1400, Baseline::Best), ProtectedRole::TitleBlock);
        let mut plan = PagePlan {
            width: 2000,
            height: 1400,
            crops: vec![CropPlan { kind: CropKind::Form, bbox: b, upscale: 1.0, guide_margin: None }],
            dropped: Vec::new(),
            blank: Vec::new(),
            unfiltered: false,
        };
        let ink = synthetic_ink(2000, 1400, &[b]);
        trim_ignored_with_baseline(&mut plan, &ink, &[b], Baseline::Best);
        if plan.crops.len() == 1 {
            hits += 1;
        }
    }
    assert_eq!(hits, 5, "TitleBlock recovery must be 5/5 on Baseline::Best");

    // Double centering immunity for TitleBlock
    let raw = vec![vec![0.9, 0.8], vec![0.1, 0.2], vec![0.85, 0.9]];
    let centered = bank_neutral_with_titleblock_immunity(
        &raw,
        &[tb_boxes[0], (200, 400, 600, 500), tb_boxes[3]],
        2000,
        1400,
        Baseline::Best,
    );
    assert_eq!(centered[0], raw[0]);
    assert_eq!(centered[2], raw[2]);

    // A0 tiled Baseline::Each disables TitleBlock immunity per tile
    assert_eq!(
        classify_protected_role(tb_boxes[0], 2000, 1400, Baseline::Each),
        ProtectedRole::Standard
    );
}

#[test]
fn gate_grounding_pcs_exempt_echo_block_blank_surge_and_padded_retry() {
    let mut patches = vec![vec![0.0; 8]; 4];
    patches[0] = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    patches[1] = vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    patches[2] = vec![0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    patches[3] = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    let grid = DocPatchGrid { grid_rows: 2, grid_cols: 2, orig_width: 200, orig_height: 200, patches };

    let leg = LegibilityMap {
        rows: 2,
        cols: 2,
        ink: vec![20.0, 20.0, 0.5, 20.0],
        sharpness: vec![2.0, 2.0, 0.1, 2.0],
        verdict: vec![
            PatchLegibility::Legible,
            PatchLegibility::Legible,
            PatchLegibility::Blank,
            PatchLegibility::Legible,
        ],
        split_applied: false,
        ink_gate: 5.0,
        sharp_gate: 0.5,
    };

    let claims = vec![
        GroundingClaim { category: "items".into(), field: "unit".into(), value: "PCS".into(), bbox: (10, 10, 80, 80) },
        GroundingClaim { category: "header".into(), field: "pol".into(), value: "AIRWAYBILL / BILL OF LADING".into(), bbox: (10, 10, 90, 90) },
        GroundingClaim { category: "parties".into(), field: "party_role".into(), value: "SHIPPER".into(), bbox: (10, 10, 90, 90) },
        GroundingClaim { category: "items".into(), field: "amount".into(), value: "999999".into(), bbox: (10, 110, 90, 190) },
    ];
    let embs = vec![
        None,
        Some(vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        None,
        Some(vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ];

    let verdicts = verify_claims_v2(&grid, &leg, &claims, &embs, &StandardDocLexicon::default(), Some(0.40));
    assert!(verdicts[0].accepted && verdicts[0].gate == VerdictGate::Accepted, "unit=PCS exempted");
    assert!(!verdicts[1].accepted && verdicts[1].gate == VerdictGate::Format, "printed label echo blocked");
    assert!(verdicts[2].accepted, "party_role exempted");
    assert!(!verdicts[3].accepted && BLANK_SURGE_SIGMA == 3.0, "blank surge rejected");

    // 15% padded retry triggers when neighbor patch is Legible
    let expanded = padded_retry_bbox((10, 110, 90, 190), &leg, 200, 200).expect("legible neighbor present");
    assert!(expanded.0 < 10 && expanded.2 > 90);

    // Name residue trim
    assert_eq!(trim_name_residue("shipper_name", "-Jane Smith /").as_deref(), Some("Jane Smith"));
    let mut mut_claims = vec![GroundingClaim { category: "p".into(), field: "buyer_name".into(), value: "-Jane Smith /".into(), bbox: (0, 0, 10, 10) }];
    let mut data = json!({ "buyer_name": "-Jane Smith /" }).as_object().unwrap().clone();
    assert_eq!(strip_name_residue(&mut mut_claims, &mut data), 1);
    assert_eq!(data["buyer_name"], "Jane Smith");
}

#[test]
fn gate_three_way_crop_luma_215_and_180_and_solid_overlap() {
    // 1. Single text crop: dilate(2) protects red pen pixel neighborhood, background pipe attenuated to Luma 215
    let mut rgb = RgbImage::from_pixel(40, 20, Rgb([250, 250, 250]));
    let mut pen_mask = vec![0u8; 40 * 20];
    for x in 0..40 {
        rgb.put_pixel(x, 10, Rgb([140, 140, 140])); // pipe line
    }
    rgb.put_pixel(20, 10, Rgb([200, 20, 20])); // pen stroke
    pen_mask[10 * 40 + 20] = 1;
    let out1 = preprocess_single_text_crop(&rgb, &pen_mask);
    assert_eq!(out1.get_pixel(2, 10)[0], PIPE_ATTENUATE_LUMA, "pipe outside dilate(2) attenuated to 215");
    assert_ne!(out1.get_pixel(20, 10)[0], PIPE_ATTENUATE_LUMA, "pen center preserved");
    assert_ne!(out1.get_pixel(22, 10)[0], PIPE_ATTENUATE_LUMA, "dilate(2) halo around pen preserved");

    // 2. Solid table 25% overlap
    let tiles = plan_solid_table_overlap((0, 0, 800, 400), 3);
    assert_eq!(tiles.len(), 3);
    assert!(tiles[0].bbox.3 > tiles[1].bbox.1, "adjacent tiles overlap");

    // 3. Borderless table Luma=180 virtual gridlines (2 rows x 4 cols)
    let mut tbl = GrayImage::from_pixel(200, 80, Luma([252]));
    for y_band in [12u32, 46] {
        for col in [15u32, 60, 105, 150] {
            for y in y_band..y_band + 16 {
                for x in col..col + 24 {
                    tbl.put_pixel(x, y, Luma([30]));
                }
            }
        }
    }
    let (with_grid, vx, vy) = inject_virtual_gridlines(&tbl, 2).expect("borderless table detected");
    assert_eq!(vx.len(), 3, "3 vertical valleys between 4 columns: {vx:?}");
    assert_eq!(vy.len(), 1, "1 horizontal valley between 2 rows: {vy:?}");
    assert_eq!(with_grid.get_pixel(vx[0], 0)[0], VIRTUAL_GRIDLINE_LUMA);
    assert_eq!(with_grid.get_pixel(20, 15)[0], 30, "original column ink untouched");
}

#[test]
fn gate_lattice_verify_blocks_ungrounded_hallucinations_100pct() {
    let s1 = ShadowToken { bytes: smallvec::SmallVec::from_slice(b"   200,000"), prob: 0.42 };
    let s2 = ShadowToken { bytes: smallvec::SmallVec::from_slice(b"200000"), prob: 0.12 };
    let s3 = ShadowToken { bytes: smallvec::SmallVec::from_slice(b"250,000"), prob: 0.05 }; // below 0.08
    let shadows = vec![s1, s2, s3];

    assert_eq!(
        verify_ledger_with_shadow_top3("   200,000", &shadows, "200,000"),
        LatticeVerifyDecision::Verified { matched_token: "200,000".into() }
    );
    assert_eq!(
        verify_ledger_with_shadow_top3("   200,000", &shadows, "999,999"),
        LatticeVerifyDecision::RejectedUngrounded { reason: "no Shadow Top-3 visual evidence >= 0.08" }
    );
    assert_eq!(
        verify_ledger_with_shadow_top3("   200,000", &shadows, "250,000"),
        LatticeVerifyDecision::RejectedUngrounded { reason: "no Shadow Top-3 visual evidence >= 0.08" }
    );
}

#[test]
fn gate_delta_centered_feed_shift_red_pen_and_ab_eval() {
    // Centered margin > mean margin
    let mut masters = MasterSet::default();
    let m1 = Master::new("A", &make_grid(2, 2, 4, 0.6, Some((0, 0.9))), synthetic_ink(400, 300, &[(20, 20, 60, 40)]), 3);
    let m2 = Master::new("B", &make_grid(2, 2, 4, 0.6, Some((3, 0.9))), synthetic_ink(400, 300, &[(200, 200, 240, 220)]), 3);
    masters.push(m1);
    masters.push(m2);
    let scan_a = make_grid(2, 2, 4, 0.6, Some((0, 0.95)));
    let m_mean = masters.best_match_by(&scan_a, MatchMethod::Mean).unwrap();
    let m_cent = masters.best_match_by(&scan_a, MatchMethod::Centered).unwrap();
    assert_eq!(m_cent.index, 0);
    assert!(m_cent.margin >= m_mean.margin);

    let zzd = masters.to_bytes();
    assert_eq!(MasterSet::parse_header(&zzd).unwrap(), 2);

    // Feed offset + red pen over black line
    let p = DeltaParams { block: 16, edge: 0, ..DeltaParams::default() };
    let line = (20, 30, 120, 34);
    let master = Master::new("m", &make_grid(1, 1, 4, 0.5, None), synthetic_ink(160, 64, &[line]), p.radius);
    let mut rgb = RgbImage::from_pixel(160, 64, Rgb([255, 255, 255]));
    for y in line.1..line.3 {
        for x in line.0..line.2 {
            rgb.put_pixel(x, y, Rgb([20, 20, 25]));
        }
    }
    for x in 60..100 {
        rgb.put_pixel(x, 31, Rgb([210, 25, 25]));
    }
    let scan_ink = InkPage::new(&image::imageops::grayscale(&rgb)).with_color(&rgb);
    let d = ink_changes(&master, &scan_ink, &p).unwrap();
    assert!(d.touches((60, 28, 100, 36), p.ink_threshold));
    assert_eq!(frame_offset(&master.ink, &master.ink), Some((0, 0)));

    let gate = evaluate_delta_gate_ab(
        &ArmRunSummary { runs: 10, done: 10, wasted: 2, ticks: 20, counters: DetectionCounters { tp: 10, fp: 0, fn_: 0, tn: 10 } },
        &ArmRunSummary { runs: 10, done: 10, wasted: 8, ticks: 80, counters: DetectionCounters { tp: 10, fp: 0, fn_: 0, tn: 10 } },
    );
    assert!(gate.passed);
}

#[test]
fn gate_docs_docx_hwpx_pdf_and_caches_round_trip() {
    let guard = WriterGuard::citizen_guard(vec!["박영희".into()], vec!["200,000원".into(), "제2조".into()]);
    assert!(guard.verify_text("차액 200,000원 (제2조 적용) LatticeDB").is_err());

    // DOCX fragmented {{var}} + dynamic table round-trip
    let mut vars = BTreeMap::new();
    vars.insert("amount".to_string(), "200,000원".to_string());
    vars.insert("article".to_string(), "제2조".to_string());
    let docx_bytes = fill_docx_template_text(
        &["심사 결과: {{amo", "unt}} 보완 ({{art", "icle}})"],
        &vars,
        &[vec!["항목".into(), "금액".into()], vec!["차액".into(), "200,000원".into()]],
        &guard,
    )
    .expect("docx build");
    let docx_ir = parse_docs_bytes("docx", &docx_bytes).expect("docx parse");
    assert!(docx_ir.plain_text.contains("200,000원"));
    assert_eq!(docx_ir.fallback, VisionFallbackNeed::None);

    // DOCX complex features: gridSpan/vMerge + outlineLvl + ins/del
    let complex_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
  <w:p><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:r><w:t>1장 총칙</w:t></w:r></w:p>
  <w:p><w:ins><w:r><w:t>추가조항</w:t></w:r></w:ins><w:del><w:delText>삭제조항</w:delText></w:del></w:p>
  <w:tbl><w:tblGrid><w:gridCol w:w="100"/><w:gridCol w:w="100"/></w:tblGrid>
    <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>병합헤더</w:t></w:r></w:p></w:tc></w:tr>
  </w:tbl>
</w:body></w:document>"#;
    let mut cur = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut cur);
        z.start_file("word/document.xml", SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).unwrap();
        z.write_all(complex_xml.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    let complex_ir = parse_docs_bytes("docx", &cur.into_inner()).unwrap();
    assert!(matches!(&complex_ir.blocks[0], DocBlock::Heading { level: 1, .. }));
    assert!(matches!(&complex_ir.blocks[1], DocBlock::Paragraph { track_changes, .. } if track_changes.contains(&TrackChange::Inserted("추가조항".into()))));

    // Native OWPML HWPX round-trip + 1x1 fake table flattening
    let hwpx_bytes = build_hwpx(
        &NoticeSpec { title: "심사 안내문", body: "1. 월세 차액 200,000원\n2. 근거 제2조" },
        &[vec!["구분".into(), "내역".into()], vec!["보완".into(), "필요".into()]],
        &guard,
    )
    .expect("hwpx build");
    let hwpx_ir = parse_hwpx(&hwpx_bytes).expect("hwpx parse");
    assert!(hwpx_ir.plain_text.contains("200,000원"));
    assert_eq!(hwpx_ir.fallback, VisionFallbackNeed::None);

    // 1x1 fake table inside HWPX is flattened to Paragraph
    let fake_hwpx_sec = r#"<?xml version="1.0"?><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:cellAddr colAddr="0" rowAddr="0"/><hp:cellSpan colSpan="1" rowSpan="1"/><hp:p><hp:run><hp:t>장식표 본문</hp:t></hp:run></hp:p></hp:tc></hp:tr></hp:tbl></hs:sec>"#;
    let mut hcur = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut hcur);
        z.start_file("Contents/section0.xml", SimpleFileOptions::default()).unwrap();
        z.write_all(fake_hwpx_sec.as_bytes()).unwrap();
        z.finish().unwrap();
    }
    let fake_ir = parse_hwpx(&hcur.into_inner()).unwrap();
    assert!(fake_ir.blocks.iter().all(|b| matches!(b, DocBlock::Paragraph { .. })));

    // HWP 5.0 OLE container parse
    let mut ole_cur = Cursor::new(Vec::new());
    {
        let mut comp = cfb::CompoundFile::create(&mut ole_cur).unwrap();
        comp.create_storage("/BodyText").unwrap();
        let para = "한글5 본문";
        let utf16: Vec<u8> = para.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let hdr: u32 = 67 | ((utf16.len() as u32) << 20);
        let mut sec_bytes = hdr.to_le_bytes().to_vec();
        sec_bytes.extend_from_slice(&utf16);
        comp.create_stream("/BodyText/Section0").unwrap().write_all(&sec_bytes).unwrap();
        comp.flush().unwrap();
    }
    let hwp_ir = parse_hwp5(&ole_cur.into_inner()).unwrap();
    assert!(hwp_ir.plain_text.contains("한글5 본문"));

    // Hangul PDF false-kerning repair & CSV
    assert_eq!(repair_hangul_false_kerning("한 국 정 부 공문서 OK 123"), "한국정부 공문서 OK 123");
    let csv_ir = parse_csv(b"a,b\n1,2\n").unwrap();
    assert!(csv_ir.plain_text.contains("a | b"));

    // Caches & Symbols round-trip in temp dir
    let dir = std::env::temp_dir().join(format!("lv_doc_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut pc = PhraseCache::open(dir.join("phrases.sgl2"), 1152);
    pc.insert("invoice number", &vec![0.25f32; 1152]);
    pc.flush().unwrap();
    let pc2 = PhraseCache::open(dir.join("phrases.sgl2"), 1152);
    assert!(pc2.get("invoice number").is_some());

    let lc = LayoutCache::open_with_precision(dir.join("templates.json"), Some("f32"));
    assert!(lc.recipe().ends_with("|w=f32"));
    let _ = lc.probe(&make_grid(2, 2, 4, 0.5, None), &LegibilityMap {
        rows: 2, cols: 2, ink: vec![10.0; 4], sharpness: vec![1.0; 4],
        verdict: vec![PatchLegibility::Legible; 4], split_applied: false, ink_gate: 1.0, sharp_gate: 0.5,
    }, true);

    let sym = SymbolDict {
        dim: 4,
        entries: vec![SymbolEntry { name: "valve".into(), category: "piping".into(), scale: 0.01, codes: vec![100, 0, 0, 0] }],
    };
    let rt_sym = SymbolDict::from_bytes(&sym.to_bytes()).unwrap();
    assert_eq!(rt_sym.match_vec(&[1.0, 0.0, 0.0, 0.0], 1)[0].0.name, "valve");
    let _ = std::fs::remove_dir_all(&dir);

    // Ensure direct build_docx also compiles & parses cleanly
    let _ = parse_docs_bytes("docx", &build_docx(&["테스트"], &[]).unwrap()).unwrap();
}
