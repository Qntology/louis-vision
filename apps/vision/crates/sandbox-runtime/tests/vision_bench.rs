mod camvx;

use camvx::{print_scenario, vx};
use sandbox_core::BaseKind;

#[test]
fn vision_benchmark_prints_its_cells() {
    let mut total = Vec::new();
    for base in [BaseKind::HumanFoot, BaseKind::Wheel] {
        total.push(print_scenario("VX", &vx::v1("mid", base)));
    }
    total.push(print_scenario("VX", &vx::v2("mid")));
    assert!(print_scenario("VX", &vx::v3("pro")).is_nan());
    assert!(total.iter().all(|s| s.is_finite()));
}

#[test]
fn the_engine_never_names_a_vision_benchmark_source() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let engine = [
        "crates/sandbox-runtime/src/vision.rs",
        "crates/sandbox-runtime/src/driver.rs",
        "crates/sandbox-runtime/src/mldrive.rs",
        "crates/sandbox-runtime/src/precision.rs",
        "crates/sandbox-runtime/src/runtime.rs",
        "crates/sandbox-world/src/odometry.rs",
        "crates/sandbox-world/src/world.rs",
        "crates/sandbox-world/src/world/acts.rs",
        "crates/sandbox-world/src/render.rs",
        "crates/sandbox-world/src/light.rs",
        "crates/sandbox-world/src/iso.rs",
        "crates/sandbox-world/src/site.rs",
        "crates/sandbox-world/src/fixture.rs",
        "crates/sandbox-world/src/explore.rs",
        "crates/sandbox-core/src/act/reach.rs",
        "crates/sandbox-core/src/act/grasp.rs",
        "crates/sandbox-core/src/kin.rs",
        "crates/sandbox-core/src/morph.rs",
        "crates/sandbox-runtime/src/vision_compat/mod.rs",
        "crates/sandbox-runtime/src/vision_compat/decide.rs",
        "crates/sandbox-runtime/src/vision_compat/locate.rs",
        "crates/sandbox-runtime/src/vision_compat/tips.rs",
        "crates/sandbox-runtime/src/vision_compat/stairs.rs",
        "crates/sandbox-runtime/src/stereo.rs",
        "crates/sandbox-runtime/src/ml.rs",
        "crates/sandbox-runtime/src/embedding.rs",
        "crates/sandbox-runtime/src/perception.rs",
    ];
    let banned = ["Loomis", "Rieser", "Thomson", "Steenhuis", "Elliott", "Madalena", "Westwood", "Hesse", "Patla", "Hollands", "Laurent", "Assaiante", "TidyBot", "Dusty", "Deyle", "Ramanan", "Holbein", "Wysog", "King, Judge", "Toussaint", "van der Burg"];
    for f in engine {
        let text = std::fs::read_to_string(format!("{root}/{f}")).unwrap_or_else(|e| panic!("{f}: {e}"));
        for b in banned {
            assert!(!text.contains(b), "{f} names the benchmark source {b}");
        }
    }
}
