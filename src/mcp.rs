use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use graphmem::application::{MemoryDetails, MemoryService, RememberRequest};
use rmcp::schemars::JsonSchema;
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let server = MemoryServer::new()?;
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

pub struct MemoryServer {
    memory: Mutex<MemoryService>,
}

impl MemoryServer {
    fn new() -> Result<Self, graphmem::application::ApplicationError> {
        Ok(Self {
            memory: Mutex::new(MemoryService::open_default()?),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, MemoryService>, CallToolResult> {
        self.memory
            .lock()
            .map_err(|_| tool_error("memory service lock is poisoned"))
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
    id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct MemoryRecord {
    id: String,
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
    id: String,
    forgotten: bool,
}

#[tool_router]
impl MemoryServer {
    #[tool(
        name = "remember",
        description = "Store durable memory. Use global for generally reusable knowledge; use repo:/absolute/path for project-specific knowledge."
    )]
    fn remember(
        &self,
        Parameters(input): Parameters<RememberInput>,
    ) -> Result<Json<MemoryRecord>, CallToolResult> {
        validate_scopes(&input.scopes)?;
        let mut service = self.lock()?;
        let memory = service
            .remember(RememberRequest {
                content: input.content,
                memory_type: input
                    .memory_type
                    .unwrap_or_else(|| "observation".to_owned()),
                importance: input.importance.unwrap_or(0.0),
                scopes: input.scopes,
            })
            .map_err(|error| tool_error(error.to_string()))?;
        let details = service
            .show(memory.id)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(record(details, None)))
    }

    #[tool(
        name = "recall",
        description = "Search memories. Without scopes, search global knowledge; with repo:/absolute/path, include that repository and global memories, with repository memories first."
    )]
    fn recall(
        &self,
        Parameters(input): Parameters<RecallInput>,
    ) -> Result<Json<RecallOutput>, CallToolResult> {
        validate_scopes(&input.scopes)?;
        let service = self.lock()?;
        let results = service
            .search_scopes(&input.query, &input.scopes, input.limit.unwrap_or(10))
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

    #[tool(name = "forget", description = "Delete a memory by id.")]
    fn forget(
        &self,
        Parameters(input): Parameters<IdInput>,
    ) -> Result<Json<ForgetOutput>, CallToolResult> {
        let id = parse_id(&input.id)?;
        self.lock()?
            .forget(id)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(ForgetOutput {
            id: input.id,
            forgotten: true,
        }))
    }

    #[tool(
        name = "inspect",
        description = "Inspect a memory by id, including its scopes."
    )]
    fn inspect(
        &self,
        Parameters(input): Parameters<IdInput>,
    ) -> Result<Json<MemoryRecord>, CallToolResult> {
        let id = parse_id(&input.id)?;
        let details = self
            .lock()?
            .show(id)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(record(details, None)))
    }
}

#[tool_handler(name = "graphmem", version = "0.1.0")]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("graphmem", "0.1.0"))
            .with_instructions("Use global for reusable knowledge and repo:/absolute/path for project-specific knowledge.")
    }
}

fn record(details: MemoryDetails, score: Option<f64>) -> MemoryRecord {
    MemoryRecord {
        id: details.memory.id.to_string(),
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

fn parse_id(value: &str) -> Result<Uuid, CallToolResult> {
    Uuid::parse_str(value).map_err(|error| tool_error(format!("invalid id: {error}")))
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
