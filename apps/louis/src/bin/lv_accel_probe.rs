//! Isolated child-process accelerator probe sidecar (`--lv-accel-probe --backend <b > --ordinal <n>`).
//! Rejects software Vulkan adapters (`llvmpipe`/`lavapipe`), reports JSON over stdout,
//! and calls `rocm_exit_now_if_used(0)` before clean exit under `--release` (`panic = "abort"`).

use lv_core::device::{maybe_run_accel_probe_from_args, probe_current_process_accel, rocm_exit_now_if_used};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let outcome = maybe_run_accel_probe_from_args(&args)
        .unwrap_or_else(|| probe_current_process_accel("auto", 0));
    println!(
        "{}",
        serde_json::to_string(&outcome).unwrap_or_else(|_| "{\"hardware_verified\":false}".into())
    );
    let exit_code = if outcome.hardware_verified || outcome.backend == "cpu" {
        0
    } else {
        2
    };
    rocm_exit_now_if_used(exit_code);
    std::process::exit(exit_code);
}
