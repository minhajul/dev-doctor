//! devdoctor — developer environment diagnostics CLI.

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;

mod cli;
mod command;
mod config;
mod diagnostics;
mod models;
mod output;
mod version;

use cli::{Category, Cli, Command};
use command::default_runner;
use config::load_default;
use diagnostics::run_categories;
use models::Report;
use output::{render_json, render_text, ColorMode};

fn main() -> ExitCode {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = writeln!(io::stderr(), "devdoctor: internal error: {info}");
        default_hook(info);
    }));

    let cli = Cli::parse();

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = writeln!(io::stderr(), "devdoctor: could not start runtime: {e}");
            return ExitCode::from(2);
        }
    };

    match rt.block_on(run(cli)) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(io::stderr(), "devdoctor: {e:#}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let config = Arc::new(load_default().map_err(|e| anyhow::anyhow!("{e}"))?);
    let color = if cli.no_color {
        ColorMode::Never
    } else {
        ColorMode::Auto
    };
    let runner = default_runner();

    let (groups, json) = match cli.command {
        None => (run_categories(runner, config, &[]).await, false),
        Some(Command::Check { category, json }) => {
            let groups = match category {
                Some(cat) => vec![cat.dispatch(runner, config).await],
                None => run_categories(runner, config, &[]).await,
            };
            (groups, json)
        }
        Some(Command::Version) => {
            println!("devdoctor {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::from(0));
        }
        Some(Command::Tools) => (vec![Category::Tools.dispatch(runner, config).await], false),
        Some(Command::Docker) => (vec![Category::Docker.dispatch(runner, config).await], false),
        Some(Command::Kubernetes) => (
            vec![Category::Kubernetes.dispatch(runner, config).await],
            false,
        ),
        Some(Command::Aws) => (vec![Category::Aws.dispatch(runner, config).await], false),
        Some(Command::Ports) => (vec![Category::Ports.dispatch(runner, config).await], false),
        Some(Command::Env) => (vec![Category::Env.dispatch(runner, config).await], false),
    };

    let report = Report::from_groups(groups);

    let stdout = io::stdout();
    let mut out = stdout.lock();
    if json {
        render_json(&report, &mut out)?;
    } else {
        render_text(&report, &mut out, color)?;
    }

    Ok(ExitCode::from(report.exit_code() as u8))
}
