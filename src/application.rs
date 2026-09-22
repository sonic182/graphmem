use std::{collections::HashMap, path::Path};

use thiserror::Error;

use crate::{
    Database, Edge, Entity, EntityReference, GraphDirection, GraphPath, Memory, Relation, Scope,
    SearchResult, StorageError, StoreStats,
    domain::{edge_document, entity_document, personalized_pagerank, text_mentions},
    infrastructure::config::{
        ConfigError, ConfigOverrides, EmbeddingConfig, RetrievalConfig, embedding_config,
        retrieval_config,
    },
    infrastructure::embedding::{Embedder, EmbeddingError, EmbeddingModel},
    infrastructure::sqlite::VectorSink,
};

pub type Result<T> = std::result::Result<T, ApplicationError>;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error(transparent)]
    Embedding(#[from] EmbeddingError),
    #[error(transparent)]
    Semantic(#[from] SemanticError),
    #[error(
        "embeddings are disabled; set [embedding] enabled = true (or unset GRAPHMEM_EMBEDDINGS) to reembed"
    )]
    EmbeddingsDisabled,
}

pub struct RememberRequest {
    pub content: String,
    pub memory_type: String,
    pub importance: f64,
    pub scopes: Vec<String>,
    pub entities: Vec<EntityReference>,
    pub relations: Vec<Relation>,
}

pub struct MemoryDetails {
    pub memory: Memory,
    pub scopes: Vec<Scope>,
}

pub struct RelateRequest {
    pub source: EntityReference,
    pub relation: String,
    pub target: EntityReference,
    pub metadata: Option<String>,
}

pub struct RelationDetails {
    pub source: Entity,
    pub edge: Edge,
    pub target: Entity,
}

pub struct GraphRequest {
    pub entity: EntityReference,
    pub direction: GraphDirection,
    pub max_depth: usize,
    pub limit: usize,
}

pub struct GraphDetails {
    pub entity: Entity,
    pub paths: Vec<GraphPath>,
}

pub struct ReembedStats {
    pub memories: usize,
    pub entities: usize,
    pub edges: usize,
    pub failures: Vec<String>,
}

pub struct MemoryService {
    database: Database,
    embedding_config: EmbeddingConfig,
    retrieval_config: RetrievalConfig,
    embedder: Option<Embedder>,
}

impl MemoryService {
    /// Opens the default store. Command-line overrides take priority over
    /// `config.toml` and built-in defaults, but not over environment variables.
    pub fn open_default(overrides: ConfigOverrides) -> Result<Self> {
        let database = Database::open_default()?;
        let data_dir = database.path().parent().ok_or(StorageError::Invalid {
            field: "database path",
            message: "has no parent directory",
        })?;
        Ok(Self {
            embedding_config: embedding_config(data_dir, &overrides)?,
            retrieval_config: retrieval_config(data_dir, &overrides)?,
            database,
            embedder: None,
        })
    }

