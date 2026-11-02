//! Reading an uploaded onboarding document into text, so its facts can be checked against what the merchant
//! entered. Two ways, both through programs installed next to the API (`poppler-utils` and `tesseract-ocr`):
//! a PDF's own text layer when it has one (the GST portal's certificates do), otherwise the page is rendered to
//! an image and read by OCR (the Registrar's certificate of incorporation is a scan). A JPEG or PNG goes straight
//! to OCR. Nothing leaves the machine and no model is involved.
//!
//! The files are untrusted, so every program runs in its own empty directory with an empty environment, a time
//! limit, a page limit and a cap on what it may print. When the programs are missing, or fail, the answer is
//! "could not read" and the document goes to a person: it is never treated as verified.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{io::AsyncReadExt, process::Command, time::timeout};

/// The kinds of file an upload may be, found from its first bytes (never from the name it was sent with).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    Png,
    Jpeg,
}

impl FileKind {
    pub fn sniff(bytes: &[u8]) -> Option<FileKind> {
        if bytes.starts_with(b"%PDF-") {
            Some(FileKind::Pdf)
        } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(FileKind::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(FileKind::Jpeg)
        } else {
            None
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            FileKind::Pdf => "application/pdf",
            FileKind::Png => "image/png",
            FileKind::Jpeg => "image/jpeg",
        }
    }

    fn ext(self) -> &'static str {
        match self {
            FileKind::Pdf => "pdf",
            FileKind::Png => "png",
            FileKind::Jpeg => "jpg",
        }
    }
}

/// What reading produced. `method` is `text` (the PDF's text layer), `ocr`, or `none` (nothing readable).
#[derive(Clone, Debug)]
pub struct Reading {
    pub text: String,
    pub method: &'static str,
}

impl Reading {
    fn none() -> Reading {
        Reading {
            text: String::new(),
            method: "none",
        }
    }
}

#[derive(Clone, Debug)]
pub enum DocReader {
    /// Reads nothing: every document goes to a person.
    Off,
    /// Uses `pdftotext`, `pdftoppm` and `tesseract` when they are installed.
    Tools,
    /// Always answers this text (tests).
    Fixed(String),
}

const MAX_PAGES: &str = "3";
const MAX_OUTPUT: usize = 200_000;
/// Enough characters of text layer to believe a PDF is not just a scan with stray marks.
const MIN_TEXT_LAYER: usize = 40;
const STEP_TIMEOUT: Duration = Duration::from_secs(40);
/// Each program may map at most this much memory (about 1.4 GiB).
const MEMORY_LIMIT_KB: u32 = 1_500_000;
/// Longest side, in pixels, of a rendered page: an A4 page at 200 dpi, however large the page claims to be.
const PAGE_PIXELS: &str = "2339";

impl DocReader {
    /// `ZYVOR_DOC_READER`: `off`, or anything else (the default) to use the programs when they are installed.
    pub fn from_env() -> DocReader {
        match std::env::var("ZYVOR_DOC_READER").as_deref() {
            Ok("off") => DocReader::Off,
            _ => DocReader::Tools,
        }
    }

    /// Whether the programs this reader needs are installed (for the startup log and the status the console shows).
    pub fn available(&self) -> bool {
        match self {
            DocReader::Off => false,
            DocReader::Fixed(_) => true,
            DocReader::Tools => on_path("pdftotext") && on_path("pdftoppm") && on_path("tesseract"),
        }
    }

    pub async fn read(&self, bytes: &[u8], kind: FileKind) -> Reading {
        match self {
            DocReader::Off => Reading::none(),
            DocReader::Fixed(t) => Reading {
                text: t.clone(),
                method: "text",
            },
            DocReader::Tools => read_with_tools(bytes, kind).await,
        }
    }
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}

