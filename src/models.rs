//! Diagnostic data model.
//!
//! Diagnostics are small, composable units of output. Each diagnostic has a
//! name, a status (Healthy / Warning / Failed), and a human-readable message.
//! Diagnostics are grouped under a [`DiagnosticGroup`] which is the unit
//! rendered by the CLI.

use serde::{Deserialize, Serialize};

/// Lifecycle status of a single diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The check passed cleanly.
    Healthy,
    /// The check passed but the result is suspicious or non-ideal.
    Warning,
    /// The check failed.
    Failed,
}

impl Status {
    /// Single-character glyph used in the terminal report.
    pub fn glyph(self) -> &'static str {
        match self {
            Status::Healthy => "✓",
            Status::Warning => "⚠",
            Status::Failed => "✗",
        }
    }

    /// Short label used in tests and JSON.
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Healthy => "healthy",
            Status::Warning => "warning",
            Status::Failed => "failed",
        }
    }
}

/// A single diagnostic result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub name: String,
    pub status: Status,
    pub message: String,
}

impl Diagnostic {
    /// Construct a healthy diagnostic.
    pub fn healthy(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Healthy,
            message: message.into(),
        }
    }

    /// Construct a warning diagnostic.
    pub fn warning(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Warning,
            message: message.into(),
        }
    }

    /// Construct a failed diagnostic.
    pub fn failed(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Failed,
            message: message.into(),
        }
    }
}

/// A named collection of diagnostics (e.g. "Tools", "Docker").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticGroup {
    pub name: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticGroup {
    /// Create an empty group.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            diagnostics: Vec::new(),
        }
    }

    /// Push a diagnostic into the group.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Return the number of healthy diagnostics in the group.
    pub fn healthy_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.status == Status::Healthy)
            .count()
    }

    /// Return the number of warning diagnostics in the group.
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.status == Status::Warning)
            .count()
    }

    /// Return the number of failed diagnostics in the group.
    pub fn failed_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.status == Status::Failed)
            .count()
    }
}

/// Aggregate summary across all groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Summary {
    pub healthy: usize,
    pub warnings: usize,
    pub failed: usize,
}

impl Summary {
    /// Compute the summary from a slice of groups.
    pub fn from_groups(groups: &[DiagnosticGroup]) -> Self {
        let mut s = Summary::default();
        for g in groups {
            s.healthy += g.healthy_count();
            s.warnings += g.warning_count();
            s.failed += g.failed_count();
        }
        s
    }

    /// Exit code recommended by the summary.
    pub fn exit_code(&self) -> i32 {
        if self.failed > 0 {
            1
        } else {
            0
        }
    }
}

/// Top-level report containing every group plus the summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub groups: Vec<DiagnosticGroup>,
    pub summary: Summary,
}

impl Report {
    /// Build a report from a list of groups.
    pub fn from_groups(groups: Vec<DiagnosticGroup>) -> Self {
        let summary = Summary::from_groups(&groups);
        Self { groups, summary }
    }

    /// Recommended process exit code for this report.
    pub fn exit_code(&self) -> i32 {
        self.summary.exit_code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_glyphs_are_distinct() {
        assert_eq!(Status::Healthy.glyph(), "✓");
        assert_eq!(Status::Warning.glyph(), "⚠");
        assert_eq!(Status::Failed.glyph(), "✗");
    }

    #[test]
    fn status_as_str_matches_serde() {
        assert_eq!(Status::Healthy.as_str(), "healthy");
        assert_eq!(Status::Warning.as_str(), "warning");
        assert_eq!(Status::Failed.as_str(), "failed");
    }

    #[test]
    fn summary_counts_all_statuses() {
        let mut g = DiagnosticGroup::new("g");
        g.push(Diagnostic::healthy("a", "ok"));
        g.push(Diagnostic::warning("b", "warn"));
        g.push(Diagnostic::failed("c", "no"));

        let s = Summary::from_groups(&[g]);
        assert_eq!(s.healthy, 1);
        assert_eq!(s.warnings, 1);
        assert_eq!(s.failed, 1);
    }

    #[test]
    fn exit_code_is_zero_when_no_failures() {
        let mut g = DiagnosticGroup::new("g");
        g.push(Diagnostic::healthy("a", "ok"));
        g.push(Diagnostic::warning("b", "warn"));

        let s = Summary::from_groups(&[g]);
        assert_eq!(s.exit_code(), 0);
    }

    #[test]
    fn exit_code_is_one_when_any_failure() {
        let mut g = DiagnosticGroup::new("g");
        g.push(Diagnostic::healthy("a", "ok"));
        g.push(Diagnostic::failed("c", "no"));

        let s = Summary::from_groups(&[g]);
        assert_eq!(s.exit_code(), 1);
    }

    #[test]
    fn report_serializes_round_trip() {
        let mut g = DiagnosticGroup::new("tools");
        g.push(Diagnostic::healthy("git", "2.51.0"));
        g.push(Diagnostic::failed("docker", "not found"));

        let report = Report::from_groups(vec![g]);
        let json = serde_json::to_string(&report).expect("serialize");
        let back: Report = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, report);
    }
}
