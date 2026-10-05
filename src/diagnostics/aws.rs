//! AWS diagnostics. Read-only. Never prints secret material.

use std::sync::Arc;
use std::time::Duration;

use tokio::join;

use crate::command::{CommandFailure, SharedRunner};
use crate::config::Config;
use crate::models::{Diagnostic, DiagnosticGroup};

pub async fn collect(runner: SharedRunner, config: Arc<Config>) -> DiagnosticGroup {
    let timeout = config.timeout();

    if which::which("aws").is_err() {
        let mut group = DiagnosticGroup::new("AWS");
        group.push(Diagnostic::failed("AWS CLI", "not found"));
        return group;
    }

    let (cli, region, sts) = join!(
        probe_cli(&runner, timeout),
        probe_region(&runner, timeout),
        probe_sts(&runner, timeout),
    );

    let mut group = DiagnosticGroup::new("AWS");
    group.push(cli);
    group.push(region);
    group.push(credentials_diagnostic(&sts));
    group.push(caller_diagnostic(&sts));
    group
}

async fn probe_cli(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner.run("aws", &["--version"], timeout).await {
        Ok(out) => {
            // `aws --version` writes to stderr; combined_output covers both.
            let v = extract_aws_version(&out.combined_output())
                .unwrap_or_else(|| "installed".to_string());
            Diagnostic::healthy("AWS CLI", v)
        }
        Err(e) => Diagnostic::failed("AWS CLI", e.to_string()),
    }
}

async fn probe_region(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner
        .run("aws", &["configure", "get", "region"], timeout)
        .await
    {
        Ok(out) => match out.stdout.trim() {
            "" => Diagnostic::warning("Region", "not configured"),
            s => Diagnostic::healthy("Region", s.to_string()),
        },
        Err(CommandFailure::NonZeroExit { .. }) => Diagnostic::warning("Region", "not configured"),
        Err(CommandFailure::Timeout) => {
            Diagnostic::warning("Region", "configure get region timed out")
        }
        Err(e) => Diagnostic::warning("Region", e.to_string()),
    }
}

async fn probe_sts(runner: &SharedRunner, timeout: Duration) -> Result<String, CommandFailure> {
    let out = runner
        .run("aws", &["sts", "get-caller-identity"], timeout)
        .await?;
    Ok(out.stdout)
}

fn credentials_diagnostic(sts: &Result<String, CommandFailure>) -> Diagnostic {
    match sts {
        Ok(_) => Diagnostic::healthy("Credentials", "configured"),
        Err(CommandFailure::NonZeroExit { .. }) => {
            Diagnostic::warning("Credentials", "not configured or invalid")
        }
        Err(CommandFailure::Timeout) => Diagnostic::warning("Credentials", "STS check timed out"),
        Err(CommandFailure::NotFound) => Diagnostic::failed("Credentials", "aws CLI not found"),
        Err(e) => Diagnostic::warning("Credentials", e.to_string()),
    }
}

fn caller_diagnostic(sts: &Result<String, CommandFailure>) -> Diagnostic {
    match sts {
        Ok(stdout) => match parse_caller_identity(stdout) {
            Some(summary) => Diagnostic::healthy("Caller", summary),
            None => Diagnostic::healthy("Caller", "unknown"),
        },
        Err(_) => Diagnostic::healthy("Caller", "unavailable"),
    }
}

/// `aws-cli/2.31.0` -> `Some("2.31.0")`.
fn extract_aws_version(s: &str) -> Option<String> {
    for token in s.split_whitespace() {
        if let Some(rest) = token.strip_prefix("aws-cli/") {
            return Some(rest.to_string());
        }
    }
    None
}

/// Render the caller identity safely. Only Account + ARN are returned.
fn parse_caller_identity(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let account = v.get("Account").and_then(|x| x.as_str());
    let arn = v.get("Arn").and_then(|x| x.as_str());
    match (account, arn) {
        (Some(a), Some(arn)) => Some(format!("account={a} arn={arn}")),
        (Some(a), None) => Some(format!("account={a}")),
        (None, Some(arn)) => Some(format!("arn={arn}")),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_aws_version_handles_standard_output() {
        assert_eq!(
            extract_aws_version("aws-cli/2.31.0 Python/3.12.0 Darwin/24.0.0 botocore/2.31.0"),
            Some("2.31.0".into())
        );
    }

    #[test]
    fn extract_aws_version_returns_none_for_other_text() {
        assert_eq!(extract_aws_version("hello world"), None);
    }

    #[test]
    fn parse_caller_identity_includes_account_and_arn() {
        let json = r#"{"UserId":"AIDA","Account":"123456789012","Arn":"arn:aws:iam::123456789012:user/me"}"#;
        let s = parse_caller_identity(json).unwrap();
        assert!(s.contains("123456789012"));
        assert!(s.contains("arn:aws:iam::123456789012:user/me"));
    }

    #[test]
    fn parse_caller_identity_returns_none_for_bad_json() {
        assert!(parse_caller_identity("not json").is_none());
    }
}
