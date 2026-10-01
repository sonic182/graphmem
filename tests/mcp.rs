use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use graphmem::Database;
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
        Self::spawn(home, directory, "off")
    }

    fn spawn(home: &Path, directory: &Path, code: &str) -> Self {
        Self::spawn_with(home, directory, &[("GRAPHMEM_CODE", code)])
    }

    fn spawn_with(home: &Path, directory: &Path, envs: &[(&str, &str)]) -> Self {
        Self::spawn_with_config(home, directory, envs, "[embedding]\nenabled = false\n")
    }

    fn spawn_with_config(
        home: &Path,
        directory: &Path,
        envs: &[(&str, &str)],
        config: &str,
    ) -> Self {
        fs::create_dir_all(home).expect("MCP test home is created");
        fs::write(home.join("config.toml"), config).expect("MCP test config is written");
        let mut child = Command::new(env!("CARGO_BIN_EXE_gmem"))
            .arg("mcp")
            .env("GRAPHMEM_HOME", home)
            .env("GIT_DIR", home.join("not-a-repository"))
            .env("GIT_WORK_TREE", home)
            .envs(envs.iter().copied())
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

    /// Writes a request without waiting for its response.
    fn send(&mut self, id: u64, method: &str, params: Value) {
        let request = json!({"jsonrpc":"2.0", "id": id, "method": method, "params": params});
        let stdin = self.stdin.as_mut().expect("MCP stdin");
        writeln!(stdin, "{request}").expect("request is written");
        stdin.flush().expect("request is flushed");
    }

    /// Reads the next JSON-RPC message from the server.
    fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("response is read");
        assert!(!line.is_empty(), "MCP server closed stdout");
        serde_json::from_str(&line).expect("response is JSON")
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(id, method, params);
        loop {
            let response = self.recv();
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
    assert!(!instructions.contains("find_symbol"));

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
    assert_eq!(
        remembered["result"]["structuredContent"]["warnings"],
        json!([])
    );
    assert_eq!(remembered["result"]["structuredContent"]["access_count"], 0);
    assert!(remembered["result"]["structuredContent"]["last_accessed_at"].is_null());
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
    assert_eq!(
        recalled["result"]["structuredContent"]["warnings"],
        json!([])
    );
    assert_eq!(
        recalled["result"]["structuredContent"]["memories"][0]["warnings"],
        json!([])
    );
    let recalled_memory = &recalled["result"]["structuredContent"]["memories"][0];
    assert_eq!(recalled_memory["access_count"], 1);
    let first_access = recalled_memory["last_accessed_at"]
        .as_i64()
        .expect("recall records an access timestamp");
    assert_eq!(
        recalled_memory["updated_at"],
        remembered["result"]["structuredContent"]["updated_at"]
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
    assert_eq!(inspected["result"]["structuredContent"]["access_count"], 2);
    assert!(
        inspected["result"]["structuredContent"]["last_accessed_at"]
            .as_i64()
            .expect("inspect records an access timestamp")
            >= first_access
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
fn embedding_resource_downloads_once_and_reuses_cached_revision() {
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::thread;
    use std::time::Duration;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!("graphmem-hub-{}-{nonce}", std::process::id()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let server_done = Arc::clone(&done);
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        while !server_done.load(Ordering::Relaxed) {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("accept failed: {error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" || header.is_empty() {
                    break;
                }
            }
            let body = r#"{"max_position_embeddings":512}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"config-etag\"\r\nX-Repo-Commit: {}\r\nConnection: close\r\n\r\n",
                body.len(), "a".repeat(40)).unwrap();
            if request.starts_with("GET ") {
                stream.write_all(body.as_bytes()).unwrap();
            }
            requests.push(request);
        }
        requests
    });
    let cache = home.join("models");
    let mut mcp = Mcp::spawn_with_config(
        &home,
        &home,
        &[
            ("GRAPHMEM_CODE", "off"),
            ("GRAPHMEM_EMBEDDINGS", "on"),
            ("GRAPHMEM_EMBEDDING_MODEL", "test-owner/test-model"),
            ("GRAPHMEM_EMBEDDING_REVISION", "test-revision"),
            ("GRAPHMEM_EMBEDDING_CACHE_DIR", cache.to_str().unwrap()),
            ("HF_ENDPOINT", &endpoint),
            ("HF_HUB_DISABLE_IMPLICIT_TOKEN", "1"),
        ],
        "[embedding]\nenabled = true\n",
    );
    mcp.request(1, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    let downloaded = mcp.request(2, "resources/read", json!({"uri":"gmem://embedding"}));
    let cached = mcp.request(3, "resources/read", json!({"uri":"gmem://embedding"}));
    drop(mcp);
    done.store(true, Ordering::Relaxed);
    let requests = server.join().unwrap();
    fs::remove_dir_all(home).unwrap();

    for response in [downloaded, cached] {
        let details: Value =
            serde_json::from_str(response["result"]["contents"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(details["model"], "test-owner/test-model");
        assert_eq!(details["revision"], "test-revision");
        assert_eq!(details["max_tokens"], 512);
    }
    assert_eq!(
        requests,
        [
            "HEAD /test-owner/test-model/resolve/test-revision/config.json HTTP/1.1\r\n",
            "GET /test-owner/test-model/resolve/test-revision/config.json HTTP/1.1\r\n",
        ]
    );
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
    let mut stored_ids = Vec::new();
    for (id, scopes) in [
        (2, json!([])),
        (3, json!(["repo:/a"])),
        (4, json!(["repo:/b"])),
    ] {
        let stored = mcp.request(
            id,
            "tools/call",
            json!({"name":"remember","arguments":{"content":"deployment rule","scopes":scopes}}),
        );
        stored_ids.push(
            stored["result"]["structuredContent"]["id"]
                .as_i64()
                .expect("remembered ID"),
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
    let limited = mcp.request(
        6,
        "tools/call",
        json!({"name":"recall","arguments":{"query":"deployment","scopes":["repo:/a"],"limit":1}}),
    );
    assert_eq!(
        limited["result"]["structuredContent"]["memories"][0]["access_count"],
        2
    );
    let database = Database::open(&home.join("memory.sqlite")).expect("database opens");
    assert_eq!(
        database
            .get_memory(stored_ids[0])
            .unwrap()
            .unwrap()
            .access_count,
        1
    );
    assert_eq!(
        database
            .get_memory(stored_ids[1])
            .unwrap()
            .unwrap()
            .access_count,
        2
    );
    assert_eq!(
        database
            .get_memory(stored_ids[2])
            .unwrap()
            .unwrap()
            .access_count,
        0
    );
    drop(mcp);
    std::fs::remove_dir_all(home).expect("MCP test data is removed");
}

#[test]
fn worktrees_share_scope() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "graphmem-mcp-worktree-test-{}-{nonce}",
        std::process::id()
    ));
    let (main, linked, other, home) = (
        root.join("main"),
        root.join("linked"),
        root.join("other"),
        root.join("home"),
    );
    fs::create_dir_all(&main).expect("test repository is created");
    let git = |dir: &Path, args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_COMMON_DIR")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_PREFIX")
                .status()
                .expect("git is available")
                .success(),
            "git {args:?} failed"
        );
    };
    git(&main, &["init", "--quiet"]);
    git(
        &main,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "seed",
        ],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );
    git(
        &root,
        &[
            "clone",
            "--quiet",
            main.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    let bare = root.join("bare.git");
    let bare_linked = root.join("bare-linked");
    git(
        &root,
        &[
            "clone",
            "--bare",
            "--quiet",
            main.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    git(
        &bare,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            bare_linked.to_str().unwrap(),
        ],
    );
    let canonical = format!(
        "repo:{}",
        main.join(".git").canonicalize().unwrap().display()
    );
    let linked_scope = format!("repo:{}", linked.canonicalize().unwrap().display());

    let mut mcp = Mcp::start_in(&home, &main);
    let initialized = mcp.request(1, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    assert!(
        initialized["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains(&canonical)
    );
    let note = mcp.request(
        2,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"shared worktree note"}}),
    );
    assert_eq!(
        note["result"]["structuredContent"]["scopes"],
        json!([canonical])
    );
    let id = note["result"]["structuredContent"]["id"].as_i64().unwrap();
    drop(mcp);

    let mut mcp = Mcp::start_in(&home, &linked);
    mcp.request(11, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    let recalled = mcp.request(
        4,
        "tools/call",
        json!({"name":"recall","arguments":{"query":"worktree note", "use_embeddings":false}}),
    );
    let memories = recalled["result"]["structuredContent"]["memories"]
        .as_array()
        .unwrap_or_else(|| panic!("unexpected recall: {recalled}"));
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0]["id"], id);
    let explicit = mcp.request(6, "tools/call", json!({"name":"inspect","arguments":{"id":id,"scopes":[format!("repo:{}", main.display())]}}));
    assert_eq!(
        explicit["result"]["structuredContent"]["content"],
        "shared worktree note"
    );
    let explicit_write = mcp.request(8, "tools/call", json!({"name":"remember","arguments":{"content":"explicit worktree note", "scopes":[linked_scope]}}));
    assert_eq!(
        explicit_write["result"]["structuredContent"]["scopes"],
        json!([canonical])
    );
    drop(mcp);

    let nested = main.join("src");
    fs::create_dir_all(&nested).unwrap();
    let mut mcp = Mcp::start_in(&home, &nested);
    mcp.request(13, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    let inspected = mcp.request(
        14,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":id}}),
    );
    assert_eq!(
        inspected["result"]["structuredContent"]["content"],
        "shared worktree note"
    );
    let arbitrary_scope = format!("repo:{}", nested.display());
    let custom = mcp.request(15, "tools/call", json!({"name":"remember","arguments":{"content":"custom scope", "scopes":[arbitrary_scope.clone()]}}));
    assert_eq!(
        custom["result"]["structuredContent"]["scopes"],
        json!([arbitrary_scope])
    );
    drop(mcp);

    let mut mcp = Mcp::start_in(&home, &other);
    mcp.request(12, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    let inaccessible = mcp.request(
        7,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":id}}),
    );
    assert_eq!(inaccessible["result"]["isError"], true);
    drop(mcp);

    let mut mcp = Mcp::start_in(&home, &bare);
    mcp.request(16, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    let bare_note = mcp.request(
        17,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"bare-linked scope note"}}),
    );
    let bare_scope = format!("repo:{}", bare.canonicalize().unwrap().display());
    assert_eq!(
        bare_note["result"]["structuredContent"]["scopes"],
        json!([bare_scope])
    );
    let bare_id = bare_note["result"]["structuredContent"]["id"]
        .as_i64()
        .unwrap();
    drop(mcp);
    let mut mcp = Mcp::start_in(&home, &bare_linked);
    mcp.request(18, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    let shared = mcp.request(
        19,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":bare_id}}),
    );
    assert_eq!(
        shared["result"]["structuredContent"]["content"],
        "bare-linked scope note"
    );
    let isolated = mcp.request(
        20,
        "tools/call",
        json!({"name":"inspect","arguments":{"id":id}}),
    );
    assert_eq!(isolated["result"]["isError"], true);
    drop(mcp);
    fs::remove_dir_all(root).expect("MCP test data is removed");
}

#[test]
fn bare_repository_uses_its_own_scope_without_falling_back_to_global() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "graphmem-mcp-bare-test-{}-{nonce}",
        std::process::id()
    ));
    let (bare, home) = (root.join("project.git"), root.join("home"));
    fs::create_dir_all(&root).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--bare", "--quiet", bare.to_str().unwrap()])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    // A bare repository with no commits still has a stable default scope.
    let mut mcp = Mcp::start_in(&home, &bare);
    let response = mcp.request(1, "initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}));
    assert!(
        response["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains(&format!("repo:{}", bare.canonicalize().unwrap().display()))
    );
    let remembered = mcp.request(
        2,
        "tools/call",
        json!({"name":"remember","arguments":{"content":"bare repository note"}}),
    );
    assert_eq!(
        remembered["result"]["structuredContent"]["scopes"],
        json!([format!("repo:{}", bare.canonicalize().unwrap().display())])
    );
    drop(mcp);
    fs::remove_dir_all(root).unwrap();
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
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_PREFIX")
            .status()
            .expect("git is available")
            .success()
    );
    let scope = format!(
        "repo:{}",
        repository
            .join(".git")
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
    assert_eq!(
        api_to_sqlite["result"]["structuredContent"]["warnings"],
        json!([])
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
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_COMMON_DIR")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_PREFIX")
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

#[cfg(feature = "code")]
#[test]
fn code_tools_are_listed_by_default_and_outline_the_checkout() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "graphmem-mcp-code-test-{}-{nonce}",
        std::process::id()
    ));
    let home = root.join("home");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("lib")).expect("repository is created");
    fs::write(
        repo.join("lib/billing.ex"),
        "defmodule Billing do\n  def charge(amount), do: amount\nend\n",
    )
    .expect("source file is written");
    fs::write(
        repo.join("lib/nested.ex"),
        "defmodule A do\n  defmodule B do\n    def c, do: 1\n  end\n  def d, do: 2\nend\n",
    )
    .expect("nested source file is written");
    fs::write(
        repo.join("lib/imports.rs"),
        "use alpha::One;\nuse beta::{\n    Two,\n    Three,\n};\nuse gamma::Four;\n",
    )
    .expect("import source file is written");
    fs::write(repo.join("lib/partial.html.eex"), "<%= @value %>\n")
        .expect("partial source file is written");
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .expect("git runs")
            .success()
    );
    let mut mcp = Mcp::spawn(&home, &repo, "on");
    let initialized = mcp.request(
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    assert!(
        initialized["result"]["instructions"]
            .as_str()
            .expect("server instructions")
            .contains("find_symbol")
    );
    let tools = mcp.request(2, "tools/list", json!({}));
    let names = tools["result"]["tools"]
        .as_array()
        .expect("tool list")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(!names.contains(&"code_index".to_owned()));
    assert!(names.contains(&"code_outline".to_owned()));
    assert!(names.contains(&"code_imports".to_owned()));
    assert!(names.contains(&"find_symbol".to_owned()));

    let unindexed = mcp.request(
        3,
        "tools/call",
        json!({"name":"find_symbol","arguments":{"query":"charge"}}),
    );
    let unindexed = &unindexed["result"]["structuredContent"];
    assert_eq!(unindexed["matches"][0]["name"], "charge/1");
    assert_eq!(unindexed["total"], 1);
    assert_eq!(unindexed["truncated"], false);

    let outline = mcp.request(
        7,
        "tools/call",
        json!({"name":"code_outline","arguments":{"path":"lib/billing.ex"}}),
    );
    let outline = &outline["result"]["structuredContent"];
    assert_eq!(outline["coverage"], "complete");
    assert_eq!(outline["total"], 2);
    assert_eq!(outline["symbols"][0]["name"], "Billing");
    assert_eq!(outline["symbols"][1]["name"], "charge/1");
    assert_eq!(outline["symbols"][1]["parent"], 0);
    assert_eq!(outline["symbols"][1]["start_line"], 2);

    let shallow = mcp.request(
        11,
        "tools/call",
        json!({
            "name":"code_outline",
            "arguments":{"path":"lib/nested.ex","depth":1,"offset":1,"limit":1}
        }),
    );
    let shallow = &shallow["result"]["structuredContent"];
    assert_eq!(shallow["total"], 3);
    assert_eq!(shallow["next_offset"], 2);
    assert_eq!(shallow["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(shallow["symbols"][0]["name"], "A.B");
    assert_eq!(shallow["symbols"][0]["index"], 1);
    assert_eq!(shallow["symbols"][0]["parent"], 0);

    let imports = mcp.request(
        8,
        "tools/call",
        json!({
            "name":"code_imports",
            "arguments":{"path":"lib/imports.rs","offset":1,"limit":1}
        }),
    );
    let imports = &imports["result"]["structuredContent"];
    assert_eq!(imports["path"], "lib/imports.rs");
    assert_eq!(imports["coverage"], "complete");
    assert!(imports.get("freshness").is_none());
    assert_eq!(imports["total"], 3);
    assert_eq!(imports["next_offset"], 2);
    assert_eq!(imports["imports"].as_array().unwrap().len(), 1);
    assert!(
        imports["imports"][0]["name"]
            .as_str()
            .unwrap()
            .contains("beta")
    );
    assert_eq!(imports["imports"][0]["start_line"], 2);
    assert_eq!(imports["imports"][0]["end_line"], 5);

    fs::write(repo.join("lib/imports.rs"), "use updated::Only;\n")
        .expect("import source file is updated");
    let refreshed = mcp.request(
        9,
        "tools/call",
        json!({"name":"code_imports","arguments":{"path":"lib/imports.rs"}}),
    );
    let refreshed = &refreshed["result"]["structuredContent"];
    assert_eq!(refreshed["total"], 1);
    assert_eq!(refreshed["imports"][0]["name"], "updated::Only");
    assert!(refreshed.get("freshness").is_none());
    assert!(refreshed.get("next_offset").is_none());

    let partial = mcp.request(
        10,
        "tools/call",
        json!({"name":"code_imports","arguments":{"path":"lib/partial.html.eex"}}),
    );
    let partial = &partial["result"]["structuredContent"];
    assert_eq!(partial["coverage"], "partial: EEx directives only");
    assert_eq!(partial["total"], 0);
    assert_eq!(partial["imports"].as_array().unwrap().len(), 0);

    let found = mcp.request(
        5,
        "tools/call",
        json!({"name":"find_symbol","arguments":{"query":"charge"}}),
    );
    let matches = &found["result"]["structuredContent"]["matches"];
    assert_eq!(matches.as_array().expect("matches").len(), 1);
    assert_eq!(matches[0]["path"], "lib/billing.ex");
    assert_eq!(matches[0]["parent"], "Billing");
    assert_eq!(matches[0]["freshness"], "fresh");

    let escaped = mcp.request(
        6,
        "tools/call",
        json!({"name":"code_outline","arguments":{"path":"../home/config.toml"}}),
    );
    assert_eq!(escaped["result"]["isError"], true);
    drop(mcp);
    fs::remove_dir_all(root).expect("MCP code test data is removed");
}

#[test]
fn code_failures_leave_the_memory_tools_available() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "graphmem-mcp-code-failure-test-{}-{nonce}",
        std::process::id()
    ));
    let home = root.join("home");
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("repository is created");
    fs::write(repo.join("app.py"), "def ping():\n    pass\n").expect("source file is written");
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .expect("git runs")
            .success()
    );
    fs::create_dir_all(&home).expect("home is created");
    fs::write(home.join("code.sqlite"), "not a database").expect("corrupt index is written");

    let tool_names = |mcp: &mut Mcp| {
        mcp.request(
            1,
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
        );
        mcp.request(2, "tools/list", json!({}))["result"]["tools"]
            .as_array()
            .expect("tool list")
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    let mut mcp = Mcp::spawn_with(&home, &repo, &[("GRAPHMEM_CODE", "on")]);
    let names = tool_names(&mut mcp);
    assert!(names.contains(&"recall".to_owned()));
    assert!(names.contains(&"find_symbol".to_owned()));
    let outline = mcp.request(
        3,
        "tools/call",
        json!({"name":"code_outline","arguments":{"path":"app.py"}}),
    );
    assert_eq!(
        outline["result"]["structuredContent"]["symbols"][0]["name"],
        "ping"
    );
    drop(mcp);

    let mut mcp = Mcp::spawn_with(
        &home,
        &repo,
        &[
            ("GRAPHMEM_CODE", "on"),
            ("GRAPHMEM_CODE_INDEX_THREADS", "0"),
        ],
    );
    let names = tool_names(&mut mcp);
    assert!(names.contains(&"recall".to_owned()));
    assert!(!names.contains(&"find_symbol".to_owned()));
    drop(mcp);

    for invalid in ["max_files = -1", "enabled = 'yes'", "index_threads = false"] {
        for code in ["on", "off"] {
            let config = format!("[embedding]\nenabled = false\n[code]\n{invalid}\n");
            let mut mcp = Mcp::spawn_with_config(&home, &repo, &[("GRAPHMEM_CODE", code)], &config);
            let names = tool_names(&mut mcp);
            assert!(names.contains(&"recall".to_owned()), "{invalid}, {code}");
            assert!(
                !names.contains(&"find_symbol".to_owned()),
                "{invalid}, {code}"
            );
            let stats = mcp.request(3, "tools/call", json!({"name":"stats","arguments":{}}));
            assert!(stats["result"]["structuredContent"].is_object(), "{stats}");
        }
    }
    fs::remove_dir_all(root).expect("MCP code failure test data is removed");
}

