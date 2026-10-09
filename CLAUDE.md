# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`devdoctor` is a Rust (edition 2021, Tokio) binary-only CLI that inspects the local dev environment (system, tools, Docker, Kubernetes, AWS, ports, required env vars) and prints a colorized or JSON health report. It is **read-only by design**: diagnostics must never change Docker/Kubernetes/AWS/system state, and must never print secret material.

## Commands

```sh
make build     # cargo build
make test      # cargo test
make lint      # cargo clippy --all-targets --all-features -- -D warnings
make check     # fmt --check + clippy + test (what to run before considering work done)
make run       # cargo run --   (e.g. `cargo run -- check --category docker --json`)

cargo test <name_substring>          # single test, e.g. `cargo test parses_check_subcommand`
cargo test --test cli                # only the integration tests in tests/cli.rs
cargo test diagnostics::docker       # tests in one module
```

Clippy runs with `-D warnings`, so any warning fails `make check`. CI (`.github/workflows/ci.yml`) runs `make check` on Ubuntu and macOS for pushes to main and every PR.

## Architecture

Flow: `main.rs` parses args (`cli.rs`), loads config (`config.rs`), builds a `SharedRunner`, dispatches categories, builds a `Report`, and renders it via `output.rs`. The process exit code comes from `Report::exit_code()`: `1` if any diagnostic `Failed`, `0` otherwise (warnings alone are `0`); `2` is reserved for usage/internal errors (config load failure, runtime failure).

- **Data model (`models.rs`)**: `Diagnostic { name, status, message, hint }` → `DiagnosticGroup` (one per category) → `Report { groups, summary }`. `Status` serializes lowercase; `hint` is omitted when `None`. This is the JSON output contract. Give every new Warning/Failed a `.with_hint(...)` with a concrete next step (command to run, thing to install); `util::failure_hint` covers errors that mean the binary itself can't run. Project hints (`tools.hints`, `ports.hints`, `env.hints`) are applied with `Diagnostic::with_configured_hint`, which replaces the built-in hint only on non-healthy results.
- **Command execution (`command.rs`)**: external processes must go through the `Runner` trait (`SharedRunner = Arc<dyn Runner>`). `CommandRunner` enforces a hard timeout with `kill_on_drop(true)` and returns `CommandFailure::{NotFound, Timeout, NonZeroExit, Io}` so diagnostics can degrade to Warning/Failed instead of erroring. Timeout comes from `config.timeout()`.
- **Diagnostics (`src/diagnostics/`)**: each module exposes `pub async fn collect(runner, config) -> DiagnosticGroup` and must never panic on missing tools. Independent probes within a module run concurrently (`tokio::join!`); `run_categories` in `diagnostics/mod.rs` spawns each category on Tokio and awaits them in the canonical `Category::all()` order, converting a task panic into a synthetic `internal` failed diagnostic. When a prerequisite probe fails (CLI can't run, daemon unreachable), report it once and omit the checks that depend on it rather than emitting repeated errors or placeholder ✓ values; see `assemble` in `docker.rs` / `aws.rs`. Shared parsing helpers (version extraction etc.) live in `diagnostics/util.rs`. Version requirements (`[tools.versions]`) use the small hand-rolled comparator in `src/version.rs`, deliberately not the `semver` crate, because tool versions are often not valid SemVer.
  - Exceptions: `system::collect()` takes no args, and `ports` ignores the runner — it does a direct TCP connect to `127.0.0.1` and calls `lsof`/`ss` via `tokio::process` for best-effort process names. Ports are informational (always healthy) unless listed in `ports.expect_listening` / `ports.expect_free`. `env::collect(config)` also ignores the runner, and `run_categories` omits the Env category from the default run when `env.required` is empty.

### Adding a new diagnostic category

1. New file under `src/diagnostics/` with a `collect` fn.
2. Add a `Category` variant in `cli.rs` (and `as_str`), plus `label()`, `dispatch()`, and `Category::all()` (update the array length) in `diagnostics/mod.rs`.
3. If it should have a shortcut subcommand, add a `Command` variant in `cli.rs` and a match arm in `main.rs::run`.
4. Any new config section goes in `config.rs`; mirror it in `config.example.toml`.

## Testing

- Unit tests live in `#[cfg(test)]` modules beside the code. Use `command::FakeRunner` (test-only) to program canned responses keyed by `"program arg1 arg2"`; unprogrammed calls return `CommandFailure::NotFound`. See `diagnostics/docker.rs` / `tools.rs` tests for the pattern.
- `tests/cli.rs` runs the real binary (`CARGO_BIN_EXE_devdoctor`) inside a `Sandbox` temp dir that is both `HOME` and the cwd, so the developer's user config and any outer `devdoctor.toml` can't leak in. Drive it through `devdoctor.toml` and stick to host-independent categories (env, tools with made-up names, loopback ports).
- Tests must not depend on Docker/AWS/kubectl being installed.

## Configuration

Two optional layers, merged as raw TOML tables before deserializing into `Config` (`config::load_layered`): the user config at `~/.config/devdoctor/config.toml` (or `$XDG_CONFIG_HOME`), then a project `devdoctor.toml` found by walking up from the cwd. Nested tables merge; scalars and arrays from the later layer replace. Each file is also validated on its own so parse errors name the offending file. Defaults live in `config.rs` (`DEFAULT_TOOLS`, `DEFAULT_PORTS`, `DEFAULT_TIMEOUT_SECONDS`); see `config.example.toml`. Missing files → defaults; malformed file or failed `Config::validate()` (cross-field checks) → error (exit 2). New config fields need serde defaults so partial files keep working.

Output colors honor `--no-color`, `NO_COLOR`, and TTY detection; JSON output must never contain ANSI codes.
