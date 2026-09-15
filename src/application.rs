use std::{collections::HashMap, path::Path};

use thiserror::Error;

use crate::{
    Database, Edge, Entity, EntityReference, GraphDirection, GraphPath, Memory, Relation, Scope,
    SearchResult, StorageError, StoreStats,
    domain::{personalized_pagerank, text_mentions},
    infrastructure::config::{
        ConfigError, EmbeddingConfig, RetrievalConfig, embedding_config, retrieval_config,
    },
    infrastructure::embedding::{Embedder, EmbeddingError, EmbeddingModel},
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
}

pub struct MemoryService {
    database: Database,
    embedding_config: EmbeddingConfig,
    retrieval_config: RetrievalConfig,
    embedder: Option<Embedder>,
    embedding_attempted: bool,
}

impl MemoryService {
    pub fn open_default() -> Result<Self> {
        let database = Database::open_default()?;
        let data_dir = database.path().parent().ok_or(StorageError::Invalid {
            field: "database path",
            message: "has no parent directory",
        })?;
        Ok(Self {
            embedding_config: embedding_config(data_dir)?,
            retrieval_config: retrieval_config(data_dir)?,
            database,
            embedder: None,
            embedding_attempted: false,
        })
    }

    pub fn database_path(&self) -> &Path {
        self.database.path()
    }

    pub fn stats(&self) -> Result<StoreStats> {
        Ok(self.database.stats()?)
    }

    pub fn remember(&mut self, request: RememberRequest) -> Result<Memory> {
        let scopes = if request.scopes.is_empty() {
            vec!["global".to_owned()]
        } else {
            request.scopes
        };
        let memory = self.database.remember_with_graph(
            &request.content,
            &request.memory_type,
            request.importance,
            &scopes,
            &request.entities,
            &request.relations,
        )?;
        self.embed_stored_memory(&memory);
        Ok(memory)
    }

    fn embed_stored_memory(&self, memory: &Memory) {
        let Some(embedder) = self.embedder.as_ref() else {
            return;
        };
        let database = &self.database;
        let stored = memory_vector(database, embedder, memory.id, &memory.content).and_then(
            |_| -> std::result::Result<(), SemanticError> {
                for entity in database.list_memory_entities(memory.id)? {
                    entity_vector(database, embedder, &entity)?;
                }
                Ok(())
            },
        );
        if let Err(error) = stored {
            tracing::warn!(%error, "storing embeddings failed; recall will backfill them");
        }
    }

