//! Development tools diagnostics.
//!
//! Checks whether each requested tool is on PATH and, if so, parses its
//! reported version. A missing tool becomes a single failed diagnostic;
//! it never breaks the rest of the report. If the config sets a version
//! requirement for a tool, an installed version that does not satisfy it
//! fails too.

use std::sync::Arc;
use std::time::Duration;

use crate::command::{CommandFailure, SharedRunner};
use crate::config::Config;
use crate::diagnostics::util::extract_leading_version;
use crate::models::{Diagnostic, DiagnosticGroup};
use crate::version::VersionReq;

/// Probe argument sets tried in order. Some tools accept `--version`,
/// some `-version`, some `-V`; `version` covers the rest.
const PROBES: &[&[&str]] = &[&["--version"], &["-version"], &["-V"], &["version"]];

/// Collect diagnostics for every configured tool, in parallel.
pub async fn collect(runner: SharedRunner, config: Arc<Config>) -> DiagnosticGroup {
    let timeout = config.timeout();
    let tools = config.resolved_tools();

    let handles: Vec<_> = tools
        .iter()
        .map(|tool| {
            let runner = runner.clone();
            let tool = tool.clone();
            let req = config.tools.versions.get(&tool).cloned();
            tokio::spawn(async move { detect_tool(&runner, &tool, req.as_ref(), timeout).await })
        })
        .collect();

    let mut group = DiagnosticGroup::new("Tools");
    for handle in handles {
        match handle.await {
            Ok(diag) => group.push(diag),
            Err(_) => continue, // task panicked; skip rather than crash
        }
    }
    group
}

async fn detect_tool(
    runner: &SharedRunner,
    tool: &str,
    req: Option<&VersionReq>,
    timeout: Duration,
) -> Diagnostic {
    for args in PROBES {
        match runner.run(tool, args, timeout).await {
            Ok(out) => {
                let version = parse_version(tool, &out.combined_output());
                return check_version(tool, version, req);
            }
            Err(CommandFailure::NotFound) => {
                return match req {
                    Some(req) => Diagnostic::failed(tool, format!("not found (requires {req})")),
                    None => Diagnostic::failed(tool, "not found"),
                };
            }
            Err(_) => continue, // try the next probe variant
        }
    }
    match req {
        Some(req) => Diagnostic::warning(
            tool,
            format!("installed but version probe failed (requires {req})"),
        ),
        None => Diagnostic::warning(tool, "installed but version probe failed"),
    }
}

fn check_version(tool: &str, version: Option<String>, req: Option<&VersionReq>) -> Diagnostic {
    match (version, req) {
        (Some(v), Some(req)) => match req.matches(&v) {
            Some(true) => Diagnostic::healthy(tool, v),
            Some(false) => Diagnostic::failed(tool, format!("{v} (requires {req})")),
            None => Diagnostic::warning(tool, format!("{v} (could not compare with {req})")),
        },
        (Some(v), None) => Diagnostic::healthy(tool, v),
        (None, Some(req)) => {
            Diagnostic::warning(tool, format!("installed, version unknown (requires {req})"))
        }
        (None, None) => Diagnostic::healthy(tool, "installed (version unknown)"),
    }
}

/// Two-pass strategy: if the line starts with the tool name, look for a
/// version in any token; otherwise fall back to scanning every token.
fn parse_version(tool: &str, output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("usage") || line.starts_with("Usage") {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        // "git version 2.51.0", "kubectl version v1.34.0",
        // "aws-cli/2.31.0 Python/3.12.0", "go version go1.22.3"
        let lower_tool = tool.to_lowercase();
        let lower_first = tokens[0].to_lowercase();
        if lower_first == lower_tool || lower_first.starts_with(&lower_tool) {
            for tok in &tokens {
                if let Some(v) = extract_leading_version(tok) {
                    return Some(v);
                }
            }
        }

        for tok in &tokens {
            if let Some(v) = extract_leading_version(tok) {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeRunner;

    #[test]
    fn parse_version_finds_named_version() {
        assert_eq!(
            parse_version("git", "git version 2.51.0\n").as_deref(),
            Some("2.51.0")
        );
    }

    #[test]
    fn parse_version_handles_v_prefix() {
        assert_eq!(
            parse_version("go", "go version go1.22.3 darwin/arm64").as_deref(),
            Some("1.22.3")
        );
    }

    #[test]
    fn parse_version_finds_first_version_like_token() {
        assert_eq!(
            parse_version("aws", "aws-cli/2.31.0 Python/3.12.0").as_deref(),
            Some("2.31.0")
        );
    }

    #[test]
    fn parse_version_returns_none_for_garbage() {
        assert!(parse_version("foo", "no version here").is_none());
    }

    #[tokio::test]
    async fn detect_tool_does_not_panic_for_missing_binary() {
        let runner: SharedRunner = Arc::new(FakeRunner::new());
        let _ = detect_tool(
            &runner,
            "definitely-missing-xyz",
            None,
            Duration::from_secs(1),
        )
        .await;
    }

    async fn detect_with(output: &str, req: &str) -> Diagnostic {
        let fake = FakeRunner::new();
        fake.program_response(
            "node",
            &["--version"],
            Ok(crate::command::CommandOutput {
                stdout: output.into(),
                stderr: String::new(),
            }),
        )
        .await;
        let runner: SharedRunner = Arc::new(fake);
        let req = VersionReq::parse(req).expect("req");
        detect_tool(&runner, "node", Some(&req), Duration::from_secs(1)).await
    }

    #[tokio::test]
    async fn satisfied_requirement_is_healthy() {
        let d = detect_with("v22.1.0\n", ">=20").await;
        assert_eq!(d.status, crate::models::Status::Healthy);
        assert_eq!(d.message, "22.1.0");
    }

    #[tokio::test]
    async fn unmet_requirement_fails_with_reason() {
        let d = detect_with("v18.19.1\n", ">=20").await;
        assert_eq!(d.status, crate::models::Status::Failed);
        assert_eq!(d.message, "18.19.1 (requires >=20)");
    }

    #[tokio::test]
    async fn unknown_version_with_requirement_warns() {
        let d = detect_with("no version here\n", ">=20").await;
        assert_eq!(d.status, crate::models::Status::Warning);
    }

    #[tokio::test]
    async fn missing_tool_with_requirement_fails() {
        let runner: SharedRunner = Arc::new(FakeRunner::new());
        let req = VersionReq::parse(">=20").expect("req");
        let d = detect_tool(&runner, "node", Some(&req), Duration::from_secs(1)).await;
        assert_eq!(d.status, crate::models::Status::Failed);
    }
}
