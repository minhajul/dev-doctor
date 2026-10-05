//! Command execution abstraction.
//!
//! Every diagnostic in the codebase runs external commands through
//! [`CommandRunner`]. This keeps timeouts, error handling, and
//! command-not-found detection consistent, and gives us a single seam
//! for tests.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

/// Reason a command run terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandFailure {
    NotFound,
    Timeout,
    NonZeroExit { status: i32, stderr: String },
    Io(String),
}

impl std::fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandFailure::NotFound => f.write_str("command not found"),
            CommandFailure::Timeout => f.write_str("timed out"),
            CommandFailure::NonZeroExit { status, stderr } => {
                if stderr.is_empty() {
                    write!(f, "exit status {status}")
                } else {
                    write!(f, "exit status {status}: {}", stderr.trim())
                }
            }
            CommandFailure::Io(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for CommandFailure {}

/// Outcome of a command run.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// Concatenate stdout and stderr with a newline between them.
    pub fn combined_output(&self) -> String {
        let mut out = String::new();
        out.push_str(self.stdout.trim_end());
        if !self.stderr.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(self.stderr.trim_end());
        }
        out
    }
}

/// Trait abstraction so tests can substitute a fake runner.
#[async_trait::async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, program: &str, args: &[&str], timeout_dur: Duration) -> CommandResult;
}

pub type CommandResult = Result<CommandOutput, CommandFailure>;

/// Default implementation backed by [`tokio::process::Command`].
#[derive(Debug, Default, Clone)]
pub struct CommandRunner;

#[async_trait::async_trait]
impl Runner for CommandRunner {
    async fn run(&self, program: &str, args: &[&str], timeout_dur: Duration) -> CommandResult {
        run_command(program, args, timeout_dur).await
    }
}

/// Run an external command with a hard timeout.
///
/// Returns:
/// * [`CommandFailure::NotFound`] if the binary doesn't exist on PATH;
/// * [`CommandFailure::Timeout`] if it runs too long;
/// * the captured [`CommandOutput`] otherwise.
pub async fn run_command(program: &str, args: &[&str], timeout_dur: Duration) -> CommandResult {
    let mut cmd = TokioCommand::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CommandFailure::NotFound);
        }
        Err(e) => return Err(CommandFailure::Io(e.to_string())),
    };

    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");

    let stdout_handle = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stdout_pipe.read_to_string(&mut buf).await;
        buf
    });
    let stderr_handle = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr_pipe.read_to_string(&mut buf).await;
        buf
    });

    match timeout(timeout_dur, child.wait()).await {
        Ok(Ok(status)) => {
            let stdout = stdout_handle.await.unwrap_or_default();
            let stderr = stderr_handle.await.unwrap_or_default();
            if status.success() {
                Ok(CommandOutput { stdout, stderr })
            } else {
                Err(CommandFailure::NonZeroExit {
                    status: status.code().unwrap_or(-1),
                    stderr,
                })
            }
        }
        Ok(Err(e)) => Err(CommandFailure::Io(e.to_string())),
        Err(_) => Err(CommandFailure::Timeout), // kill_on_drop(true) handles cleanup
    }
}

/// Convenience wrapper around the default runner.
#[allow(dead_code)]
pub async fn run(program: &str, args: &[&str], timeout_dur: Duration) -> CommandResult {
    run_command(program, args, timeout_dur).await
}

/// A runner that can be shared across async tasks cheaply.
pub type SharedRunner = Arc<dyn Runner>;

/// Build a default shared runner.
pub fn default_runner() -> SharedRunner {
    Arc::new(CommandRunner)
}

/// In-memory fake runner for tests.
#[cfg(test)]
mod fake {
    use super::*;
    use tokio::sync::Mutex;

    #[derive(Debug, Default, Clone)]
    pub struct FakeRunner {
        pub responses: Arc<Mutex<Vec<(String, CommandResult)>>>,
        pub fallback: Arc<Mutex<Option<CommandResult>>>,
    }

    impl FakeRunner {
        pub fn new() -> Self {
            Self {
                responses: Arc::new(Mutex::new(Vec::new())),
                fallback: Arc::new(Mutex::new(Some(Err(CommandFailure::NotFound)))),
            }
        }

        pub async fn program_response(&self, program: &str, args: &[&str], result: CommandResult) {
            let key = format!("{} {}", program, args.join(" "));
            self.responses.lock().await.push((key, result));
        }

        fn make_key(program: &str, args: &[&str]) -> String {
            format!("{} {}", program, args.join(" "))
        }
    }

    #[async_trait::async_trait]
    impl Runner for FakeRunner {
        async fn run(&self, program: &str, args: &[&str], _timeout_dur: Duration) -> CommandResult {
            let key = Self::make_key(program, args);
            let responses = self.responses.lock().await;
            for (k, r) in responses.iter() {
                if k == &key {
                    return r.clone();
                }
            }
            self.fallback
                .lock()
                .await
                .clone()
                .unwrap_or(Err(CommandFailure::NotFound))
        }
    }
}

#[cfg(test)]
pub(crate) use fake::FakeRunner;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_command_returns_not_found_for_missing_program() {
        let result = run_command(
            "definitely-not-a-real-binary-xyz",
            &[],
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(result.unwrap_err(), CommandFailure::NotFound);
    }

    #[tokio::test]
    async fn fake_runner_programs_responses() {
        let fake = FakeRunner::new();
        fake.program_response(
            "echo",
            &["hello"],
            Ok(CommandOutput {
                stdout: "hello\n".into(),
                stderr: String::new(),
            }),
        )
        .await;

        let out = fake
            .run("echo", &["hello"], Duration::from_secs(1))
            .await
            .expect("ok");
        assert_eq!(out.stdout, "hello\n");
    }

    #[tokio::test]
    async fn fake_runner_falls_back_to_not_found() {
        let fake = FakeRunner::new();
        let err = fake
            .run("nope", &[], Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(err, CommandFailure::NotFound);
    }

    #[test]
    fn command_output_combined_output() {
        let o = CommandOutput {
            stdout: "out".into(),
            stderr: "err".into(),
        };
        assert_eq!(o.combined_output(), "out\nerr");
    }
}
