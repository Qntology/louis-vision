mod camvx;

use camvx::{cam, print_scenario};

#[test]
fn camera_benchmark_prints_its_cells() {
    let s1 = print_scenario("CAM", &cam::c1());
    let s2 = print_scenario("CAM", &cam::c2());
    assert!(s1.is_finite() && s2.is_finite());
}

#[test]
fn the_engine_never_names_a_camera_benchmark_source() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let engine = [
        "crates/sandbox-world/src/world/eye.rs",
        "crates/sandbox-world/src/render.rs",
        "crates/sandbox-world/src/light.rs",
        "crates/sandbox-world/src/iso.rs",
        "crates/sandbox-world/src/fixture.rs",
        "crates/sandbox-world/src/site.rs",
        "crates/sandbox-core/src/kin.rs",
        "crates/sandbox-core/src/morph.rs",
        "crates/sandbox-core/src/act/reach.rs",
        "crates/sandbox-runtime/src/vision.rs",
        "crates/sandbox-runtime/src/record.rs",
        "crates/sandbox-runtime/src/views.rs",
        "crates/sandbox-runtime/src/chase.rs",
    ];
    let banned = ["Milella", "Reina", "Moon, Leary", "Moon 2025", "Waite", "Menz", "Kavanagh"];
    for f in engine {
        let text = std::fs::read_to_string(format!("{root}/{f}")).unwrap_or_else(|e| panic!("{f}: {e}"));
        for b in banned {
            assert!(!text.contains(b), "{f} names the benchmark source {b}");
        }
    }
}
