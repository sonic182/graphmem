use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use graphmem::{
    Edge, Entity, EntityReference, GraphDirection, GraphHop, GraphPath, Relation, StoreStats,
    application::{
        GraphDetails, GraphRequest, MemoryDetails, MemoryService, RelateRequest, RelationDetails,
        RememberRequest,
    },
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
    #[serde(default)]
    entities: Vec<EntityInput>,
    #[serde(default)]
    relations: Vec<RelateInput>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct RecallInput {
    query: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    use_embeddings: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EntityInput {
    kind: String,
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RelateInput {
    source: EntityInput,
    relation: String,
    target: EntityInput,
    #[serde(default)]
    metadata: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum GraphDirectionInput {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphInput {
    kind: String,
    name: String,
    #[serde(default)]
    direction: Option<GraphDirectionInput>,
    #[serde(default)]
    max_depth: Option<usize>,
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

#[derive(Debug, Serialize, JsonSchema)]
struct StatsOutput {
    memories: i64,
    scopes: i64,
    entities: i64,
    edges: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct EntityRecord {
    id: i64,
    kind: String,
    name: String,
    canonical_name: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
struct EdgeRecord {
    id: i64,
    source_id: i64,
    relation: String,
    target_id: i64,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RelationOutput {
    source: EntityRecord,
    edge: EdgeRecord,
    target: EntityRecord,
}

#[derive(Debug, Serialize, JsonSchema)]
struct GraphHopOutput {
    entity: EntityRecord,
    edge: EdgeRecord,
    direction: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct GraphPathOutput {
    hops: Vec<GraphHopOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct GraphOutput {
    entity: EntityRecord,
    paths: Vec<GraphPathOutput>,
}

#[tool_router]
impl MemoryServer {
    #[tool(
        name = "remember",
        description = "Store a durable narrative memory: a fact, decision, rationale, preference, or project rule. Optionally attach verified entities and directed relations from this memory; their endpoints become retrieval context and repeated relations reuse the existing graph edge. When scopes are omitted, store it in the Git repository containing the server's working directory; outside a Git repository, store it in global. Pass global for generally reusable knowledge or repo:/absolute/path to override the default."
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
                entities: input.entities.into_iter().map(entity_reference).collect(),
                relations: input.relations.into_iter().map(relation).collect(),
            })
            .map_err(|error| tool_error(error.to_string()))?;
        let details = service
            .show(memory.id)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(record(details, None)))
    }

    #[tool(
        name = "recall",
        description = "Search scoped narrative memories with local semantic embeddings and Personalized PageRank over linked memory/entity context and bidirectional relations. Scope filtering happens before ranking, so out-of-scope memory is never returned. Set use_embeddings to false to force SQLite FTS5 lexical ranking for comparison without loading the model; it defaults to true. The Qwen embedding model downloads into the local Graphmem data directory on first enabled recall; if it cannot load, recall falls back to SQLite FTS5 lexical ranking. When scopes are omitted, search the Git repository containing the server's working directory plus global memory, with repository memories first; outside a Git repository, search global memory. Pass repo:/absolute/path to override the default."
    )]
    fn recall(
        &self,
        Parameters(input): Parameters<RecallInput>,
    ) -> Result<Json<RecallOutput>, CallToolResult> {
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        let mut service = self.lock()?;
        let results = service
            .search_scopes_with_embeddings(
                &input.query,
                &scopes,
                input.limit.unwrap_or(10),
                input.use_embeddings.unwrap_or(true),
            )
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
        name = "stats",
        description = "Report read-only totals for both separate local stores: all scoped narrative memories and scopes, plus all unscoped graph entities and edges. This returns counts only; use recall for memory content and graph for relationship paths."
    )]
    fn stats(&self) -> Result<Json<StatsOutput>, CallToolResult> {
        let stats = self
            .lock()?
            .stats()
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(stats_output(stats)))
    }

    #[tool(
        name = "relate",
        description = "Store a graph-only, durable directed relationship between named entities; it does not create or link a narrative memory. Use source -> relation -> target with a concise snake_case relation such as depends_on, uses, or solved_by. Use this only for verified, reusable facts; it creates missing entities from kind and name, and repeating the same relationship reuses the existing edge. Graph entities and edges are currently unscoped, so every graph query sees the same local graph."
    )]
    fn relate(
        &self,
        Parameters(input): Parameters<RelateInput>,
    ) -> Result<Json<RelationOutput>, CallToolResult> {
        let details = self
            .lock()?
            .relate(RelateRequest {
                source: entity_reference(input.source),
                relation: input.relation,
                target: entity_reference(input.target),
                metadata: input.metadata,
            })
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(relation_output(details)))
    }

    #[tool(
        name = "graph",
        description = "Inspect the unscoped entity graph only; this does not search narrative memories. Use it before adding uncertain or duplicate relations. By default, traverse one hop in both directions. Set direction to incoming or outgoing when needed; max_depth is 1 through 3 and limit is 1 through 100. Each result path starts at the requested entity; an incoming hop means the related entity points to the preceding entity."
    )]
    fn graph(
        &self,
        Parameters(input): Parameters<GraphInput>,
    ) -> Result<Json<GraphOutput>, CallToolResult> {
        let request = graph_request(input)?;
        let details = self
            .lock()?
            .graph(request)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(graph_output(details)))
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
            .with_instructions("Graphmem has two separate local stores: scoped narrative memory and an entity graph. The entity graph is unscoped. Use remember for verified facts, decisions, rationale, constraints, preferences, and reusable project rules; add entities and relations when the memory establishes durable retrieval context. Use relate for verified graph-only facts and graph to inspect bounded paths. Use recall before work when that context may matter, stats for read-only counts, and inspect before forgetting when uncertain. Do not store secrets, credentials, private personal data, transient debugging output, or unverified speculation. Omitted memory scopes use the Git repository containing the server's startup working directory and include global memories during recall; outside a Git repository, they use global. Pass global for reusable knowledge or repo:/absolute/path to override that default. Prefer recall over creating duplicate memories.")
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

