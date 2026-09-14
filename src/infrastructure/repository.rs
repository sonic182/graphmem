use std::{path::PathBuf, process::Command};

pub fn git_repository_root() -> Option<PathBuf> {
    let directory = std::env::current_dir().ok()?;
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    PathBuf::from(String::from_utf8(output.stdout).ok()?.trim())
        .canonicalize()
        .ok()
}
