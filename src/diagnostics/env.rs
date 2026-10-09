//! Environment variable diagnostics.
//!
//! Checks that each variable in `env.required` is set. Values are never
//! read into the report: only "set", "set but empty", or "not set".

use std::ffi::OsString;
use std::sync::Arc;

use crate::config::Config;
use crate::models::{Diagnostic, DiagnosticGroup};

/// Collect diagnostics for every required environment variable.
pub async fn collect(config: Arc<Config>) -> DiagnosticGroup {
    collect_with(&config.env.required, |name| std::env::var_os(name))
}

fn collect_with(required: &[String], lookup: impl Fn(&str) -> Option<OsString>) -> DiagnosticGroup {
    let mut group = DiagnosticGroup::new("Environment");
    for name in required {
        let diag = match lookup(name) {
            Some(v) if v.is_empty() => Diagnostic::warning(name, "set but empty"),
            Some(_) => Diagnostic::healthy(name, "set"),
            None => Diagnostic::failed(name, "not set"),
        };
        group.push(diag);
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Status;

    #[test]
    fn reports_set_empty_and_missing_without_values() {
        let required = vec![
            "SET".to_string(),
            "EMPTY".to_string(),
            "MISSING".to_string(),
        ];
        let group = collect_with(&required, |name| match name {
            "SET" => Some("hunter2".into()),
            "EMPTY" => Some("".into()),
            _ => None,
        });

        let statuses: Vec<_> = group.diagnostics.iter().map(|d| d.status).collect();
        assert_eq!(
            statuses,
            vec![Status::Healthy, Status::Warning, Status::Failed]
        );
        assert!(group
            .diagnostics
            .iter()
            .all(|d| !d.message.contains("hunter2")));
    }
}
