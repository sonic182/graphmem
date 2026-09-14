use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};
use uuid::Uuid;

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Mcp {
    fn start(home: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_graphmem"))
            .arg("mcp")
            .env("GRAPHMEM_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("MCP server starts");
        Self {
            stdin: Some(child.stdin.take().expect("MCP stdin")),
            stdout: BufReader::new(child.stdout.take().expect("MCP stdout")),
            child,
        }
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        let request = json!({"jsonrpc":"2.0", "id": id, "method": method, "params": params});
        let stdin = self.stdin.as_mut().expect("MCP stdin");
        writeln!(stdin, "{request}").expect("request is written");
        stdin.flush().expect("request is flushed");
        let mut line = String::new();
        loop {
            line.clear();
            self.stdout.read_line(&mut line).expect("response is read");
            assert!(!line.is_empty(), "MCP server closed stdout");
            let response: Value = serde_json::from_str(&line).expect("response is JSON");
            if response.get("id") == Some(&json!(id)) {
                return response;
            }
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.wait();
    }
}

#[test]
fn serves_memory_lifecycle_over_stdio() {
    let id = Uuid::now_v7();
    let home = std::env::temp_dir().join(format!("graphmem-mcp-test-{id}"));
    let mut mcp = Mcp::start(&home);
    let initialized = mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "graphmem");

    let tools = mcp.request(2, "tools/list", json!({}));
    let names = tools["result"]["tools"]
        .as_array()
        .expect("tool list")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["forget", "inspect", "recall", "remember"]);

    let remembered = mcp.request(
        3,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"stdio memory"}}),
    );
    let id = remembered["result"]["structuredContent"]["id"]
        .as_str()
        .expect("remembered id")
        .to_owned();
    assert_eq!(
        remembered["result"]["structuredContent"]["scopes"],
        json!(["global"])
    );

    drop(mcp);
    let mut mcp = Mcp::start(&home);
    mcp.request(
        4,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    let recalled = mcp.request(
        5,
        "tools/call",
        json!({"name":"recall","arguments":{"query":"stdio"}}),
    );
    assert_eq!(
        recalled["result"]["structuredContent"]["memories"][0]["id"],
        id
    );

    let inspected = mcp.request(
        6,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":id}}),
    );
    assert_eq!(
        inspected["result"]["structuredContent"]["content"],
        "stdio memory"
    );

    let forgotten = mcp.request(
        7,
        "tools/call",
        json!({"name":"forget","arguments":{"id":id}}),
    );
    assert_eq!(forgotten["result"]["structuredContent"]["forgotten"], true);
    drop(mcp);
    std::fs::remove_dir_all(home).expect("MCP test data is removed");
}

#[test]
fn repository_scopes_are_prioritized_and_isolated() {
    let id = Uuid::now_v7();
    let home = std::env::temp_dir().join(format!("graphmem-mcp-scope-test-{id}"));
    let mut mcp = Mcp::start(&home);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    for (id, scopes) in [
        (2, json!([])),
        (3, json!(["repo:/a"])),
        (4, json!(["repo:/b"])),
    ] {
        mcp.request(
            id,
            "tools/call",
            json!({"name":"remember","arguments":{"content":"deployment rule","scopes":scopes}}),
        );
    }
    let recalled = mcp.request(
        5,
        "tools/call",
        json!({"name":"recall","arguments":{"query":"deployment","scopes":["repo:/a"]}}),
    );
    let memories = recalled["result"]["structuredContent"]["memories"]
        .as_array()
        .expect("scoped memories");
    assert_eq!(memories.len(), 2);
    assert_eq!(memories[0]["scopes"], json!(["repo:/a"]));
    assert_eq!(memories[1]["scopes"], json!(["global"]));
    drop(mcp);
    std::fs::remove_dir_all(home).expect("MCP test data is removed");
}
