use std::path::Path;

use thiserror::Error;

use crate::{
    Database, Edge, Entity, GraphDirection, GraphPath, Memory, Scope, SearchResult, StorageError,
    StoreStats, domain::text_mentions,
};

pub type Result<T> = std::result::Result<T, ApplicationError>;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("{0} not found")]
    NotFound(&'static str),
}

pub struct RememberRequest {
    pub content: String,
    pub memory_type: String,
    pub importance: f64,
    pub scopes: Vec<String>,
}

pub struct MemoryDetails {
    pub memory: Memory,
    pub scopes: Vec<Scope>,
}

pub struct EntityReference {
    pub kind: String,
    pub name: String,
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

pub struct MemoryService {
    database: Database,
}

impl MemoryService {
    pub fn open_default() -> Result<Self> {
        Ok(Self {
            database: Database::open_default()?,
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
        for scope in &scopes {
            self.database.get_scope_by_name(scope)?;
        }

        let memory = self.database.create_memory(
            &request.content,
            &request.memory_type,
            request.importance,
        )?;

        let mut scope_ids = Vec::new();
        for name in scopes {
            let scope = match self.database.get_scope_by_name(&name)? {
                Some(scope) => scope,
                None => self.database.create_scope(&name)?,
            };
            if !scope_ids.contains(&scope.id) {
                scope_ids.push(scope.id);
            }
        }

        if let Err(error) = self.database.attach_scopes(memory.id, &scope_ids) {
            let _ = self.database.delete_memory(memory.id);
            return Err(error.into());
        }
        Ok(memory)
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
        &self,
        query: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        Ok(self.database.search_memories(query, scope, limit)?)
    }

    pub fn search_scopes(
        &self,
        query: &str,
        scopes: &[String],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let scopes = if scopes.is_empty() {
            vec!["global".to_owned()]
        } else {
            scopes.to_vec()
        };
        let proximate_names = self.graph_proximate_names(query)?;
        Ok(self
            .database
            .search_memories_in_scopes(query, &scopes, limit, &proximate_names)?)
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
        Ok(RelationDetails {
            source,
            edge,
            target,
        })
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
