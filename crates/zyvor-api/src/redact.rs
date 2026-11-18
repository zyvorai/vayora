//! Best-effort redaction of secrets and PAN-like values in log lines. Secrets should still never be
//! logged by construction; this is a second line of defence when a field slips through.

use std::io::{self, Write};
use std::sync::Mutex;

use tracing_subscriber::fmt::MakeWriter;

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

/// Replace the run of key characters that follows each case-insensitive prefix. Steps by whole
/// characters, so a multi-byte character in the line (an ellipsis, Hindi, Odia) is copied through
/// unchanged and never sliced.
fn redact_prefixed(s: &str, prefixes: &[&str]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let hit = prefixes.iter().find(|p| {
            rest.is_char_boundary(p.len().min(rest.len()))
                && rest.len() >= p.len()
                && rest[..p.len()].eq_ignore_ascii_case(p)
        });
        if let Some(p) = hit {
            out.push_str(&rest[..p.len()]);
            out.push_str("[REDACTED]");
            i += p.len();
            while let Some(c) = s[i..].chars().next() {
                if !is_key_char(c) {
                    break;
                }
                i += c.len_utf8();
            }
            continue;
        }
        let Some(c) = rest.chars().next() else { break };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn redact_bearer(s: &str) -> String {
    redact_prefixed(s, &["bearer "])
}

fn redact_pans(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut digits = String::new();
            let mut j = i;
            while j < chars.len() {
                let c = chars[j];
                if c.is_ascii_digit() {
                    digits.push(c);
                    j += 1;
                } else if (c == ' ' || c == '-')
                    && j + 1 < chars.len()
                    && chars[j + 1].is_ascii_digit()
                {
                    j += 1;
                } else {
                    break;
                }
            }
            if (13..=19).contains(&digits.len()) {
                out.push_str("****");
                out.push_str(&digits[digits.len() - 4..]);
                i = j;
                continue;
            }
            out.push(chars[start]);
            i = start + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn redact_vpas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut a = out.len();
            while a > 0 {
                let c = out.as_bytes()[a - 1] as char;
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    a -= 1;
                } else {
                    break;
                }
            }
            let local_len = out.len() - a;
            let mut b = i + 1;
            while b < chars.len() && chars[b].is_ascii_alphabetic() {
                b += 1;
            }
            let handle: String = chars[i + 1..b].iter().collect();
            // UPI handles are alphabetic bank codes (okhdfcbank, ybl), not email domains.
            if local_len >= 2 && handle.len() >= 2 && b < chars.len().saturating_add(1) {
                let boundary_ok =
                    b == chars.len() || !(chars[b].is_ascii_alphanumeric() || chars[b] == '.');
                if boundary_ok {
                    out.truncate(a);
                    out.push_str("[REDACTED]@");
                    out.push_str(&handle);
                    i = b;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Replace common secret / card / VPA-like patterns in a log line.
pub fn redact_text(s: &str) -> String {
    let s = redact_prefixed(
        s,
        &[
            "zk_test_",
            "zk_live_",
            "rzp_test_",
            "rzp_live_",
            "sk_test_",
            "sk_live_",
            "whsec_",
        ],
    );
    let s = redact_bearer(&s);
    let s = redact_pans(&s);
    redact_vpas(&s)
}

/// `MakeWriter` that redacts every line before it reaches the underlying sink.
pub struct RedactingMakeWriter<W> {
    inner: Mutex<W>,
}

impl<W> RedactingMakeWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}

impl<'a, W> MakeWriter<'a> for RedactingMakeWriter<W>
where
    W: Write + Send + 'a,
{
    type Writer = Guard<'a, W>;

    fn make_writer(&'a self) -> Self::Writer {
        Guard {
            inner: &self.inner,
            buf: Vec::new(),
        }
    }
}

pub struct Guard<'a, W> {
    inner: &'a Mutex<W>,
    buf: Vec<u8>,
}

impl<W: Write> Write for Guard<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line = self.buf.drain(..=i).collect::<Vec<_>>();
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            let text = String::from_utf8_lossy(&line);
            let redacted = redact_text(&text);
            let mut g = self.inner.lock().expect("redact writer");
            g.write_all(redacted.as_bytes())?;
            g.write_all(b"\n")?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let text = String::from_utf8_lossy(&self.buf);
            let redacted = redact_text(&text);
            self.buf.clear();
            let mut g = self.inner.lock().expect("redact writer");
            g.write_all(redacted.as_bytes())?;
        }
        self.inner.lock().expect("redact writer").flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_keys_pans_and_vpas() {
        assert!(redact_text("zk_test_abc123XYZ").contains("[REDACTED]"));
        assert!(!redact_text("zk_test_abc123XYZ").contains("abc123"));
        assert!(redact_text("Authorization: Bearer tok_secret_value").contains("[REDACTED]"));
        assert_eq!(
            redact_text("card 4111 1111 1111 1111 ok"),
            "card ****1111 ok"
        );
        assert_eq!(
            redact_text("pay shop@okhdfcbank"),
            "pay [REDACTED]@okhdfcbank"
        );
        assert_eq!(redact_text("plain log line"), "plain log line");
    }
}

#[cfg(test)]
mod utf8_tests {
    use super::*;

    #[test]
    fn multibyte_text_is_copied_through_and_never_panics() {
        // the real crash: a truncated SQL statement ending in an ellipsis in a slow-query warning
        let ellipsis = "SELECT * FROM payments WHERE note = 'x' AND memo = '".to_string()
            + &"a".repeat(150)
            + "…";
        assert_eq!(redact_text(&ellipsis), ellipsis);
        let hindi = "भुगतान सफल रहा ₹५०० — धन्यवाद";
        assert_eq!(redact_text(hindi), hindi);
        let odia = "ଟଙ୍କା ଦେଇଛନ୍ତି";
        assert_eq!(redact_text(odia), odia);
    }

    #[test]
    fn secrets_next_to_multibyte_text_are_still_redacted() {
        let out = redact_text("key=rzp_live_abc123XYZ… done ✓ Bearer tok_en-9 ଟଙ୍କା");
        assert!(!out.contains("abc123XYZ"), "{out}");
        assert!(!out.contains("tok_en-9"), "{out}");
        assert!(out.contains("rzp_live_[REDACTED]"), "{out}");
        assert!(out.contains("Bearer [REDACTED]"), "{out}");
        assert!(out.contains("… done ✓"), "{out}");
        assert!(out.contains("ଟଙ୍କା"), "{out}");
    }

    #[test]
    fn prefixes_are_case_insensitive_and_a_short_tail_is_safe() {
        assert_eq!(redact_text("WHSEC_AbC"), "WHSEC_[REDACTED]");
        assert_eq!(redact_text("bearer "), "bearer [REDACTED]");
        assert_eq!(redact_text("whsec"), "whsec");
        assert_eq!(redact_text("é"), "é");
    }
}
