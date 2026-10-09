//! AWS diagnostics. Read-only. Never prints secret material.

use std::sync::Arc;
use std::time::Duration;

use tokio::join;

use crate::command::{CommandFailure, SharedRunner};
use crate::config::Config;
use crate::diagnostics::util::failure_hint;
use crate::models::{Diagnostic, DiagnosticGroup, Status};

pub async fn collect(runner: SharedRunner, config: Arc<Config>) -> DiagnosticGroup {
    let timeout = config.timeout();

    if which::which("aws").is_err() {
        let mut group = DiagnosticGroup::new("AWS");
        group.push(
            Diagnostic::failed("AWS CLI", "not found")
                .with_hint("install AWS CLI v2: https://aws.amazon.com/cli/"),
        );
        return group;
    }

    let (cli, region, sts) = join!(
        probe_cli(&runner, timeout),
        probe_region(&runner, timeout),
        probe_sts(&runner, timeout),
    );

    assemble(cli, region, sts)
}

/// If the CLI itself can't run, the other probes only repeat its error, so
/// report the CLI alone. Caller is shown only when STS succeeded; otherwise
/// Credentials already explains why.
fn assemble(
    cli: Diagnostic,
    region: Diagnostic,
    sts: Result<String, CommandFailure>,
) -> DiagnosticGroup {
    let mut group = DiagnosticGroup::new("AWS");
    let cli_ok = cli.status == Status::Healthy;
    group.push(cli);
    if !cli_ok {
        return group;
    }
    group.push(region);
    group.push(credentials_diagnostic(&sts));
    if let Some(caller) = caller_diagnostic(&sts) {
        group.push(caller);
    }
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
        Err(e) => {
            let diag = Diagnostic::failed("AWS CLI", e.to_string());
            match failure_hint(&e) {
                Some(h) => diag.with_hint(h),
                None => diag,
            }
        }
    }
}

const SET_REGION: &str = "`aws configure set region <region>` or export AWS_REGION";

async fn probe_region(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner
        .run("aws", &["configure", "get", "region"], timeout)
        .await
    {
        Ok(out) => match out.stdout.trim() {
            "" => Diagnostic::warning("Region", "not configured").with_hint(SET_REGION),
            s => Diagnostic::healthy("Region", s.to_string()),
        },
        Err(CommandFailure::NonZeroExit { .. }) => {
            Diagnostic::warning("Region", "not configured").with_hint(SET_REGION)
        }
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
                .with_hint("`aws sso login` or `aws configure`")
        }
        Err(CommandFailure::Timeout) => Diagnostic::warning("Credentials", "STS check timed out"),
        Err(CommandFailure::NotFound) => Diagnostic::failed("Credentials", "aws CLI not found"),
        Err(e) => Diagnostic::warning("Credentials", e.to_string()),
    }
}

fn caller_diagnostic(sts: &Result<String, CommandFailure>) -> Option<Diagnostic> {
    let stdout = sts.as_ref().ok()?;
    Some(match parse_caller_identity(stdout) {
        Some(summary) => Diagnostic::healthy("Caller", summary),
        None => Diagnostic::warning("Caller", "could not parse STS response"),
    })
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

    fn names(group: &DiagnosticGroup) -> Vec<&str> {
        group.diagnostics.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn broken_cli_reports_only_the_cli() {
        let err = CommandFailure::Io("Bad CPU type in executable (os error 86)".into());
        let group = assemble(
            Diagnostic::failed("AWS CLI", err.to_string()),
            Diagnostic::warning("Region", err.to_string()),
            Err(err),
        );
        assert_eq!(names(&group), vec!["AWS CLI"]);
        assert_eq!(group.failed_count(), 1);
    }

    #[test]
    fn failed_sts_omits_caller() {
        let group = assemble(
            Diagnostic::healthy("AWS CLI", "2.31.0"),
            Diagnostic::healthy("Region", "eu-west-1"),
            Err(CommandFailure::NonZeroExit {
                status: 255,
                stderr: "Unable to locate credentials".into(),
            }),
        );
        assert_eq!(names(&group), vec!["AWS CLI", "Region", "Credentials"]);
        assert_eq!(group.diagnostics[2].status, Status::Warning);
    }

    #[test]
    fn successful_sts_shows_caller() {
        let json = r#"{"Account":"123456789012","Arn":"arn:aws:iam::123456789012:user/me"}"#;
        let group = assemble(
            Diagnostic::healthy("AWS CLI", "2.31.0"),
            Diagnostic::healthy("Region", "eu-west-1"),
            Ok(json.into()),
        );
        assert_eq!(
            names(&group),
            vec!["AWS CLI", "Region", "Credentials", "Caller"]
        );
        assert_eq!(group.healthy_count(), 4);
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
