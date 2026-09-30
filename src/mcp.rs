use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

#[cfg(feature = "code")]
use graphmem::application::code::{CodeService, MAX_FIND_LIMIT};
use graphmem::{
    Edge, Entity, EntityReference, GraphDirection, GraphHop, GraphPath, Memory, Relation, Scope,
    StoreStats,
    application::{
        GraphDetails, GraphRequest, MemoryService, RelateRequest, RelationDetails, RememberRequest,
    },
    infrastructure::config::{ConfigOverrides, EmbeddingConfig},
    infrastructure::embedding::{MAX_EMBEDDING_TOKENS, embedding_details},
    infrastructure::repository::git_repository,
};
use rmcp::schemars::JsonSchema;
use rmcp::{
    ErrorData as McpError, Json, RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, ServerCapabilities, ServerInfo,
    },
    service::{MaybeSendFuture, RequestContext},
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};

pub async fn run(overrides: ConfigOverrides) -> Result<(), Box<dyn std::error::Error>> {
    let server = MemoryServer::new(overrides)?;
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

pub struct MemoryServer {
    memory: Mutex<MemoryService>,
    default_scope: String,
    embedding: EmbeddingConfig,
    tool_router: ToolRouter<Self>,
    #[cfg(feature = "code")]
    code: Option<Mutex<CodeService>>,
}

impl MemoryServer {
    fn new(overrides: ConfigOverrides) -> Result<Self, graphmem::application::ApplicationError> {
        let service = MemoryService::open_default(overrides)?;
        let embedding = service.embedding_config().clone();
        let default_scope = current_scope();
        #[cfg_attr(not(feature = "code"), allow(unused_mut))]
        let mut tool_router = Self::tool_router();
        #[cfg(feature = "code")]
        let code = match CodeService::open_default() {
            Ok(code) => code.map(Mutex::new),
            Err(error) => {
                tracing::warn!(%error, "code tools disabled");
                eprintln!("warning: code tools disabled: {error}");
                None
            }
        };
        #[cfg(feature = "code")]
        if code.is_some() {
            tool_router += Self::code_router();
        }
        Ok(Self {
            memory: Mutex::new(service),
            default_scope,
            embedding,
            tool_router,
            #[cfg(feature = "code")]
            code,
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
            resolve_scopes(scopes)
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct RememberInput {
    /// The memory content.
    content: String,
    /// Optional category or label. Defaults to observation.
    #[serde(default)]
    memory_type: Option<String>,
    /// Optional relative importance from 0.0 to 1.0. Defaults to 0.0.
    #[serde(default)]
    importance: Option<f64>,
    /// Where to store the memory: global or repo:/absolute/path. Omit to use
    /// the server's default scope.
    #[serde(default)]
    scopes: Vec<String>,
    /// Optional entities connected to this memory for graph-assisted recall.
    /// Each is an object, not a bare name, e.g.
    /// [{"kind": "module", "name": "auth"}, {"kind": "service", "name": "billing-api"}].
    #[serde(default)]
    entities: Vec<EntityInput>,
    /// Optional directed relations to store in the graph and link to this
    /// memory.
    #[serde(default)]
    relations: Vec<RelateInput>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct RecallInput {
    /// What to retrieve. Natural language works; with use_embeddings false this
    /// is lexical: words, "quoted phrases", and prefix* terms match if any of
    /// them does, ranked by BM25; an uppercase AND, OR, NOT, or NEAR switches
    /// to exact FTS5 syntax.
    query: String,
    /// Where to search: global or repo:/absolute/path. Omit to search the
    /// server's default scope and global memories.
    #[serde(default)]
    scopes: Vec<String>,
    /// Maximum memories to return. Defaults to 10.
    #[serde(default)]
    limit: Option<usize>,
    /// Defaults to true. Set false to use FTS5 lexical search instead of
    /// embeddings.
    #[serde(default)]
    use_embeddings: Option<bool>,
    /// Optional filter on the memory's category, applied before ranking.
    /// Matching ignores case. Use the value stored by remember, such as
    /// decision, convention, constraint, incident, or observation; a category
    /// nothing was stored under returns no memories.
    #[serde(default)]
    memory_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
struct EntityInput {
    /// Type of entity, e.g. "module", "repo", "service", "table", "person".
    kind: String,
    /// Name of entity, e.g. "billing-api".
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
struct RelateInput {
    /// Entity the relation points from.
    source: EntityInput,
    /// Relation from source to target.
    relation: String,
    /// Entity the relation points to.
    target: EntityInput,
    /// Optional short note qualifying the edge.
    #[serde(default)]
    metadata: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
enum GraphDirectionInput {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphInput {
    /// Kind of the entity to start from.
    kind: String,
    /// Name of the entity to start from.
    name: String,
    /// Which edges to follow. Defaults to both.
    #[serde(default)]
    direction: Option<GraphDirectionInput>,
    /// Hops to traverse, 1 to 3. Defaults to 1.
    #[serde(default)]
    max_depth: Option<usize>,
    /// Maximum paths to return, 1 to 100.
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct IdInput {
    /// Memory id, as returned by recall or remember.
    id: i64,
    /// Where to look for the memory: global or repo:/absolute/path. Omit to
    /// use the server's default scope. Global memories are reachable from any
    /// scope. Pass the target repository explicitly when it differs from the
    /// one the server started in, as remember and recall accept it.
    #[serde(default)]
    scopes: Vec<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct UpdateInput {
    /// Memory id, as returned by recall or remember.
    id: i64,
    /// Replacement content. Omit to keep the stored content.
    #[serde(default)]
    content: Option<String>,
    /// Replacement category. Omit to keep the stored one.
    #[serde(default)]
    memory_type: Option<String>,
    /// Replacement importance from 0.0 to 1.0. Omit to keep the stored one.
    #[serde(default)]
    importance: Option<f64>,
    /// Where to look for the memory: global or repo:/absolute/path. Omit to
    /// use the server's default scope. Global memories are reachable from any
    /// scope. Pass the target repository explicitly when it differs from the
    /// one the server started in, as remember and recall accept it.
    #[serde(default)]
    scopes: Vec<String>,
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
    /// Set when an embedding had to truncate its input to the model's token
    /// limit, so content past the limit did not affect ranking.
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RecallOutput {
    memories: Vec<MemoryRecord>,
    warnings: Vec<String>,
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
    warnings: Vec<String>,
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
        description = "Store a narrative memory. Optional entities and directed relations connect \
             it to the graph for graph-assisted recall. Repeated relations reuse the existing \
             edge. Omit scopes to use the server's default scope. Content longer than the \
             embedding model's token limit is truncated before embedding; the response's warnings \
             field reports when that happens."
    )]
    fn remember(
        &self,
        Parameters(input): Parameters<RememberInput>,
    ) -> Result<Json<MemoryRecord>, CallToolResult> {
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        let mut service = self.lock()?;
        let stored = service.remember(RememberRequest {
            content: input.content,
            memory_type: input
                .memory_type
                .unwrap_or_else(|| "observation".to_owned()),
            importance: input.importance.unwrap_or(0.0),
            scopes,
            entities: input.entities.into_iter().map(entity_reference).collect(),
            relations: input.relations.into_iter().map(relation).collect(),
        });
        // drained before the error propagates, so a failed call cannot leave a
        // stale flag that warns on the next successful one
        let warning = service.take_embedding_warning();
        let memory = stored.map_err(|error| tool_error(error.to_string()))?;
        let details = service
            .show(memory.id, None)
            .map_err(|error| tool_error(error.to_string()))?;
        let mut record = record(details.memory, details.scopes, None);
        record.warnings = warning.into_iter().collect();
        Ok(Json(record))
    }

    #[tool(
        name = "recall",
        description = "Search scoped narrative memories. Ranking scores memories, entities, and \
             relations by meaning, then propagates rank along graph edges. Scores are relative \
             within a query. Scope filtering happens before ranking, so out-of-scope memory is \
             never returned. If the embedding model cannot load, recall falls back to lexical \
             ranking. When embedding input was truncated to the model's token limit, the response \
             includes a warnings field; use use_embeddings false to rank the full text lexically."
    )]
    fn recall(
        &self,
        Parameters(input): Parameters<RecallInput>,
    ) -> Result<Json<RecallOutput>, CallToolResult> {
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        let mut service = self.lock()?;
        let found = service.search_scopes_with_embeddings(
            &input.query,
            &scopes,
            input.limit.unwrap_or(10),
            input.use_embeddings.unwrap_or(true),
            input.memory_type.as_deref(),
        );
        let warning = service.take_embedding_warning();
        let results = found.map_err(|error| tool_error(error.to_string()))?;
        let memory_ids = results
            .iter()
            .map(|result| result.memory.id)
            .collect::<Vec<_>>();
        let mut scopes_by_memory = service
            .scopes_for(&memory_ids)
            .map_err(|error| tool_error(error.to_string()))?;
        let memories = results
            .into_iter()
            .map(|result| {
                let scopes = scopes_by_memory
                    .remove(&result.memory.id)
                    .unwrap_or_default();
                record(result.memory, scopes, Some(result.score))
            })
            .collect::<Vec<_>>();
        Ok(Json(RecallOutput {
            memories,
            warnings: warning.into_iter().collect(),
        }))
    }

    #[tool(
        name = "stats",
        description = "Report read-only totals: scoped memory and scope counts, plus unscoped \
             graph entity and edge counts. Counts only; use recall for content and graph for \
             paths."
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
        description = "Store a graph-only directed relationship between named entities; it does \
             not create or link a narrative memory. Missing entities are created, and repeating \
             the same relationship reuses the existing edge. The graph is unscoped and shared by \
             every query."
    )]
    fn relate(
        &self,
        Parameters(input): Parameters<RelateInput>,
    ) -> Result<Json<RelationOutput>, CallToolResult> {
        let mut service = self.lock()?;
        let related = service.relate(RelateRequest {
            source: entity_reference(input.source),
            relation: input.relation,
            target: entity_reference(input.target),
            metadata: input.metadata,
        });
        let warning = service.take_embedding_warning();
        let details = related.map_err(|error| tool_error(error.to_string()))?;
        let mut output = relation_output(details);
        output.warnings = warning.into_iter().collect();
        Ok(Json(output))
    }

    #[tool(
        name = "graph",
        description = "Inspect the unscoped entity graph around one entity; this does not search \
             narrative memories. Each result path starts at the requested entity; an incoming hop \
             means the related entity points to the preceding entity."
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
        description = "Permanently delete one memory by id. The id must name a memory in the \
             given scopes, in global, or with no scope at all; omitted scopes use the server's \
             default scope. Pass the repository explicitly to reach a memory outside it."
    )]
    fn forget(
        &self,
        Parameters(input): Parameters<IdInput>,
    ) -> Result<Json<ForgetOutput>, CallToolResult> {
        let id = input.id;
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        self.lock()?
            .forget(id, Some(&scopes))
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(ForgetOutput {
            id,
            forgotten: true,
        }))
    }

    #[tool(
        name = "update",
        description = "Revise one memory in place by id. Fields you pass replace the stored \
             ones; fields you omit are kept. Prefer this over storing a second memory when a \
             decision or convention has changed. Scopes, entities, and relations are left \
             untouched; changed content is re-embedded in the same transaction, so a failed \
             embedding changes nothing. The id must name a memory in the given scopes, in \
             global, or with no scope at all; omitted scopes use the server's default scope. \
             Pass the repository explicitly to reach a memory outside it."
    )]
    fn update(
        &self,
        Parameters(input): Parameters<UpdateInput>,
    ) -> Result<Json<MemoryRecord>, CallToolResult> {
        let id = input.id;
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        let mut service = self.lock()?;
        service
            .update(
                id,
                input.content,
                input.memory_type,
                input.importance,
                Some(&scopes),
            )
            .map_err(|error| tool_error(error.to_string()))?;
        let details = service
            .show(id, None)
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(record(details.memory, details.scopes, None)))
    }

    #[tool(
        name = "inspect",
        description = "Return one memory by id, including its content, metadata, and scopes. \
             The id must name a memory in the given scopes, in global, or with no scope at all; \
             omitted scopes use the server's default scope. Pass the repository explicitly to \
             reach a memory outside it."
    )]
    fn inspect(
        &self,
        Parameters(input): Parameters<IdInput>,
    ) -> Result<Json<MemoryRecord>, CallToolResult> {
        let id = input.id;
        validate_scopes(&input.scopes)?;
        let scopes = self.scopes(input.scopes);
        let details = self
            .lock()?
            .inspect(id, Some(&scopes))
            .map_err(|error| tool_error(error.to_string()))?;
        Ok(Json(record(details.memory, details.scopes, None)))
    }
}

