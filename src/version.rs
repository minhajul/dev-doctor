//! Minimal version requirements for tool checks.
//!
//! Tool versions in the wild are rarely strict SemVer (`go1.22`, `2.51.0.windows.1`,
//! `20`), so instead of the `semver` crate this compares dotted numeric
//! components, padding missing components with zero.
//!
//! Requirement grammar: one or more comma-separated comparators, each an
//! operator (`>=`, `>`, `<=`, `<`, `=`) followed by a version. A bare version
//! means `=`, and `=` matches by prefix: `=1.22` accepts `1.22.0` and `1.22.7`.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A parsed version requirement such as `>=1.22, <2`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VersionReq {
    raw: String,
    comparators: Vec<Comparator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Comparator {
    op: Op,
    version: Vec<u64>,
}

impl VersionReq {
    /// Parse a requirement string.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut comparators = Vec::new();
        for part in raw.split(',') {
            let part = part.trim();
            let (op, rest) = [
                (">=", Op::Ge),
                ("<=", Op::Le),
                (">", Op::Gt),
                ("<", Op::Lt),
                ("=", Op::Eq),
            ]
            .iter()
            .find_map(|(sym, op)| part.strip_prefix(sym).map(|rest| (*op, rest)))
            .unwrap_or((Op::Eq, part));
            let rest = rest.trim();
            let version = parse_components(rest)
                .filter(|_| rest.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .ok_or_else(|| format!("invalid version requirement {raw:?}"))?;
            comparators.push(Comparator { op, version });
        }
        Ok(Self {
            raw: raw.trim().to_string(),
            comparators,
        })
    }

    /// Whether `version` (e.g. `"1.22.3"`) satisfies every comparator.
    /// Returns `None` if `version` has no leading numeric component.
    pub fn matches(&self, version: &str) -> Option<bool> {
        let actual = parse_components(version)?;
        Some(self.comparators.iter().all(|c| c.matches(&actual)))
    }
}

impl Comparator {
    fn matches(&self, actual: &[u64]) -> bool {
        if self.op == Op::Eq {
            return actual
                .iter()
                .chain(std::iter::repeat(&0))
                .zip(&self.version)
                .all(|(a, b)| a == b);
        }
        let ord = compare(actual, &self.version);
        match self.op {
            Op::Ge => ord != Ordering::Less,
            Op::Gt => ord == Ordering::Greater,
            Op::Le => ord != Ordering::Greater,
            Op::Lt => ord == Ordering::Less,
            Op::Eq => unreachable!(),
        }
    }
}

fn compare(a: &[u64], b: &[u64]) -> Ordering {
    let len = a.len().max(b.len());
    (0..len)
        .map(|i| {
            let x = a.get(i).copied().unwrap_or(0);
            let y = b.get(i).copied().unwrap_or(0);
            x.cmp(&y)
        })
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal)
}

/// Leading dot-separated numeric components: `"1.22.3-rc1"` → `[1, 22, 3]`.
fn parse_components(s: &str) -> Option<Vec<u64>> {
    let mut out = Vec::new();
    for part in s.split('.') {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            break;
        }
        out.push(digits.parse().ok()?);
        if digits.len() != part.len() {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl TryFrom<String> for VersionReq {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<VersionReq> for String {
    fn from(req: VersionReq) -> Self {
        req.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(req: &str, v: &str) -> bool {
        VersionReq::parse(req)
            .expect("parse")
            .matches(v)
            .expect("version")
    }

    #[test]
    fn comparison_operators() {
        assert!(ok(">=20", "20.0.0"));
        assert!(ok(">=20", "22.1.0"));
        assert!(!ok(">=20", "18.19.1"));
        assert!(ok(">1.2", "1.2.1"));
        assert!(!ok(">1.2", "1.2.0"));
        assert!(ok("<2", "1.99"));
        assert!(!ok("<2", "2.0.0"));
        assert!(ok("<=2", "2.0.0"));
    }

    #[test]
    fn exact_matches_by_prefix() {
        assert!(ok("=1.22", "1.22.7"));
        assert!(ok("1.22", "1.22"));
        assert!(!ok("1.22", "1.23.0"));
        assert!(!ok("=1.22.3", "1.22"));
    }

    #[test]
    fn ranges_require_all_comparators() {
        assert!(ok(">=1.22, <2", "1.24.0"));
        assert!(!ok(">=1.22, <2", "2.0.0"));
    }

    #[test]
    fn suffixes_are_ignored() {
        assert!(ok(">=2.40", "2.51.0.windows.1"));
        assert!(ok(">=1.0", "1.0.0-rc1"));
    }

    #[test]
    fn invalid_requirements_are_rejected() {
        assert!(VersionReq::parse("").is_err());
        assert!(VersionReq::parse(">=abc").is_err());
        assert!(VersionReq::parse("~1.2").is_err());
        assert!(VersionReq::parse(">=1.x").is_err());
    }

    #[test]
    fn non_numeric_version_is_none() {
        let req = VersionReq::parse(">=1").expect("parse");
        assert_eq!(req.matches("unknown"), None);
    }
}
