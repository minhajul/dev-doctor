//! End-to-end tests that run the real `devdoctor` binary.
//!
//! Each test gets its own temp directory used as both `HOME` and the working
//! directory, so neither the developer's user config nor a `devdoctor.toml`
//! further up the tree leaks in. Tests stick to categories that don't depend
//! on the host (env, tools with made-up names, ports on loopback), so they
//! pass without Docker, Kubernetes or AWS installed.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, contents).expect("write");
        path
    }

    /// Write the user config at `$HOME/.config/devdoctor/config.toml`.
    fn user_config(&self, contents: &str) {
        self.write(".config/devdoctor/config.toml", contents);
    }

    /// Write the project config at the sandbox root.
    fn project_config(&self, contents: &str) {
        self.write("devdoctor.toml", contents);
    }

    fn cmd(&self) -> Command {
        self.cmd_in(self.path())
    }

    fn cmd_in(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_devdoctor"));
        cmd.current_dir(cwd)
            .env("HOME", self.path())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("NO_COLOR");
        cmd
    }
}

fn run(cmd: &mut Command) -> Output {
    cmd.output().expect("run devdoctor")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf8 stdout")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf8 stderr")
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("invalid JSON ({e}):\n{}", stdout(out)))
}

/// The diagnostics of the only group in a single-category JSON report.
fn diagnostics(report: &Value) -> &Vec<Value> {
    let groups = report["groups"].as_array().expect("groups");
    assert_eq!(groups.len(), 1, "expected one group: {report}");
    groups[0]["diagnostics"].as_array().expect("diagnostics")
}

fn find<'a>(diags: &'a [Value], name: &str) -> &'a Value {
    diags
        .iter()
        .find(|d| d["name"] == name)
        .unwrap_or_else(|| panic!("no diagnostic named {name}: {diags:?}"))
}

/// A loopback port nothing is listening on (bound, then released).
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

