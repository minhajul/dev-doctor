//! Terminal and JSON output rendering for [`Report`].
//!
//! Honors `NO_COLOR` and the `--no-color` flag by falling back to plain text.

use std::io::{self, Write};

use owo_colors::{OwoColorize, Stream::Stdout};

use crate::models::{Report, Status};

/// Color preference for the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Never,
}

/// Render a [`Report`] to the given writer as human-readable text.
pub fn render_text<W: Write>(report: &Report, out: &mut W, mode: ColorMode) -> io::Result<()> {
    let use_color = match mode {
        ColorMode::Never => false,
        ColorMode::Auto => std::io::IsTerminal::is_terminal(&io::stdout()) && env_allows_color(),
    };

    // Header
    if use_color {
        writeln!(out, "{}", "Developer Environment".bold().underline())?;
    } else {
        writeln!(out, "Developer Environment")?;
        writeln!(out, "----------------------")?;
    }
    writeln!(out)?;

    for group in &report.groups {
        // Group title
        if use_color {
            writeln!(out, "{}", group.name.bold())?;
        } else {
            writeln!(out, "{}", group.name)?;
        }

        let width = group
            .diagnostics
            .iter()
            .map(|d| d.name.len())
            .max()
            .unwrap_or(0);

        for diag in &group.diagnostics {
            let glyph = diag.status.glyph();
            let colored_glyph = if use_color {
                match diag.status {
                    Status::Healthy => glyph.if_supports_color(Stdout, |s| s.green()).to_string(),
                    Status::Warning => glyph.if_supports_color(Stdout, |s| s.yellow()).to_string(),
                    Status::Failed => glyph.if_supports_color(Stdout, |s| s.red()).to_string(),
                }
            } else {
                glyph.to_string()
            };

            let name = format!("{:<width$}", diag.name, width = width);
            let colored_name = if use_color {
                name.if_supports_color(Stdout, |s| s.bold()).to_string()
            } else {
                name
            };
            let colored_msg = if use_color {
                match diag.status {
                    Status::Failed => diag
                        .message
                        .if_supports_color(Stdout, |s| s.red())
                        .to_string(),
                    Status::Warning => diag
                        .message
                        .if_supports_color(Stdout, |s| s.yellow())
                        .to_string(),
                    Status::Healthy => diag.message.clone(),
                }
            } else {
                diag.message.clone()
            };

            writeln!(out, "{} {}  {}", colored_glyph, colored_name, colored_msg)?;
        }
        writeln!(out)?;
    }

    // Summary
    if use_color {
        writeln!(out, "{}", "Summary".bold())?;
    } else {
        writeln!(out, "Summary")?;
        writeln!(out, "-------")?;
    }
    writeln!(
        out,
        "Healthy: {}    Warnings: {}    Failed: {}",
        report.summary.healthy, report.summary.warnings, report.summary.failed
    )?;

    Ok(())
}

/// Render a [`Report`] as compact JSON.
pub fn render_json<W: Write>(report: &Report, out: &mut W) -> io::Result<()> {
    let json = serde_json::to_string_pretty(report).map_err(io::Error::other)?;
    writeln!(out, "{json}")?;
    Ok(())
}

fn env_allows_color() -> bool {
    if let Ok(v) = std::env::var("NO_COLOR") {
        if !v.is_empty() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Diagnostic, DiagnosticGroup};

    fn sample_report() -> Report {
        let mut g = DiagnosticGroup::new("Tools");
        g.push(Diagnostic::healthy("git", "2.51.0"));
        g.push(Diagnostic::warning("redis", "not reachable"));
        g.push(Diagnostic::failed("postgres", "not found"));
        Report::from_groups(vec![g])
    }

    #[test]
    fn text_output_contains_all_diagnostics_and_summary() {
        let r = sample_report();
        let mut buf = Vec::new();
        render_text(&r, &mut buf, ColorMode::Never).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("Developer Environment"));
        assert!(out.contains("Tools"));
        assert!(out.contains("git"));
        assert!(out.contains("2.51.0"));
        assert!(out.contains("redis"));
        assert!(out.contains("postgres"));
        assert!(out.contains("Summary"));
        assert!(out.contains("Healthy: 1"));
        assert!(out.contains("Warnings: 1"));
        assert!(out.contains("Failed: 1"));
    }

    #[test]
    fn json_output_is_machine_readable_and_no_colors() {
        let r = sample_report();
        let mut buf = Vec::new();
        render_json(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("\"groups\""));
        assert!(s.contains("\"summary\""));
        assert!(s.contains("\"healthy\": 1"));
        assert!(s.contains("\"warnings\": 1"));
        assert!(s.contains("\"failed\": 1"));
        // Sanity: no ANSI escape codes.
        assert!(!s.contains('\u{1b}'));
    }

    #[test]
    fn text_output_in_auto_mode_does_not_panic_when_redirected() {
        // When running under cargo test, stdout is not a tty, so this exercises
        // the "no color" branch even in Auto mode.
        let r = sample_report();
        let mut buf = Vec::new();
        render_text(&r, &mut buf, ColorMode::Auto).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("Summary"));
    }
}
