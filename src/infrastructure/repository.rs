use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct GitRepository {
    pub common_dir: PathBuf,
    pub worktree_roots: Vec<PathBuf>,
}

pub fn git_repository(directory: &Path) -> Option<GitRepository> {
    let directory = directory.canonicalize().ok()?;
    let common_dir = git(&directory, &["rev-parse", "--git-common-dir"])?;
    let common_dir = directory.join(common_dir.trim_end()).canonicalize().ok()?;

    let worktrees = git(&directory, &["worktree", "list", "--porcelain", "-z"]).unwrap_or_default();
    let worktree_roots = worktrees
        .split('\0')
        .filter_map(|field| field.strip_prefix("worktree "))
        .filter_map(|root| Path::new(root).canonicalize().ok())
        .collect();
    Some(GitRepository {
        common_dir,
        worktree_roots,
    })
}

fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .output()
        .ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}
