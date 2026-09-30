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
