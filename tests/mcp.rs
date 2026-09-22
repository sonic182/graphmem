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
    assert_eq!(
        initialized["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    let instructions = initialized["result"]["instructions"]
        .as_str()
        .expect("server instructions");
    assert!(!instructions.contains("Do not store"));
    assert!(instructions.contains("startup working directory"));
    assert!(instructions.contains("two separate local stores"));
    assert!(instructions.contains("unscoped entity graph"));
    assert!(instructions.contains("Embeddings are disabled"));

    let tools = mcp.request(2, "tools/list", json!({}));
    let listed_tools = tools["result"]["tools"].as_array().expect("tool list");
    assert_eq!(listed_tools.len(), 8);
    assert!(
        listed_tools
            .iter()
            .all(|tool| { tool["inputSchema"].is_object() && tool["outputSchema"].is_object() })
    );
    assert!(
        listed_tools
            .iter()
            .all(|tool| !tool["inputSchema"].to_string().contains("\"$ref\"")),
        "input schemas must inline nested types for tool providers that do not resolve $defs"
    );
    let names = listed_tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "forget", "graph", "inspect", "recall", "relate", "remember", "stats", "update"
        ]
    );
    let recall = listed_tools
        .iter()
        .find(|tool| tool["name"] == "recall")
        .expect("recall tool is listed");
    let description = recall["description"].as_str().expect("recall description");
    assert!(description.contains("falls back to lexical ranking"));
    assert!(description.contains("out-of-scope memory is never returned"));
    assert!(
        recall["inputSchema"]["properties"]["use_embeddings"]["description"]
            .as_str()
            .expect("use_embeddings description")
            .contains("FTS5 lexical search")
    );
    assert!(
        recall["inputSchema"]["properties"]["scopes"]["description"]
            .as_str()
            .expect("scopes description")
            .contains("server's default scope")
    );
    let stats = listed_tools
        .iter()
        .find(|tool| tool["name"] == "stats")
        .expect("stats tool is listed");
    assert!(
        stats["description"]
            .as_str()
            .unwrap()
            .contains("Counts only")
    );

    let resources = mcp.request(90, "resources/list", json!({}));
    let listed_resources = resources["result"]["resources"]
        .as_array()
        .expect("resource list");
    assert_eq!(listed_resources[0]["uri"], "gmem://embedding");
    assert_eq!(listed_resources[0]["mimeType"], "application/json");

    let read = mcp.request(91, "resources/read", json!({"uri":"gmem://embedding"}));
    let text = read["result"]["contents"][0]["text"]
        .as_str()
        .expect("resource text");
    let details: Value = serde_json::from_str(text).expect("resource text is JSON");
    assert_eq!(details["enabled"], false);
    assert!(details["max_tokens"].is_null());
    assert!(details["model"].as_str().unwrap().contains("MiniLM"));

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

#[test]
fn update_revises_a_memory_in_place() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "graphmem-mcp-update-test-{}-{nonce}",
        std::process::id()
    ));
    let mut mcp = Mcp::start(&home);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    let remembered = mcp.request(
        2,
        "tools/call",
        json!({"name":"remember","arguments":{
            "content":"deploy from the release branch",
            "memory_type":"convention",
            "scopes":["global"]
        }}),
    );
    let id = remembered["result"]["structuredContent"]["id"]
        .as_i64()
        .expect("remembered id");

    let updated = mcp.request(
        3,
        "tools/call",
        json!({"name":"update","arguments":{
            "id":id,
            "content":"deploy from tags, not the release branch",
            "memory_type":"decision"
        }}),
    );
    let record = &updated["result"]["structuredContent"];
    assert_eq!(record["id"], id);
    assert_eq!(
        record["content"],
        "deploy from tags, not the release branch"
    );
    assert_eq!(record["memory_type"], "decision");
    assert_eq!(record["scopes"], json!(["global"]), "scopes are untouched");

    // Omitted fields keep their stored value.
    let importance_only = mcp.request(
        4,
        "tools/call",
        json!({"name":"update","arguments":{"id":id,"importance":0.8}}),
    );
    let record = &importance_only["result"]["structuredContent"];
    assert_eq!(
        record["content"],
        "deploy from tags, not the release branch"
    );
    assert_eq!(record["memory_type"], "decision");
    assert_eq!(record["importance"], 0.8);

    // The revision replaces the original rather than adding a second memory.
    let stats = mcp.request(5, "tools/call", json!({"name":"stats","arguments":{}}));
    assert_eq!(stats["result"]["structuredContent"]["memories"], 1);

    let missing = mcp.request(
        6,
        "tools/call",
        json!({"name":"update","arguments":{"id":id + 999,"content":"nothing here"}}),
    );
    assert_eq!(missing["result"]["isError"], true);

    drop(mcp);
    fs::remove_dir_all(home).expect("MCP test data is removed");
}