/// A first index of a large checkout takes seconds. It must run off the Tokio
/// workers, or the tools that do not touch the code index wait behind it.
#[cfg(feature = "code")]
#[test]
fn code_refresh_does_not_block_memory_tools() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "graphmem-mcp-code-blocking-test-{}-{nonce}",
        std::process::id()
    ));
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("repository is created");
    for file in 0..4000 {
        let source = (0..10)
            .map(|function| format!("def handler_{file}_{function}(value):\n    return value\n"))
            .collect::<String>();
        fs::write(repo.join(format!("module_{file}.py")), source).expect("source file is written");
    }
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .expect("git runs")
            .success()
    );

    // A fresh home per server, so each one starts with an empty code index.
    // With the fewest workers, a handler that blocks one starves every tool.
    for (workers, code_calls) in [("1", 1), ("2", 3)] {
        let home = root.join(format!("home-{workers}"));
        let mut mcp = Mcp::spawn_with(
            &home,
            &repo,
            &[
                ("GRAPHMEM_CODE", "on"),
                ("GRAPHMEM_CODE_INDEX_THREADS", "1"),
                ("GRAPHMEM_TOKIO_WORKER_THREADS", workers),
            ],
        );
        mcp.request(
            1,
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
        );
        let code_index = rusqlite::Connection::open(home.join("code.sqlite"))
            .expect("code index database is available");
        code_index
            .busy_timeout(std::time::Duration::from_secs(5))
            .expect("code index reads wait for writes");
        // Code calls take ids 10.., the memory call id 2.
        for call in 0..code_calls {
            let (name, arguments) = match call {
                0 | 1 => ("find_symbol", json!({"query":"handler_0_0"})),
                _ => ("code_outline", json!({"path":"module_1.py"})),
            };
            mcp.send(
                10 + call,
                "tools/call",
                json!({"name":name,"arguments":arguments}),
            );
        }
        let indexing_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let indexed_files: i64 = code_index
                .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
                .expect("code index progress is readable");
            if indexed_files > 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < indexing_deadline,
                "code index did not begin indexing files"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(code_index);
        mcp.send(2, "tools/call", json!({"name":"stats","arguments":{}}));

        // The code calls contend for the index lock in no fixed order, so a
        // cheap `code_outline` (id 12) may answer early. The slow refresh is
        // in `find_symbol` (ids 10 and 11), which must not answer before stats.
        let mut answered = Vec::new();
        for _ in 0..=code_calls {
            let response = mcp.recv();
            assert_ne!(response["result"]["isError"], true, "{response}");
            assert!(
                response["result"]["structuredContent"].is_object(),
                "{response}"
            );
            answered.push(response["id"].as_u64().expect("response id"));
        }
        let stats = answered
            .iter()
            .position(|&id| id == 2)
            .expect("stats answered");
        assert!(
            answered[..stats].iter().all(|&id| id == 12),
            "stats waited behind the code index with {workers} worker(s): {answered:?}"
        );
        drop(mcp);
    }
    fs::remove_dir_all(root).expect("MCP code blocking test data is removed");
}
