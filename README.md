# devdoctor

A fast, friendly CLI for inspecting the local developer environment. `devdoctor`
checks common tools, services, and platform-engineering dependencies, and prints
a clean, colorized health report.

It is **read-only**: it never modifies Docker, Kubernetes, AWS, or your system
configuration. It is safe to run in CI.

```
Developer Environment
----------------------
System
✓ OS            macOS
✓ Architecture  arm64
✓ Shell         zsh
✓ Hostname      macbook
✓ User          minhaj

Tools
✓ git           2.51.0
✓ docker        28.1.0
⚠ node          installed but version probe failed: exit status 1
✗ postgres      not found

AWS
✓ AWS CLI       2.31.0
✓ Region        ap-southeast-1
⚠ Credentials   not configured
✓ Caller        account=123456789012 arn=arn:aws:iam::123456789012:user/me

Ports
✓ :3000         available
✗ :6379         in use (redis-server)

Summary
Healthy: 8    Warnings: 2    Failed: 1
```

## Features

- Cross-platform: macOS and Linux.
- Diagnostic categories: system, tools, Docker, Kubernetes, AWS, ports.
- Concurrent execution with per-command timeouts (no hangs).
- Colorized terminal output, `NO_COLOR`-aware.
- JSON output for CI (`devdoctor check --json`).
- Read-only by design: no secret material is ever printed.
- Configurable via `~/.config/devdoctor/config.toml`.

## Installation

### From source

```sh
cargo install --path .
```

The binary lands in `~/.cargo/bin/devdoctor` (or your `CARGO_HOME` equivalent).

### Release build

```sh
cargo build --release
# binary at ./target/release/devdoctor
```

You can copy that binary anywhere on your `$PATH`.

## Usage

```sh
# default: run all categories
devdoctor

# explicit form
devdoctor check

# one category only
devdoctor check --category tools
devdoctor check --category docker
devdoctor check --category kubernetes
devdoctor check --category aws
devdoctor check --category ports

# shortcut subcommands
devdoctor tools
devdoctor docker
devdoctor kubernetes
devdoctor aws
devdoctor ports

# JSON output
devdoctor check --json

# disable colors
devdoctor --no-color
NO_COLOR=1 devdoctor

# version
devdoctor version
```

## Configuration

Optional config at `~/.config/devdoctor/config.toml`:

```toml
timeout_seconds = 5

[tools]
enabled = [
    "git",
    "docker",
    "go",
    "rustc",
    "node",
    "kubectl",
    "aws",
]

[ports]
check = [3000, 5432, 6379, 8080]

[kubernetes]
warn_default_namespace = true
```

If the file is missing, defaults are used. A malformed file produces a clear
error.

## Exit codes

| Code | Meaning                                                |
|-----:|--------------------------------------------------------|
|   0  | All checks healthy, or only warnings.                  |
|   1  | One or more diagnostic failures.                       |
|   2  | Invalid CLI usage or internal error.                   |

Warnings alone do **not** produce a non-zero exit code, so the CLI is safe to
gate shell scripts and CI on.

## JSON output

```sh
devdoctor check --json
```

```json
{
  "groups": [
    {
      "name": "Tools",
      "diagnostics": [
        { "name": "git", "status": "healthy", "message": "2.51.0" },
        { "name": "docker", "status": "failed", "message": "not found" }
      ]
    }
  ],
  "summary": { "healthy": 1, "warnings": 0, "failed": 1 }
}
```

JSON output is always free of ANSI escape codes.

## Architecture

```
src/
├── main.rs          # entry point, arg parsing, dispatch
├── cli.rs           # clap derive types
├── config.rs        # TOML config loading & defaults
├── command.rs       # Runner trait + tokio CommandRunner + FakeRunner
├── output.rs        # terminal and JSON rendering
├── models.rs        # Status / Diagnostic / DiagnosticGroup / Report / Summary
└── diagnostics/
    ├── mod.rs       # fan-out across categories
    ├── system.rs    # OS / arch / shell / hostname / user
    ├── tools.rs     # CLI tool presence + version parsing
    ├── docker.rs    # docker info, compose, container counts
    ├── kubernetes.rs# kubectl context, namespace, cluster
    ├── aws.rs       # aws cli, region, sts caller identity
    └── ports.rs     # TCP probe on configured ports
```

### Diagnostic model

Every check is a `Diagnostic` carrying a `Status` (`Healthy` / `Warning` /
`Failed`) and a message. Diagnostics are grouped by category
(`DiagnosticGroup`) and aggregated into a `Report` with a `Summary`. The
`Summary` decides the exit code.

### Command execution abstraction

All external processes go through the `Runner` trait in `command.rs`:

```rust
#[async_trait::async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, program: &str, args: &[&str], timeout_dur: Duration)
        -> Result<CommandOutput, CommandFailure>;
}
```

The default implementation spawns via `tokio::process::Command`, enforces a
hard timeout (`kill_on_drop(true)`), and distinguishes "not found" from
"timed out" from "non-zero exit" so diagnostics can degrade gracefully.

Tests use `FakeRunner` to program canned responses without touching real
binaries.

### Concurrency

Independent categories (e.g. Docker, AWS, Kubernetes, ports) are dispatched on
the Tokio runtime with `tokio::spawn` and awaited in a stable order.

## Development

```sh
make build       # debug build
make release     # release build
make test        # cargo test
make fmt         # cargo fmt
make lint        # cargo clippy --all-targets --all-features -- -D warnings
make check       # fmt-check + clippy + test
make run         # cargo run
make install     # cargo install --path .
make clean       # cargo clean
```

## Roadmap

- Database health: PostgreSQL, MySQL, Redis.
- DNS / internet connectivity.
- GitHub authentication status (`gh auth status`).
- SSH agent.
- Terraform providers.
- Disk, memory, CPU usage.
- SSL / local certificate expiry.
- Homebrew / system package manager inventory.
- Git repository discovery.
- Tailscale / VPN state.

The architecture is intentionally small and composable so each of these
becomes a new file under `src/diagnostics/` plus a `Category` variant.

## License

MIT OR Apache-2.0
