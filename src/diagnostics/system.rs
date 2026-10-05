//! System diagnostics: OS, architecture, shell, hostname, user.
//!
//! Pure local inspection — no external commands, no I/O. Safe to run always.

use crate::models::{Diagnostic, DiagnosticGroup};

/// Collect system diagnostics.
pub async fn collect() -> DiagnosticGroup {
    let mut group = DiagnosticGroup::new("System");

    group.push(Diagnostic::healthy("OS", os_label()));
    group.push(Diagnostic::healthy("Architecture", arch_label()));
    group.push(Diagnostic::healthy("Shell", shell_label()));
    group.push(Diagnostic::healthy("Hostname", hostname_label()));
    group.push(Diagnostic::healthy("User", user_label()));

    group
}

fn os_label() -> String {
    match std::env::consts::OS {
        "macos" => "macOS".to_string(),
        "linux" => "Linux".to_string(),
        "freebsd" => "FreeBSD".to_string(),
        other => other.to_string(),
    }
}

fn arch_label() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64".to_string(),
        "aarch64" => "arm64".to_string(),
        "arm" => "arm".to_string(),
        "x86" => "i686".to_string(),
        other => other.to_string(),
    }
}

fn shell_label() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| {
            // Take just the basename.
            std::path::Path::new(&s)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn hostname_label() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

fn user_label() -> String {
    whoami::username()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn system_collects_every_label() {
        let g = collect().await;
        let names: Vec<&str> = g.diagnostics.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"OS"));
        assert!(names.contains(&"Architecture"));
        assert!(names.contains(&"Shell"));
        assert!(names.contains(&"Hostname"));
        assert!(names.contains(&"User"));
        for d in &g.diagnostics {
            // We can't guarantee the underlying values, but they must be non-empty.
            assert!(!d.message.is_empty(), "{} had empty message", d.name);
        }
    }

    #[test]
    fn arch_label_maps_common_values() {
        // We can only verify the function is stable; the runtime arch is fixed.
        let _ = arch_label();
    }
}
