use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use graphmem::{
    application::{MemoryDetails, MemoryService, RememberRequest},
    infrastructure::repository::git_repository_root,
};
use rmcp::schemars::JsonSchema;
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let server = MemoryServer::new()?;
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

pub struct MemoryServer {
    memory: Mutex<MemoryService>,
    default_scope: String,
}

impl MemoryServer {
    fn new() -> Result<Self, graphmem::application::ApplicationError> {
        Ok(Self {
            memory: Mutex::new(MemoryService::open_default()?),
            default_scope: current_scope(),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, MemoryService>, CallToolResult> {
        self.memory
            .lock()
            .map_err(|_| tool_error("memory service lock is poisoned"))
    }

    fn scopes(&self, scopes: Vec<String>) -> Vec<String> {
        if scopes.is_empty() {
            vec![self.default_scope.clone()]
        } else {
            scopes
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct RememberInput {
    content: String,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    importance: Option<f64>,
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct RecallInput {
    query: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct IdInput {
    id: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct MemoryRecord {
    id: i64,
    content: String,
    memory_type: String,
    importance: f64,
    created_at: i64,
    updated_at: i64,
    last_accessed_at: Option<i64>,
    access_count: i64,
    scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RecallOutput {
    memories: Vec<MemoryRecord>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ForgetOutput {
    id: i64,
    forgotten: bool,
}

#[tool_router]
impl MemoryServer {
    #[tool(
        name = "remember",
        description = "Store a durable fact, decision, preference, or project rule. When scopes are omitted, store it in the Git repository containing the server's working directory; outside a Git repository, store it in global. Pass global for generally reusable knowledge or repo:/absolute/path to override the default."
    )]
    fn remember(
        &self,
        Parameters(input): Parameters<RememberInput>,
    ) -> Result<Json<MemoryRecord>, CallToolResult> {
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        let mut service = self.lock()?;
        let memory = service
            .remember(RememberRequest {
                content: input.content,
                memory_type: input
                    .memory_type
                    .unwrap_or_else(|| "observation".to_owned()),
                importance: input.importance.unwrap_or(0.0),
                scopes,
            })
            .map_err(|error| tool_error(error.to_string()))?;
        let details = service
            .show(memory.id)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(record(details, None)))
    }

    #[tool(
        name = "recall",
        description = "Search memory content using SQLite FTS5, not semantic or vector search. Prefer concise keywords over a question. The query uses FTS5 MATCH grammar: whitespace is implicit AND (deploy retry); double quotes make a phrase (\"integration test\"); a trailing * makes a prefix (migrat*); uppercase AND, OR, and NOT combine expressions; parentheses group expressions; and NEAR(term1 term2, N) finds terms close together. Quote text containing punctuation or operators when it should be literal. If FTS5 rejects the expression, the server retries it as one quoted literal phrase. Results are ranked by FTS5 BM25, then importance and recency. Full grammar: https://sqlite.org/fts5.html. When scopes are omitted, search the Git repository containing the server's working directory plus global memory, with repository memories first; outside a Git repository, search global memory. Pass repo:/absolute/path to override the default."
    )]
    fn recall(
        &self,
        Parameters(input): Parameters<RecallInput>,
    ) -> Result<Json<RecallOutput>, CallToolResult> {
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        let service = self.lock()?;
        let results = service
            .search_scopes(&input.query, &scopes, input.limit.unwrap_or(10))
            .map_err(|error| tool_error(error.to_string()))?;
        let memories = results
            .into_iter()
            .map(|result| {
                let details = service
                    .show(result.memory.id)
                    .map_err(|error| tool_error(error.to_string()))?;
                Ok(record(details, Some(result.score)))
            })
            .collect::<Result<Vec<_>, CallToolResult>>()?;
        Ok(Json(RecallOutput { memories }))
    }

    #[tool(
        name = "forget",
        description = "Permanently delete one memory by id. Use inspect first when the id or contents are uncertain."
    )]
    fn forget(
        &self,
        Parameters(input): Parameters<IdInput>,
    ) -> Result<Json<ForgetOutput>, CallToolResult> {
        let id = input.id;
        self.lock()?
            .forget(id)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(ForgetOutput {
            id,
            forgotten: true,
        }))
    }

    #[tool(
        name = "inspect",
        description = "Return one memory by id, including its content, metadata, and scopes. Use this before forgetting a memory."
    )]
    fn inspect(
        &self,
        Parameters(input): Parameters<IdInput>,
    ) -> Result<Json<MemoryRecord>, CallToolResult> {
        let id = input.id;
        let details = self
            .lock()?
            .show(id)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(record(details, None)))
    }
}

#[tool_handler(name = "gmem", version = "0.1.0")]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("gmem", "0.1.0"))
            .with_instructions("Graphmem is durable, local memory for coding agents. Recall before starting work when prior decisions, repository conventions, or preferences may matter; use the recall tool's FTS5 query guidance. Remember only verified facts, decisions, constraints, preferences, and reusable project rules that will help a future session. Do not store secrets, credentials, private personal data, transient debugging output, or unverified speculation. Omitted scopes use the Git repository containing the server's startup working directory and include global memories during recall; outside a Git repository, they use global. Pass global for reusable knowledge or repo:/absolute/path to override that default. Prefer recall over creating duplicate memories, and inspect before forgetting when uncertain.")
    }
}

fn record(details: MemoryDetails, score: Option<f64>) -> MemoryRecord {
    MemoryRecord {
        id: details.memory.id,
        content: details.memory.content,
        memory_type: details.memory.memory_type,
        importance: details.memory.importance,
        created_at: details.memory.created_at,
        updated_at: details.memory.updated_at,
        last_accessed_at: details.memory.last_accessed_at,
        access_count: details.memory.access_count,
        scopes: details.scopes.into_iter().map(|scope| scope.name).collect(),
        score,
    }
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

fn validate_scopes(scopes: &[String]) -> Result<(), CallToolResult> {
    if scopes.iter().all(|scope| {
        let scope = scope.trim();
        scope == "global"
            || scope
                .strip_prefix("repo:")
                .is_some_and(|path| Path::new(path).is_absolute())
    }) {
        Ok(())
    } else {
        Err(tool_error("scope must be global or repo:/absolute/path"))
    }
}

fn current_scope() -> String {
    git_repository_root()
        .and_then(|path| path.to_str().map(|path| format!("repo:{path}")))
        .unwrap_or_else(|| "global".to_owned())
}