fn stats_output(stats: StoreStats) -> StatsOutput {
    StatsOutput {
        memories: stats.memories,
        scopes: stats.scopes,
        entities: stats.entities,
        edges: stats.edges,
    }
}

fn entity_reference(input: EntityInput) -> EntityReference {
    EntityReference {
        kind: input.kind,
        name: input.name,
    }
}

fn relation(input: RelateInput) -> Relation {
    Relation {
        source: entity_reference(input.source),
        relation: input.relation,
        target: entity_reference(input.target),
        metadata: input.metadata,
    }
}

fn graph_request(input: GraphInput) -> Result<GraphRequest, CallToolResult> {
    let max_depth = input.max_depth.unwrap_or(1);
    if !(1..=3).contains(&max_depth) {
        return Err(tool_error("max_depth must be between 1 and 3"));
    }
    let limit = input.limit.unwrap_or(25);
    if !(1..=100).contains(&limit) {
        return Err(tool_error("limit must be between 1 and 100"));
    }
    let direction = match input.direction.unwrap_or(GraphDirectionInput::Both) {
        GraphDirectionInput::Incoming => GraphDirection::Incoming,
        GraphDirectionInput::Outgoing => GraphDirection::Outgoing,
        GraphDirectionInput::Both => GraphDirection::Both,
    };
    Ok(GraphRequest {
        entity: EntityReference {
            kind: input.kind,
            name: input.name,
        },
        direction,
        max_depth,
        limit,
    })
}

fn entity_record(entity: Entity) -> EntityRecord {
    EntityRecord {
        id: entity.id,
        kind: entity.kind,
        name: entity.name,
        canonical_name: entity.canonical_name,
        created_at: entity.created_at,
        updated_at: entity.updated_at,
    }
}

fn edge_record(edge: Edge) -> EdgeRecord {
    EdgeRecord {
        id: edge.id,
        source_id: edge.source_id,
        relation: edge.relation,
        target_id: edge.target_id,
        created_at: edge.created_at,
        metadata: edge.metadata,
    }
}

fn relation_output(details: RelationDetails) -> RelationOutput {
    RelationOutput {
        source: entity_record(details.source),
        edge: edge_record(details.edge),
        target: entity_record(details.target),
    }
}

fn graph_output(details: GraphDetails) -> GraphOutput {
    GraphOutput {
        entity: entity_record(details.entity),
        paths: details.paths.into_iter().map(graph_path_output).collect(),
    }
}

fn graph_path_output(path: GraphPath) -> GraphPathOutput {
    GraphPathOutput {
        hops: path.hops.into_iter().map(graph_hop_output).collect(),
    }
}

fn graph_hop_output(hop: GraphHop) -> GraphHopOutput {
    GraphHopOutput {
        entity: entity_record(hop.entity),
        edge: edge_record(hop.edge),
        direction: match hop.direction {
            GraphDirection::Incoming => "incoming",
            GraphDirection::Outgoing => "outgoing",
            GraphDirection::Both => unreachable!(),
        }
        .to_owned(),
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