#[tool_handler(name = "gmem", router = self.tool_router)]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("gmem", env!("CARGO_PKG_VERSION")))
        .with_instructions(format!(
            "Graphmem has two separate local stores: scoped narrative memory and an \
                 unscoped entity graph. Omitted scopes use the Git repository containing the \
                 server's startup working directory (shared by linked worktrees) and include \
                 global memories during recall; outside a Git repository they use global. \
                 Default scope: {}. Pass global or repo:/absolute/path to choose a scope. {}{}",
            self.default_scope,
            embedding_note(&self.embedding),
            self.code_note()
        ))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListResourcesResult::with_all_items(vec![
            Resource::new(EMBEDDING_RESOURCE_URI, "embedding")
                .with_description(
                    "Active embedding model and the exact token limit it truncates input to",
                )
                .with_mime_type("application/json"),
        ])))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, McpError>> + MaybeSendFuture + '_ {
        self.read_embedding_resource(request.uri)
    }
}

const EMBEDDING_RESOURCE_URI: &str = "gmem://embedding";

impl MemoryServer {
    /// Reads the model's `config.json` off the async worker and without the
    /// service lock: the fetch can reach the network on a cold cache, and every
    /// tool call would otherwise wait behind it.
    async fn read_embedding_resource(
        &self,
        uri: String,
    ) -> std::result::Result<ReadResourceResponse, McpError> {
        if uri != EMBEDDING_RESOURCE_URI {
            return Err(McpError::invalid_params(
                format!("unknown resource: {uri}"),
                None,
            ));
        }
        let config = self.embedding.clone();
        let details = tokio::task::spawn_blocking(move || embedding_details(&config))
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        let body = serde_json::to_string_pretty(&serde_json::json!({
            "enabled": details.enabled,
            "model": details.model,
            "revision": details.revision,
            "max_tokens": details.max_tokens,
        }))
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(body, uri).with_mime_type("application/json"),
        ])
        .into())
    }
}

