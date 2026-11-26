//! CLI entry point for `vision` (Tier 2-B2 60fps concert/broadcast XR engine).

use std::net::SocketAddr;

use anyhow::Result;
use vision::rig_capture::{STEREO_KEYFRAME_QUALITY, MAX_STAGE_TALLY_LUMA_PCT};
use vision::run_selftest;
use vision::show_io::DEFAULT_OSC_ADDR;
use vision::validate_local_socket;

fn parse_osc_addr(args: &[String]) -> Result<SocketAddr> {
    let raw = args
        .windows(2)
        .find(|w| w[0] == "--osc-addr")
        .map(|w| w[1].as_str())
        .unwrap_or(DEFAULT_OSC_ADDR);
    let parsed: SocketAddr = raw.parse()?;
    validate_local_socket(parsed, "--osc-addr")
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let osc_addr = parse_osc_addr(&args)?;

    if args.iter().any(|a| a == "--selftest") {
        println!("{}", run_selftest(Some(osc_addr))?);
        return Ok(());
    }

    if args.iter().any(|a| a == "--headless-rig") {
        let msg = run_selftest(Some(osc_addr))?;
        println!(
            "{}",
            serde_json::json!({
                "app": "vision",
                "mode": "headless-rig",
                "osc_target": osc_addr.to_string(),
                "selftest": msg,
            })
        );
        return Ok(());
    }

    println!(
        "{}",
        serde_json::json!({
            "app": "vision",
            "tier": "2-B2",
            "osc_target": osc_addr.to_string(),
            "stereo_keyframe_quality": STEREO_KEYFRAME_QUALITY,
            "tally_red_luma_pct": MAX_STAGE_TALLY_LUMA_PCT,
            "air_gapped": true,
        })
    );
    Ok(())
}
