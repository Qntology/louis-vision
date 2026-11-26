//! `docs_synth_mcp` (:8787) — CLI batch emitter (`--batch-emit`) & strict loopback JSON-RPC 2.0 MCP server.

use std::path::PathBuf;

use anyhow::{bail, Result};
use louis_docs::eval_nn::corpus_plan_manifest::{
    audit_corpus_hygiene, emit_corpus_plan, CorpusSplitKind,
};
use louis_docs::eval_nn::mcp_bus::{
    bind_strict_loopback_mcp, serve_loopback_jsonrpc_once, DEFAULT_DOCS_SYNTH_PORT, MCP_PROTOCOL_VER,
};
use serde_json::{json, Value};

fn handle_tool(name: &str, args: &Value) -> Result<Value> {
    match name {
        "docs_synth_ping" => Ok(json!({
            "service": "docs_synth_mcp",
            "protocol": MCP_PROTOCOL_VER,
            "loopback_only": true
        })),
        "docs_synth_emit_bundle" | "docs_synth_batch_emit" => {
            let split_raw = args.get("split").and_then(Value::as_str).unwrap_or("diag");
            let split = match split_raw {
                "diag" => CorpusSplitKind::Diag,
                "holdout" => CorpusSplitKind::Holdout,
                other => bail!("unsupported split: {other}"),
            };
            let release_seal = args
                .get("release_seal")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let include_xr = args
                .get("include_xr_twin")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let dump_synth = args
                .get("dump_synth")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let out_dir = PathBuf::from(
                args.get("out_dir")
                    .and_then(Value::as_str)
                    .unwrap_or("target/synth_corpus/diag"),
            );
            let doms = args.get("domains").and_then(Value::as_array).map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            });
            let sum = emit_corpus_plan(
                split,
                release_seal,
                &out_dir,
                include_xr,
                dump_synth,
                doms.as_deref(),
            )?;
            Ok(serde_json::to_value(sum)?)
        }
        "docs_synth_audit_hygiene" => {
            let dir = PathBuf::from(
                args.get("corpus_dir")
                    .and_then(Value::as_str)
                    .unwrap_or("target/synth_corpus/diag"),
            );
            let rep = audit_corpus_hygiene(&dir)?;
            Ok(serde_json::to_value(rep)?)
        }
        other => bail!("unknown docs_synth_mcp tool: {other}"),
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--serve") {
        let mut port = DEFAULT_DOCS_SYNTH_PORT;
        let mut once = false;
        let mut i = 1usize;
        while i < args.len() {
            match args[i].as_str() {
                "--port" if i + 1 < args.len() => {
                    port = args[i + 1].parse()?;
                    i += 1;
                }
                "--once" => once = true,
                _ => {}
            }
            i += 1;
        }
        let (listener, local) = bind_strict_loopback_mcp(port)?;
        eprintln!("[docs_synth_mcp] listening strictly on loopback {local}");
        if once {
            serve_loopback_jsonrpc_once(&listener, handle_tool)?;
        } else {
            loop {
                serve_loopback_jsonrpc_once(&listener, handle_tool)?;
            }
        }
        return Ok(());
    }

    if args.iter().any(|a| a == "--batch-emit") {
        let mut split = CorpusSplitKind::Diag;
        let mut release_seal = false;
        let mut include_xr_twin = false;
        let mut dump_synth = false;
        let mut run_audit = false;
        let mut out_dir = PathBuf::from("target/synth_corpus/diag");
        let mut domains: Option<Vec<String>> = None;

        let mut i = 1usize;
        while i < args.len() {
            match args[i].as_str() {
                "--split" if i + 1 < args.len() => {
                    split = match args[i + 1].as_str() {
                        "diag" => CorpusSplitKind::Diag,
                        "holdout" => CorpusSplitKind::Holdout,
                        other => bail!("unsupported --split {other}"),
                    };
                    i += 1;
                }
                "--release-seal" => release_seal = true,
                "--include-xr-twin" => include_xr_twin = true,
                "--dump-synth" => dump_synth = true,
                "--audit-hygiene" => run_audit = true,
                "--out" if i + 1 < args.len() => {
                    out_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
                "--domains" if i + 1 < args.len() => {
                    let list: Vec<String> = args[i + 1]
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    domains = Some(list);
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        if split == CorpusSplitKind::Holdout {
            include_xr_twin = true;
            dump_synth = true;
        }
        let summary = emit_corpus_plan(
            split,
            release_seal,
            &out_dir,
            include_xr_twin,
            dump_synth,
            domains.as_deref(),
        )?;
        println!("{}", serde_json::to_string_pretty(&summary)?);
        if run_audit {
            let rep = audit_corpus_hygiene(&out_dir)?;
            println!("{}", serde_json::to_string_pretty(&rep)?);
        }
        return Ok(());
    }

    println!(
        "{}",
        json!({
            "bin": "docs_synth_mcp",
            "default_port": DEFAULT_DOCS_SYNTH_PORT,
            "usage": [
                "--serve [--port 8787] [--once]",
                "--batch-emit --split diag --domains ALL --include-xr-twin --dump-synth --out <dir>"
            ]
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use louis_docs::eval_nn::mcp_bus::call_loopback_vision_mcp;
    use std::time::Duration;

    #[test]
    fn docs_synth_mcp_loopback_roundtrip_ping_emit_and_audit() {
        let (listener, local) = bind_strict_loopback_mcp(0).unwrap();
        let h1 = std::thread::spawn(move || serve_loopback_jsonrpc_once(&listener, handle_tool));
        let pong = call_loopback_vision_mcp(
            local,
            1,
            "docs_synth_ping",
            &json!({}),
            Duration::from_secs(3),
        )
        .unwrap();
        assert_eq!(pong["service"], "docs_synth_mcp");
        assert_eq!(pong["loopback_only"], true);
        h1.join().unwrap().unwrap();

        let dir = std::env::temp_dir().join(format!("louis-mcp-bin-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let emit_res = handle_tool(
            "docs_synth_batch_emit",
            &json!({
                "split": "diag",
                "domains": ["DOM-01"],
                "include_xr_twin": false,
                "dump_synth": true,
                "out_dir": dir.to_string_lossy()
            }),
        )
        .unwrap();
        assert!(emit_res["bundle_count"].as_u64().unwrap() > 0);
        let hyg = handle_tool(
            "docs_synth_audit_hygiene",
            &json!({ "corpus_dir": dir.to_string_lossy() }),
        )
        .unwrap();
        assert_eq!(hyg["zero_tolerance_ready"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
