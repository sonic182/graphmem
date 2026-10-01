use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct GitRepository {
    pub common_dir: PathBuf,
    pub checkout_root: Option<PathBuf>,
}

pub fn git_repository(directory: &Path) -> Option<GitRepository> {
    let directory = directory.canonicalize().ok()?;
    let common_dir = git(&directory, &["rev-parse", "--git-common-dir"])?;
    let common_dir = directory.join(common_dir.trim_end()).canonicalize().ok()?;

    let checkout_root = git(&directory, &["rev-parse", "--show-toplevel"])
        .and_then(|root| Path::new(root.trim_end()).canonicalize().ok());
    Some(GitRepository {
        common_dir,
        checkout_root,
    })
}

/// Tracked and untracked, non-ignored files, relative to `checkout_root`.
#[cfg(feature = "code")]
pub fn list_files(checkout_root: &Path) -> Option<Vec<PathBuf>> {
    let output = git_output(
        checkout_root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    Some(
        output
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
            .filter_map(|path| std::str::from_utf8(path).ok())
            .map(PathBuf::from)
            .collect(),
    )
}

#[cfg(feature = "code")]
pub fn resolve_commit(checkout_root: &Path, revision: &str) -> Option<String> {
    if revision.starts_with('-') {
        return None;
    }
    let commit = format!("{revision}^{{commit}}");
    let id = git(
        checkout_root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &commit,
        ],
    )?;
    Some(id.trim_end().to_owned())
}

#[cfg(feature = "code")]
pub fn merge_base(checkout_root: &Path, base: &str, head: &str) -> Option<String> {
    let id = git(checkout_root, &["merge-base", base, head])?;
    Some(id.trim_end().to_owned())
}

#[cfg(feature = "code")]
pub struct ChangedFile {
    pub status: char,
    pub old_path: Option<String>,
    pub path: String,
    pub utf8: bool,
}

#[cfg(feature = "code")]
pub fn changed_files(checkout_root: &Path, base: &str, head: &str) -> Option<Vec<ChangedFile>> {
    let output = git_output(
        checkout_root,
        &[
            "diff",
            "--name-status",
            "-z",
            "-M",
            "--no-ext-diff",
            base,
            head,
        ],
    )?;
    let mut fields = output
        .split(|&byte| byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| {
            (
                String::from_utf8_lossy(field).into_owned(),
                str::from_utf8(field).is_ok(),
            )
        });
    let mut files = Vec::new();
    while let Some((status, _)) = fields.next() {
        let status = status.chars().next()?;
        let (first, first_utf8) = fields.next()?;
        files.push(if matches!(status, 'R' | 'C') {
            let (path, utf8) = fields.next()?;
            ChangedFile {
                status,
                old_path: Some(first),
                path,
                utf8: utf8 && first_utf8,
            }
        } else {
            ChangedFile {
                status,
                old_path: None,
                path: first,
                utf8: first_utf8,
            }
        });
    }
    Some(files)
}

#[cfg(feature = "code")]
pub fn file_at(checkout_root: &Path, commit: &str, path: &str) -> Option<Vec<u8>> {
    // ponytail: one git process per file version; git cat-file --batch if large PRs are slow
    git_output(
        checkout_root,
        &["cat-file", "blob", &format!("{commit}:{path}")],
    )
}

fn git(directory: &Path, args: &[&str]) -> Option<String> {
    String::from_utf8(git_output(directory, args)?).ok()
}

fn git_output(directory: &Path, args: &[&str]) -> Option<Vec<u8>> {
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
    output.status.success().then_some(output.stdout)
}
