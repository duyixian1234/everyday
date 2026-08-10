//! MCP stdio end-to-end test — drives the built binary over the wire.
//!
//! Spawns `everyday mcp serve` and speaks line-delimited JSON-RPC (the MCP
//! stdio framing) against it. Asserts the whole tool surface contract:
//! initialize handshake, `tools/list` projection, a real `tools/call`, error
//! semantics (unknown tool → protocol error, failing tool → `isError`), and —
//! the protocol-hygiene core of the design ([F014](../../docs/adr/F014-mcp-module.md))
//! — that stdout carries nothing but JSON-RPC and the process exits 0 on EOF.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};

const MAX_TOOLS: usize = 60; // lock the ballpark; exact count can drift with new actions

struct Server {
    child: Child,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_everyday"))
            .args(["mcp", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn `everyday mcp serve`");
        let reader = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            reader,
            next_id: 1,
        }
    }

    fn send(&mut self, method: &str, params: Option<Value>) {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_line(&req);
    }

    /// Send a notification (no `id` — JSON-RPC notifications must not carry one).
    fn send_notification(&mut self, method: &str) {
        self.write_line(&json!({"jsonrpc": "2.0", "method": method}));
    }

    fn write_line(&mut self, req: &Value) {
        let mut line = serde_json::to_string(req).unwrap();
        line.push('\n');
        self.child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(line.as_bytes())
            .expect("write to stdin");
        self.child.stdin.as_mut().unwrap().flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("read stdout line");
        assert!(n > 0, "server closed stdout early");
        let v: Value = serde_json::from_str(&line).expect("stdout line must be valid JSON-RPC");
        assert_eq!(v["jsonrpc"], "2.0", "stdout line must be JSON-RPC");
        v
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn initialize_handshake(s: &mut Server) {
    s.send(
        "initialize",
        Some(json!({
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": {"name": "mcp_stdio_test", "version": "0.1"},
        })),
    );
    let resp = s.recv();
    assert!(
        resp["result"]["protocolVersion"].is_string(),
        "initialize must negotiate a protocol version: {resp}"
    );
    s.send_notification("notifications/initialized");
}

#[test]
fn serve_stderr_is_quiet_by_default() {
    // `mcp serve` is a dispatch like any other: without `-v` the middleware
    // progress lines (and any initialize warning) must be silenced on stderr.
    // This is the leveled-logging contract (F015) applied to the long-lived
    // server path — stderr stays clean, stdout stays pure JSON-RPC.
    let mut child = Command::new(env!("CARGO_BIN_EXE_everyday"))
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn `everyday mcp serve`");

    // Handshake once so the server is fully inside its stdio loop before we
    // close stdin (an early EOF would make rmcp error out instead of exiting
    // cleanly, which is not what this test asserts).
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": {"name": "quiet_test", "version": "0.1"},
        },
    });
    let mut line = serde_json::to_string(&init).unwrap();
    line.push('\n');
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(line.as_bytes())
        .expect("write initialize");
    child.stdin.as_mut().unwrap().flush().unwrap();
    let mut resp = String::new();
    {
        let stdout = child.stdout.as_mut().expect("stdout");
        BufReader::new(stdout)
            .read_line(&mut resp)
            .expect("read initialize response");
    }
    assert!(
        resp.contains("\"jsonrpc\""),
        "expected a JSON-RPC initialize response, got: {resp}"
    );
    // Complete the handshake so the server is fully initialized before EOF
    // (rmcp's clean-shutdown path on EOF requires it).
    let notif = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let mut nline = serde_json::to_string(&notif).unwrap();
    nline.push('\n');
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(nline.as_bytes())
        .expect("write initialized");
    child.stdin.as_mut().unwrap().flush().unwrap();

    drop(child.stdin.take());
    let status = child.wait().expect("wait for server exit");
    assert!(status.success(), "server must exit 0 on stdin EOF");

    let mut stderr = String::new();
    use std::io::Read;
    child
        .stderr
        .as_mut()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert!(
        !stderr.contains(" start")
            && !stderr.contains("ok in")
            && !stderr.contains("error in")
            && !stderr.contains("warning:"),
        "default (WARN) must silence middleware progress on serve; stderr was:\n{stderr}"
    );
}

#[test]
fn stdio_server_full_round_trip() {
    let mut s = Server::spawn();
    initialize_handshake(&mut s);

    // tools/list — full projection, mcp itself excluded, name shape intact.
    s.send("tools/list", None);
    let resp = s.recv();
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert!(
        (40..=MAX_TOOLS).contains(&tools.len()),
        "expected ~50 tools, got {}",
        tools.len()
    );
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in &names {
        assert!(n.contains('_'), "tool name {n} must be `<module>_<action>`");
        assert!(!n.starts_with("mcp_"), "mcp itself must not be projected");
    }
    assert!(names.contains(&"config_path"));
    assert!(names.contains(&"mail_list"));
    let first = &tools[0];
    assert!(first["description"].is_string());
    assert!(first["inputSchema"]["type"] == "object");

    // tools/call — success path returns the --json-rendered output.
    s.send(
        "tools/call",
        Some(json!({"name": "config_path", "arguments": {}})),
    );
    let resp = s.recv();
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().expect("text content");
    assert!(!text.is_empty(), "config_path must return a path");

    // unknown tool → protocol error -32601 (unroutable request).
    s.send(
        "tools/call",
        Some(json!({"name": "no_such_tool", "arguments": {}})),
    );
    let resp = s.recv();
    assert_eq!(
        resp["error"]["code"], -32601,
        "unknown tool → METHOD_NOT_FOUND: {resp}"
    );

    // failing tool → isError true with the message in content.
    s.send(
        "tools/call",
        Some(json!({"name": "config_get", "arguments": {}})),
    );
    let resp = s.recv();
    let result = &resp["result"];
    assert_eq!(
        result["isError"], true,
        "config_get with no args must fail: {resp}"
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("usage")
    );

    // stdin EOF → clean exit 0; every stdout line above was valid JSON-RPC
    // (asserted in recv), so stdout carried nothing else.
    drop(s.child.stdin.take());
    let status = s.child.wait().expect("wait for server exit");
    assert!(status.success(), "server must exit 0 on stdin EOF");
}
