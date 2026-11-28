//! CLI entry point for `louis-docs` (Tier 2-B1 air-gapped docs automation engine).

use anyhow::Result;
use louis_docs::eval_nn::nl_search_eval::{run_nl_search_nn_eval, NlSearchEvalConfig};
use louis_docs::eval_nn::schema::EvalSplit as NlEvalSplit;
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
    if args.iter().any(|a| a == "--eval-nn-search") {
        let split_arg = args
            .windows(2)
            .find(|w| w[0] == "--split")
            .map(|w| w[1].as_str())
            .unwrap_or("diag");
        let release_seal = args.iter().any(|a| a == "--release-seal");
        let split = match split_arg {
            "holdout" => {
                if !release_seal {
                    eprintln!("[LOUIS-DOCS] ERROR: --split holdout requires --release-seal");
                    std::process::exit(2);
                }
                NlEvalSplit::Holdout
            }
            "diag" => NlEvalSplit::Diag,
            other => {
                eprintln!("[LOUIS-DOCS] ERROR: unknown --split '{other}'");
                std::process::exit(2);
            }
        };
        let cases_per_domain = args
            .windows(2)
            .find(|w| w[0] == "--cases-per-domain")
            .and_then(|w| w[1].parse::<usize>().ok())
            .unwrap_or(2)
            .max(1);
        let domains_filter = args
            .windows(2)
            .find(|w| w[0] == "--domains")
            .and_then(|w| {
                let items: Vec<String> = w[1]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if items.is_empty()
                    || items
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case("ALL") || s == "*")
                {
                    None
                } else {
                    Some(items)
                }
            });
        let out_dir = args
            .windows(2)
            .find(|w| w[0] == "--out-dir")
            .map(|w| std::path::PathBuf::from(&w[1]));
        let dump_synth = args.iter().any(|a| a == "--dump-synth");
        let structured_layout = args
            .windows(2)
            .find(|w| w[0] == "--layout")
            .is_some_and(|w| w[1].eq_ignore_ascii_case("structured"));

        let cfg = NlSearchEvalConfig {
            split,
            release_seal,
            cases_per_domain,
            domains_filter,
            include_xr_twin: true,
            out_dir,
            dump_synth,
            structured_layout,
        };
        let report = run_nl_search_nn_eval(&cfg)?;
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