impl MemoryServer {
    #[cfg(feature = "code")]
    fn code_note(&self) -> &'static str {
        if self.code.is_some() {
            " find_symbol finds definitions by name and code_outline lists a file's symbols with \
             line ranges, through a separate code index of the Git checkout that both keep fresh \
             themselves (never through memory). Use them before grepping or reading whole files \
             to locate code; use text search for call sites and references, which they do not \
             index."
        } else {
            ""
        }
    }

    #[cfg(not(feature = "code"))]
    fn code_note(&self) -> &'static str {
        ""
    }
}

#[cfg(feature = "code")]
#[derive(Debug, Deserialize, JsonSchema)]
struct CodeOutlineInput {
    /// File to outline, relative to the checkout root; an absolute path inside
    /// the checkout also works.
    path: String,
    /// Index of the first symbol to return. Defaults to 0.
    #[serde(default)]
    offset: Option<usize>,
    /// Maximum symbols to return, 1 to 500. Defaults to 200.
    #[serde(default)]
    limit: Option<usize>,
    /// Absolute path inside the Git checkout to read. Omit to use the
    /// server's startup directory.
    #[serde(default)]
    root: Option<String>,
}

#[cfg(feature = "code")]
#[derive(Debug, Deserialize, JsonSchema)]
struct FindSymbolInput {
    /// Symbol name, name prefix, or last name segments, e.g. "fetch_user",
    /// "ConsentLive" for "MyAppWeb.ConsentLive", or "render/2" for an Elixir
    /// function of arity 2.
    query: String,
    /// Optional kind filter, e.g. "function", "class", "module", "table".
    /// Usages are returned only when asked for: "import" (import lines),
    /// "component" and "slot" (HEEx tags), "expression" (EEx directives).
    #[serde(default)]
    kind: Option<String>,
    /// Maximum matches to return, 1 to 100. Defaults to 20.
    #[serde(default)]
    limit: Option<usize>,
    /// Absolute path inside the Git checkout to search. Omit to use the
    /// server's startup directory.
    #[serde(default)]
    root: Option<String>,
}

