//! Development tools diagnostics.
//!
//! Checks whether each requested tool is on PATH and, if so, parses its
//! reported version. A missing tool becomes a single failed diagnostic;
//! it never breaks the rest of the report.

use std::sync::Arc;
use std::time::Duration;

use crate::command::{CommandFailure, SharedRunner};
use crate::config::Config;
use crate::diagnostics::util::extract_leading_version;
use crate::models::{Diagnostic, DiagnosticGroup};

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
            tokio::spawn(async move { detect_tool(&runner, &tool, timeout).await })
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

async fn detect_tool(runner: &SharedRunner, tool: &str, timeout: Duration) -> Diagnostic {
    for args in PROBES {
        match runner.run(tool, args, timeout).await {
            Ok(out) => {
                let version = parse_version(tool, &out.combined_output());
                let msg = version.unwrap_or_else(|| "installed (version unknown)".to_string());
                return Diagnostic::healthy(tool, msg);
            }
            Err(CommandFailure::NotFound) => return Diagnostic::failed(tool, "not found"),
            Err(_) => continue, // try the next probe variant
        }
    }
    Diagnostic::warning(tool, "installed but version probe failed")
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
        let _ = detect_tool(&runner, "definitely-missing-xyz", Duration::from_secs(1)).await;
    }
}
