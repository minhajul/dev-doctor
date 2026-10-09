//! Command-line argument parsing.
//!
//! Uses `clap` derive to build the top-level CLI surface.

use clap::{Parser, Subcommand, ValueEnum};

/// Top-level CLI arguments.
#[derive(Debug, Parser)]
#[command(
    name = "devdoctor",
    version,
    about = "Developer environment diagnostics — inspect your local dev stack.",
    long_about = "devdoctor inspects the local development environment and reports on common \
                  dependencies (Docker, Kubernetes, AWS, ports, languages, ...). It is read-only \
                  and safe to run in CI.",
    after_help = "Examples:\n  \
                  devdoctor\n  \
                  devdoctor check --category tools\n  \
                  devdoctor docker\n  \
                  devdoctor check --json"
)]
pub struct Cli {
    /// Optional subcommand. When omitted, `check` is run.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run diagnostics across all (or selected) categories.
    ///
    /// This is the default command when none is provided.
    #[command(visible_alias = "c")]
    Check {
        /// Restrict diagnostics to a single category.
        #[arg(long, value_enum)]
        category: Option<Category>,

        /// Emit the result as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Run only the development-tools diagnostics.
    #[command(visible_alias = "t")]
    Tools,

    /// Run only the Docker diagnostics.
    #[command(visible_alias = "d")]
    Docker,

    /// Run only the Kubernetes diagnostics.
    #[command(visible_alias = "k")]
    Kubernetes,

    /// Run only the AWS diagnostics.
    #[command(visible_alias = "a")]
    Aws,

    /// Run only the port diagnostics.
    #[command(visible_alias = "p")]
    Ports,

    /// Run only the required environment variable diagnostics.
    #[command(visible_alias = "e")]
    Env,

    /// Print version information.
    Version,
}

/// Diagnostic categories selectable from the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum Category {
    System,
    Tools,
    Docker,
    Kubernetes,
    Aws,
    Ports,
    Env,
}

#[allow(dead_code)]
impl Category {
    /// String identifier used internally and in JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::System => "system",
            Category::Tools => "tools",
            Category::Docker => "docker",
            Category::Kubernetes => "kubernetes",
            Category::Aws => "aws",
            Category::Ports => "ports",
            Category::Env => "env",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn defaults_to_no_subcommand() {
        let cli = Cli::try_parse_from(["devdoctor"]).expect("parse");
        assert!(cli.command.is_none());
        assert!(!cli.no_color);
    }

    #[test]
    fn parses_check_subcommand() {
        let cli = Cli::try_parse_from(["devdoctor", "check"]).expect("parse");
        match cli.command.unwrap() {
            Command::Check { category, json } => {
                assert!(category.is_none());
                assert!(!json);
            }
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn parses_check_with_category_and_json() {
        let cli = Cli::try_parse_from(["devdoctor", "check", "--category", "tools", "--json"])
            .expect("parse");
        match cli.command.unwrap() {
            Command::Check { category, json } => {
                assert_eq!(category, Some(Category::Tools));
                assert!(json);
            }
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn parses_shortcut_subcommands() {
        let cli = Cli::try_parse_from(["devdoctor", "docker"]).expect("parse");
        assert!(matches!(cli.command.unwrap(), Command::Docker));
    }

    #[test]
    fn no_color_flag_is_global() {
        let cli = Cli::try_parse_from(["devdoctor", "--no-color", "tools"]).expect("parse");
        assert!(cli.no_color);
    }

    #[test]
    fn unknown_category_fails() {
        let result = Cli::try_parse_from(["devdoctor", "check", "--category", "bogus"]);
        assert!(result.is_err());
    }

    #[test]
    fn category_as_str_matches_known_values() {
        assert_eq!(Category::System.as_str(), "system");
        assert_eq!(Category::Tools.as_str(), "tools");
        assert_eq!(Category::Docker.as_str(), "docker");
        assert_eq!(Category::Kubernetes.as_str(), "kubernetes");
        assert_eq!(Category::Aws.as_str(), "aws");
        assert_eq!(Category::Ports.as_str(), "ports");
        assert_eq!(Category::Env.as_str(), "env");
    }

    #[test]
    fn version_subcommand_parses() {
        let cli = Cli::try_parse_from(["devdoctor", "version"]).expect("parse");
        assert!(matches!(cli.command.unwrap(), Command::Version));
    }
}
