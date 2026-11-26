//! CLI entry point for `louis` (Tier 2-B1 air-gapped office automation engine).

use anyhow::Result;
use louis::run_selftest;
use louis::workflow::evaluate_all_packs;
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

    println!(
        "{}",
        serde_json::json!({
            "app": "louis",
            "tier": "2-B1",
            "mode": mode,
            "packs": 7,
            "air_gapped": true,
        })
    );
    Ok(())
}
