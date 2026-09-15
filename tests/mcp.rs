use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Mcp {
    fn start(home: &Path) -> Self {
        Self::start_in(home, home)
    }

    fn start_in(home: &Path, directory: &Path) -> Self {
        fs::create_dir_all(home).expect("MCP test home is created");
        fs::write(home.join("config.toml"), "[embedding]\nenabled = false\n")
            .expect("MCP test embeddings are disabled");
        let mut child = Command::new(env!("CARGO_BIN_EXE_gmem"))
            .arg("mcp")
            .env("GRAPHMEM_HOME", home)
            .current_dir(directory)
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
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home =
        std::env::temp_dir().join(format!("graphmem-mcp-test-{}-{nonce}", std::process::id()));
    let mut mcp = Mcp::start(&home);
    let initialized = mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "gmem");
    let instructions = initialized["result"]["instructions"]
        .as_str()
        .expect("server instructions");
    assert!(instructions.contains("Do not store secrets"));
    assert!(instructions.contains("startup working directory"));
    assert!(instructions.contains("two separate local stores"));
    assert!(instructions.contains("entity graph is unscoped"));

    let tools = mcp.request(2, "tools/list", json!({}));
    let listed_tools = tools["result"]["tools"].as_array().expect("tool list");
    assert_eq!(listed_tools.len(), 7);
    assert!(
        listed_tools
            .iter()
            .all(|tool| { tool["inputSchema"].is_object() && tool["outputSchema"].is_object() })
    );
    let names = listed_tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "forget", "graph", "inspect", "recall", "relate", "remember", "stats"
        ]
    );
    let recall = listed_tools
        .iter()
        .find(|tool| tool["name"] == "recall")
        .expect("recall tool is listed");
    let description = recall["description"].as_str().expect("recall description");
    assert!(description.contains("Personalized PageRank"));
    assert!(description.contains("SQLite FTS5 lexical ranking"));
    assert!(recall["inputSchema"]["properties"]["use_embeddings"].is_object());
    assert!(description.contains("server's working directory"));
    assert!(description.contains("out-of-scope memory is never returned"));
    let stats = listed_tools
        .iter()
        .find(|tool| tool["name"] == "stats")
        .expect("stats tool is listed");
    assert!(
        stats["description"]
            .as_str()
            .unwrap()
            .contains("counts only")
    );

    let invalid = mcp.request(
        8,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":"not-a-number"}}),
    );
    assert_eq!(invalid["result"]["isError"], true);

    let remembered = mcp.request(
        3,
        "tools/call",
        json!({"name":"remember","arguments":{
            "content":"stdio memory",
            "entities":[{"kind":"component","name":"MCP"}],
            "relations":[{
                "source":{"kind":"component","name":"MCP"},
                "relation":"uses",
                "target":{"kind":"transport","name":"stdio"}
            }]
        }}),
    );
    let id = remembered["result"]["structuredContent"]["id"]
        .as_i64()
        .expect("remembered id");
    assert_eq!(
        remembered["result"]["structuredContent"]["scopes"],
        json!(["global"])
    );
    let graph = mcp.request(
        9,
        "tools/call",
        json!({"name":"graph","arguments":{"kind":"component","name":"MCP","direction":"outgoing"}}),
    );
    assert_eq!(
        graph["result"]["structuredContent"]["paths"][0]["hops"][0]["entity"]["name"],
        "stdio"
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
        json!({"name":"recall","arguments":{"query":"stdio","use_embeddings":false}}),
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
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "graphmem-mcp-scope-test-{}-{nonce}",
        std::process::id()
    ));
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

#[test]
fn omitted_scopes_default_to_the_server_repository() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "graphmem-mcp-default-scope-test-{}-{nonce}",
        std::process::id()
    ));
    let repository = root.join("repository");
    let home = root.join("home");
    fs::create_dir_all(&repository).expect("test repository is created");
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repository)
            .status()
            .expect("git is available")
            .success()
    );
    let scope = format!(
        "repo:{}",
        repository
            .canonicalize()
            .expect("repository path is canonical")
            .display()
    );
    let mut mcp = Mcp::start_in(&home, &repository);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    let global = mcp.request(
        2,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"scope default","scopes":["global"]}}),
    );
    assert_eq!(
        global["result"]["structuredContent"]["scopes"],
        json!(["global"])
    );
    let repository_memory = mcp.request(
        3,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"scope default"}}),
    );
    assert_eq!(
        repository_memory["result"]["structuredContent"]["scopes"],
        json!([scope])
    );
    let recalled = mcp.request(
        4,
        "tools/call",
        json!({"name":"recall","arguments":{"query":"scope default"}}),
    );
    let memories = recalled["result"]["structuredContent"]["memories"]
        .as_array()
        .expect("scoped memories");
    assert_eq!(memories.len(), 2);
    assert_eq!(memories[0]["scopes"], json!([scope]));
    assert_eq!(memories[1]["scopes"], json!(["global"]));
    drop(mcp);
    fs::remove_dir_all(root).expect("MCP test data is removed");
}

