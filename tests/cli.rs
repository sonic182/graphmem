use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn data_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    std::env::temp_dir().join(format!("graphmem-cli-test-{}-{nonce}", std::process::id()))
}

fn run(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gmem"))
        .args(args)
        .env("GRAPHMEM_HOME", data_dir)
        .output()
        .expect("gmem runs")
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

#[test]
fn memory_lifecycle_works_across_cli_processes() {
    let data_dir = data_dir();
    let remembered = stdout(run(
        &data_dir,
        &[
            "remember",
            "Use cargo nextest for integration tests",
            "--type",
            "convention",
            "--importance",
            "0.8",
            "--scope",
            "repo:/workspace/graphmem",
        ],
    ));
    let id = remembered
        .trim()
        .strip_prefix("remembered: ")
        .expect("remember output contains an ID")
        .to_owned();

    let shown = stdout(run(&data_dir, &["show", &id]));
    assert!(shown.contains("type: convention"));
    assert!(shown.contains("repo:/workspace/graphmem"));
    assert!(shown.contains("Use cargo nextest"));

    let listed = stdout(run(
        &data_dir,
        &["list", "--scope", "repo:/workspace/graphmem"],
    ));
    assert!(listed.contains(&id));

    let searched = stdout(run(&data_dir, &["search", "nextest"]));
    assert!(searched.contains(&id));
    assert!(
        searched
            .split('\n')
            .next()
            .and_then(|line| line.split('\t').next())
            .and_then(|score| score.parse::<f64>().ok())
            .is_some()
    );

    let scopes = stdout(run(&data_dir, &["scopes"]));
    assert!(scopes.contains("repo:/workspace/graphmem"));

    let doctor = stdout(run(&data_dir, &["doctor"]));
    assert!(doctor.contains("status: healthy"));

    let forgotten = stdout(run(&data_dir, &["forget", &id]));
    assert_eq!(forgotten, format!("forgot: {id}\n"));

    let missing = run(&data_dir, &["show", &id]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("memory not found"));

    let invalid = run(&data_dir, &["show", "invalid"]);
    assert!(!invalid.status.success());

    fs::remove_dir_all(data_dir).expect("test data directory is removed");
}
