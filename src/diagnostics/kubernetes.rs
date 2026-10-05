//! Kubernetes diagnostics. Read-only: we never mutate cluster state.

use std::sync::Arc;
use std::time::Duration;

use tokio::join;

use crate::command::{CommandFailure, SharedRunner};
use crate::config::Config;
use crate::diagnostics::util::{first_nonempty_line, first_version_token, trimmed_stdout};
use crate::models::{Diagnostic, DiagnosticGroup};

pub async fn collect(runner: SharedRunner, config: Arc<Config>) -> DiagnosticGroup {
    let timeout = config.timeout();

    if which::which("kubectl").is_err() {
        let mut group = DiagnosticGroup::new("Kubernetes");
        group.push(Diagnostic::failed("kubectl", "not found"));
        return group;
    }

    let (version, ctx, ns, cluster) = join!(
        probe_version(&runner, timeout),
        probe_context(&runner, timeout),
        probe_namespace(&runner, timeout),
        probe_cluster(&runner, timeout),
    );

    let mut group = DiagnosticGroup::new("Kubernetes");
    group.push(version);
    group.push(ctx);
    group.push(namespace_diagnostic(ns, &config));
    group.push(cluster);
    group
}

async fn probe_version(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    // Try JSON first; fall back to plain text.
    if let Ok(out) = runner
        .run(
            "kubectl",
            &["version", "--client=true", "-o", "json"],
            timeout,
        )
        .await
    {
        return match parse_kubectl_version_json(&out.combined_output()) {
            Some(v) => Diagnostic::healthy("kubectl", v),
            None => Diagnostic::healthy("kubectl", "installed (version unknown)"),
        };
    }
    match runner
        .run("kubectl", &["version", "--client"], timeout)
        .await
    {
        Ok(out) => {
            let v = first_version_token(&out.combined_output())
                .unwrap_or_else(|| "installed".to_string());
            Diagnostic::healthy("kubectl", v)
        }
        Err(e) => Diagnostic::warning("kubectl", e.to_string()),
    }
}

async fn probe_context(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner
        .run("kubectl", &["config", "current-context"], timeout)
        .await
    {
        Ok(out) => match trimmed_stdout(&out) {
            Some(c) => Diagnostic::healthy("Context", c),
            None => Diagnostic::warning("Context", "no current context"),
        },
        Err(e) => Diagnostic::warning("Context", e.to_string()),
    }
}

async fn probe_namespace(runner: &SharedRunner, timeout: Duration) -> Option<String> {
    let out = runner
        .run(
            "kubectl",
            &[
                "config",
                "view",
                "--minify",
                "-o",
                "jsonpath={.contexts[0].context.namespace}",
            ],
            timeout,
        )
        .await
        .ok()?;
    trimmed_stdout(&out)
}

fn namespace_diagnostic(ns: Option<String>, config: &Config) -> Diagnostic {
    match ns {
        Some(n) if n == "default" && config.kubernetes.warn_default_namespace => {
            Diagnostic::warning(
                "Namespace",
                "default (consider using a dedicated namespace)",
            )
        }
        Some(n) => Diagnostic::healthy("Namespace", n),
        None => Diagnostic::healthy("Namespace", "default"),
    }
}

async fn probe_cluster(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner.run("kubectl", &["cluster-info"], timeout).await {
        Ok(_) => Diagnostic::healthy("Cluster", "reachable"),
        Err(CommandFailure::Timeout) => Diagnostic::failed("Cluster", "connection timed out"),
        Err(CommandFailure::NonZeroExit { stderr, .. }) => {
            let detail = first_nonempty_line(&stderr).unwrap_or_else(|| "unreachable".to_string());
            Diagnostic::warning("Cluster", format!("not reachable ({detail})"))
        }
        Err(e) => Diagnostic::warning("Cluster", e.to_string()),
    }
}

fn parse_kubectl_version_json(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let client = v.get("clientVersion")?;
    let major = client.get("major")?.as_str()?;
    let minor = client.get("minor")?.as_str()?;
    let git = client.get("gitVersion")?.as_str()?;
    Some(if git.is_empty() {
        format!("{major}.{minor}")
    } else {
        git.trim_start_matches('v').to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kubectl_version_handles_json() {
        let json = r#"{
            "clientVersion": {
                "major": "1",
                "minor": "34",
                "gitVersion": "v1.34.0",
                "gitCommit": "abc"
            }
        }"#;
        assert_eq!(parse_kubectl_version_json(json).as_deref(), Some("1.34.0"));
    }

    #[test]
    fn parse_kubectl_version_falls_back_to_major_minor() {
        let json = r#"{ "clientVersion": { "major": "1", "minor": "30", "gitVersion": "" } }"#;
        assert_eq!(parse_kubectl_version_json(json).as_deref(), Some("1.30"));
    }
}
