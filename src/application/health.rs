use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use super::{MemoryService, Result};
use crate::{
    Memory, Scope,
    domain::{RecallLogStats, referenced_paths, scope_checkout_root},
    infrastructure::{repository::list_files, sqlite::now_millis},
};

const MILLIS_PER_DAY: i64 = 86_400_000;

#[derive(Debug, Clone, Copy)]
pub struct HealthOptions {
    pub idle_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeHealth {
    pub memory_type: String,
    pub total: usize,
    pub never_accessed: usize,
    pub idle: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleMemory {
    pub id: i64,
    pub memory_type: String,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthReport {
    pub idle_days: u32,
    pub memories: usize,
    pub never_accessed: usize,
    pub idle: usize,
    pub by_type: Vec<TypeHealth>,
    pub recalls: RecallLogStats,
    pub recall_log_days: Option<i64>,
    pub stale: Vec<StaleMemory>,
    pub unchecked: usize,
}

impl MemoryService {
    pub fn health(&self, options: HealthOptions) -> Result<HealthReport> {
        let now = now_millis()?;
        let cutoff = now - i64::from(options.idle_days) * MILLIS_PER_DAY;
        let memories = self.database.list_all_memories()?;
        let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let scopes = self.database.scopes_by_memory(&ids)?;

        let mut by_type = BTreeMap::<&str, (usize, usize, usize)>::new();
        let (mut never_accessed, mut idle) = (0, 0);
        for memory in &memories {
            let never = memory.access_count == 0;
            let is_idle = memory.created_at <= cutoff
                && memory.last_accessed_at.is_none_or(|at| at <= cutoff);
            never_accessed += usize::from(never);
            idle += usize::from(is_idle);
            let totals = by_type.entry(&memory.memory_type).or_default();
            totals.0 += 1;
            totals.1 += usize::from(never);
            totals.2 += usize::from(is_idle);
        }
        let mut by_type = by_type
            .into_iter()
            .map(|(memory_type, (total, never_accessed, idle))| TypeHealth {
                memory_type: memory_type.to_owned(),
                total,
                never_accessed,
                idle,
            })
            .collect::<Vec<_>>();
        by_type.sort_by(|a, b| {
            b.total
                .cmp(&a.total)
                .then(a.memory_type.cmp(&b.memory_type))
        });

        let (stale, unchecked) = stale_memories(&memories, &scopes);
        let recalls = self.database.recall_log_stats()?;
        let recall_log_days = recalls.since.map(|at| (now - at) / MILLIS_PER_DAY);

        Ok(HealthReport {
            idle_days: options.idle_days,
            memories: memories.len(),
            never_accessed,
            idle,
            by_type,
            recalls,
            recall_log_days,
            stale,
            unchecked,
        })
    }
}

struct Checkout {
    root: PathBuf,
    files: Option<Vec<String>>,
}

impl Checkout {
    fn open(root: PathBuf) -> Self {
        let files = list_files(&root).map(|files| {
            files
                .iter()
                .filter_map(|file| file.to_str().map(str::to_owned))
                .collect()
        });
        Self { root, files }
    }

    fn contains(&self, path: &str) -> bool {
        if self.root.join(path).exists() {
            return true;
        }
        let Some(files) = &self.files else {
            return false;
        };
        let nested = format!("/{path}");
        let suffix_only = !path.contains('/') && path.starts_with('.');
        files.iter().any(|file| {
            file.ends_with(&nested)
                || (suffix_only && file.ends_with(path))
                || (path.ends_with('/') && file.contains(&nested))
        })
    }
}

fn stale_memories(
    memories: &[Memory],
    scopes: &HashMap<i64, Vec<Scope>>,
) -> (Vec<StaleMemory>, usize) {
    let mut checkouts = HashMap::<PathBuf, Checkout>::new();
    let mut stale = Vec::new();
    let mut unchecked = 0;
    for memory in memories {
        let roots = scopes
            .get(&memory.id)
            .into_iter()
            .flatten()
            .filter_map(|scope| scope_checkout_root(&scope.name))
            .filter(|root| root.is_dir())
            .collect::<Vec<_>>();
        if roots.is_empty() {
            unchecked += 1;
            continue;
        }
        for root in &roots {
            checkouts
                .entry(root.clone())
                .or_insert_with(|| Checkout::open(root.clone()));
        }
        let missing = referenced_paths(&memory.content)
            .into_iter()
            .filter(|path| !roots.iter().any(|root| checkouts[root].contains(path)))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            stale.push(StaleMemory {
                id: memory.id,
                memory_type: memory.memory_type.clone(),
                missing,
            });
        }
    }
    (stale, unchecked)
}
