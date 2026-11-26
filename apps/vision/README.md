# vision-sandbox (`apps/vision`)

Unified Tier 2-B2 workspace in [`Qntology/louis-vision`](https://github.com/Qntology/louis-vision), combining:

1. **`vision-sandbox`** (workspace root package, 100% air-gapped): 60fps concert/broadcast XR live engine (`calibration`, `interlock`, `live_loop`, `rig_capture`, `sandbox_stereo`, `show_io`) with zero WAN HTTP crates, zero `lancedb`, and zero `tauri`.
2. **Vision Sandbox Robotics Simulator** (consolidated from `noto-sandbox`):
   - `crates/sandbox-core` — Hand presets (`SandboxConfig`), kinematics, dynamics, rigid/shaping contact physics, body presets (`BodyPlan`), whole-body balance & composite N:N / action / effect / tactile benchmarks (`bench`).
   - `crates/sandbox-world` — Deterministic open-world grid simulator, software raycaster (RGB/ID/depth/stereo), obstacles, cargo, terrain, and crew simulation.
   - `crates/sandbox-search` — Lexical + embedding natural-language command parser.
   - `crates/sandbox-quest` — Quest planner and worker track progression.
   - `crates/sandbox-runtime` — Runtime orchestrator, SigLIP2/TIPSv2/TTM bridge (`ml_bridge`), `lv_spatial` compat (`vision_compat`), SDS v2 ledger (`sds`), time-series forecast (`ts`), local MCP server (`mcp`), and dev web server (`examples/serve.rs`).
   - `crates/sandbox-data` — Synthetic dataset schemas and LanceDB store (`lancedb` isolated here).
   - `src-tauri` (`vision-sandbox-app`) — Tauri v2 desktop application & pinned Hugging Face weight manager (`tauri` + `reqwest` isolated here).

## Crate Rename Table (Migrated from `noto-sandbox`)

| Legacy (`noto-sandbox`) | Unified (`vision-sandbox`) |
|---|---|
| `noto-core` | `sandbox-core` |
| `noto-data` | `sandbox-data` |
| `world-sim` | `sandbox-world` |
| `noto-search` | `sandbox-search` |
| `noto-quest` | `sandbox-quest` |
| `noto-runtime` | `sandbox-runtime` |
| `noto-sim-app` | `vision-sandbox-app` |
| `NotoConfig` / `NOTO_*` | `SandboxConfig` / `VISION_SANDBOX_*` |

## Quick Commands

Run from repository root (`louis-vision`) with serial builds when constrained on memory/disk:

```bash
# Pin & purity verification + Tier 2 release seal
python3 .github/scripts/verify-pins.py --all
python3 .github/scripts/release_seal.py --release-seal --split holdout

# Air-gapped XR live engine self-test
cargo run --manifest-path apps/vision/Cargo.toml -p vision-sandbox -- --selftest

# Robotics simulator workspace check & dev web server
cargo check --manifest-path apps/vision/Cargo.toml --workspace --all-targets
cargo run --manifest-path apps/vision/Cargo.toml -p sandbox-runtime --example serve
```

Backend helper scripts inside `apps/vision/` (`_cargo.bat`, `_cargo.sh`, `test_runner.bat`, `test_runner.sh`) support `cpu`, `cuda`, `rocm`, `vulkan`, and `metal` (macOS). Runtime model weights (`models/`), generated LanceDB tables (`data/lance/`), SDS ledgers (`data/runtime/`), and test logs (`test_logs/`) are excluded via `.gitignore`.