#[cfg(feature = "code")]
#[derive(Debug, Serialize, JsonSchema)]
struct CodeSymbolRecord {
    /// Position of the symbol in the file's outline.
    index: usize,
    /// `index` of the syntactically enclosing symbol.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<usize>,
    name: String,
    kind: String,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    signature: String,
}

#[cfg(feature = "code")]
#[derive(Debug, Serialize, JsonSchema)]
struct CodeOutlineOutput {
    path: String,
    language: String,
    /// "complete", or "partial: <reason>" when symbols may be missing.
    coverage: String,
    total: usize,
    symbols: Vec<CodeSymbolRecord>,
    /// Offset of the next page, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

#[cfg(feature = "code")]
#[derive(Debug, Serialize, JsonSchema)]
struct SymbolMatchRecord {
    path: String,
    language: String,
    /// "fresh", "stale" (file changed since indexing), or "missing".
    freshness: String,
    /// Name of the enclosing symbol, such as the module or class.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    name: String,
    kind: String,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    signature: String,
}

#[cfg(feature = "code")]
#[derive(Debug, Serialize, JsonSchema)]
struct FindSymbolOutput {
    matches: Vec<SymbolMatchRecord>,
    /// Number of matches, including those beyond `limit`.
    total: usize,
    /// True when the checkout has more source files than the index accepts,
    /// so a definition may be missing.
    truncated: bool,
}

#[cfg(feature = "code")]
#[tool_router(router = code_router)]
impl MemoryServer {
    #[tool(
        name = "code_outline",
        description = "List the definitions in one source file of a Git checkout: modules, \
             classes, functions, components, SQL objects, and imports, with 1-based line and \
             column ranges and the index of each symbol's syntactic parent. Reads the file itself \
             and re-indexes it when it changed, so the result is never stale. coverage is \
             \"partial: <reason>\" when syntax errors or template limits may hide symbols. \
             Supports Rust, Go, Zig, C, C++ (including .h), Python, JavaScript/JSX, \
             TypeScript/TSX, Elixir (including ~H), HEEx, EEx, Ruby, PHP, SQL, Bash, CSS, SCSS, \
             and HTML/HEEx <script> and <style>."
    )]
    fn code_outline(
        &self,
        Parameters(input): Parameters<CodeOutlineInput>,
    ) -> Result<Json<CodeOutlineOutput>, CallToolResult> {
        let limit = input.limit.unwrap_or(200);
        if !(1..=500).contains(&limit) {
            return Err(tool_error("limit must be between 1 and 500"));
        }
        let offset = input.offset.unwrap_or(0);
        let directory = code_directory(input.root)?;
        let outline = self
            .code()?
            .outline(&directory, &input.path)
            .map_err(|error| tool_error(error.to_string()))?;
        let total = outline.symbols.len();
        let symbols = outline
            .symbols
            .into_iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(index, symbol)| CodeSymbolRecord {
                index,
                parent: symbol.parent,
                name: symbol.name,
                kind: symbol.kind,
                start_line: symbol.start.line,
                start_column: symbol.start.column,
                end_line: symbol.end.line,
                end_column: symbol.end.column,
                signature: symbol.signature,
            })
            .collect();
        let next_offset = offset.saturating_add(limit);
        Ok(Json(CodeOutlineOutput {
            path: outline.path,
            language: outline.language,
            coverage: outline.coverage.as_text(),
            total,
            symbols,
            next_offset: (next_offset < total).then_some(next_offset),
        }))
    }

    #[tool(
        name = "find_symbol",
        description = "Find definitions by name in a Git checkout. Refreshes the checkout's code \
             index first (only changed files are re-read), so results cover every source file. \
             Exact names come first (an Elixir name also matches name/arity), then names ending \
             in .query (ConsentLive finds MyAppWeb.ConsentLive), then prefix matches; matching \
             ignores case. Imports and HEEx component/slot usages are left out unless asked for \
             with kind. Returns every match with its path, line range, and enclosing symbol, and \
             never picks one as the resolved target; total counts matches beyond limit, and \
             truncated means the checkout exceeded the index's file limit."
    )]
    fn find_symbol(
        &self,
        Parameters(input): Parameters<FindSymbolInput>,
    ) -> Result<Json<FindSymbolOutput>, CallToolResult> {
        let limit = input.limit.unwrap_or(20);
        if !(1..=MAX_FIND_LIMIT).contains(&limit) {
            return Err(tool_error("limit must be between 1 and 100"));
        }
        let directory = code_directory(input.root)?;
        let found = self
            .code()?
            .find_symbol(&directory, &input.query, input.kind.as_deref(), limit)
            .map_err(|error| tool_error(error.to_string()))?;
        let matches = found
            .hits
            .into_iter()
            .map(|hit| SymbolMatchRecord {
                path: hit.path,
                language: hit.language,
                freshness: hit.freshness.as_str().to_owned(),
                parent: hit.parent,
                name: hit.symbol.name,
                kind: hit.symbol.kind,
                start_line: hit.symbol.start.line,
                start_column: hit.symbol.start.column,
                end_line: hit.symbol.end.line,
                end_column: hit.symbol.end.column,
                signature: hit.symbol.signature,
            })
            .collect();
        Ok(Json(FindSymbolOutput {
            matches,
            total: found.total,
            truncated: found.truncated,
        }))
    }
}

