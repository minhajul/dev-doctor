//! Small shared helpers used by multiple diagnostic modules.

use crate::command::{CommandFailure, CommandOutput};

/// Pull a leading version substring out of a single token.
///
/// Handles: `"2.51.0"`, `"v2.51.0"`, `"go1.22.3"`, `"aws-cli/2.31.0"`,
/// `"2.31.0-Python"`. Returns `None` if the token contains no digit.
pub(crate) fn extract_leading_version(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    let mut i = 0;

    // Skip a non-digit prefix made of allowed name characters.
    while i < bytes.len() && !bytes[i].is_ascii_digit() {
        let c = bytes[i];
        if !(c.is_ascii_alphabetic() || matches!(c, b'-' | b'_' | b'.' | b'/' | b'+')) {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let digit_start = i;

    // Walk the version proper: digits, then `.` separated digit groups,
    // then an optional pre-release/build suffix (`.` `-` `_` `+` and digits).
    // We stop at the first character that is not part of a SemVer-ish version.
    let mut last_good_end = i;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit() {
            i += 1;
            last_good_end = i;
        } else if matches!(c, b'.' | b'-' | b'_' | b'+') {
            i += 1;
        } else {
            break;
        }
    }

    Some(token[digit_start..last_good_end].to_string())
}

/// A hint for failures that mean the binary itself is unusable, independent
/// of which tool it is. Currently: a binary built for another CPU
/// architecture (macOS "Bad CPU type", os error 86; Linux "Exec format
/// error", os error 8).
pub(crate) fn failure_hint(err: &CommandFailure) -> Option<String> {
    let CommandFailure::Io(msg) = err else {
        return None;
    };
    if !(msg.contains("os error 86") || msg.contains("os error 8)")) {
        return None;
    }
    let mut hint = format!(
        "binary was built for another CPU architecture; reinstall a native {} build",
        std::env::consts::ARCH
    );
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        hint.push_str(" or install Rosetta: `softwareupdate --install-rosetta`");
    }
    Some(hint)
}

/// Walk whitespace-separated tokens and return the first leading version.
pub(crate) fn first_version_token(s: &str) -> Option<String> {
    for line in s.lines() {
        for tok in line.split_whitespace() {
            if let Some(v) = extract_leading_version(tok) {
                return Some(v);
            }
        }
    }
    None
}

/// First non-blank line of `s`, trimmed.
pub(crate) fn first_nonempty_line(s: &str) -> Option<String> {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// Count non-blank lines in `s`.
pub(crate) fn count_nonblank_lines(s: &str) -> usize {
    s.lines().filter(|l| !l.trim().is_empty()).count()
}

/// Trim `CommandOutput::stdout` and treat empty as `None`.
pub(crate) fn trimmed_stdout(out: &CommandOutput) -> Option<String> {
    let s = out.stdout.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_leading_version_rules() {
        assert_eq!(extract_leading_version("2.51.0").as_deref(), Some("2.51.0"));
        assert_eq!(
            extract_leading_version("v2.51.0").as_deref(),
            Some("2.51.0")
        );
        assert_eq!(
            extract_leading_version("go1.22.3").as_deref(),
            Some("1.22.3")
        );
        assert_eq!(
            extract_leading_version("aws-cli/2.31.0").as_deref(),
            Some("2.31.0")
        );
        assert_eq!(
            extract_leading_version("2.31.0-Python").as_deref(),
            Some("2.31.0")
        );
        assert_eq!(extract_leading_version("hello"), None);
        assert_eq!(extract_leading_version(""), None);
        assert_eq!(extract_leading_version("vfoo"), None);
    }

    #[test]
    fn first_version_token_finds_first_match() {
        assert_eq!(
            first_version_token("Docker version 28.1.0, build abc").as_deref(),
            Some("28.1.0")
        );
        assert_eq!(
            first_version_token("Client Version: v1.34.0\nServer Version: v1.34.0").as_deref(),
            Some("1.34.0")
        );
        assert_eq!(first_version_token("no version"), None);
    }

    #[test]
    fn failure_hint_detects_wrong_architecture() {
        let mac = CommandFailure::Io("Bad CPU type in executable (os error 86)".into());
        let linux = CommandFailure::Io("Exec format error (os error 8)".into());
        assert!(failure_hint(&mac).unwrap().contains("CPU architecture"));
        assert!(failure_hint(&linux).is_some());
        assert!(failure_hint(&CommandFailure::Io(
            "Permission denied (os error 13)".into()
        ))
        .is_none());
        assert!(failure_hint(&CommandFailure::Io("weird (os error 80)".into())).is_none());
        assert!(failure_hint(&CommandFailure::Timeout).is_none());
    }

    #[test]
    fn first_nonempty_line_trims() {
        assert_eq!(first_nonempty_line(""), None);
        assert_eq!(first_nonempty_line("\n\n"), None);
        assert_eq!(
            first_nonempty_line("\n  hello world  \nbye").as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn count_nonblank_lines_works() {
        assert_eq!(count_nonblank_lines(""), 0);
        assert_eq!(count_nonblank_lines("\n\n"), 0);
        assert_eq!(count_nonblank_lines("a\nb\nc"), 3);
        assert_eq!(count_nonblank_lines("a\n\n  \nb"), 2);
    }

    #[test]
    fn trimmed_stdout_handles_empty() {
        let out = CommandOutput {
            stdout: "  hello  \n".into(),
            stderr: String::new(),
        };
        assert_eq!(trimmed_stdout(&out).as_deref(), Some("hello"));

        let empty = CommandOutput {
            stdout: "   \n".into(),
            stderr: String::new(),
        };
        assert_eq!(trimmed_stdout(&empty), None);
    }
}
