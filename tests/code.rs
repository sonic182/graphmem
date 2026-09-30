#![cfg(feature = "code")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    repo: PathBuf,
}

impl Sandbox {
    /// A Git repository holding a copy of `tests/fixtures/code`.
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "graphmem-code-{name}-{}-{nonce}",
            std::process::id()
        ));
        let home = root.join("home");
        let repo = root.join("repo");
        fs::create_dir_all(&home).expect("test home is created");
        fs::write(home.join("config.toml"), "[embedding]\nenabled = false\n")
            .expect("test embeddings are disabled");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/code"),
            &repo,
        );
        git(&repo, &["init", "-q"]);
        Self { root, home, repo }
    }

    fn gmem(&self, directory: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gmem"))
            .args(args)
            .env("GRAPHMEM_HOME", &self.home)
            .env_remove("GRAPHMEM_CODE")
            .current_dir(directory)
            .output()
            .expect("gmem runs")
    }

    /// Runs gmem in the repository and returns stdout, failing on error.
    fn run(&self, args: &[&str]) -> String {
        let output = self.gmem(&self.repo, args);
        assert!(
            output.status.success(),
            "gmem {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("stdout is UTF-8")
    }

    fn fail(&self, args: &[&str]) -> String {
        let output = self.gmem(&self.repo, args);
        assert!(
            !output.status.success(),
            "gmem {args:?} unexpectedly succeeded"
        );
        String::from_utf8(output.stderr).expect("stderr is UTF-8")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("fixture directory is created");
    for entry in fs::read_dir(from).expect("fixture directory is readable") {
        let entry = entry.expect("fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("fixture file is copied");
        }
    }
}

fn git(directory: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(directory)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn outlines_every_supported_language_with_nesting_and_ranges() {
    let sandbox = Sandbox::new("languages");
    let indexed = sandbox.run(&["code", "index"]);
    assert!(indexed.contains("18 indexed"), "{indexed}");

    let expectations: &[(&str, &[&str])] = &[
        (
            "native/lib.rs",
            &[
                "\trust\tcomplete",
                "1-1\timport std::fmt",
                "3-7\tmodule billing",
                "4-6\t  function charge",
                "9-11\tstruct Invoice",
                "13-17\timpl Invoice",
                "14-16\t  method new",
                "20-22\t  method fmt",
                "25-31\tinterface Priced",
                "28-30\t  method discounted",
                "34-34\t  enumMember Draft",
            ],
        ),
        (
            "native/main.go",
            &[
                "\tgo\tcomplete",
                "5-7\tstruct Server",
                "6-6\t  field Addr",
                "9-11\tmethod Start",
                "14-14\t  method Serve",
                "17-19\tfunction main",
            ],
        ),
        (
            "native/util.c",
            &[
                "\tc\tcomplete",
                "3-6\tstruct point",
                "8-8\tenum color",
                "10-12\tfunction add",
            ],
        ),
        (
            "native/widget.h",
            &[
                "\tcpp\tcomplete",
                "5-12\tclass Widget",
                "7-7\t  constructor Widget",
                "8-8\t  method name",
            ],
        ),
        (
            "native/widget.cpp",
            &["3-9\tnamespace ui", "5-7\t  function render"],
        ),
        (
            "native/build.zig",
            &[
                "\tzig\tcomplete",
                "1-1\timport std",
                "3-10\tstruct Point",
                "4-4\t  field x",
                "7-9\t  function sum",
                "12-12\tenum Mode",
                "16-18\tfunction main",
                "20-23\ttest point sum",
            ],
        ),
        (
            "lib/demo_web/components/core_components.ex",
            &[
                "\telixir\tcomplete",
                "1-40\tmodule DemoWeb.CoreComponents",
                "2-2\t  import Phoenix.Component",
                "8-15\t  function flash/1",
                "11-11\t    component .icon",
                // Two clauses, one symbol spanning both.
                "17-23\t  function button/1",
                "21-21\t    component .icon",
                "25-25\t  function icon_class/1",
                "27-31\t  macro __using__/1",
                "33-39\t  module DemoWeb.CoreComponents.Helpers",
                "34-36\t    function hide/2",
                "38-38\t    function version/0",
            ],
        ),
        (
            "lib/demo_web/controllers/page_html/home.html.heex",
            &[
                "\theex\tcomplete",
                "1-8\tcomponent Layouts.app",
                "2-5\t  component .header",
                "4-4\t    slot :subtitle",
                "6-6\t  component .button",
            ],
        ),
        (
            "lib/demo_web/templates/page/index.html.eex",
            &[
                "\teex\tpartial: EEx directives only",
                "3-3\texpression for user <- @users do",
                "4-4\texpression link user.name",
            ],
        ),
        (
            "app/service.py",
            &[
                "1-1\timport os",
                "4-6\tclass BillingService",
                "5-6\t  method charge",
                "9-10\tfunction load_config",
            ],
        ),
        (
            "web/cart.jsx",
            &[
                "3-5\tfunction Cart",
                "7-11\tclass CartStore",
                "8-10\t  method add",
                "13-18\tobject default",
                "14-16\t  method mounted",
                "17-17\t  method updated",
            ],
        ),
        (
            "web/api.ts",
            &["1-3\tinterface Order", "5-7\tfunction fetchOrder"],
        ),
        (
            "web/badge.tsx",
            &["3-3\tconstant Badge", "5-7\tfunction Badges"],
        ),
        (
            "app/models/invoice.rb",
            &[
                "1-7\tmodule Billing",
                "2-6\t  class Invoice",
                "3-5\t    method total",
            ],
        ),
        (
            "src/Report.php",
            &[
                "5-11\tclass Report",
                "7-10\t  method render",
                "13-16\tfunction build_report",
            ],
        ),
        (
            "db/schema.sql",
            &[
                "\tsql\tcomplete",
                "1-4\ttable accounts",
                "6-6\tindex accounts_email_idx",
                "8-8\tview active_accounts",
                "10-12\tfunction account_count",
            ],
        ),
        (
            "scripts/deploy.sh",
            &["4-6\tfunction build", "8-10\tfunction deploy"],
        ),
        ("public/index.html", &["6-8\tfunction refreshStatus"]),
    ];
    for (path, lines) in expectations {
        let outline = sandbox.run(&["code", "outline", path]);
        for line in *lines {
            assert!(
                outline.contains(line),
                "{path}: missing {line:?} in\n{outline}"
            );
        }
    }
    let heex = sandbox.run(&[
        "code",
        "outline",
        "lib/demo_web/controllers/page_html/home.html.heex",
    ]);
    assert!(!heex.contains(" p"), "plain tags are not symbols:\n{heex}");
}

#[test]
fn nested_definitions_without_bundled_members_are_outlined_and_found() {
    let sandbox = Sandbox::new("nested-definitions");
    let cases = [
        (
            "app/factory.py",
            "def factory():\n    def helper():\n        pass\n    return helper\n",
            "2-3\t  function helper",
            "helper",
            "app/factory.py:2:5\tfunction\thelper\tfactory\tfresh\n",
        ),
        (
            "native/constants.rs",
            "mod settings {\n    const RETRIES: u8 = 3;\n}\n",
            "2-2\t  constant RETRIES",
            "RETRIES",
            "native/constants.rs:2:5\tconstant\tRETRIES\tsettings\tfresh\n",
        ),
    ];
    for (path, source, symbol, query, expected) in cases {
        fs::write(sandbox.repo.join(path), source).expect("nested source is written");
        let outline = sandbox.run(&["code", "outline", path]);
        assert!(outline.contains("\tcomplete\n"), "{outline}");
        assert!(
            outline.contains(symbol),
            "{path}: missing {symbol} in {outline}"
        );
        assert_eq!(sandbox.run(&["code", "find", query]), expected);
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_filenames_do_not_abort_checkout_discovery() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let sandbox = Sandbox::new("non-utf8-filenames");
    fs::write(sandbox.repo.join(OsStr::from_bytes(b"bad\xff.txt")), "text")
        .expect("tracked non-UTF-8 filename is written");
    git(&sandbox.repo, &["add", "."]);
    fs::write(
        sandbox.repo.join(OsStr::from_bytes(b"bad\xfe.py")),
        "def skipped():\n    pass\n",
    )
    .expect("untracked non-UTF-8 filename is written");
    let indexed = sandbox.run(&["code", "index"]);
    assert!(indexed.contains("18 indexed"), "{indexed}");
    assert_eq!(
        sandbox.run(&["code", "find", "load_config"]),
        "app/service.py:9:1\tfunction\tload_config\t\tfresh\n"
    );
}

#[test]
fn find_refreshes_the_index_and_matches_definitions_by_last_segment() {
    let sandbox = Sandbox::new("refresh");
    fs::write(sandbox.repo.join(".gitignore"), "ignored.py\n").expect("gitignore is written");
    fs::write(sandbox.repo.join("ignored.py"), "def hidden():\n    pass\n")
        .expect("ignored file is written");
    fs::write(
        sandbox.repo.join("app/latin1.py"),
        b"def caf\xe9():\n    pass\n",
    )
    .expect("Latin-1 file is written");
    assert_eq!(sandbox.run(&["code", "find", "hidden"]), "");
    assert_eq!(
        sandbox.run(&["code", "find", "load_config"]),
        "app/service.py:9:1\tfunction\tload_config\t\tfresh\n"
    );
    assert!(
        sandbox
            .fail(&["code", "outline", "app/latin1.py"])
            .contains("not valid UTF-8")
    );

    let service = sandbox.repo.join("app/service.py");
    let mut source = fs::read_to_string(&service).expect("service is readable");
    source.insert_str(0, "\n\n");
    fs::write(&service, source).expect("service is edited");
    assert_eq!(
        sandbox.run(&["code", "find", "load_config"]),
        "app/service.py:11:1\tfunction\tload_config\t\tfresh\n"
    );

    fs::remove_file(sandbox.repo.join("scripts/deploy.sh")).expect("script is deleted");
    assert_eq!(sandbox.run(&["code", "find", "deploy"]), "");
    let reindexed = sandbox.run(&["code", "index"]);
    assert!(
        reindexed.contains("0 indexed, 17 unchanged, 0 removed, 1 skipped"),
        "{reindexed}"
    );

    assert_eq!(
        sandbox.run(&["code", "find", "Helpers"]),
        "lib/demo_web/components/core_components.ex:33:3\tmodule\t\
         DemoWeb.CoreComponents.Helpers\tDemoWeb.CoreComponents\tfresh\n"
    );
    let icons = sandbox.run(&["code", "find", "icon"]);
    assert!(
        icons
            .starts_with("lib/demo_web/components/core_components.ex:25:3\tfunction\ticon_class/1"),
        "{icons}"
    );
    assert!(!icons.contains("\tcomponent\t"), "{icons}");
    assert_eq!(
        sandbox
            .run(&["code", "find", ".icon", "--kind", "component"])
            .lines()
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn refuses_paths_that_leave_the_checkout() {
    let sandbox = Sandbox::new("paths");
    let outside = sandbox.root.join("outside.py");
    fs::write(&outside, "def secret():\n    pass\n").expect("outside file is written");
    std::os::unix::fs::symlink(&outside, sandbox.repo.join("link.py")).expect("symlink");

    assert!(
        sandbox
            .fail(&["code", "outline", "../outside.py"])
            .contains("`..`")
    );
    assert!(
        sandbox
            .fail(&["code", "outline", outside.to_str().unwrap()])
            .contains("outside the checkout")
    );
    assert!(
        sandbox
            .fail(&["code", "outline", "link.py"])
            .contains("symlinks")
    );

    let vendor = sandbox.repo.join("vendor");
    fs::create_dir_all(&vendor).expect("vendor directory is created");
    fs::write(vendor.join("lib.py"), "def vendored():\n    pass\n").expect("vendor file");
    git(&sandbox.repo, &["add", "vendor"]);
    fs::remove_dir_all(&vendor).expect("vendor directory is removed");
    let outside_dir = sandbox.root.join("outside_dir");
    fs::create_dir_all(&outside_dir).expect("outside directory is created");
    fs::write(outside_dir.join("lib.py"), "def secret_dir():\n    pass\n")
        .expect("outside directory file is written");
    std::os::unix::fs::symlink(&outside_dir, &vendor).expect("directory symlink");

    sandbox.run(&["code", "index"]);
    assert_eq!(sandbox.run(&["code", "find", "secret"]), "");
}

#[test]
fn indexes_each_linked_worktree_separately() {
    let sandbox = Sandbox::new("worktrees");
    git(&sandbox.repo, &["add", "."]);
    git(&sandbox.repo, &["commit", "-q", "-m", "fixtures"]);
    let worktree = sandbox.root.join("feature");
    git(
        &sandbox.repo,
        &["worktree", "add", "-q", worktree.to_str().unwrap()],
    );
    fs::write(
        worktree.join("app/feature.py"),
        "def only_in_feature():\n    pass\n",
    )
    .expect("worktree file is written");

    sandbox.run(&["code", "index"]);
    let output = sandbox.gmem(&worktree, &["code", "index"]);
    assert!(output.status.success());

    assert_eq!(sandbox.run(&["code", "find", "only_in_feature"]), "");
    let found = sandbox.gmem(&worktree, &["code", "find", "only_in_feature"]);
    assert_eq!(
        String::from_utf8_lossy(&found.stdout),
        "app/feature.py:1:1\tfunction\tonly_in_feature\t\tfresh\n"
    );
}

#[test]
fn config_can_disable_the_code_tools() {
    let sandbox = Sandbox::new("disabled");
    fs::write(
        sandbox.home.join("config.toml"),
        "[embedding]\nenabled = false\n[code]\nenabled = false\n",
    )
    .expect("config is written");
    assert!(
        sandbox
            .fail(&["code", "index"])
            .contains("code tools are disabled")
    );

    fs::write(
        sandbox.home.join("config.toml"),
        "[embedding]\nenabled = false\n[code]\nmax_files = 1\nindex_threads = 2\n",
    )
    .expect("config is written");
    let output = sandbox.gmem(&sandbox.repo, &["code", "index"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 indexed"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("too many source files"));

    fs::write(
        sandbox.home.join("config.toml"),
        "[embedding]\nenabled = false\n[code]\nindex_threads = 0\n",
    )
    .expect("config is written");
    assert!(
        sandbox
            .fail(&["code", "index"])
            .contains("\"auto\" or a positive integer")
    );
}