#[cfg(feature = "code")]
impl MemoryServer {
    fn code(&self) -> Result<MutexGuard<'_, CodeService>, CallToolResult> {
        self.code
            .as_ref()
            .ok_or_else(|| tool_error("code tools are disabled"))?
            .lock()
            .map_err(|_| tool_error("code index lock is poisoned"))
    }
}

#[cfg(feature = "code")]
fn code_directory(root: Option<String>) -> Result<std::path::PathBuf, CallToolResult> {
    match root {
        Some(root) if Path::new(&root).is_absolute() => Ok(root.into()),
        Some(_) => Err(tool_error("root must be an absolute path")),
        None => std::env::current_dir().map_err(|error| tool_error(error.to_string())),
    }
}

/// Tells an MCP client which model ranks recall and how long input is handled,
/// so it can interpret `warnings` and prefer lexical search when it needs
/// content past the model's token limit.
fn embedding_note(embedding: &EmbeddingConfig) -> String {
    if embedding.enabled {
        format!(
            "Recall embeds memories, entities, and relations with {}; input longer than the \
             model's max_position_embeddings (512 tokens for MiniLM checkpoints), capped at {} \
             tokens, is truncated before embedding, so content past that limit does not affect \
             ranking. remember, recall, and relate report truncation in their warnings field; for \
             exact-token lookup of long content use recall with use_embeddings false, which ranks \
             the full text lexically. Read the {} resource for the active model and its exact \
             token limit.",
            embedding.model, MAX_EMBEDDING_TOKENS, EMBEDDING_RESOURCE_URI
        )
    } else {
        "Embeddings are disabled; recall ranks the full text with SQLite FTS5 lexical search."
            .to_owned()
    }
}