#[test]
fn version_subcommand_prints_package_version() {
    let sb = Sandbox::new();
    let out = run(sb.cmd().arg("version"));
    assert!(out.status.success());
    assert_eq!(
        stdout(&out).trim(),
        format!("devdoctor {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn invalid_usage_exits_two() {
    let sb = Sandbox::new();
    let out = run(sb.cmd().args(["check", "--category", "bogus"]));
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn env_report_covers_statuses_hints_and_never_prints_values() {
    let sb = Sandbox::new();
    sb.project_config(
        r#"
[env]
required = ["DD_SET", "DD_EMPTY", "DD_MISSING"]

[env.hints]
DD_MISSING = "cp .env.example .env"
"#,
    );
    let out = run(sb
        .cmd()
        .args(["check", "--category", "env", "--json"])
        .env("DD_SET", "super-secret-value")
        .env("DD_EMPTY", "")
        .env_remove("DD_MISSING"));

    assert_eq!(out.status.code(), Some(1), "a failure exits 1");
    assert!(!stdout(&out).contains("super-secret-value"));

    let report = json(&out);
    let diags = diagnostics(&report);
    assert_eq!(find(diags, "DD_SET")["status"], "healthy");
    assert!(find(diags, "DD_SET").get("hint").is_none());
    assert_eq!(find(diags, "DD_EMPTY")["status"], "warning");
    assert!(find(diags, "DD_EMPTY")["hint"].is_string(), "built-in hint");
    assert_eq!(find(diags, "DD_MISSING")["status"], "failed");
    assert_eq!(find(diags, "DD_MISSING")["hint"], "cp .env.example .env");
    assert_eq!(
        report["summary"],
        serde_json::json!({ "healthy": 1, "warnings": 1, "failed": 1 })
    );
}

#[test]
fn warnings_alone_exit_zero() {
    let sb = Sandbox::new();
    sb.project_config("[env]\nrequired = [\"DD_EMPTY\"]\n");
    let out = run(sb.cmd().arg("env").env("DD_EMPTY", ""));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn project_config_is_found_from_a_subdirectory() {
    let sb = Sandbox::new();
    sb.project_config("[env]\nrequired = [\"DD_MISSING\"]\n");
    let nested = sb.path().join("src/deep");
    std::fs::create_dir_all(&nested).expect("mkdir");

    let out = run(sb
        .cmd_in(&nested)
        .args(["check", "--category", "env", "--json"])
        .env_remove("DD_MISSING"));
    assert_eq!(
        find(diagnostics(&json(&out)), "DD_MISSING")["status"],
        "failed"
    );
}

#[test]
fn project_config_merges_over_user_config() {
    let sb = Sandbox::new();
    sb.user_config("[env]\nrequired = [\"DD_MISSING\"]\n");
    sb.project_config("[env.hints]\nDD_MISSING = \"from the project\"\n");

    let out = run(sb
        .cmd()
        .args(["check", "--category", "env", "--json"])
        .env_remove("DD_MISSING"));
    let report = json(&out);
    let d = find(diagnostics(&report), "DD_MISSING");
    assert_eq!(
        d["status"], "failed",
        "required list comes from user config"
    );
    assert_eq!(
        d["hint"], "from the project",
        "hint comes from project config"
    );
}

#[test]
fn malformed_project_config_exits_two_and_names_the_file() {
    let sb = Sandbox::new();
    sb.project_config("timeout_seconds = \"soon\"\n");
    let out = run(sb.cmd().arg("env"));
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("devdoctor.toml"), "{}", stderr(&out));
}

#[test]
fn missing_tool_with_requirement_fails_with_hint() {
    let sb = Sandbox::new();
    sb.project_config(
        r#"
[tools]
enabled = []

[tools.versions]
devdoctor-no-such-tool = ">=1"
"#,
    );
    let out = run(sb.cmd().args(["check", "--category", "tools", "--json"]));
    assert_eq!(out.status.code(), Some(1));

    let report = json(&out);
    let diags = diagnostics(&report);
    assert_eq!(diags.len(), 1, "only the required tool is checked");
    let d = &diags[0];
    assert_eq!(d["status"], "failed");
    assert_eq!(d["message"], "not found (requires >=1)");
    assert!(d["hint"].is_string());
}

#[test]
fn port_expectations_decide_status() {
    let sb = Sandbox::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let busy = listener.local_addr().expect("addr").port();
    let free = free_port();
    sb.project_config(&format!(
        "[ports]\nexpect_listening = [{busy}, {free}]\n[ports.hints]\n{free} = \"start it\"\n"
    ));

    let out = run(sb.cmd().args(["check", "--category", "ports", "--json"]));
    let report = json(&out);
    let diags = diagnostics(&report);
    assert_eq!(diags.len(), 2, "defaults are not probed: {diags:?}");
    assert_eq!(find(diags, &format!(":{busy}"))["status"], "healthy");
    let missing = find(diags, &format!(":{free}"));
    assert_eq!(missing["status"], "failed");
    assert_eq!(missing["hint"], "start it");
    drop(listener);
}

#[test]
fn text_output_renders_hints_without_color_codes() {
    let sb = Sandbox::new();
    sb.project_config("[env]\nrequired = [\"DD_MISSING\"]\n");
    let out = run(sb
        .cmd()
        .args(["--no-color", "env"])
        .env_remove("DD_MISSING"));
    let text = stdout(&out);

    assert!(!text.contains('\x1b'), "no ANSI codes: {text:?}");
    assert!(
        text.contains("Environment\n✗ DD_MISSING  not set\n"),
        "{text}"
    );
    assert!(
        text.contains("\n              → export DD_MISSING="),
        "{text}"
    );
    assert!(
        text.contains("Healthy: 0    Warnings: 0    Failed: 1"),
        "{text}"
    );
}

#[test]
fn json_output_never_contains_color_codes() {
    let sb = Sandbox::new();
    sb.project_config("[env]\nrequired = [\"DD_MISSING\"]\n");
    // No --no-color and NO_COLOR unset: JSON must still be plain.
    let out = run(sb
        .cmd()
        .args(["check", "--category", "env", "--json"])
        .env_remove("DD_MISSING"));
    assert!(!stdout(&out).contains('\x1b'));
}
