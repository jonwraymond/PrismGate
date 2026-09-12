//! Conservative secret redaction before FTS5 indexing.
//!
//! Applied to overflow titles, previews, and session-event payloads so
//! searchable context never retains common credential patterns. False
//! positives are preferred over leaking a key into `session_search`.

use std::sync::LazyLock;

use regex::Regex;

static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            Regex::new(r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----")
                .expect("pem pattern"),
            "[REDACTED_PEM]",
        ),
        (
            Regex::new(r"\bAKIA[0-9A-Z]{16}\b").expect("aws akia"),
            "[REDACTED_AWS_KEY]",
        ),
        (
            Regex::new(r"\bASIA[0-9A-Z]{16}\b").expect("aws asia"),
            "[REDACTED_AWS_KEY]",
        ),
        (
            Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").expect("github token"),
            "[REDACTED_GITHUB_TOKEN]",
        ),
        (
            Regex::new(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b").expect("github pat"),
            "[REDACTED_GITHUB_TOKEN]",
        ),
        (
            Regex::new(r"\bsk-[A-Za-z0-9]{20,}\b").expect("openai-style key"),
            "[REDACTED_API_KEY]",
        ),
        (
            Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b").expect("slack token"),
            "[REDACTED_SLACK_TOKEN]",
        ),
        (
            Regex::new(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b")
                .expect("jwt"),
            "[REDACTED_JWT]",
        ),
        (
            Regex::new(r"(?i)\b(api[_-]?key|secret|token|password|passwd|authorization|bearer)\b\s*[:=]\s*\S{8,}")
                .expect("assignment"),
            "$1=[REDACTED]",
        ),
    ]
});

/// Replace common credential material with placeholders.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for (re, repl) in PATTERNS.iter() {
        out = re.replace_all(&out, *repl).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_aws_and_openai_keys() {
        let aws = format!("AKIA{}{}", "A".repeat(8), "B".repeat(8));
        let sk = format!("sk-{}", "c".repeat(24));
        let out = redact(&format!("{aws} and {sk}"));
        assert!(!out.contains(&aws));
        assert!(!out.contains(&sk));
        assert!(out.contains("[REDACTED_AWS_KEY]"));
        assert!(out.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn redacts_assignment_style_secrets() {
        let out = redact("password=hunter2secret api_key: supersecretvalue");
        assert!(!out.contains("hunter2secret"));
        assert!(!out.contains("supersecretvalue"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_pem_blocks() {
        let dashes = "-".repeat(5);
        let kind = format!("{}{}", "PRI", "VATE");
        let body = "M".repeat(40);
        let pem =
            format!("{dashes}BEGIN {kind} KEY{dashes}\n{body}\n{dashes}END {kind} KEY{dashes}");
        let out = redact(&pem);
        assert!(!out.contains(&body));
        assert!(out.contains("[REDACTED_PEM]"));
    }

    #[test]
    fn leaves_ordinary_text() {
        assert_eq!(redact("search github issues"), "search github issues");
    }
}