fn record(memory: Memory, scopes: Vec<Scope>, score: Option<f64>) -> MemoryRecord {
    MemoryRecord {
        id: memory.id,
        content: memory.content,
        memory_type: memory.memory_type,
        importance: memory.importance,
        created_at: memory.created_at,
        updated_at: memory.updated_at,
        last_accessed_at: memory.last_accessed_at,
        access_count: memory.access_count,
        scopes: scopes.into_iter().map(|scope| scope.name).collect(),
        score,
        warnings: Vec::new(),
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
        warnings: Vec::new(),
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
    std::env::current_dir()
        .ok()
        .and_then(|path| git_repository(&path))
        .and_then(|repo| repo.common_dir.to_str().map(|path| format!("repo:{path}")))
        .unwrap_or_else(|| "global".to_owned())
}

fn resolve_scopes(scopes: Vec<String>) -> Vec<String> {
    let mut resolved = Vec::new();
    for scope in scopes {
        let Some(path) = scope.strip_prefix("repo:") else {
            if !resolved.contains(&scope) {
                resolved.push(scope);
            }
            continue;
        };
        let canonical = Path::new(path).canonicalize();
        let repository = git_repository(Path::new(path));
        let Some(repo) = repository.filter(|repo| {
            canonical.as_ref().is_ok_and(|path| {
                path == &repo.common_dir || repo.checkout_root.as_ref() == Some(path)
            })
        }) else {
            if !resolved.contains(&scope) {
                resolved.push(scope);
            }
            continue;
        };
        if let Some(path) = repo.common_dir.to_str() {
            let value = format!("repo:{path}");
            if !resolved.contains(&value) {
                resolved.push(value);
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::embedding_note;
    use graphmem::infrastructure::config::EmbeddingConfig;

    fn config(enabled: bool, model: &str) -> EmbeddingConfig {
        EmbeddingConfig {
            enabled,
            model: model.to_owned(),
            revision: "main".to_owned(),
            cache_dir: PathBuf::from("/nonexistent"),
            backend: "cpu".to_owned(),
            batch_size: None,
        }
    }

    #[test]
    fn embedding_note_names_the_model_and_truncation_behavior() {
        let note = embedding_note(&config(true, "sentence-transformers/all-MiniLM-L6-v2"));
        assert!(note.contains("all-MiniLM-L6-v2"));
        assert!(note.contains("warnings"));
        assert!(note.contains("use_embeddings false"));

        assert!(embedding_note(&config(false, "ignored")).contains("FTS5"));
    }
}
