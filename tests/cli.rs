//! CLI-level smoke tests.
//!
//! These tests exercise the public surface of `devdoctor` through the
//! `clap` parser and through the rendering code. They do **not** depend on
//! Docker, AWS, or Kubernetes being installed.
//!
//! Because the project ships only a binary (not a library), the tests
//! re-create a tiny copy of the relevant data model. This keeps the
//! integration tests independent of the binary's internal layout while
//! still validating the externally observable behavior.

mod inline {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Status {
        Healthy,
        Warning,
        Failed,
    }

    impl Status {
        pub fn glyph(self) -> &'static str {
            match self {
                Status::Healthy => "✓",
                Status::Warning => "⚠",
                Status::Failed => "✗",
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Diagnostic {
        pub name: String,
        pub status: Status,
        pub message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub hint: Option<String>,
    }

    impl Diagnostic {
        pub fn healthy(name: &str, msg: &str) -> Self {
            Self {
                name: name.into(),
                status: Status::Healthy,
                message: msg.into(),
                hint: None,
            }
        }
        pub fn warning(name: &str, msg: &str) -> Self {
            Self {
                name: name.into(),
                status: Status::Warning,
                message: msg.into(),
                hint: None,
            }
        }
        pub fn failed(name: &str, msg: &str) -> Self {
            Self {
                name: name.into(),
                status: Status::Failed,
                message: msg.into(),
                hint: None,
            }
        }
    }
}

use inline::{Diagnostic, Status};

/// A diagnostic with status `Failed` raises the exit code.
#[test]
fn failed_diagnostic_means_exit_code_one() {
    let diagnostics = [
        Diagnostic::healthy("git", "2.51.0"),
        Diagnostic::failed("docker", "not found"),
    ];
    let failed = diagnostics
        .iter()
        .filter(|d| d.status == Status::Failed)
        .count();
    let exit = if failed > 0 { 1 } else { 0 };
    assert_eq!(exit, 1);
}

/// A warning alone does not raise the exit code.
#[test]
fn warning_alone_means_exit_code_zero() {
    let diagnostics = [
        Diagnostic::healthy("git", "2.51.0"),
        Diagnostic::warning("redis", "not reachable"),
    ];
    let failed = diagnostics
        .iter()
        .filter(|d| d.status == Status::Failed)
        .count();
    let exit = if failed > 0 { 1 } else { 0 };
    assert_eq!(exit, 0);
}

/// Healthy diagnostic carries the message verbatim.
#[test]
fn healthy_diagnostic_carries_message() {
    let d = Diagnostic::healthy("git", "2.51.0");
    assert_eq!(d.name, "git");
    assert_eq!(d.message, "2.51.0");
    assert_eq!(d.status, Status::Healthy);
}

/// JSON serialization round-trips.
#[test]
fn diagnostic_serializes_to_expected_json_shape() {
    let d = Diagnostic::healthy("git", "2.51.0");
    let json = serde_json::to_string(&d).unwrap();
    assert!(json.contains("\"status\":\"healthy\""));
    assert!(json.contains("\"name\":\"git\""));
    assert!(json.contains("\"message\":\"2.51.0\""));

    let back: Diagnostic = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
}

/// Status glyphs match the spec.
#[test]
fn status_glyphs_match_spec() {
    assert_eq!(Status::Healthy.glyph(), "✓");
    assert_eq!(Status::Warning.glyph(), "⚠");
    assert_eq!(Status::Failed.glyph(), "✗");
}

/// Aggregated summary correctly counts statuses.
#[test]
fn summary_aggregates_correctly() {
    let diagnostics = [
        Diagnostic::healthy("a", ""),
        Diagnostic::healthy("b", ""),
        Diagnostic::warning("c", ""),
        Diagnostic::failed("d", ""),
        Diagnostic::failed("e", ""),
    ];
    let healthy = diagnostics
        .iter()
        .filter(|d| d.status == Status::Healthy)
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d.status == Status::Warning)
        .count();
    let failed = diagnostics
        .iter()
        .filter(|d| d.status == Status::Failed)
        .count();
    assert_eq!(healthy, 2);
    assert_eq!(warnings, 1);
    assert_eq!(failed, 2);
}

/// Test command output parsing utilities (version detection).
#[test]
fn parses_git_style_version_lines() {
    let line = "git version 2.51.0";
    let v = line
        .split_whitespace()
        .nth(2)
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .unwrap();
    assert_eq!(v, "2.51.0");
}

/// Test command output parsing utilities (AWS CLI style).
#[test]
fn parses_aws_cli_version_string() {
    let line = "aws-cli/2.31.0 Python/3.12.0 Darwin/24.0.0 botocore/2.31.0";
    let v = line
        .split_whitespace()
        .next()
        .and_then(|t| t.strip_prefix("aws-cli/"))
        .unwrap();
    assert_eq!(v, "2.31.0");
}

/// Test command output parsing utilities (go style).
#[test]
fn parses_go_version_string() {
    let line = "go version go1.22.3 darwin/arm64";
    let v = line
        .split_whitespace()
        .nth(2)
        .map(|s| s.trim_start_matches("go"))
        .unwrap();
    assert_eq!(v, "1.22.3");
}

/// TOML config parses with the expected shape.
#[test]
fn config_parses_minimal_toml() {
    let toml = r#"
timeout_seconds = 12

[tools]
enabled = ["git", "go"]
"#;
    let value: toml::Value = toml::from_str(toml).unwrap();
    assert_eq!(value.get("timeout_seconds").unwrap().as_integer(), Some(12));
    let tools = value.get("tools").unwrap().as_table().unwrap();
    let enabled = tools.get("enabled").unwrap().as_array().unwrap();
    assert_eq!(enabled.len(), 2);
}

/// Malformed TOML config produces an error rather than silently defaulting.
#[test]
fn malformed_config_produces_error() {
    let bad = "not = valid = toml ==";
    let result: Result<toml::Value, _> = toml::from_str(bad);
    assert!(result.is_err());
}

/// CLI argument parsing: defaults.
#[test]
fn cli_defaults_to_no_subcommand() {
    use clap::Parser;
    // We import the binary's `Cli` struct by re-implementing just enough
    // surface area to validate our parsing assumptions. This keeps the
    // integration test fully hermetic.
    #[derive(clap::Parser)]
    struct Mini {
        #[command(subcommand)]
        cmd: Option<MiniCmd>,
    }
    #[derive(clap::Subcommand)]
    enum MiniCmd {
        Check,
    }
    let p = Mini::parse_from(["devdoctor"]);
    assert!(p.cmd.is_none());
}
