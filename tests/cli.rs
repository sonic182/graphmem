use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Mcp {
    fn start(home: &Path) -> Self {
        fs::create_dir_all(home).expect("MCP test home is created");
        fs::write(home.join("config.toml"), "[embedding]\nenabled = false\n")
            .expect("MCP test embeddings are disabled");
        let mut child = Command::new(env!("CARGO_BIN_EXE_gmem"))
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

fn data_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    std::env::temp_dir().join(format!("graphmem-cli-test-{}-{nonce}", std::process::id()))
}

fn run(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gmem"))
        .args(args)
        .env("GRAPHMEM_HOME", data_dir)
        .env("GRAPHMEM_EMBEDDINGS", "off")
        .output()
        .expect("gmem runs")
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

#[test]
fn memory_lifecycle_works_across_cli_processes() {
    let data_dir = data_dir();
    let remembered = stdout(run(
        &data_dir,
        &[
            "remember",
            "Use cargo nextest for integration tests",
            "--type",
            "convention",
            "--importance",
            "0.8",
            "--scope",
            "repo:/workspace/graphmem",
        ],
    ));
    let id = remembered
        .trim()
        .strip_prefix("remembered: ")
        .expect("remember output contains an ID")
        .to_owned();

    let shown = stdout(run(&data_dir, &["show", &id]));
    assert!(shown.contains("type: convention"));
    assert!(shown.contains("repo:/workspace/graphmem"));
    assert!(shown.contains("Use cargo nextest"));

    let listed = stdout(run(
        &data_dir,
        &["list", "--scope", "repo:/workspace/graphmem"],
    ));
    assert!(listed.contains(&id));

    let searched = stdout(run(&data_dir, &["search", "nextest"]));
    assert!(searched.contains(&id));
    assert!(
        searched
            .split('\n')
            .next()
            .and_then(|line| line.split('\t').next())
            .and_then(|score| score.parse::<f64>().ok())
            .is_some()
    );

    let scopes = stdout(run(&data_dir, &["scopes"]));
    assert!(scopes.contains("repo:/workspace/graphmem"));

    let doctor = stdout(run(&data_dir, &["doctor"]));
    assert!(doctor.contains("status: healthy"));

    let forgotten = stdout(run(&data_dir, &["forget", &id]));
    assert_eq!(forgotten, format!("forgot: {id}\n"));

    let missing = run(&data_dir, &["show", &id]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("memory not found"));

    let invalid = run(&data_dir, &["show", "invalid"]);
    assert!(!invalid.status.success());

    let flush_without_confirmation = run(&data_dir, &["flush"]);
    assert!(!flush_without_confirmation.status.success());
    assert!(
        String::from_utf8_lossy(&flush_without_confirmation.stderr).contains("rerun with --yes")
    );
    stdout(run(
        &data_dir,
        &["remember", "temporary memory before flush"],
    ));
    let flushed = stdout(run(&data_dir, &["flush", "--yes"]));
    assert_eq!(flushed, "flushed all memories and graph data\n");
    assert!(stdout(run(&data_dir, &["list"])).is_empty());
    assert!(stdout(run(&data_dir, &["scopes"])).is_empty());

    fs::remove_dir_all(data_dir).expect("test data directory is removed");
}

#[test]
fn graph_command_inspects_entities_seeded_via_mcp() {
    let data_dir = data_dir();
    let mut mcp = Mcp::start(&data_dir);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    mcp.request(
        2,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"component","name":"api"},"relation":"depends_on","target":{"kind":"database","name":"sqlite"}}}),
    );
    mcp.request(
        3,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"database","name":"sqlite"},"relation":"uses","target":{"kind":"language","name":"rust"}}}),
    );
    drop(mcp);

    let graphed = stdout(run(
        &data_dir,
        &[
            "graph",
            "component",
            "api",
            "--direction",
            "outgoing",
            "--max-depth",
            "2",
        ],
    ));
    assert!(graphed.contains("depends_on\tdatabase\tsqlite"));
    assert!(graphed.contains("uses\tlanguage\trust"));

    let invalid_depth = run(
        &data_dir,
        &["graph", "component", "api", "--max-depth", "4"],
    );
    assert!(!invalid_depth.status.success());
    assert!(
        String::from_utf8_lossy(&invalid_depth.stderr)
            .contains("max_depth must be between 1 and 3")
    );

    fs::remove_dir_all(data_dir).expect("test data directory is removed");
}
