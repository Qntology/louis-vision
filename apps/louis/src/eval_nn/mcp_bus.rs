//! Zero-Async (`std::net` sync-only), strict loopback (`127.0.0.1` / `::1`) MCP 2025-06-18 bridge.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};

pub const DEFAULT_DOCS_SYNTH_PORT: u16 = 8787;
pub const DEFAULT_VISION_MCP_PORT: u16 = 8788;
pub const MCP_PROTOCOL_VER: &str = "2025-06-18";

/// Rejects wildcard (`0.0.0.0`, `::`), LAN (`192.168.x.x`), and WAN addresses; only loopback is permitted.
pub fn ensure_strict_loopback(addr: &SocketAddr) -> Result<()> {
    ensure!(
        addr.ip().is_loopback(),
        "MCP synth bus must bind/connect strictly to loopback, got {}",
        addr.ip()
    );
    Ok(())
}

/// Binds a synchronous loopback listener on `127.0.0.1:<port>` (supports `:0` dynamic ports).
pub fn bind_strict_loopback_mcp(port: u16) -> Result<(TcpListener, SocketAddr)> {
    let target = SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port);
    ensure_strict_loopback(&target)?;
    let listener = TcpListener::bind(target)
        .with_context(|| format!("failed to bind loopback MCP on {target}"))?;
    let local = listener.local_addr()?;
    Ok((listener, local))
}

/// Calls a loopback MCP `tools/call` endpoint over synchronous HTTP/1.1 JSON-RPC 2.0.
pub fn call_loopback_vision_mcp(
    vision_addr: SocketAddr,
    req_id: u64,
    tool_name: &str,
    arguments: &Value,
    timeout: Duration,
) -> Result<Value> {
    ensure_strict_loopback(&vision_addr)?;
    let rpc_body = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "protocolVersion": MCP_PROTOCOL_VER,
        "method": "tools/call",
        "params": { "name": tool_name, "arguments": arguments }
    })
    .to_string();

    let mut stream = TcpStream::connect_timeout(&vision_addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let http_req = format!(
        "POST /mcp HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        vision_addr,
        rpc_body.len(),
        rpc_body
    );
    stream.write_all(http_req.as_bytes())?;
    stream.flush()?;

    let mut raw_resp = String::new();
    stream.read_to_string(&mut raw_resp)?;
    let (_, body) = raw_resp
        .split_once("\r\n\r\n")
        .context("malformed HTTP response")?;
    let rpc: Value = serde_json::from_str(body.trim())?;
    if let Some(err) = rpc.get("error") {
        bail!("vision MCP RPC error: {err}");
    }
    if rpc.pointer("/result/isError").and_then(Value::as_bool) == Some(true) {
        let msg = rpc
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or("tool error");
        bail!("vision MCP tool '{tool_name}' failed: {msg}");
    }
    let text = rpc
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())))
}

/// Serves one synchronous JSON-RPC HTTP/1.1 MCP `tools/call` request over an accepted loopback socket.
pub fn serve_loopback_jsonrpc_once<F>(listener: &TcpListener, mut handler: F) -> Result<()>
where
    F: FnMut(&str, &Value) -> Result<Value>,
{
    let (mut stream, peer) = listener.accept()?;
    ensure_strict_loopback(&peer)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = [0u8; 65_536];
    let n = stream.read(&mut buf)?;
    let raw = String::from_utf8_lossy(&buf[..n]);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or(&raw)
        .trim();
    let req: Value = serde_json::from_str(body)?;
    let id = req.get("id").cloned().unwrap_or(json!(1));
    let tool_name = req
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = req
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let resp_payload = match handler(tool_name, &args) {
        Ok(val) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "isError": false,
                "content": [{ "type": "text", "text": val.to_string() }]
            }
        }),
        Err(err) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "isError": true,
                "content": [{ "type": "text", "text": err.to_string() }]
            }
        }),
    }
    .to_string();

    let http_resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        resp_payload.len(),
        resp_payload
    );
    stream.write_all(http_resp.as_bytes())?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback_and_completes_loopback_handshake() {
        for bad in ["0.0.0.0:8787", "192.168.0.10:8787", "8.8.8.8:8788"] {
            let addr: SocketAddr = bad.parse().unwrap();
            assert!(ensure_strict_loopback(&addr).is_err(), "accepted non-loopback {bad}");
            assert!(
                call_loopback_vision_mcp(addr, 1, "ping", &json!({}), Duration::from_millis(100)).is_err(),
                "called non-loopback {bad}"
            );
        }

        let (listener, local) = bind_strict_loopback_mcp(0).unwrap();
        assert!(local.ip().is_loopback());
        let srv = std::thread::spawn(move || {
            serve_loopback_jsonrpc_once(&listener, |name, args| {
                Ok(json!({ "echo_tool": name, "protocol": MCP_PROTOCOL_VER, "args": args }))
            })
        });
        let out = call_loopback_vision_mcp(
            local,
            7,
            "planar_stereo_render",
            &json!({ "seed": 20260501 }),
            Duration::from_secs(3),
        )
        .unwrap();
        srv.join().unwrap().unwrap();
        assert_eq!(out["echo_tool"], "planar_stereo_render");
        assert_eq!(out["protocol"], MCP_PROTOCOL_VER);
        assert_eq!(out["args"]["seed"], 20260501);
    }
}
