//! Docker diagnostics.
//!
//! Uses the Docker CLI exclusively. Read-only: we never modify daemon state.

use std::sync::Arc;
use std::time::Duration;

use tokio::join;

use crate::command::{CommandFailure, SharedRunner};
use crate::config::Config;
use crate::diagnostics::util::{count_nonblank_lines, first_nonempty_line, first_version_token};
use crate::models::{Diagnostic, DiagnosticGroup};

pub async fn collect(runner: SharedRunner, config: Arc<Config>) -> DiagnosticGroup {
    let timeout = config.timeout();
    let runner = runner;

    // All five probes are mutually independent — fire them in parallel.
    let (cli, daemon, compose, counts) = join!(
        probe_cli(&runner, timeout),
        probe_daemon(&runner, timeout),
        probe_compose(&runner, timeout),
        container_counts(&runner, timeout),
    );

    let mut group = DiagnosticGroup::new("Docker");
    group.push(cli);
    group.push(daemon);
    group.push(compose);
    let (running, stopped, images) = counts;
    group.push(Diagnostic::healthy(
        "Running containers",
        running.to_string(),
    ));
    group.push(Diagnostic::healthy(
        "Stopped containers",
        stopped.to_string(),
    ));
    group.push(Diagnostic::healthy("Images", images.to_string()));
    group
}

async fn probe_cli(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner.run("docker", &["--version"], timeout).await {
        Ok(out) => {
            let v = first_version_token(&out.combined_output())
                .unwrap_or_else(|| "installed".to_string());
            Diagnostic::healthy("Docker CLI", v)
        }
        Err(CommandFailure::NotFound) => Diagnostic::failed("Docker CLI", "not found"),
        Err(e) => Diagnostic::failed("Docker CLI", e.to_string()),
    }
}

async fn probe_daemon(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner.run("docker", &["info"], timeout).await {
        Ok(_) => Diagnostic::healthy("Daemon", "reachable"),
        Err(CommandFailure::NotFound) => Diagnostic::failed("Daemon", "docker binary not found"),
        Err(CommandFailure::NonZeroExit { stderr, .. }) => {
            let detail = first_nonempty_line(&stderr).unwrap_or_else(|| "unreachable".to_string());
            Diagnostic::failed("Daemon", format!("not reachable ({detail})"))
        }
        Err(CommandFailure::Timeout) => Diagnostic::failed("Daemon", "timed out"),
        Err(CommandFailure::Io(msg)) => Diagnostic::failed("Daemon", msg),
    }
}

async fn probe_compose(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    // Prefer the v2 plugin form; fall back to the legacy standalone binary.
    match runner.run("docker", &["compose", "version"], timeout).await {
        Ok(out) => match first_version_token(&out.combined_output()) {
            Some(v) => return Diagnostic::healthy("Compose", v),
            None => return Diagnostic::warning("Compose", "installed (version unknown)"),
        },
        Err(CommandFailure::NotFound) => {
            return Diagnostic::warning("Compose", "not installed");
        }
        Err(_) => { /* fall through to docker-compose probe */ }
    }

    match runner.run("docker-compose", &["--version"], timeout).await {
        Ok(out) => match first_version_token(&out.combined_output()) {
            Some(v) => Diagnostic::healthy("Compose", v),
            None => Diagnostic::warning("Compose", "installed (version unknown)"),
        },
        Err(_) => Diagnostic::warning("Compose", "not installed"),
    }
}

/// Best-effort container/image counts. Zeros on any error so the rest of
/// the report stays usable.
async fn container_counts(runner: &SharedRunner, timeout: Duration) -> (usize, usize, usize) {
    let (running, stopped, images) = join!(
        count_cmd(runner, &["ps", "-q"], timeout),
        count_cmd(
            runner,
            &["ps", "-a", "-q", "--filter", "status=exited"],
            timeout,
        ),
        count_cmd(runner, &["images", "-q"], timeout),
    );
    (running, stopped, images)
}

async fn count_cmd(runner: &SharedRunner, args: &[&str], timeout: Duration) -> usize {
    match runner.run("docker", args, timeout).await {
        Ok(out) => count_nonblank_lines(&out.stdout),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeRunner;

    #[tokio::test]
    async fn collect_emits_six_diagnostics() {
        let runner: SharedRunner = Arc::new(FakeRunner::new());
        let cfg = Arc::new(Config::default());
        let group = collect(runner, cfg).await;
        assert_eq!(group.diagnostics.len(), 6);
    }
}
