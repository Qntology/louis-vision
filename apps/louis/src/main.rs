//! CLI entry point for `louis-docs` (Tier 2-B1 air-gapped docs automation engine).

use anyhow::Result;
use louis_docs::run_selftest;
use louis_docs::workflow::{evaluate_all_packs, evaluate_docs_nn_suite, EvalSplit};
use lv_domain::EngineMode;

fn parse_mode(args: &[String]) -> EngineMode {
    args.windows(2)
        .find(|w| w[0] == "--mode")
        .map(|w| EngineMode::from_str_flag(Some(&w[1])))
        .unwrap_or_else(EngineMode::from_env)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = parse_mode(&args);

    if args.iter().any(|a| a == "--selftest") {
        println!("{}", run_selftest(mode)?);
        return Ok(());
    }
    if args.iter().any(|a| a == "--eval-packs") {
        let summary = evaluate_all_packs(mode);
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    if args.iter().any(|a| a == "--eval-nn") {
        let split_arg = args
            .windows(2)
            .find(|w| w[0] == "--split")
            .map(|w| w[1].as_str())
            .unwrap_or("diag");
        let all_18 = args.iter().any(|a| a == "--all-18-packs");
        let release_seal = args.iter().any(|a| a == "--release-seal");
        let split = match split_arg {
            "holdout" => {
                if !release_seal {
                    eprintln!("[LOUIS-DOCS] ERROR: --split holdout requires --release-seal");
                    std::process::exit(2);
                }
                EvalSplit::HoldoutSeal
            }
            "diag" => {
                if all_18 {
                    EvalSplit::DiagAll18
                } else {
                    EvalSplit::DiagCore
                }
            }
            other => {
                eprintln!("[LOUIS-DOCS] ERROR: unknown --split '{other}'");
                std::process::exit(2);
            }
        };
        let report = evaluate_docs_nn_suite(mode, split, release_seal)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if report.exit_code != 0 {
            std::process::exit(report.exit_code);
        }
        return Ok(());
    }

    println!(
        "{}",
        serde_json::json!({
            "app": "louis-docs",
            "tier": "2-B1",
            "mode": mode,
            "packs": 7,
            "air_gapped": true,
        })
    );
    Ok(())
}
