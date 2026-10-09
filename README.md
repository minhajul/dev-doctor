# Dev Doctor

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
                → install postgres or add it to your PATH

AWS
✓ AWS CLI       2.31.0
✓ Region        ap-southeast-1
✓ Credentials   configured
✓ Caller        account=123456789012 arn=arn:aws:iam::123456789012:user/me

Ports
✗ :3000         in use (node), expected free
                → stop whatever owns it (`lsof -iTCP:3000 -sTCP:LISTEN` shows the process)
✓ :6379         in use (redis-server)

Summary
Healthy: 12    Warnings: 1    Failed: 2
```

## Features

- Cross-platform: macOS and Linux.
- Diagnostic categories: system, tools, Docker, Kubernetes, AWS, ports,
  required environment variables.
- Concurrent execution with per-command timeouts (no hangs).
- Colorized terminal output, `NO_COLOR`-aware.
- Every warning and failure comes with a hint for what to do next.
- JSON output for CI (`devdoctor check --json`).
- Read-only by design: no secret material is ever printed.
- Configurable via `~/.config/devdoctor/config.toml`, plus a per-project
  `devdoctor.toml` you can commit to a repo.

## Installation

### Prebuilt binaries

Each [release](https://github.com/minhajul/dev-doctor/releases) has archives
for macOS (Apple Silicon, Intel) and Linux (x86_64, ARM64; static musl
builds), each with a `.sha256` checksum.

```sh
gh release download --repo minhajul/dev-doctor --pattern '*aarch64-apple-darwin.tar.gz'
tar xzf devdoctor-*.tar.gz
mv devdoctor-*/devdoctor ~/.local/bin/   # or anywhere on your $PATH
```

The macOS binaries aren't signed. If you download one with a browser, clear
the quarantine flag before running it: `xattr -d com.apple.quarantine devdoctor`.

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
devdoctor check --category env

# shortcut subcommands
devdoctor tools
devdoctor docker
devdoctor kubernetes
devdoctor aws
devdoctor ports
devdoctor env

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

### Project config

Commit a `devdoctor.toml` to a repository to describe what *that project*
needs. `devdoctor` looks for it in the current directory and then each parent
directory, and merges it over the user config key by key: nested tables merge,
and any value the project sets (including lists) replaces the user's.

```toml
# devdoctor.toml at the repo root
[tools]
enabled = ["git", "go", "docker"]

# Minimum (or exact) versions. Tools listed here are checked even if they
# are not in `enabled`.
[tools.versions]
go = ">=1.22"
node = ">=20, <23"
terraform = "1.9"      # bare or `=` version matches by prefix: 1.9.x
```

A tool whose version doesn't meet its requirement is reported as failed, e.g.
`✗ node  18.19.1 (requires >=20, <23)`. Supported operators are `>=`, `>`,
`<=`, `<` and `=`; combine several with commas.

Ports can carry an expectation. By default a port is reported either way
(in use or available) without failing; with an expectation, the wrong state
fails:

```toml
[ports]
expect_listening = [5432, 6379]   # Postgres and Redis must be running
expect_free = [3000]              # the dev server needs this port
```

When only expectations are set, the default port list is not probed. Add
`check = [...]` to probe other ports for information.

Required environment variables are checked for presence only; their values
are never read into the report. An empty value is a warning, a missing one a
failure. The Environment section only appears when this list is set.

```toml
[env]
required = ["DATABASE_URL", "STRIPE_API_KEY"]
```

Every warning and failure carries a built-in hint. A project can replace it
with its own instructions for tools, ports and env vars:

```toml
[tools.hints]
node = "run `nvm use` (version pinned in .nvmrc)"

[ports.hints]
5432 = "start the database: `docker compose up -d db`"

[env.hints]
DATABASE_URL = "copy the defaults: `cp .env.example .env`"
```

Hints only show for checks that didn't pass.

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
        {
          "name": "docker",
          "status": "failed",
          "message": "not found",
          "hint": "install docker or add it to your PATH"
        }
      ]
    }
  ],
  "summary": { "healthy": 1, "warnings": 0, "failed": 1 }
}
```

JSON output is always free of ANSI escape codes.

### Diagnostic model

Every check is a `Diagnostic` carrying a `Status` (`Healthy` / `Warning` /
`Failed`), a message, and an optional `hint` (the suggested next step, shown
under warnings and failures and omitted from JSON when absent). Diagnostics are grouped by category
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

### Releasing

1. Bump `version` in `Cargo.toml` and commit.
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`.

The Release workflow checks that the tag matches `Cargo.toml`, builds all
four targets, and publishes a GitHub Release with generated notes. Tags with a
suffix (`v0.2.0-rc.1`) become pre-releases. Run the workflow manually from the
Actions tab to build the artifacts without releasing; PRs that change
`release.yml` do the same.

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

MIT
