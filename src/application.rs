use std::path::Path;

use thiserror::Error;

use crate::{Database, Memory, Scope, SearchResult, StorageError};

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
        Ok(self
            .database
            .search_memories_in_scopes(query, &scopes, limit)?)
    }

    pub fn forget(&self, id: i64) -> Result<()> {
        if self.database.delete_memory(id)? {
            Ok(())
        } else {
            Err(ApplicationError::NotFound("memory"))
        }
    }

    pub fn scopes(&self) -> Result<Vec<Scope>> {
        Ok(self.database.list_scopes()?)
    }
}
