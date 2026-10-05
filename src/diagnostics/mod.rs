//! Diagnostic modules.
//!
//! Each submodule implements a single [`DiagnosticGroup`] and exposes a
//! `pub async fn collect(...) -> DiagnosticGroup` function. The functions
//! take a shared [`SharedRunner`] and the loaded [`Config`], and never
//! panic on missing external tools.

pub mod aws;
pub mod docker;
pub mod kubernetes;
pub mod ports;
pub mod system;
pub mod tools;
pub(crate) mod util;

use std::sync::Arc;

use crate::cli::Category;
use crate::command::SharedRunner;
use crate::config::Config;
use crate::models::DiagnosticGroup;

impl Category {
    /// Human-readable label used as the group name in reports.
    pub fn label(self) -> &'static str {
        match self {
            Category::System => "System",
            Category::Tools => "Tools",
            Category::Docker => "Docker",
            Category::Kubernetes => "Kubernetes",
            Category::Aws => "AWS",
            Category::Ports => "Ports",
        }
    }

    /// Dispatch a single category to its collecting function.
    pub async fn dispatch(self, runner: SharedRunner, config: Arc<Config>) -> DiagnosticGroup {
        match self {
            Category::System => system::collect().await,
            Category::Tools => tools::collect(runner, config).await,
            Category::Docker => docker::collect(runner, config).await,
            Category::Kubernetes => kubernetes::collect(runner, config).await,
            Category::Aws => aws::collect(runner, config).await,
            Category::Ports => ports::collect(runner, config).await,
        }
    }
}

/// Fan out across the requested categories concurrently.
///
/// Independent categories (e.g. Docker, AWS, Kubernetes, ports) run in
/// parallel via `tokio::spawn`.
pub async fn run_categories(
    runner: SharedRunner,
    config: Arc<Config>,
    categories: &[Category],
) -> Vec<DiagnosticGroup> {
    let order: Vec<Category> = if categories.is_empty() {
        Category::all().to_vec()
    } else {
        Category::all()
            .into_iter()
            .filter(|c| categories.contains(c))
            .collect()
    };

    let mut handles = Vec::with_capacity(order.len());
    for cat in order {
        let runner = runner.clone();
        let config = config.clone();
        handles.push((cat, tokio::spawn(cat.dispatch(runner, config))));
    }

    let mut groups = Vec::with_capacity(handles.len());
    for (cat, handle) in handles {
        // A panic inside a spawned task is a real bug; surface it as a
        // synthetic failed diagnostic rather than crashing the whole CLI.
        match handle.await {
            Ok(group) => groups.push(group),
            Err(e) => {
                let mut g = DiagnosticGroup::new(cat.label());
                g.push(crate::models::Diagnostic::failed(
                    "internal",
                    format!("diagnostic panicked: {e}"),
                ));
                groups.push(g);
            }
        }
    }
    groups
}

/// Canonical execution order. Used both as the default fan-out order and
/// when filtering a user-supplied category list.
impl Category {
    pub fn all() -> [Category; 6] {
        [
            Category::System,
            Category::Tools,
            Category::Docker,
            Category::Kubernetes,
            Category::Aws,
            Category::Ports,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_labels_are_stable() {
        assert_eq!(Category::System.label(), "System");
        assert_eq!(Category::Tools.label(), "Tools");
        assert_eq!(Category::Docker.label(), "Docker");
        assert_eq!(Category::Kubernetes.label(), "Kubernetes");
        assert_eq!(Category::Aws.label(), "AWS");
        assert_eq!(Category::Ports.label(), "Ports");
    }

    #[test]
    fn category_all_includes_every_variant() {
        assert_eq!(Category::all().len(), 6);
    }
}
