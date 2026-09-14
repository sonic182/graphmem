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