#[test]
fn recall_filters_by_memory_type() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "graphmem-mcp-type-test-{}-{nonce}",
        std::process::id()
    ));
    let mut mcp = Mcp::start(&home);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    for (id, memory_type) in [(2, "decision"), (3, "convention")] {
        mcp.request(
            id,
            "tools/call",
            json!({"name":"remember","arguments":{
                "content":format!("release process {memory_type}"),
                "memory_type":memory_type,
                "scopes":["global"]
            }}),
        );
    }

    let unfiltered = mcp.request(
        4,
        "tools/call",
        json!({"name":"recall","arguments":{"query":"release process","use_embeddings":false}}),
    );
    assert_eq!(
        unfiltered["result"]["structuredContent"]["memories"]
            .as_array()
            .expect("memories")
            .len(),
        2
    );

    let filtered = mcp.request(
        5,
        "tools/call",
        json!({"name":"recall","arguments":{
            "query":"release process",
            "use_embeddings":false,
            "memory_type":"DECISION"
        }}),
    );
    let memories = filtered["result"]["structuredContent"]["memories"]
        .as_array()
        .expect("filtered memories");
    assert_eq!(memories.len(), 1, "matching ignores case");
    assert_eq!(memories[0]["memory_type"], "decision");

    let unknown = mcp.request(
        6,
        "tools/call",
        json!({"name":"recall","arguments":{
            "query":"release process",
            "use_embeddings":false,
            "memory_type":"incident"
        }}),
    );
    assert!(
        unknown["result"]["structuredContent"]["memories"]
            .as_array()
            .expect("memories")
            .is_empty()
    );

    drop(mcp);
    fs::remove_dir_all(home).expect("MCP test data is removed");
}

#[test]
fn id_addressed_tools_guard_scope_but_accept_an_explicit_target() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "graphmem-mcp-scope-guard-test-{}-{nonce}",
        std::process::id()
    ));
    let home = root.join("home");
    let (first, second) = (root.join("first"), root.join("second"));
    for repository in [&first, &second] {
        fs::create_dir_all(repository).expect("test repository is created");
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(repository)
                .status()
                .expect("git is available")
                .success()
        );
    }

    // Store one memory in the first repository's scope, and one global.
    let mut mcp = Mcp::start_in(&home, &first);
    mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    let private = mcp.request(
        2,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"first repository secret"}}),
    );
    let private_id = private["result"]["structuredContent"]["id"]
        .as_i64()
        .expect("private id");
    let shared = mcp.request(
        3,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"shared note","scopes":["global"]}}),
    );
    let shared_id = shared["result"]["structuredContent"]["id"]
        .as_i64()
        .expect("shared id");
    drop(mcp);

    // A server started in the second repository shares the store, so the ids
    // are guessable; every id-addressed tool must still refuse the first
    // repository's memory.
    let mut mcp = Mcp::start_in(&home, &second);
    mcp.request(
        4,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    for (id, tool, arguments) in [
        (5, "inspect", json!({"id": private_id})),
        (
            6,
            "update",
            json!({"id": private_id, "content": "overwritten from the second repository"}),
        ),
        (7, "forget", json!({"id": private_id})),
    ] {
        let response = mcp.request(id, "tools/call", json!({"name":tool,"arguments":arguments}));
        assert_eq!(
            response["result"]["isError"], true,
            "{tool} reached another repository's memory"
        );
    }

    // Global memories stay reachable from either repository.
    let inspected = mcp.request(
        8,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":shared_id}}),
    );
    assert_eq!(
        inspected["result"]["structuredContent"]["content"],
        "shared note"
    );

    // Naming the other repository explicitly is allowed, as it is for remember
    // and recall: the guard stops a guessed id, not a declared target.
    let first_scope = format!(
        "repo:{}",
        first
            .canonicalize()
            .expect("repository path is canonical")
            .display()
    );
    let targeted = mcp.request(
        9,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":private_id,"scopes":[first_scope]}}),
    );
    assert_eq!(
        targeted["result"]["structuredContent"]["content"],
        "first repository secret"
    );
    let revised = mcp.request(
        10,
        "tools/call",
        json!({"name":"update","arguments":{
            "id":private_id,
            "content":"first repository secret, revised from elsewhere",
            "scopes":[first_scope]
        }}),
    );
    assert_eq!(
        revised["result"]["structuredContent"]["content"],
        "first repository secret, revised from elsewhere"
    );
    let invalid = mcp.request(
        11,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":private_id,"scopes":["not-a-scope"]}}),
    );
    assert_eq!(invalid["result"]["isError"], true);
    drop(mcp);

    // The refused calls left the memory alone.
    let mut mcp = Mcp::start_in(&home, &first);
    mcp.request(
        12,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    let intact = mcp.request(
        13,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":private_id}}),
    );
    assert_eq!(
        intact["result"]["structuredContent"]["content"],
        "first repository secret, revised from elsewhere",
        "the unscoped calls changed nothing; only the explicitly scoped update did"
    );
    drop(mcp);
    fs::remove_dir_all(root).expect("MCP test data is removed");
}
