use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    Database, EntityReference, Relation,
    application::semantic_results,
    infrastructure::config::RetrievalConfig,
    infrastructure::embedding::{EmbeddingError, EmbeddingModel},
};

const SCOPE: &str = "repo:/eval";
const DIMENSIONS: usize = 12;

/// Tokens sharing a dimension stand in for synonyms, so a query can match a
/// memory that shares no literal word with it. Dimension 0 is a constant
/// present in every vector, which reproduces the narrow band of high cosines
/// that real sentence embeddings produce and that flat seeding cannot rank.
const VOCABULARY: [(&str, usize); 21] = [
    ("retry", 1),
    ("backoff", 1),
    ("redelivery", 1),
    ("timeout", 2),
    ("deadline", 2),
    ("sqlite", 3),
    ("storage", 3),
    ("migration", 4),
    ("schema", 4),
    ("embedding", 5),
    ("vector", 5),
    ("scope", 6),
    ("repository", 6),
    ("graph", 7),
    ("entity", 7),
    ("mcp", 8),
    ("stdio", 8),
    ("cli", 9),
    ("command", 9),
    ("logging", 10),
    ("stderr", 10),
];

struct BagOfWords;

impl BagOfWords {
    fn vector(text: &str) -> Vec<f32> {
        let mut vector = [0.0f32; DIMENSIONS];
        vector[0] = 1.0;
        let vocabulary = HashMap::from(VOCABULARY);
        for word in text.split_whitespace() {
            let word = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            if let Some(&dimension) = vocabulary.get(word.as_str()) {
                vector[dimension] += 1.0;
            }
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        vector.iter().map(|value| value / norm).collect()
    }
}

impl EmbeddingModel for BagOfWords {
    fn model_name(&self) -> &str {
        "bag-of-words"
    }

    fn revision(&self) -> &str {
        "eval"
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(Self::vector(query))
    }

    fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(Self::vector(document))
    }
}

struct Fixture {
    database: Database,
    root: PathBuf,
    ids: HashMap<&'static str, i64>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "graphmem-retrieval-eval-{}-{nonce}",
            std::process::id()
        ));
        let mut fixture = Self {
            database: Database::open(&root.join("memory.sqlite")).expect("database opens"),
            root,
            ids: HashMap::new(),
        };
        let worker = entity("component", "ingest worker");
        let policy = entity("policy", "delivery policy");
        let store = entity("component", "memory store");

        fixture.remember(
            "migration",
            "the schema migration runs once on open",
            std::slice::from_ref(&store),
            &[],
        );
        fixture.remember(
            "storage",
            "sqlite storage keeps one connection per process",
            std::slice::from_ref(&store),
            &[],
        );
        fixture.remember(
            "retry",
            "failed batches retry with exponential backoff",
            std::slice::from_ref(&policy),
            &[Relation {
                source: worker.clone(),
                relation: "uses".to_owned(),
                target: policy.clone(),
                metadata: None,
            }],
        );
        fixture.remember(
            "multi_hop",
            "operators page the on call rotation after three failures",
            std::slice::from_ref(&policy),
            &[],
        );
        fixture.remember("unlinked", "the cli command prints a table", &[], &[]);
        fixture.remember(
            "distractor",
            "storage migration notes mention the graph entity cache",
            &[],
            &[],
        );
        fixture.remember("logging", "logging goes to stderr not stdio", &[], &[]);
        fixture.remember("mcp", "the mcp server speaks stdio", &[], &[]);
        fixture
    }

    fn remember(
        &mut self,
        label: &'static str,
        content: &str,
        entities: &[EntityReference],
        relations: &[Relation],
    ) {
        let memory = self
            .database
            .remember_with_graph(
                content,
                "fact",
                0.0,
                &[SCOPE.to_owned()],
                entities,
                relations,
            )
            .expect("fixture memory is stored");
        self.ids.insert(label, memory.id);
    }

    fn results(&self, query: &str, config: &RetrievalConfig) -> Vec<(&'static str, f64)> {
        let results = semantic_results(
            &self.database,
            &BagOfWords,
            config,
            query,
            Some(&[SCOPE.to_owned()]),
            10,
        )
        .expect("semantic recall succeeds")
        .expect("semantic recall has seeds");
        let labels = self
            .ids
            .iter()
            .map(|(label, id)| (*id, *label))
            .collect::<HashMap<_, _>>();
        results
            .iter()
            .map(|result| (labels[&result.memory.id], result.score))
            .collect()
    }

    fn recall(&self, query: &str, config: &RetrievalConfig) -> Vec<&'static str> {
        self.results(query, config)
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }

    fn score(&self, query: &str, config: &RetrievalConfig, label: &str) -> f64 {
        self.results(query, config)
            .into_iter()
            .find(|(entry, _)| *entry == label)
            .map(|(_, score)| score)
            .unwrap_or_default()
    }
}

