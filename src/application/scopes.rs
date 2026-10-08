//! Resolve repository aliases and validate explicit scope selections.

use std::path::Path;

use super::{ApplicationError, Result};
use crate::infrastructure::repository::git_repository;

pub(super) fn is_writable(scope: &str, current: &str) -> bool {
    scope == current || scope == "global"
}

pub(super) fn ensure_writable(scope: &str, current: &str) -> Result<()> {
    if !is_writable(scope, current) {
        return Err(ApplicationError::ReadOnlyScope {
            scope: scope.to_owned(),
            current: current.to_owned(),
        });
    }
    Ok(())
}

pub(super) fn current_scope() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| git_repository(&path))
        .and_then(|repo| repo.common_dir.to_str().map(|path| format!("repo:{path}")))
        .unwrap_or_else(|| "global".to_owned())
}

pub(super) fn resolve(scopes: &[String], default_scope: &str) -> Result<Vec<String>> {
    if scopes.is_empty() {
        return Ok(vec![default_scope.to_owned()]);
    }
    let mut resolved = Vec::new();
    for scope in scopes {
        let scope = scope.trim();
        let value = if scope == "global" {
            scope.to_owned()
        } else {
            let path = scope
                .strip_prefix("repo:")
                .filter(|path| Path::new(path).is_absolute())
                .ok_or(ApplicationError::InvalidScope)?;
            let canonical = Path::new(path).canonicalize();
            let repository = git_repository(Path::new(path));
            match repository.filter(|repo| {
                canonical.as_ref().is_ok_and(|path| {
                    path == &repo.common_dir || repo.checkout_root.as_ref() == Some(path)
                })
            }) {
                Some(repo) => repo
                    .common_dir
                    .to_str()
                    .map_or_else(|| scope.to_owned(), |path| format!("repo:{path}")),
                None => scope.to_owned(),
            }
        };
        if !resolved.contains(&value) {
            resolved.push(value);
        }
    }
    Ok(resolved)
}