#[test]
fn relates_normalized_entities_and_traverses_bounded_paths() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "graphmem-mcp-graph-test-{}-{nonce}",
        std::process::id()
    ));
    let mut mcp = Mcp::start(&home);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    let api_to_sqlite = mcp.request(
        2,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":" Component ","name":" API "},"relation":"depends_on","target":{"kind":"database","name":"SQLite"},"metadata":"first metadata"}}),
    );
    let api_id = api_to_sqlite["result"]["structuredContent"]["source"]["id"]
        .as_i64()
        .expect("API id");
    let edge_id = api_to_sqlite["result"]["structuredContent"]["edge"]["id"]
        .as_i64()
        .expect("edge id");
    assert_eq!(
        api_to_sqlite["result"]["structuredContent"]["source"]["canonical_name"],
        "api"
    );
    let repeated = mcp.request(
        3,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"component","name":"api"},"relation":"depends_on","target":{"kind":"database","name":"sqlite"},"metadata":"ignored metadata"}}),
    );
    assert_eq!(
        repeated["result"]["structuredContent"]["source"]["id"],
        api_id
    );
    assert_eq!(
        repeated["result"]["structuredContent"]["edge"]["id"],
        edge_id
    );
    assert_eq!(
        repeated["result"]["structuredContent"]["edge"]["metadata"],
        "first metadata"
    );
    mcp.request(
        4,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"database","name":"SQLite"},"relation":"uses","target":{"kind":"language","name":"Rust"}}}),
    );
    mcp.request(
        5,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"language","name":"Rust"},"relation":"supports","target":{"kind":"component","name":"API"}}}),
    );

    let outgoing = mcp.request(
        6,
        "tools/call",
        json!({"name":"graph","arguments":{"kind":"component","name":"api","direction":"outgoing","max_depth":3,"limit":25}}),
    );
    let paths = outgoing["result"]["structuredContent"]["paths"]
        .as_array()
        .expect("outgoing paths");
    assert!(paths.iter().any(|path| {
        path["hops"].as_array().is_some_and(|hops| {
            hops.len() == 2
                && hops[1]["entity"]["name"] == "Rust"
                && hops.iter().all(|hop| hop["direction"] == "outgoing")
        })
    }));
    assert!(paths.iter().all(|path| {
        path["hops"]
            .as_array()
            .is_none_or(|hops| hops.iter().all(|hop| hop["entity"]["id"] != api_id))
    }));

    let incoming = mcp.request(
        7,
        "tools/call",
        json!({"name":"graph","arguments":{"kind":"language","name":"rust","direction":"incoming","max_depth":2}}),
    );
    let paths = incoming["result"]["structuredContent"]["paths"]
        .as_array()
        .expect("incoming paths");
    assert!(paths.iter().any(|path| {
        path["hops"].as_array().is_some_and(|hops| {
            hops.len() == 2
                && hops[1]["entity"]["name"] == "API"
                && hops.iter().all(|hop| hop["direction"] == "incoming")
        })
    }));
    assert_eq!(
        mcp.request(
            8,
            "tools/call",
            json!({"name":"graph","arguments":{"kind":"component","name":"api","max_depth":4}}),
        )["result"]["isError"],
        true
    );
    drop(mcp);
    fs::remove_dir_all(home).expect("MCP test data is removed");
}