    pub fn database_path(&self) -> &Path {
        self.database.path()
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.database.schema_version()?)
    }

    pub fn stats(&self) -> Result<StoreStats> {
        Ok(self.database.stats()?)
    }

    /// Stores a memory and, when embeddings are enabled, its vectors in the
    /// same transaction. The model loads on first use; if it cannot load or
    /// embed, nothing is stored and the error is returned.
    pub fn remember(&mut self, request: RememberRequest) -> Result<Memory> {
        let scopes = if request.scopes.is_empty() {
            vec!["global".to_owned()]
        } else {
            request.scopes
        };
        let Self {
            database,
            embedding_config,
            embedder,
            ..
        } = self;
        let config: &EmbeddingConfig = embedding_config;
        let embedder = ensure_embedder(config, embedder, || Embedder::load(config))?;
        let revision = embedder.map(revision_key).unwrap_or_default();
        let mut embed = |documents: &[&str]| -> std::result::Result<_, SemanticError> {
            Ok(embedder.map_or(Ok(Vec::new()), |embedder| {
                embedder.embed_documents(documents)
            })?)
        };
        let sink = embedder.map(|embedder| VectorSink {
            model: &embedder.model_name,
            revision: &revision,
            embed: &mut embed,
        });
        Ok(database.remember_with_graph_and_vectors(
            &request.content,
            &request.memory_type,
            request.importance,
            &scopes,
            &request.entities,
            &request.relations,
            sink,
        )?)
    }

    pub fn list(&self, scope: Option<&str>, limit: usize) -> Result<Vec<Memory>> {
        match scope {
            Some(scope) => Ok(self.database.list_memories_in_scope(scope, limit)?),
            None => Ok(self.database.list_memories(limit)?),
        }
    }

    pub fn show(&self, id: i64) -> Result<MemoryDetails> {
        let memory = self
            .database
            .get_memory(id)?
            .ok_or(ApplicationError::NotFound("memory"))?;
        let scopes = self.database.list_memory_scopes(id)?;
        Ok(MemoryDetails { memory, scopes })
    }

    pub fn scopes_for(&self, memory_ids: &[i64]) -> Result<HashMap<i64, Vec<Scope>>> {
        Ok(self.database.scopes_by_memory(memory_ids)?)
    }

    pub fn search(
        &mut self,
        query: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let scopes = scope.map(|scope| vec![scope.to_owned()]);
        self.search_with_scopes(query, scopes.as_deref(), limit, None)
    }

    pub fn search_scopes(
        &mut self,
        query: &str,
        scopes: &[String],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_scopes_with_embeddings(query, scopes, limit, true, None)
    }

    pub fn search_scopes_with_embeddings(
        &mut self,
        query: &str,
        scopes: &[String],
        limit: usize,
        use_embeddings: bool,
        memory_type: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let scopes = if scopes.is_empty() {
            vec!["global".to_owned()]
        } else {
            scopes.to_vec()
        };
        if use_embeddings {
            self.search_with_scopes(query, Some(&scopes), limit, memory_type)
        } else {
            self.lexical_search(query, Some(&scopes), limit, memory_type)
        }
    }

    /// `memory_type` narrows the candidates before ranking, like the scope
    /// filter. It applies to the scoped path only; `scopes: None` searches
    /// every memory and ignores it.
    fn search_with_scopes(
        &mut self,
        query: &str,
        scopes: Option<&[String]>,
        limit: usize,
        memory_type: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        match self.semantic_search(query, scopes, limit, memory_type) {
            Ok(Some(results)) => Ok(results),
            Ok(None) => self.lexical_search(query, scopes, limit, memory_type),
            Err(SemanticError::Embedding(error)) => {
                tracing::warn!(%error, "embedding unavailable; using lexical recall");
                eprintln!("embedding unavailable; using lexical recall: {error}");
                self.lexical_search(query, scopes, limit, memory_type)
            }
            Err(SemanticError::Storage(error)) => Err(error.into()),
        }
    }

    fn semantic_search(
        &mut self,
        query: &str,
        scopes: Option<&[String]>,
        limit: usize,
        memory_type: Option<&str>,
    ) -> std::result::Result<Option<Vec<SearchResult>>, SemanticError> {
        let Self {
            database,
            embedding_config,
            retrieval_config,
            embedder,
        } = self;
        let config: &EmbeddingConfig = embedding_config;
        let embedder = match ensure_embedder(config, embedder, || Embedder::load(config)) {
            Ok(Some(embedder)) => embedder,
            Ok(None) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        semantic_results(
            database,
            embedder,
            retrieval_config,
            query,
            scopes,
            limit,
            memory_type,
        )
    }

    pub fn reembed_all(&mut self) -> Result<ReembedStats> {
        if !self.embedding_config.enabled {
            return Err(ApplicationError::EmbeddingsDisabled);
        }
        let Self {
            database,
            embedding_config,
            embedder,
            ..
        } = self;
        if embedder.is_none() {
            *embedder = Some(Embedder::load(embedding_config)?);
        }
        let embedder = embedder.as_ref().expect("embedder just loaded");

        let mut stats = ReembedStats {
            memories: 0,
            entities: 0,
            edges: 0,
            failures: Vec::new(),
        };

        // Embed in chunks; a failing chunk is retried item by item so the
        // report still names each failing record.
        let batch_size = embedder.batch_size();
        let memories = database.list_all_memories()?;
        for chunk in memories.chunks(batch_size) {
            if fill_missing_vectors(database, embedder, chunk, &[], &[]).is_ok() {
                stats.memories += chunk.len();
                continue;
            }
            for memory in chunk {
                match memory_vector(database, embedder, memory.id, &memory.content) {
                    Ok(_) => stats.memories += 1,
                    Err(error) => stats
                        .failures
                        .push(format!("memory {}: {error}", memory.id)),
                }
            }
        }

        let entities = database.list_entities()?;
        for chunk in entities.chunks(batch_size) {
            if fill_missing_vectors(database, embedder, &[], chunk, &[]).is_ok() {
                stats.entities += chunk.len();
                continue;
            }
            for entity in chunk {
                match entity_vector(database, embedder, entity) {
                    Ok(_) => stats.entities += 1,
                    Err(error) => stats
                        .failures
                        .push(format!("entity {}: {error}", entity.id)),
                }
            }
        }
        let entities_by_id: HashMap<i64, &Entity> =
            entities.iter().map(|entity| (entity.id, entity)).collect();

        let mut edges = database.list_all_edges()?;
        edges.retain(|edge| {
            let linked = entities_by_id.contains_key(&edge.source_id)
                && entities_by_id.contains_key(&edge.target_id);
            if !linked {
                stats
                    .failures
                    .push(format!("edge {}: source or target entity missing", edge.id));
            }
            linked
        });
        for chunk in edges.chunks(batch_size) {
            if fill_missing_vectors(database, embedder, &[], &entities, chunk).is_ok() {
                stats.edges += chunk.len();
                continue;
            }
            for edge in chunk {
                let source = entities_by_id[&edge.source_id];
                let target = entities_by_id[&edge.target_id];
                match edge_vector(database, embedder, edge, source, target) {
                    Ok(_) => stats.edges += 1,
                    Err(error) => stats.failures.push(format!("edge {}: {error}", edge.id)),
                }
            }
        }

        Ok(stats)
    }

    fn lexical_search(
        &self,
        query: &str,
        scopes: Option<&[String]>,
        limit: usize,
        memory_type: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let Some(scopes) = scopes else {
            return Ok(self.database.search_memories(query, None, limit)?);
        };
        let proximate_names = self.graph_proximate_names(query)?;
        Ok(self.database.search_memories_in_scopes(
            query,
            scopes,
            limit,
            &proximate_names,
            memory_type,
        )?)
    }

    fn graph_proximate_names(&self, query: &str) -> Result<Vec<String>> {
        let entities = self.database.list_entities()?;
        let matched = entities
            .iter()
            .filter(|entity| text_mentions(query, &entity.canonical_name));
        let mut names = Vec::new();
        for entity in matched {
            names.push(entity.canonical_name.clone());
            let paths = self
                .database
                .graph_paths(entity.id, GraphDirection::Both, 1, 25)?;
            for path in paths {
                for hop in path.hops {
                    names.push(hop.entity.canonical_name);
                }
            }
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// Replaces the fields that are `Some` and keeps the rest. Changed content
    /// is re-embedded in the same transaction as the update, so a model that
    /// cannot load or embed leaves the memory untouched, exactly as `remember`
    /// stores nothing on an embedding failure.
    pub fn update(
        &mut self,
        id: i64,
        content: Option<String>,
        memory_type: Option<String>,
        importance: Option<f64>,
    ) -> Result<Memory> {
        let current = self
            .database
            .get_memory(id)?
            .ok_or(ApplicationError::NotFound("memory"))?;
        let content = content.unwrap_or(current.content);
        let memory_type = memory_type.unwrap_or(current.memory_type);
        let importance = importance.unwrap_or(current.importance);

        let Self {
            database,
            embedding_config,
            embedder,
            ..
        } = self;
        let config: &EmbeddingConfig = embedding_config;
        let embedder = ensure_embedder(config, embedder, || Embedder::load(config))?;
        let revision = embedder.map(revision_key).unwrap_or_default();
        let mut embed = |documents: &[&str]| -> std::result::Result<_, SemanticError> {
            Ok(embedder.map_or(Ok(Vec::new()), |embedder| {
                embedder.embed_documents(documents)
            })?)
        };
        let sink = embedder.map(|embedder| VectorSink {
            model: &embedder.model_name,
            revision: &revision,
            embed: &mut embed,
        });
        if !database.update_memory_with_vector(id, &content, &memory_type, importance, sink)? {
            return Err(ApplicationError::NotFound("memory"));
        }
        database
            .get_memory(id)?
            .ok_or(ApplicationError::NotFound("memory"))
    }

    pub fn forget(&self, id: i64) -> Result<()> {
        if self.database.delete_memory(id)? {
            Ok(())
        } else {
            Err(ApplicationError::NotFound("memory"))
        }
    }

    pub fn flush(&mut self) -> Result<()> {
        Ok(self.database.flush()?)
    }

    pub fn scopes(&self) -> Result<Vec<Scope>> {
        Ok(self.database.list_scopes()?)
    }

    /// Stores a relation and embeds its entities and edge. Entity and edge
    /// rows are not transactional here; on an embedding error they stay
    /// without vectors (a retry reuses them) and the error is returned.
    pub fn relate(&mut self, request: RelateRequest) -> Result<RelationDetails> {
        let source = self.find_or_create_entity(request.source)?;
        let target = self.find_or_create_entity(request.target)?;
        let edge =
            match self
                .database
                .get_edge_by_endpoints(source.id, &request.relation, target.id)?
            {
                Some(edge) => edge,
                None => self.database.create_edge(
                    source.id,
                    &request.relation,
                    target.id,
                    request.metadata.as_deref(),
                )?,
            };
        let details = RelationDetails {
            source,
            edge,
            target,
        };
        let Self {
            database,
            embedding_config,
            embedder,
            ..
        } = self;
        let config: &EmbeddingConfig = embedding_config;
        if let Some(embedder) = ensure_embedder(config, embedder, || Embedder::load(config))? {
            fill_missing_vectors(
                database,
                embedder,
                &[],
                &[details.source.clone(), details.target.clone()],
                std::slice::from_ref(&details.edge),
            )?;
        }
        Ok(details)
    }

    pub fn graph(&self, request: GraphRequest) -> Result<GraphDetails> {
        let entity = self.find_entity(request.entity)?;
        Ok(GraphDetails {
            paths: self.database.graph_paths(
                entity.id,
                request.direction,
                request.max_depth,
                request.limit,
            )?,
            entity,
        })
    }

    fn find_entity(&self, entity: EntityReference) -> Result<Entity> {
        let kind = entity.kind.trim().to_lowercase();
        let canonical_name = entity.name.trim().to_lowercase();
        self.database
            .get_entity_by_canonical(&kind, &canonical_name)?
            .ok_or(ApplicationError::NotFound("entity"))
    }

    fn find_or_create_entity(&self, entity: EntityReference) -> Result<Entity> {
        let kind = entity.kind.trim().to_lowercase();
        let name = entity.name.trim();
        let canonical_name = name.to_lowercase();
        match self
            .database
            .get_entity_by_canonical(&kind, &canonical_name)?
        {
            Some(entity) => Ok(entity),
            None => Ok(self.database.create_entity(&kind, name, &canonical_name)?),
        }
    }
}

#[derive(Debug, Error)]
pub enum SemanticError {
    #[error(transparent)]
    Embedding(#[from] EmbeddingError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

pub(crate) fn semantic_results<M: EmbeddingModel>(
    database: &Database,
    embedder: &M,
    retrieval: &RetrievalConfig,
    query: &str,
    scopes: Option<&[String]>,
    limit: usize,
    memory_type: Option<&str>,
) -> std::result::Result<Option<Vec<SearchResult>>, SemanticError> {
    let query_vector = embedder.embed_query(query)?;
    let memories = match scopes {
        Some(scopes) => database.list_memories_in_scopes(scopes, memory_type)?,
        None => database.list_all_memories()?,
    };
    if memories.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let entities = database.list_entities()?;
    let edges = database.list_all_edges()?;
    let (memory_vectors, entity_vectors, edge_vectors) =
        fill_missing_vectors(database, embedder, &memories, &entities, &edges)?;
    let mut entity_nodes = HashMap::new();
    for (index, entity) in entities.iter().enumerate() {
        entity_nodes.insert(entity.id, memories.len() + index);
    }
    let mut adjacency = vec![Vec::new(); memories.len() + entities.len()];
    let mut memory_scores = Vec::new();
    let mut entity_scores = Vec::new();
    let mut edge_scores = Vec::new();
    let mut edge_endpoints = Vec::new();
    let mut anchors = Vec::new();

    let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let mut memory_entities = database.entities_by_memory(&memory_ids)?;
    for (index, memory) in memories.iter().enumerate() {
        let vector = match memory_vectors.get(&memory.id) {
            Some(vector) => std::borrow::Cow::Borrowed(vector),
            None => std::borrow::Cow::Owned(memory_vector(
                database,
                embedder,
                memory.id,
                &memory.content,
            )?),
        };
        memory_scores.push((index, dot_product(&query_vector, &vector)));
        for entity in memory_entities.remove(&memory.id).unwrap_or_default() {
            if let Some(&entity_node) = entity_nodes.get(&entity.id) {
                add_link(&mut adjacency, index, entity_node);
            }
        }
    }

    for entity in &entities {
        let node = entity_nodes[&entity.id];
        let vector = match entity_vectors.get(&entity.id) {
            Some(vector) => std::borrow::Cow::Borrowed(vector),
            None => std::borrow::Cow::Owned(entity_vector(database, embedder, entity)?),
        };
        entity_scores.push((node, dot_product(&query_vector, &vector)));
        if text_mentions(query, &entity.canonical_name) {
            anchors.push((node, retrieval.entity_anchor_weight));
        }
    }

    for edge in edges {
        let Some(&source_node) = entity_nodes.get(&edge.source_id) else {
            continue;
        };
        let Some(&target_node) = entity_nodes.get(&edge.target_id) else {
            continue;
        };
        add_link(&mut adjacency, source_node, target_node);
        let source = &entities[source_node - memories.len()];
        let target = &entities[target_node - memories.len()];
        let vector = match edge_vectors.get(&edge.id) {
            Some(vector) => std::borrow::Cow::Borrowed(vector),
            None => {
                std::borrow::Cow::Owned(edge_vector(database, embedder, &edge, source, target)?)
            }
        };
        edge_scores.push((edge_endpoints.len(), dot_product(&query_vector, &vector)));
        edge_endpoints.push((source_node, target_node));
    }

    let mut memory_seeds = sharpen(
        memory_scores,
        retrieval.seed_top_k,
        retrieval.seed_temperature,
    );
    let mut graph_seeds = sharpen(
        entity_scores,
        retrieval.seed_top_k,
        retrieval.seed_temperature,
    );
    for (edge, weight) in sharpen(
        edge_scores,
        retrieval.seed_top_k,
        retrieval.seed_temperature,
    ) {
        let (source_node, target_node) = edge_endpoints[edge];
        graph_seeds.push((source_node, weight * 0.5));
        graph_seeds.push((target_node, weight * 0.5));
    }
    scale(&mut graph_seeds, 1.0 - retrieval.memory_seed_weight);
    graph_seeds.extend(anchors);
    scale(&mut memory_seeds, retrieval.memory_seed_weight);
    memory_seeds.extend(graph_seeds);
    if memory_seeds.is_empty() {
        return Ok(None);
    }

    let rank = personalized_pagerank(
        &adjacency,
        &memory_seeds,
        retrieval.damping,
        PAGERANK_ITERATIONS,
    );
    let mut results = memories
        .into_iter()
        .enumerate()
        .filter_map(|(index, memory)| {
            (rank[index] > 0.0).then_some(SearchResult {
                memory,
                score: rank[index],
            })
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.memory.importance.total_cmp(&left.memory.importance))
            .then_with(|| right.memory.updated_at.cmp(&left.memory.updated_at))
            .then_with(|| right.memory.id.cmp(&left.memory.id))
    });
    results.truncate(limit);
    Ok(Some(results))
}

/// Loads the model on first use. `Ok(None)` means embeddings are disabled.
/// A failed load is not cached, so the next call retries it: a transient
/// network or disk failure does not disable embeddings for the rest of the
/// process. A successful load is reused for the lifetime of the service.
fn ensure_embedder<'a, E>(
    config: &EmbeddingConfig,
    slot: &'a mut Option<E>,
    load: impl Fn() -> std::result::Result<E, EmbeddingError>,
) -> std::result::Result<Option<&'a E>, EmbeddingError> {
    if !config.enabled {
        return Ok(None);
    }
    if slot.is_none() {
        *slot = Some(load()?);
    }
    Ok(slot.as_ref())
}

/// Embeds and stores, in one model call per kind, every given memory,
/// entity, and edge that has no vector for the current model yet. Edges
/// whose endpoints are not in `entities` are skipped.
type VectorMap = HashMap<i64, Vec<f32>>;

fn fill_missing_vectors<M: EmbeddingModel>(
    database: &Database,
    embedder: &M,
    memories: &[Memory],
    entities: &[Entity],
    edges: &[Edge],
) -> std::result::Result<(VectorMap, VectorMap, VectorMap), SemanticError> {
    let model = embedder.model_name();
    let revision = revision_key(embedder);

    let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    let mut memory_vectors = database.memory_embeddings(&memory_ids, model, &revision)?;
    let mut pending = Vec::new();
    for memory in memories {
        if !memory_vectors.contains_key(&memory.id) {
            pending.push((memory.id, memory.content.clone()));
        }
    }
    for ((id, _), vector) in pending.iter().zip(embed_pending(embedder, &pending)?) {
        database.store_memory_embedding(*id, model, &revision, &vector)?;
        memory_vectors.insert(*id, vector);
    }

    let entity_ids = entities.iter().map(|entity| entity.id).collect::<Vec<_>>();
    let mut entity_vectors = database.entity_embeddings(&entity_ids, model, &revision)?;
    let mut pending = Vec::new();
    for entity in entities {
        if !entity_vectors.contains_key(&entity.id) {
            pending.push((entity.id, entity_document(entity)));
        }
    }
    for ((id, _), vector) in pending.iter().zip(embed_pending(embedder, &pending)?) {
        database.store_entity_embedding(*id, model, &revision, &vector)?;
        entity_vectors.insert(*id, vector);
    }

    let entities_by_id: HashMap<i64, &Entity> =
        entities.iter().map(|entity| (entity.id, entity)).collect();
    let edge_ids = edges.iter().map(|edge| edge.id).collect::<Vec<_>>();
    let mut edge_vectors = database.edge_embeddings(&edge_ids, model, &revision)?;
    let mut pending = Vec::new();
    for edge in edges {
        let (Some(source), Some(target)) = (
            entities_by_id.get(&edge.source_id),
            entities_by_id.get(&edge.target_id),
        ) else {
            continue;
        };
        if !edge_vectors.contains_key(&edge.id) {
            pending.push((edge.id, edge_document(edge, source, target)));
        }
    }
    for ((id, _), vector) in pending.iter().zip(embed_pending(embedder, &pending)?) {
        database.store_edge_embedding(*id, model, &revision, &vector)?;
        edge_vectors.insert(*id, vector);
    }
    Ok((memory_vectors, entity_vectors, edge_vectors))
}

fn embed_pending<M: EmbeddingModel>(
    embedder: &M,
    pending: &[(i64, String)],
) -> std::result::Result<Vec<Vec<f32>>, EmbeddingError> {
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    let documents = pending
        .iter()
        .map(|(_, document)| document.as_str())
        .collect::<Vec<_>>();
    let vectors = embedder.embed_documents(&documents)?;
    if vectors.len() != documents.len() {
        return Err(EmbeddingError::VectorCount {
            expected: documents.len(),
            actual: vectors.len(),
        });
    }
    Ok(vectors)
}

fn memory_vector<M: EmbeddingModel>(
    database: &Database,
    embedder: &M,
    memory_id: i64,
    content: &str,
) -> std::result::Result<Vec<f32>, SemanticError> {
    let revision = revision_key(embedder);
    if let Some(vector) = database.memory_embedding(memory_id, embedder.model_name(), &revision)? {
        return Ok(vector);
    }
    let vector = embedder.embed_document(content)?;
    database.store_memory_embedding(memory_id, embedder.model_name(), &revision, &vector)?;
    Ok(vector)
}

fn entity_vector<M: EmbeddingModel>(
    database: &Database,
    embedder: &M,
    entity: &Entity,
) -> std::result::Result<Vec<f32>, SemanticError> {
    let revision = revision_key(embedder);
    if let Some(vector) = database.entity_embedding(entity.id, embedder.model_name(), &revision)? {
        return Ok(vector);
    }
    let vector = embedder.embed_document(&entity_document(entity))?;
    database.store_entity_embedding(entity.id, embedder.model_name(), &revision, &vector)?;
    Ok(vector)
}

fn edge_vector<M: EmbeddingModel>(
    database: &Database,
    embedder: &M,
    edge: &Edge,
    source: &Entity,
    target: &Entity,
) -> std::result::Result<Vec<f32>, SemanticError> {
    let revision = revision_key(embedder);
    if let Some(vector) = database.edge_embedding(edge.id, embedder.model_name(), &revision)? {
        return Ok(vector);
    }
    let vector = embedder.embed_document(&edge_document(edge, source, target))?;
    database.store_edge_embedding(edge.id, embedder.model_name(), &revision, &vector)?;
    Ok(vector)
}

// Stored vectors are keyed by model and revision only, so a change to the text
// built by domain::entity_document or domain::edge_document must bump this or stale vectors
// silently survive.
const DOCUMENT_FORMAT: &str = "d1";
const PAGERANK_ITERATIONS: usize = 32;

fn revision_key<M: EmbeddingModel>(embedder: &M) -> String {
    format!("{}#{DOCUMENT_FORMAT}", embedder.revision())
}

fn sharpen(mut scored: Vec<(usize, f64)>, top_k: usize, temperature: f64) -> Vec<(usize, f64)> {
    scored.retain(|(_, score)| score.is_finite());
    scored.sort_by(|left, right| right.1.total_cmp(&left.1));
    scored.truncate(top_k);
    let Some(&(_, best)) = scored.first() else {
        return Vec::new();
    };
    scored
        .into_iter()
        .map(|(key, score)| (key, ((score - best) / temperature).exp()))
        .collect()
}

fn scale(seeds: &mut [(usize, f64)], weight: f64) {
    let total = seeds.iter().map(|(_, seed)| seed).sum::<f64>();
    if total <= 0.0 {
        return;
    }
    for (_, seed) in seeds {
        *seed = *seed / total * weight;
    }
}

fn add_link(adjacency: &mut [Vec<usize>], first: usize, second: usize) {
    if first == second {
        return;
    }
    adjacency[first].push(second);
    adjacency[second].push(first);
}

fn dot_product(left: &[f32], right: &[f32]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use std::cell::Cell;

    use crate::{
        Database, EntityReference, Relation,
        infrastructure::config::{EmbeddingConfig, RetrievalConfig},
        infrastructure::embedding::{EmbeddingError, EmbeddingModel},
        infrastructure::sqlite::VectorSink,
    };

    use super::{
        MemoryService, SemanticError, ensure_embedder, fill_missing_vectors, revision_key,
        semantic_results,
    };

    /// Counts `embed_documents` calls; vectors come from `FakeEmbedder`.
    #[derive(Default)]
    struct CountingEmbedder {
        calls: Cell<usize>,
    }

    impl EmbeddingModel for CountingEmbedder {
        fn model_name(&self) -> &str {
            FakeEmbedder.model_name()
        }

        fn revision(&self) -> &str {
            FakeEmbedder.revision()
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
            FakeEmbedder.embed_query(query)
        }

        fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError> {
            FakeEmbedder.embed_document(document)
        }

        fn embed_documents(&self, documents: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.calls.set(self.calls.get() + 1);
            documents
                .iter()
                .map(|document| FakeEmbedder.embed_document(document))
                .collect()
        }
    }

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is valid")
            .as_nanos();
        std::env::temp_dir().join(format!("graphmem-{label}-{}-{nonce}", std::process::id()))
    }

    fn api_depends_on_retry() -> Relation {
        Relation {
            source: EntityReference {
                kind: "component".to_owned(),
                name: "api".to_owned(),
            },
            relation: "depends_on".to_owned(),
            target: EntityReference {
                kind: "solution".to_owned(),
                name: "retry".to_owned(),
            },
            metadata: None,
        }
    }

    #[test]
    fn remember_stores_memory_entity_and_edge_vectors_in_one_call() {
        let root = temp_root("remember-vectors");
        let mut database = Database::open(&root.join("memory.sqlite")).expect("database opens");
        let embedder = CountingEmbedder::default();
        let revision = revision_key(&embedder);
        let mut embed = |documents: &[&str]| -> Result<_, SemanticError> {
            Ok(embedder.embed_documents(documents)?)
        };
        let memory = database
            .remember_with_graph_and_vectors(
                "semantic seed",
                "fact",
                0.0,
                &["global".to_owned()],
                &[],
                &[api_depends_on_retry()],
                Some(VectorSink {
                    model: "test-model",
                    revision: &revision,
                    embed: &mut embed,
                }),
            )
            .expect("memory is stored");

        assert_eq!(embedder.calls.get(), 1);
        let has = |vector: Option<Vec<f32>>| vector.is_some();
        assert!(has(database
            .memory_embedding(memory.id, "test-model", &revision)
            .unwrap()));
        for entity in database.list_entities().unwrap() {
            assert!(has(database
                .entity_embedding(entity.id, "test-model", &revision)
                .unwrap()));
        }
        let edge = &database.list_all_edges().unwrap()[0];
        assert!(has(database
            .edge_embedding(edge.id, "test-model", &revision)
            .unwrap()));
        drop(database);
        fs::remove_dir_all(root).expect("test database is removed");
    }

    #[test]
    fn remember_stores_nothing_when_embedding_fails() {
        let root = temp_root("remember-rollback");
        let mut database = Database::open(&root.join("memory.sqlite")).expect("database opens");
        let mut embed = |_: &[&str]| -> Result<Vec<Vec<f32>>, SemanticError> {
            Err(EmbeddingError::EmptyEmbedding.into())
        };
        let result = database.remember_with_graph_and_vectors(
            "never stored",
            "fact",
            0.0,
            &["global".to_owned()],
            &[],
            &[api_depends_on_retry()],
            Some(VectorSink {
                model: "test-model",
                revision: "r",
                embed: &mut embed,
            }),
        );

        assert!(result.is_err());
        let stats = database.stats().unwrap();
        assert_eq!((stats.memories, stats.entities, stats.edges), (0, 0, 0));
        drop(database);
        fs::remove_dir_all(root).expect("test database is removed");
    }

    #[test]
    fn update_reembeds_the_memory_in_one_transaction() {
        let root = temp_root("update-vectors");
        let mut database = Database::open(&root.join("memory.sqlite")).expect("database opens");
        let embedder = CountingEmbedder::default();
        let revision = revision_key(&embedder);
        let mut embed = |documents: &[&str]| -> Result<_, SemanticError> {
            Ok(embedder.embed_documents(documents)?)
        };
        let memory = database
            .remember_with_graph_and_vectors(
                "semantic seed for the upload path",
                "fact",
                0.0,
                &["global".to_owned()],
                &[],
                &[],
                Some(VectorSink {
                    model: "test-model",
                    revision: &revision,
                    embed: &mut embed,
                }),
            )
            .expect("memory is stored");
        let before = database
            .memory_embedding(memory.id, "test-model", &revision)
            .unwrap()
            .expect("memory is embedded");

        let mut embed = |documents: &[&str]| -> Result<_, SemanticError> {
            Ok(embedder.embed_documents(documents)?)
        };
        let updated = database
            .update_memory_with_vector(
                memory.id,
                "drop the upload queue entirely",
                "decision",
                0.9,
                Some(VectorSink {
                    model: "test-model",
                    revision: &revision,
                    embed: &mut embed,
                }),
            )
            .expect("memory is updated");

        assert!(updated);
        let stored = database.get_memory(memory.id).unwrap().unwrap();
        assert_eq!(stored.content, "drop the upload queue entirely");
        assert_eq!(stored.memory_type, "decision");
        let after = database
            .memory_embedding(memory.id, "test-model", &revision)
            .unwrap()
            .expect("updated memory is re-embedded");
        assert_ne!(before, after);
        drop(database);
        fs::remove_dir_all(root).expect("test database is removed");
    }

    #[test]
    fn update_stores_nothing_when_embedding_fails() {
        let root = temp_root("update-rollback");
        let mut database = Database::open(&root.join("memory.sqlite")).expect("database opens");
        let memory = database
            .remember_with_graph(
                "retry the upload",
                "fact",
                0.0,
                &["global".to_owned()],
                &[],
                &[],
            )
            .expect("memory is stored");
        database
            .store_memory_embedding(memory.id, "test-model", "r", &[1.0, 0.0])
            .expect("memory is embedded");
        let mut embed = |_: &[&str]| -> Result<Vec<Vec<f32>>, SemanticError> {
            Err(EmbeddingError::EmptyEmbedding.into())
        };

        let result = database.update_memory_with_vector(
            memory.id,
            "never stored",
            "decision",
            0.9,
            Some(VectorSink {
                model: "test-model",
                revision: "r",
                embed: &mut embed,
            }),
        );

        assert!(result.is_err());
        let stored = database.get_memory(memory.id).unwrap().unwrap();
        assert_eq!(stored.content, "retry the upload");
        assert_eq!(stored.memory_type, "fact");
        assert_eq!(
            database
                .memory_embedding(memory.id, "test-model", "r")
                .unwrap(),
            Some(vec![1.0, 0.0])
        );
        drop(database);
        fs::remove_dir_all(root).expect("test database is removed");
    }

    #[test]
    fn fill_missing_vectors_batches_each_kind_and_skips_cached_rows() {
        let root = temp_root("fill-vectors");
        let mut database = Database::open(&root.join("memory.sqlite")).expect("database opens");
        for content in ["first", "second", "third"] {
            database
                .remember_with_graph(
                    content,
                    "fact",
                    0.0,
                    &["global".to_owned()],
                    &[],
                    &[api_depends_on_retry()],
                )
                .expect("memory is stored");
        }
        let memories = database.list_all_memories().unwrap();
        let entities = database.list_entities().unwrap();
        let edges = database.list_all_edges().unwrap();
        let embedder = CountingEmbedder::default();

        fill_missing_vectors(&database, &embedder, &memories, &entities, &edges).unwrap();
        assert_eq!(
            embedder.calls.get(),
            3,
            "one call each for memories, entities, edges"
        );

        fill_missing_vectors(&database, &embedder, &memories, &entities, &edges).unwrap();
        assert_eq!(
            embedder.calls.get(),
            3,
            "cached rows are not embedded again"
        );
        drop(database);
        fs::remove_dir_all(root).expect("test database is removed");
    }

    #[test]
    fn ensure_embedder_retries_a_failed_load_and_reuses_a_successful_one() {
        let config = EmbeddingConfig {
            enabled: true,
            model: "test-model".to_owned(),
            revision: "main".to_owned(),
            cache_dir: std::path::PathBuf::from("unused"),
            backend: "cpu".to_owned(),
            batch_size: None,
        };
        let attempts = Cell::new(0);
        let load = || {
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 {
                Err(EmbeddingError::Backend("transient failure".to_owned()))
            } else {
                Ok(7u32)
            }
        };

        let mut slot: Option<u32> = None;
        assert!(ensure_embedder(&config, &mut slot, load).is_err());
        assert!(
            slot.is_none(),
            "a failed load must not poison the slot for the rest of the process"
        );
        assert_eq!(
            ensure_embedder(&config, &mut slot, load).unwrap(),
            Some(&7u32),
            "the next call retries the load"
        );
        assert_eq!(attempts.get(), 2);
        assert!(ensure_embedder(&config, &mut slot, load).is_ok());
        assert_eq!(attempts.get(), 2, "a loaded model is not reloaded");

        let disabled = EmbeddingConfig {
            enabled: false,
            ..config
        };
        let mut slot: Option<u32> = None;
        assert!(
            ensure_embedder(&disabled, &mut slot, || {
                panic!("a disabled embedder must not load")
            })
            .unwrap()
            .is_none()
        );
    }

    struct FakeEmbedder;

    impl EmbeddingModel for FakeEmbedder {
        fn model_name(&self) -> &str {
            "test-model"
        }

        fn revision(&self) -> &str {
            "test-revision"
        }

        fn embed_query(&self, _: &str) -> Result<Vec<f32>, EmbeddingError> {
            Ok(vec![1.0, 0.0])
        }

        fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError> {
            Ok(
                if document.contains("semantic seed") || document.contains("api depends_on retry") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                },
            )
        }
    }

    #[test]
    fn lexical_override_skips_embedding_initialization() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "graphmem-lexical-override-test-{}-{nonce}",
            std::process::id()
        ));
        let mut service = MemoryService {
            database: Database::open(&root.join("memory.sqlite")).expect("database opens"),
            retrieval_config: RetrievalConfig::default(),
            embedding_config: EmbeddingConfig {
                enabled: true,
                model: "missing-model".to_owned(),
                revision: "main".to_owned(),
                cache_dir: root.join("models"),
                backend: "cpu".to_owned(),
                batch_size: None,
            },
            embedder: None,
        };
        // Stored directly: `remember` would load (and try to download) the model.
        service
            .database
            .remember_with_graph(
                "lexical comparison needle",
                "test",
                0.0,
                &["global".to_owned()],
                &[],
                &[],
            )
            .expect("memory is stored");

        let results = service
            .search_scopes_with_embeddings(
                "lexical comparison needle",
                &["global".to_owned()],
                10,
                false,
                None,
            )
            .expect("lexical recall succeeds");

        assert_eq!(results.len(), 1);
        assert!(service.embedder.is_none());
        fs::remove_dir_all(root).expect("test data directory is removed");
    }

    #[test]
    fn semantic_pagerank_retrieves_multi_hop_memories_without_crossing_scopes() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "graphmem-semantic-test-{}-{nonce}",
            std::process::id()
        ));
        let mut database = Database::open(&root.join("memory.sqlite")).expect("database opens");
        let api = EntityReference {
            kind: "component".to_owned(),
            name: "api".to_owned(),
        };
        let retry = EntityReference {
            kind: "solution".to_owned(),
            name: "retry".to_owned(),
        };
        let seed = database
            .remember_with_graph(
                "semantic seed",
                "fact",
                0.0,
                &["repo:/a".to_owned()],
                std::slice::from_ref(&api),
                &[Relation {
                    source: api.clone(),
                    relation: "depends_on".to_owned(),
                    target: retry.clone(),
                    metadata: None,
                }],
            )
            .expect("seed is stored");
        let linked = database
            .remember_with_graph(
                "multi hop detail",
                "fact",
                0.0,
                &["repo:/a".to_owned()],
                std::slice::from_ref(&retry),
                &[],
            )
            .expect("linked memory is stored");
        let excluded = database
            .remember_with_graph(
                "private detail",
                "fact",
                0.0,
                &["repo:/b".to_owned()],
                &[retry],
                &[],
            )
            .expect("excluded memory is stored");

        let results = semantic_results(
            &database,
            &FakeEmbedder,
            &RetrievalConfig::default(),
            "different wording",
            Some(&["repo:/a".to_owned()]),
            10,
            None,
        )
        .expect("semantic recall succeeds")
        .expect("semantic recall has seeds");
        let ids = results
            .iter()
            .map(|result| result.memory.id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&seed.id));
        assert!(ids.contains(&linked.id));
        assert!(!ids.contains(&excluded.id));

        let filtered = semantic_results(
            &database,
            &FakeEmbedder,
            &RetrievalConfig::default(),
            "different wording",
            Some(&["repo:/a".to_owned()]),
            10,
            Some("FACT"),
        )
        .expect("filtered recall succeeds")
        .expect("filtered recall has seeds");
        assert_eq!(filtered.len(), results.len(), "case-insensitive type match");
        assert!(
            semantic_results(
                &database,
                &FakeEmbedder,
                &RetrievalConfig::default(),
                "different wording",
                Some(&["repo:/a".to_owned()]),
                10,
                Some("decision"),
            )
            .expect("unmatched recall succeeds")
            .is_none_or(|results| results.is_empty()),
            "a type nothing was stored under returns no memories"
        );
        let revision = revision_key(&FakeEmbedder);
        assert!(
            database
                .memory_embedding(seed.id, "test-model", &revision)
                .unwrap()
                .is_some()
        );
        assert!(
            database
                .edge_embedding(1, "test-model", &revision)
                .unwrap()
                .is_some()
        );
        drop(database);
        fs::remove_dir_all(root).expect("test database is removed");
    }
}