    fn embed_stored_relation(&self, details: &RelationDetails) {
        let Some(embedder) = self.embedder.as_ref() else {
            return;
        };
        let database = &self.database;
        let stored = entity_vector(database, embedder, &details.source)
            .and(entity_vector(database, embedder, &details.target))
            .and(edge_vector(
                database,
                embedder,
                &details.edge,
                &details.source,
                &details.target,
            ));
        if let Err(error) = stored {
            tracing::warn!(%error, "storing embeddings failed; recall will backfill them");
        }
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

    pub fn search(
        &mut self,
        query: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let scopes = scope.map(|scope| vec![scope.to_owned()]);
        self.search_with_scopes(query, scopes.as_deref(), limit)
    }

    pub fn search_scopes(
        &mut self,
        query: &str,
        scopes: &[String],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_scopes_with_embeddings(query, scopes, limit, true)
    }

    pub fn search_scopes_with_embeddings(
        &mut self,
        query: &str,
        scopes: &[String],
        limit: usize,
        use_embeddings: bool,
    ) -> Result<Vec<SearchResult>> {
        let scopes = if scopes.is_empty() {
            vec!["global".to_owned()]
        } else {
            scopes.to_vec()
        };
        if use_embeddings {
            self.search_with_scopes(query, Some(&scopes), limit)
        } else {
            self.lexical_search(query, Some(&scopes), limit)
        }
    }

    fn search_with_scopes(
        &mut self,
        query: &str,
        scopes: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        match self.semantic_search(query, scopes, limit) {
            Ok(Some(results)) => Ok(results),
            Ok(None) => self.lexical_search(query, scopes, limit),
            Err(SemanticError::Embedding(error)) => {
                tracing::warn!(%error, "embedding unavailable; using lexical recall");
                eprintln!("embedding unavailable; using lexical recall: {error}");
                self.lexical_search(query, scopes, limit)
            }
            Err(SemanticError::Storage(error)) => Err(error.into()),
        }
    }

    fn semantic_search(
        &mut self,
        query: &str,
        scopes: Option<&[String]>,
        limit: usize,
    ) -> std::result::Result<Option<Vec<SearchResult>>, SemanticError> {
        let Self {
            database,
            embedding_config,
            retrieval_config,
            embedder,
            embedding_attempted,
        } = self;
        if !embedding_config.enabled {
            return Ok(None);
        }
        if embedder.is_none() && !*embedding_attempted {
            *embedding_attempted = true;
            *embedder = Some(Embedder::load(embedding_config)?);
        }
        let Some(embedder) = embedder.as_ref() else {
            return Ok(None);
        };
        semantic_results(database, embedder, retrieval_config, query, scopes, limit)
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

        let memories = database.list_all_memories()?;
        for memory in &memories {
            memory_vector(database, embedder, memory.id, &memory.content)?;
        }

        let entities = database.list_entities()?;
        for entity in &entities {
            entity_vector(database, embedder, entity)?;
        }
        let entities_by_id: HashMap<i64, &Entity> =
            entities.iter().map(|entity| (entity.id, entity)).collect();

        let edges = database.list_all_edges()?;
        for edge in &edges {
            // FK constraints (edges.source_id/target_id REFERENCES entities
            // ON DELETE CASCADE) guarantee both endpoints exist.
            let source = entities_by_id[&edge.source_id];
            let target = entities_by_id[&edge.target_id];
            edge_vector(database, embedder, edge, source, target)?;
        }

        Ok(ReembedStats {
            memories: memories.len(),
            entities: entities.len(),
            edges: edges.len(),
        })
    }

    fn lexical_search(
        &self,
        query: &str,
        scopes: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let Some(scopes) = scopes else {
            return Ok(self.database.search_memories(query, None, limit)?);
        };
        let proximate_names = self.graph_proximate_names(query)?;
        Ok(self
            .database
            .search_memories_in_scopes(query, scopes, limit, &proximate_names)?)
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

    pub fn relate(&self, request: RelateRequest) -> Result<RelationDetails> {
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
        self.embed_stored_relation(&details);
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
) -> std::result::Result<Option<Vec<SearchResult>>, SemanticError> {
    let query_vector = embedder.embed_query(query)?;
    let memories = match scopes {
        Some(scopes) => database.list_memories_in_scopes(scopes)?,
        None => database.list_all_memories()?,
    };
    if memories.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let entities = database.list_entities()?;
    let edges = database.list_all_edges()?;
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
        let vector = memory_vector(database, embedder, memory.id, &memory.content)?;
        memory_scores.push((index, dot_product(&query_vector, &vector)));
        for entity in memory_entities.remove(&memory.id).unwrap_or_default() {
            if let Some(&entity_node) = entity_nodes.get(&entity.id) {
                add_link(&mut adjacency, index, entity_node);
            }
        }
    }

    for entity in &entities {
        let node = entity_nodes[&entity.id];
        let vector = entity_vector(database, embedder, entity)?;
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
        let vector = edge_vector(database, embedder, &edge, source, target)?;
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

pub(crate) fn entity_document(entity: &Entity) -> String {
    format!("{} {}", entity.kind, entity.name)
}

pub(crate) fn edge_document(edge: &Edge, source: &Entity, target: &Entity) -> String {
    format!(
        "{} {} {}",
        source.name,
        edge.relation.replace('_', " "),
        target.name
    )
}

// Stored vectors are keyed by model and revision only, so a change to the text
// built by entity_document or edge_document must bump this or stale vectors
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

    use crate::{
        Database, EntityReference, Relation,
        infrastructure::config::{EmbeddingConfig, RetrievalConfig},
        infrastructure::embedding::{EmbeddingError, EmbeddingModel},
    };

    use super::{MemoryService, RememberRequest, revision_key, semantic_results};

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
            },
            embedder: None,
            embedding_attempted: false,
        };
        service
            .remember(RememberRequest {
                content: "lexical comparison needle".to_owned(),
                memory_type: "test".to_owned(),
                importance: 0.0,
                scopes: vec!["global".to_owned()],
                entities: vec![],
                relations: vec![],
            })
            .expect("memory is stored");

        let results = service
            .search_scopes_with_embeddings(
                "lexical comparison needle",
                &["global".to_owned()],
                10,
                false,
            )
            .expect("lexical recall succeeds");

        assert_eq!(results.len(), 1);
        assert!(!service.embedding_attempted);
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
