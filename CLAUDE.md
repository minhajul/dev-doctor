# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`devdoctor` is a Rust (edition 2021, Tokio) binary-only CLI that inspects the local dev environment (system, tools, Docker, Kubernetes, AWS, ports) and prints a colorized or JSON health report. It is **read-only by design**: diagnostics must never change Docker/Kubernetes/AWS/system state, and must never print secret material.

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

Clippy runs with `-D warnings`, so any warning fails `make check`.

## Architecture

Flow: `main.rs` parses args (`cli.rs`), loads config (`config.rs`), builds a `SharedRunner`, dispatches categories, builds a `Report`, and renders it via `output.rs`. The process exit code comes from `Report::exit_code()`: `1` if any diagnostic `Failed`, `0` otherwise (warnings alone are `0`); `2` is reserved for usage/internal errors (config load failure, runtime failure).

- **Data model (`models.rs`)**: `Diagnostic { name, status, message }` → `DiagnosticGroup` (one per category) → `Report { groups, summary }`. `Status` serializes lowercase; this is the JSON output contract.
- **Command execution (`command.rs`)**: external processes must go through the `Runner` trait (`SharedRunner = Arc<dyn Runner>`). `CommandRunner` enforces a hard timeout with `kill_on_drop(true)` and returns `CommandFailure::{NotFound, Timeout, NonZeroExit, Io}` so diagnostics can degrade to Warning/Failed instead of erroring. Timeout comes from `config.timeout()`.
- **Diagnostics (`src/diagnostics/`)**: each module exposes `pub async fn collect(runner, config) -> DiagnosticGroup` and must never panic on missing tools. Independent probes within a module run concurrently (`tokio::join!`); `run_categories` in `diagnostics/mod.rs` spawns each category on Tokio and awaits them in the canonical `Category::all()` order, converting a task panic into a synthetic `internal` failed diagnostic. Shared parsing helpers (version extraction etc.) live in `diagnostics/util.rs`.
  - Exceptions: `system::collect()` takes no args, and `ports` ignores the runner — it does a direct TCP connect to `127.0.0.1` and calls `lsof`/`ss` via `tokio::process` for best-effort process names.

### Adding a new diagnostic category

1. New file under `src/diagnostics/` with a `collect` fn.
2. Add a `Category` variant in `cli.rs` (and `as_str`), plus `label()`, `dispatch()`, and `Category::all()` (update the array length) in `diagnostics/mod.rs`.
3. If it should have a shortcut subcommand, add a `Command` variant in `cli.rs` and a match arm in `main.rs::run`.
4. Any new config section goes in `config.rs`; mirror it in `config.example.toml`.

## Testing

- Unit tests live in `#[cfg(test)]` modules beside the code. Use `command::FakeRunner` (test-only) to program canned responses keyed by `"program arg1 arg2"`; unprogrammed calls return `CommandFailure::NotFound`. See `diagnostics/docker.rs` / `tools.rs` tests for the pattern.
- `tests/cli.rs` cannot import crate internals (there is no lib target), so it re-declares a small copy of the data model in an `inline` module. If you change the JSON shape, status glyphs, or exit-code semantics, update that copy too.
- Tests must not depend on Docker/AWS/kubectl being installed.

## Configuration

Two optional layers, merged as raw TOML tables before deserializing into `Config` (`config::load_layered`): the user config at `~/.config/devdoctor/config.toml` (or `$XDG_CONFIG_HOME`), then a project `devdoctor.toml` found by walking up from the cwd. Nested tables merge; scalars and arrays from the later layer replace. Each file is also validated on its own so parse errors name the offending file. Defaults live in `config.rs` (`DEFAULT_TOOLS`, `DEFAULT_PORTS`, `DEFAULT_TIMEOUT_SECONDS`); see `config.example.toml`. Missing files → defaults; malformed file → error (exit 2). New config fields need serde defaults so partial files keep working.

Output colors honor `--no-color`, `NO_COLOR`, and TTY detection; JSON output must never contain ANSI codes.
