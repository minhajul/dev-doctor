//! Docker diagnostics.
//!
//! Uses the Docker CLI exclusively. Read-only: we never modify daemon state.

use std::sync::Arc;
use std::time::Duration;

use tokio::join;

use crate::command::{CommandFailure, SharedRunner};
use crate::config::Config;
use crate::diagnostics::util::{
    count_nonblank_lines, failure_hint, first_nonempty_line, first_version_token,
};
use crate::models::{Diagnostic, DiagnosticGroup, Status};

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

    assemble(cli, daemon, compose, counts)
}

type Counts = (Option<usize>, Option<usize>, Option<usize>);

/// If the CLI can't run, the other probes only repeat its error, so report
/// the CLI alone. Container and image counts need a reachable daemon, so
/// they are left out when it isn't.
fn assemble(
    cli: Diagnostic,
    daemon: Diagnostic,
    compose: Diagnostic,
    counts: Counts,
) -> DiagnosticGroup {
    let mut group = DiagnosticGroup::new("Docker");
    let cli_ok = cli.status == Status::Healthy;
    let daemon_ok = daemon.status == Status::Healthy;
    group.push(cli);
    if !cli_ok {
        return group;
    }
    group.push(daemon);
    group.push(compose);
    if !daemon_ok {
        return group;
    }
    let (running, stopped, images) = counts;
    for (name, count) in [
        ("Running containers", running),
        ("Stopped containers", stopped),
        ("Images", images),
    ] {
        group.push(match count {
            Some(n) => Diagnostic::healthy(name, n.to_string()),
            None => Diagnostic::warning(name, "could not be counted"),
        });
    }
    group
}

async fn probe_cli(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner.run("docker", &["--version"], timeout).await {
        Ok(out) => {
            let v = first_version_token(&out.combined_output())
                .unwrap_or_else(|| "installed".to_string());
            Diagnostic::healthy("Docker CLI", v)
        }
        Err(CommandFailure::NotFound) => Diagnostic::failed("Docker CLI", "not found")
            .with_hint("install Docker Desktop, OrbStack, or Colima"),
        Err(e) => {
            let diag = Diagnostic::failed("Docker CLI", e.to_string());
            match failure_hint(&e) {
                Some(h) => diag.with_hint(h),
                None => diag,
            }
        }
    }
}

const START_DAEMON: &str =
    "start your Docker runtime (Docker Desktop, OrbStack, or `colima start`)";

async fn probe_daemon(runner: &SharedRunner, timeout: Duration) -> Diagnostic {
    match runner.run("docker", &["info"], timeout).await {
        Ok(_) => Diagnostic::healthy("Daemon", "reachable"),
        Err(CommandFailure::NotFound) => Diagnostic::failed("Daemon", "docker binary not found"),
        Err(CommandFailure::NonZeroExit { stderr, .. }) => {
            let detail = first_nonempty_line(&stderr).unwrap_or_else(|| "unreachable".to_string());
            Diagnostic::failed("Daemon", format!("not reachable ({detail})"))
                .with_hint(START_DAEMON)
        }
        Err(CommandFailure::Timeout) => {
            Diagnostic::failed("Daemon", "timed out").with_hint(START_DAEMON)
        }
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
            return Diagnostic::warning("Compose", "not installed").with_hint(INSTALL_COMPOSE);
        }
        Err(_) => { /* fall through to docker-compose probe */ }
    }

    match runner.run("docker-compose", &["--version"], timeout).await {
        Ok(out) => match first_version_token(&out.combined_output()) {
            Some(v) => Diagnostic::healthy("Compose", v),
            None => Diagnostic::warning("Compose", "installed (version unknown)"),
        },
        Err(_) => Diagnostic::warning("Compose", "not installed").with_hint(INSTALL_COMPOSE),
    }
}

const INSTALL_COMPOSE: &str = "install the Docker Compose plugin";

/// Best-effort container/image counts; `None` where a count failed.
async fn container_counts(runner: &SharedRunner, timeout: Duration) -> Counts {
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

async fn count_cmd(runner: &SharedRunner, args: &[&str], timeout: Duration) -> Option<usize> {
    let out = runner.run("docker", args, timeout).await.ok()?;
    Some(count_nonblank_lines(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeRunner;

    use crate::command::CommandOutput;

    fn names(group: &DiagnosticGroup) -> Vec<&str> {
        group.diagnostics.iter().map(|d| d.name.as_str()).collect()
    }

    fn ok(stdout: &str) -> crate::command::CommandResult {
        Ok(CommandOutput {
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }

    #[tokio::test]
    async fn missing_cli_reports_only_the_cli() {
        let runner: SharedRunner = Arc::new(FakeRunner::new());
        let group = collect(runner, Arc::new(Config::default())).await;
        assert_eq!(names(&group), vec!["Docker CLI"]);
        assert_eq!(group.failed_count(), 1);
    }

    #[tokio::test]
    async fn unreachable_daemon_omits_counts() {
        let fake = FakeRunner::new();
        fake.program_response(
            "docker",
            &["--version"],
            ok("Docker version 29.4.0, build x"),
        )
        .await;
        fake.program_response(
            "docker",
            &["info"],
            Err(CommandFailure::NonZeroExit {
                status: 1,
                stderr: "Cannot connect to the Docker daemon".into(),
            }),
        )
        .await;
        fake.program_response("docker", &["compose", "version"], ok("v2.30.0"))
            .await;
        let group = collect(Arc::new(fake), Arc::new(Config::default())).await;
        assert_eq!(names(&group), vec!["Docker CLI", "Daemon", "Compose"]);
        assert_eq!(group.diagnostics[1].status, Status::Failed);
    }

    #[tokio::test]
    async fn reachable_daemon_reports_counts() {
        let fake = FakeRunner::new();
        fake.program_response(
            "docker",
            &["--version"],
            ok("Docker version 29.4.0, build x"),
        )
        .await;
        fake.program_response("docker", &["info"], ok("Server: ..."))
            .await;
        fake.program_response("docker", &["compose", "version"], ok("v2.30.0"))
            .await;
        fake.program_response("docker", &["ps", "-q"], ok("a1\nb2\n"))
            .await;
        fake.program_response("docker", &["images", "-q"], ok("i1\n"))
            .await;
        // `ps -a ... status=exited` is left unprogrammed, so it fails.
        let group = collect(Arc::new(fake), Arc::new(Config::default())).await;

        assert_eq!(group.diagnostics.len(), 6);
        assert_eq!(group.diagnostics[3].message, "2");
        assert_eq!(group.diagnostics[4].status, Status::Warning);
        assert_eq!(group.diagnostics[5].message, "1");
    }
}
