//! A minimal text PDF: A4 pages of Helvetica lines, built by hand so an invoice email can carry a copy without a
//! PDF library. Text only (no logo, no table borders). Characters Helvetica's WinAnsi set lacks become `?`, except
//! the rupee sign, which is written as "Rs.".

const PAGE_H: i32 = 842;
const TOP: i32 = 790;
const LEADING: i32 = 16;
const LINES_PER_PAGE: usize = ((TOP - 50) / LEADING) as usize;
const WRAP: usize = 88;

fn encode(s: &str) -> String {
    let mut out = String::new();
    for c in s.replace('₹', "Rs.").chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if (' '..='~').contains(&c) => out.push(c),
            c if (c as u32) >= 0xA0 && (c as u32) <= 0xFF => {
                out.push_str(&format!("\\{:03o}", c as u32))
            }
            _ => out.push('?'),
        }
    }
    out
}

fn wrap(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for w in line.split(' ') {
        if !cur.is_empty() && cur.chars().count() + 1 + w.chars().count() > WRAP {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    out.push(cur);
    out
}

/// A PDF whose first line is `title` in a larger size, followed by `lines` (long ones wrap, blank ones are kept).
pub fn text_pdf(title: &str, lines: &[String]) -> Vec<u8> {
    let mut all: Vec<(bool, String)> = vec![(true, title.to_string()), (false, String::new())];
    for l in lines {
        for w in wrap(l) {
            all.push((false, w));
        }
    }
    let pages: Vec<&[(bool, String)]> = all.chunks(LINES_PER_PAGE).collect();
    // object numbers: 1 catalog, 2 pages, 3 font, 4 bold font, then (page, content) pairs
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        String::new(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .into(),
    ];
    let mut kids = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let page_no = 5 + 2 * i;
        kids.push(format!("{page_no} 0 R"));
        let mut c = String::from("BT\n");
        let mut y = TOP;
        for (bold, text) in page.iter() {
            let (font, size) = if *bold { ("F2", 16) } else { ("F1", 10) };
            c.push_str(&format!(
                "/{font} {size} Tf 1 0 0 1 50 {y} Tm ({}) Tj\n",
                encode(text)
            ));
            y -= LEADING;
        }
        c.push_str("ET\n");
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 {PAGE_H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>", page_no + 1));
        objs.push(format!("<< /Length {} >>\nstream\n{c}endstream", c.len()));
    }
    objs[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    );
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objs.len() + 1
    ));
    for o in offsets {
        out.push_str(&format!("{o:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objs.len() + 1
    ));
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pdf_has_its_parts_and_escapes_text() {
        let p = text_pdf(
            "Invoice INV-1",
            &["Total: ₹1,250.50 (due) \\ done".into(), "é ü ✓".into()],
        );
        let s = String::from_utf8(p).unwrap();
        assert!(s.starts_with("%PDF-1.4\n") && s.trim_end().ends_with("%%EOF"));
        assert!(s.contains("(Invoice INV-1) Tj"));
        assert!(s.contains("Rs.1,250.50 \\(due\\) \\\\ done"));
        assert!(s.contains("\\351 \\374 ?"));
        assert!(s.contains("/Count 1"));
        // every object offset in the table points at its "N 0 obj"
        let xref = s.rfind("xref\n").unwrap();
        for (i, line) in s[xref..].lines().skip(3).take(9).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(s[off..].starts_with(&format!("{} 0 obj", i + 1)), "{line}");
        }
    }

    #[test]
    fn a_long_document_runs_over_pages() {
        let lines: Vec<String> = (0..120).map(|i| format!("line {i}")).collect();
        let s = String::from_utf8(text_pdf("T", &lines)).unwrap();
        assert!(
            s.contains("/Count 3"),
            "{}",
            &s[s.find("/Count").unwrap()..][..12]
        );
    }
}
