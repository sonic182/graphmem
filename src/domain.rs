use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub id: Uuid,
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
    pub id: Uuid,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub canonical_name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: Uuid,
    pub source_id: Uuid,
    pub relation: String,
    pub target_id: Uuid,
    pub created_at: i64,
    pub metadata: Option<String>,
}