/// A directory only this process can use, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Option<Scratch> {
        let dir = std::env::temp_dir().join(format!("zyvor-doc-{}", zyvor_core::new_id("rd")));
        std::fs::create_dir(&dir).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
        }
        Some(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run one program in `dir`, return what it printed (capped), or None when it failed or ran too long.
async fn run(dir: &Path, program: &str, args: &[&str]) -> Option<String> {
    // `ulimit -v` caps the program's address space, so a hostile page cannot take the API container's memory with it
    // (ignored where the shell does not support it); one OCR thread keeps it small
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -v {MEMORY_LIMIT_KB} 2>/dev/null; exec \"$0\" \"$@\""
        ))
        .arg(program)
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("OMP_THREAD_LIMIT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let work = async {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        while buf.len() < MAX_OUTPUT {
            let n = out.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        buf.truncate(MAX_OUTPUT);
        let status = child.wait().await.ok()?;
        status
            .success()
            .then(|| String::from_utf8_lossy(&buf).into_owned())
    };
    timeout(STEP_TIMEOUT, work).await.ok().flatten()
}

async fn read_with_tools(bytes: &[u8], kind: FileKind) -> Reading {
    let Some(scratch) = Scratch::new() else {
        return Reading::none();
    };
    let dir = scratch.0.as_path();
    let input = format!("in.{}", kind.ext());
    if std::fs::write(dir.join(&input), bytes).is_err() {
        return Reading::none();
    }
    match kind {
        FileKind::Pdf => {
            if on_path("pdftotext")
                && let Some(t) = run(
                    dir,
                    "pdftotext",
                    &["-layout", "-f", "1", "-l", MAX_PAGES, &input, "-"],
                )
                .await
                && t.trim().chars().count() >= MIN_TEXT_LAYER
            {
                return Reading {
                    text: t,
                    method: "text",
                };
            }
            // a scan: render the first pages, then read them
            if !(on_path("pdftoppm") && on_path("tesseract")) {
                return Reading::none();
            }
            if run(
                dir,
                "pdftoppm",
                &[
                    "-scale-to",
                    PAGE_PIXELS,
                    "-png",
                    "-f",
                    "1",
                    "-l",
                    "2",
                    &input,
                    "page",
                ],
            )
            .await
            .is_none()
            {
                return Reading::none();
            }
            let mut pages: Vec<String> = std::fs::read_dir(dir)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with("page") && n.ends_with(".png"))
                .collect();
            pages.sort();
            let mut text = String::new();
            for p in pages.iter().take(2) {
                if let Some(t) = run(dir, "tesseract", &[p, "-", "--psm", "4"]).await {
                    text.push_str(&t);
                    text.push('\n');
                }
            }
            ocr_result(text)
        }
        FileKind::Png | FileKind::Jpeg => {
            if !on_path("tesseract") {
                return Reading::none();
            }
            match run(dir, "tesseract", &[&input, "-", "--psm", "4"]).await {
                Some(t) => ocr_result(t),
                None => Reading::none(),
            }
        }
    }
}

fn ocr_result(text: String) -> Reading {
    if text.trim().chars().count() < MIN_TEXT_LAYER {
        Reading::none()
    } else {
        Reading {
            text,
            method: "ocr",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_known_by_their_first_bytes_not_their_names() {
        assert_eq!(FileKind::sniff(b"%PDF-1.4 ..."), Some(FileKind::Pdf));
        assert_eq!(
            FileKind::sniff(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            Some(FileKind::Png)
        );
        assert_eq!(
            FileKind::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(FileKind::Jpeg)
        );
        assert_eq!(FileKind::sniff(b"MZ\x90\x00 an executable"), None);
        assert_eq!(FileKind::sniff(b"<html><script>"), None);
        assert_eq!(FileKind::sniff(b""), None);
    }

    #[tokio::test]
    async fn the_off_reader_reads_nothing_and_a_fixed_one_answers_what_it_was_given() {
        let r = DocReader::Off.read(b"%PDF-1.4", FileKind::Pdf).await;
        assert_eq!((r.method, r.text.as_str()), ("none", ""));
        let r = DocReader::Fixed("hello".into())
            .read(b"x", FileKind::Pdf)
            .await;
        assert_eq!((r.method, r.text.as_str()), ("text", "hello"));
    }
}
