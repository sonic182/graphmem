#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub memory_type: String,
    pub importance: f64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_accessed_at: Option<i64>,
    pub access_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub memory: Memory,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub canonical_name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: i64,
    pub source_id: i64,
    pub relation: String,
    pub target_id: i64,
    pub created_at: i64,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityReference {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub source: EntityReference,
    pub relation: String,
    pub target: EntityReference,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphDirection {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphHop {
    pub entity: Entity,
    pub edge: Edge,
    pub direction: GraphDirection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphPath {
    pub hops: Vec<GraphHop>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreStats {
    pub memories: i64,
    pub scopes: i64,
    pub entities: i64,
    pub edges: i64,
}

pub fn text_mentions(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim();
    !needle.is_empty() && haystack.to_lowercase().contains(&needle.to_lowercase())
}

pub fn personalized_pagerank(adjacency: &[Vec<usize>], seeds: &[(usize, f64)]) -> Vec<f64> {
    let mut restart = vec![0.0; adjacency.len()];
    for &(node, weight) in seeds {
        if node < restart.len() && weight.is_finite() && weight > 0.0 {
            restart[node] += weight;
        }
    }
    let total = restart.iter().sum::<f64>();
    if total == 0.0 {
        return restart;
    }
    for weight in &mut restart {
        *weight /= total;
    }

    let mut rank = restart.clone();
    for _ in 0..32 {
        let mut next = restart
            .iter()
            .map(|weight| weight * 0.5)
            .collect::<Vec<_>>();
        for (node, neighbors) in adjacency.iter().enumerate() {
            if neighbors.is_empty() {
                for (target, weight) in restart.iter().enumerate() {
                    next[target] += rank[node] * 0.5 * weight;
                }
            } else {
                let share = rank[node] * 0.5 / neighbors.len() as f64;
                for &neighbor in neighbors {
                    if neighbor < next.len() {
                        next[neighbor] += share;
                    }
                }
            }
        }
        rank = next;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::{personalized_pagerank, text_mentions};

    #[test]
    fn matches_case_insensitively() {
        assert!(text_mentions("Retry the SQLite connection", "sqlite"));
    }

    #[test]
    fn matches_substring() {
        assert!(text_mentions("circuit breaker tripped", "circuit breaker"));
    }

    #[test]
    fn rejects_non_match() {
        assert!(!text_mentions("retry the connection", "sqlite"));
    }

    #[test]
    fn rejects_empty_needle() {
        assert!(!text_mentions("retry the connection", "   "));
    }

    #[test]
    fn pagerank_carries_a_seed_across_multiple_hops() {
        let rank = personalized_pagerank(&[vec![1], vec![0, 2], vec![1], vec![]], &[(0, 1.0)]);
        assert!(rank[2] > rank[3]);
    }
}