fn entity(kind: &str, name: &str) -> EntityReference {
    EntityReference {
        kind: kind.to_owned(),
        name: name.to_owned(),
    }
}

#[test]
fn ranks_the_memory_that_matches_the_whole_query_first() {
    let fixture = Fixture::new();
    let ranking = fixture.recall("schema migration", &RetrievalConfig::default());
    assert_eq!(ranking[0], "migration");
}

#[test]
fn matches_a_paraphrase_that_shares_no_literal_word() {
    let fixture = Fixture::new();
    let ranking = fixture.recall("redelivery after deadline", &RetrievalConfig::default());
    assert_eq!(ranking[0], "retry");
}

#[test]
fn reaches_a_memory_that_only_the_graph_connects_to_the_query() {
    let fixture = Fixture::new();
    let ranking = fixture.recall("ingest worker retry", &RetrievalConfig::default());
    let multi_hop = position(&ranking, "multi_hop");
    let unlinked = position(&ranking, "unlinked");
    assert!(
        multi_hop < unlinked,
        "graph-linked memory should outrank an unlinked one: {ranking:?}"
    );
}

#[test]
fn separates_the_best_result_from_the_tail() {
    let fixture = Fixture::new();
    // The memory channel alone, so this measures seed sharpening rather than
    // rank arriving through the graph.
    let memory_only = RetrievalConfig {
        memory_seed_weight: 1.0,
        entity_anchor_weight: 0.0,
        ..RetrievalConfig::default()
    };
    let scores = fixture
        .results("schema migration", &memory_only)
        .into_iter()
        .map(|(_, score)| score)
        .collect::<Vec<_>>();
    let total = scores.iter().sum::<f64>();
    assert!(
        scores[0] > total * 0.25,
        "flat seeding spreads rank evenly instead of ranking: {scores:?}"
    );
}

#[test]
fn the_graph_channel_weight_moves_score_toward_linked_memories() {
    let fixture = Fixture::new();
    let memory_only = RetrievalConfig {
        memory_seed_weight: 1.0,
        entity_anchor_weight: 0.0,
        ..RetrievalConfig::default()
    };
    let graph_heavy = RetrievalConfig {
        memory_seed_weight: 0.05,
        ..RetrievalConfig::default()
    };
    let query = "ingest worker retry";
    let without_graph = fixture.score(query, &memory_only, "multi_hop");
    let with_graph = fixture.score(query, &graph_heavy, "multi_hop");
    assert!(
        with_graph > without_graph,
        "graph weight should lift the linked memory: {with_graph} vs {without_graph}"
    );
}

/// Run with `cargo nextest run -E 'test(sweeps)' --run-ignored all --no-capture`
/// to compare a knob's effect on the fixture before changing its default.
#[test]
#[ignore = "prints a tuning table rather than asserting"]
fn sweeps_the_memory_channel_weight() {
    let fixture = Fixture::new();
    for weight in [0.05, 0.25, 0.5, 0.75, 0.95] {
        let config = RetrievalConfig {
            memory_seed_weight: weight,
            ..RetrievalConfig::default()
        };
        let ranking = fixture
            .results("ingest worker retry", &config)
            .into_iter()
            .map(|(label, score)| format!("{label}={score:.4}"))
            .collect::<Vec<_>>();
        println!("memory_seed_weight={weight}: {}", ranking.join(" "));
    }
}

fn position(ranking: &[&str], label: &str) -> usize {
    ranking
        .iter()
        .position(|entry| *entry == label)
        .unwrap_or(usize::MAX)
}
