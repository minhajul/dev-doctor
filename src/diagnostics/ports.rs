//! Port diagnostics.
//!
//! Each port is probed with a fast TCP connect on `127.0.0.1:<port>` with
//! a short timeout. We also try `lsof` / `ss` for best-effort process
//! identification; if those fail, the diagnostic still works.
//!
//! By default either state is healthy and the result is informational.
//! Ports listed in `expect_listening` / `expect_free` fail when they are in
//! the other state.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Command as TokioCommand;

use crate::config::Config;
use crate::models::{Diagnostic, DiagnosticGroup};

/// Collect diagnostics for each configured port.
pub async fn collect(
    _runner: crate::command::SharedRunner,
    config: Arc<Config>,
) -> DiagnosticGroup {
    let ports = config.resolved_ports();

    let mut handles = Vec::with_capacity(ports.len());
    for port in ports {
        let expect = if config.ports.expect_listening.contains(&port) {
            Expect::Listening
        } else if config.ports.expect_free.contains(&port) {
            Expect::Free
        } else {
            Expect::Either
        };
        handles.push(tokio::spawn(async move { probe_port(port, expect).await }));
    }

    let mut group = DiagnosticGroup::new("Ports");
    for handle in handles {
        if let Ok(diag) = handle.await {
            group.push(diag);
        }
    }
    group
}

/// What the config expects to find on a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Either,
    Listening,
    Free,
}

async fn probe_port(port: u16, expect: Expect) -> Diagnostic {
    let name = format!(":{port}");
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    match TcpStream::connect_timeout(&addr, Duration::from_millis(750)) {
        Ok(_) => {
            let msg = match lookup_process(port).await {
                Some(proc) => format!("in use ({proc})"),
                None => "in use".to_string(),
            };
            if expect == Expect::Free {
                Diagnostic::failed(name, format!("{msg}, expected free")).with_hint(format!(
                    "stop whatever owns it (`lsof -iTCP:{port} -sTCP:LISTEN` shows the process)"
                ))
            } else {
                Diagnostic::healthy(name, msg)
            }
        }
        Err(_) if expect == Expect::Listening => {
            Diagnostic::failed(name, "nothing listening, expected a service")
                .with_hint(format!("start the service that should listen on :{port}"))
        }
        Err(_) => Diagnostic::healthy(name, "available".to_string()),
    }
}

/// Best-effort process lookup via `lsof` (macOS, Linux) or `ss` (Linux).
/// Runs through tokio so it doesn't block the runtime worker.
async fn lookup_process(port: u16) -> Option<String> {
    let lsof_args = ["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-F", "c"];
    if let Ok(Ok(out)) = tokio::time::timeout(
        Duration::from_millis(500),
        TokioCommand::new("lsof").args(lsof_args).output(),
    )
    .await
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if let Some(name) = line.strip_prefix('c') {
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    let ss_args = ["-ltnp", &format!("sport = :{port}")];
    if let Ok(Ok(out)) = tokio::time::timeout(
        Duration::from_millis(500),
        TokioCommand::new("ss").args(ss_args).output(),
    )
    .await
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().nth(1) {
                if let Some(name) = extract_process_from_ss(line) {
                    return Some(name);
                }
            }
        }
    }

    None
}

/// Pull a process name out of an `ss` line if present.
fn extract_process_from_ss(line: &str) -> Option<String> {
    let start = line.find("users:((")?;
    let rest = &line[start + "users:((".len()..];
    let end = rest.find("))")?;
    let inner = &rest[..end];
    let name = inner.split(',').next()?;
    Some(name.trim_matches('"').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::SharedRunner;
    use std::sync::Arc;

    #[test]
    fn extract_process_from_ss_finds_name() {
        let line = "LISTEN 0 128 *:3000 *:* users:((\"node\",pid=1234,fd=21))";
        assert_eq!(extract_process_from_ss(line).as_deref(), Some("node"));
    }

    #[test]
    fn extract_process_from_ss_handles_missing_users() {
        let line = "LISTEN 0 128 *:3000 *:*";
        assert_eq!(extract_process_from_ss(line), None);
    }

    use crate::models::Status;

    #[tokio::test]
    async fn expectations_decide_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let busy = listener.local_addr().expect("addr").port();
        let free = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            l.local_addr().expect("addr").port()
        };

        assert_eq!(
            probe_port(busy, Expect::Listening).await.status,
            Status::Healthy
        );
        assert_eq!(
            probe_port(busy, Expect::Either).await.status,
            Status::Healthy
        );
        assert_eq!(probe_port(busy, Expect::Free).await.status, Status::Failed);
        assert_eq!(probe_port(free, Expect::Free).await.status, Status::Healthy);
        assert_eq!(
            probe_port(free, Expect::Either).await.status,
            Status::Healthy
        );
        assert_eq!(
            probe_port(free, Expect::Listening).await.status,
            Status::Failed
        );
    }

    #[tokio::test]
    async fn collect_produces_one_diagnostic_per_port() {
        let cfg = Arc::new(Config::default());
        let runner: SharedRunner = Arc::new(crate::command::CommandRunner);
        let group = collect(runner, cfg).await;
        assert_eq!(
            group.diagnostics.len(),
            Config::default().resolved_ports().len()
        );
        for d in &group.diagnostics {
            assert!(d.name.starts_with(':'), "unexpected name {}", d.name);
        }
    }
}
