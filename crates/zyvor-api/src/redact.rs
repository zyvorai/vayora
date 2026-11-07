//! Best-effort redaction of secrets and PAN-like values in log lines. Secrets should still never be
//! logged by construction; this is a second line of defence when a field slips through.

use std::io::{self, Write};
use std::sync::Mutex;

use tracing_subscriber::fmt::MakeWriter;

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

fn redact_prefixed(s: &str, prefixes: &[&str]) -> String {
    let mut out = String::with_capacity(s.len());
    let lower = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let mut hit = None;
        for p in prefixes {
            let pl = p.to_ascii_lowercase();
            if lower[i..].starts_with(&pl) {
                hit = Some(p.len());
                break;
            }
        }
        if let Some(plen) = hit {
            out.push_str(&s[i..i + plen]);
            out.push_str("[REDACTED]");
            i += plen;
            while i < bytes.len() && is_key_char(bytes[i] as char) {
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn redact_bearer(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let lower = s.to_ascii_lowercase();
    let mut i = 0;
    while i < s.len() {
        if lower[i..].starts_with("bearer ") {
            out.push_str(&s[i..i + 7]);
            out.push_str("[REDACTED]");
            i += 7;
            while i < s.len() && is_key_char(s.as_bytes()[i] as char) {
                i += 1;
            }
            continue;
        }
        out.push(s.as_bytes()[i] as char);
        i += 1;
    }
    out
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