#[test]
fn graph_inspection_preserves_alternative_simple_paths() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "graphmem-mcp-graph-quality-test-{}-{nonce}",
        std::process::id()
    ));
    let mut mcp = Mcp::start(&home);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    for (id, source, target) in [
        (2, "root", "left"),
        (3, "root", "right"),
        (4, "left", "target"),
        (5, "right", "target"),
    ] {
        mcp.request(
            id,
            "tools/call",
            json!({"name":"relate","arguments":{"source":{"kind":"node","name":source},"relation":"leads_to","target":{"kind":"node","name":target}}}),
        );
    }
    let graph = mcp.request(
        6,
        "tools/call",
        json!({"name":"graph","arguments":{"kind":"node","name":"root","direction":"outgoing","max_depth":2}}),
    );
    let paths = graph["result"]["structuredContent"]["paths"]
        .as_array()
        .expect("graph paths");
    let target_paths = paths
        .iter()
        .filter(|path| {
            path["hops"]
                .as_array()
                .is_some_and(|hops| hops.len() == 2 && hops[1]["entity"]["name"] == "target")
        })
        .collect::<Vec<_>>();
    assert_eq!(target_paths.len(), 2);
    assert!(
        target_paths
            .iter()
            .any(|path| path["hops"][0]["entity"]["name"] == "left")
    );
    assert!(
        target_paths
            .iter()
            .any(|path| path["hops"][0]["entity"]["name"] == "right")
    );
    drop(mcp);
    fs::remove_dir_all(home).expect("MCP test data is removed");
}

#[test]
fn recall_boosts_direct_graph_neighbors_but_not_second_hops() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "graphmem-mcp-graph-proximity-test-{}-{nonce}",
        std::process::id()
    ));
    let mut mcp = Mcp::start(&home);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    mcp.request(
        2,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"problem","name":"outage"},"relation":"solved_by","target":{"kind":"solution","name":"retry"}}}),
    );
    mcp.request(
        3,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"solution","name":"retry"},"relation":"requires","target":{"kind":"change","name":"deployment"}}}),
    );
    let direct_neighbor = mcp.request(
        4,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"incident resolved by retry","importance":0.0,"scopes":["global"]}}),
    );
    let direct_neighbor_id = direct_neighbor["result"]["structuredContent"]["id"]
        .as_i64()
        .expect("direct neighbor memory id");
    let second_hop = mcp.request(
        5,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"incident deployment steps","importance":1.0,"scopes":["global"]}}),
    );
    let second_hop_id = second_hop["result"]["structuredContent"]["id"]
        .as_i64()
        .expect("second hop memory id");
    let unrelated = mcp.request(
        6,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"incident unrelated notes","importance":0.9,"scopes":["global"]}}),
    );
    let unrelated_id = unrelated["result"]["structuredContent"]["id"]
        .as_i64()
        .expect("unrelated memory id");
    let recalled = mcp.request(
        7,
        "tools/call",
        json!({"name":"recall","arguments":{"query":"incident OR outage","scopes":["global"]}}),
    );
    let memories = recalled["result"]["structuredContent"]["memories"]
        .as_array()
        .expect("recalled memories");
    assert_eq!(memories.len(), 3);
    assert_eq!(memories[0]["id"], direct_neighbor_id);
    assert_eq!(memories[1]["id"], second_hop_id);
    assert_eq!(memories[2]["id"], unrelated_id);
    drop(mcp);
    fs::remove_dir_all(home).expect("MCP test data is removed");
}

#[test]
fn stats_reports_both_stores_without_mutating_them() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "graphmem-mcp-stats-test-{}-{nonce}",
        std::process::id()
    ));
    let mut mcp = Mcp::start(&home);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    let empty = mcp.request(2, "tools/call", json!({"name":"stats","arguments":{}}));
    assert_eq!(
        empty["result"]["structuredContent"],
        json!({"memories":0,"scopes":0,"entities":0,"edges":0})
    );
    mcp.request(
        3,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"stats test memory","scopes":["global"]}}),
    );
    mcp.request(
        4,
        "tools/call",
        json!({"name":"relate","arguments":{"source":{"kind":"component","name":"API"},"relation":"uses","target":{"kind":"database","name":"SQLite"}}}),
    );
    let populated = mcp.request(5, "tools/call", json!({"name":"stats","arguments":{}}));
    assert_eq!(
        populated["result"]["structuredContent"],
        json!({"memories":1,"scopes":1,"entities":2,"edges":1})
    );
    let repeated = mcp.request(6, "tools/call", json!({"name":"stats","arguments":{}}));
    assert_eq!(
        repeated["result"]["structuredContent"],
        populated["result"]["structuredContent"]
    );
    drop(mcp);
    fs::remove_dir_all(home).expect("MCP test data is removed");
}
