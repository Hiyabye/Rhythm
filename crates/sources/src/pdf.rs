//! The text layer of a PDF that a main-board notice shows as its whole body.

/// Extracted text beyond this is not kept as body text; the PDF stays attached.
const MAX_TEXT: usize = 512 * 1024;

/// Returns the PDF's text, one line per extracted line with whitespace runs
/// collapsed, or `None` when it has no text layer (a scan) or cannot be read.
/// The extractor is third-party code over downloaded bytes, so a panic counts
/// as an unreadable file instead of ending the sync job.
pub fn text(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"%PDF-") {
        return None;
    }
    let raw = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes))
        .ok()?
        .ok()?;
    let text = raw
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty() && text.len() <= MAX_TEXT).then_some(text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn extracts_korean_text_from_a_captured_notice_pdf() {
        let text = super::text(include_bytes!("fixtures/main-pdf-551680.pdf")).unwrap();
        assert!(
            text.contains("2026학년도 동계계절수업 수강신청 안내"),
            "{text}"
        );
        assert!(
            text.contains("수강신청 담아놓기 11. 18(수) 11:00"),
            "{text}"
        );
        assert!(!text.contains("  "));
    }
    #[test]
    fn rejects_non_pdf_and_unreadable_bytes() {
        assert_eq!(super::text(b"<html>error</html>"), None);
        assert_eq!(super::text(b"%PDF-1.4\nnot really a pdf"), None);
    }
}
