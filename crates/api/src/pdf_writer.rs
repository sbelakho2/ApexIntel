//! Pure Rust PDF document writer for intelligence reports.
//!
//! Converts [`apex_insights::pdf_report::PdfReport`] into downloadable PDF
//! byte buffers using `printpdf`.  This module is a lightweight counterpart
//! to the worker crate's [`apex_worker::pdf_export`] which writes to files.
//!
//! # Sensei-Rams
//! - Clean, functional, no decorative elements
//! - Monospace font (Courier) for technical content
//! - Header with title, date, classification
//! - Footer with page numbers
//! - No gradients, shadows, or ornamentation

use anyhow::{Context, Result};
use apex_insights::pdf_report::{PageSize, PdfReport, SourceRef};
use printpdf::*;

/// A4 margins: 20 mm on all sides.
const MARGIN_LEFT_MM: f64 = 20.0;
const MARGIN_RIGHT_MM: f64 = 20.0;
const MARGIN_TOP_MM: f64 = 20.0;
const MARGIN_BOTTOM_MM: f64 = 20.0;

/// Body font size.
const BODY_SIZE: f64 = 9.0;
/// Heading font size.
const HEADING_SIZE: f64 = 11.0;
/// Small text (meta, footer).
const SMALL_SIZE: f64 = 7.0;

/// Rough estimate: average character width in mm for a given font size.
fn avg_char_width_mm(font_size_pt: f64) -> f64 {
    font_size_pt * 0.3528 // 1 pt ≈ 0.3528 mm
}

/// Estimate how many characters fit in a given width in mm.
fn chars_fit(width_mm: f64, font_size_pt: f64) -> usize {
    let cw = avg_char_width_mm(font_size_pt);
    if cw <= 0.0 {
        return 80;
    }
    (width_mm / cw).floor().max(10.0) as usize
}

/// Dimensions in Mm for the two supported page sizes.
fn page_dimensions_mm(size: &PageSize) -> (Mm, Mm) {
    let (w_mm, h_mm) = size.dimensions_mm();
    (Mm(w_mm as f32), Mm(h_mm as f32))
}

/// Render a [`PdfReport`] into an in-memory PDF byte vector.
pub fn render_report_to_pdf(report: &PdfReport) -> Result<Vec<u8>> {
    let (page_w, page_h) = page_dimensions_mm(&report.page_size);
    let content_width_mm = page_w.0 as f64 - MARGIN_LEFT_MM - MARGIN_RIGHT_MM;

    let (doc, page_idx, layer_idx) =
        PdfDocument::new(&report.title, page_w, page_h, "ApexIntel Report");

    let font_bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .context("failed to load HelveticaBold")?;
    let font_regular = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .context("failed to load Helvetica")?;
    let font_mono = doc
        .add_builtin_font(BuiltinFont::Courier)
        .context("failed to load Courier")?;

    let pdf = PdfDocWriter {
        doc: &doc,
        font_bold,
        font_regular,
        _font_mono: font_mono,
        page_w,
        page_h,
        content_width_mm,
    };

    let mut state = PageState::new(page_idx, layer_idx, page_h.0 as f64);
    pdf.render_all(report, &mut state)?;

    let bytes = doc.save_to_bytes().context("failed to save PDF to bytes")?;
    Ok(bytes)
}

// ─── Internal helpers ───────────────────────────────────────────────────────

struct PdfDocWriter<'a> {
    doc: &'a PdfDocumentReference,
    font_bold: IndirectFontRef,
    font_regular: IndirectFontRef,
    _font_mono: IndirectFontRef,
    page_w: Mm,
    page_h: Mm,
    content_width_mm: f64,
}

struct PageState {
    pages: Vec<(PdfPageIndex, PdfLayerIndex)>,
    current_page: PdfPageIndex,
    current_layer: PdfLayerIndex,
    /// Cursor Y position in mm from bottom of page
    cursor_y: f64,
    _page_h_mm: f64,
    page_num: usize,
}

impl PageState {
    fn new(page: PdfPageIndex, layer: PdfLayerIndex, _page_h_mm: f64) -> Self {
        Self {
            pages: vec![(page, layer)],
            current_page: page,
            current_layer: layer,
            cursor_y: _page_h_mm - MARGIN_TOP_MM,
            _page_h_mm,
            page_num: 1,
        }
    }

    fn add_page(&mut self, doc: &PdfDocumentReference, page_w: Mm, page_h: Mm) {
        let (page, layer) = doc.add_page(page_w, page_h, "Content");
        self.pages.push((page, layer));
        self.current_page = page;
        self.current_layer = layer;
        self.cursor_y = page_h.0 as f64 - MARGIN_TOP_MM;
        self.page_num += 1;
    }

    fn remaining_height(&self) -> f64 {
        self.cursor_y - MARGIN_BOTTOM_MM
    }
}

impl PdfDocWriter<'_> {
    /// Render the full report onto one or more pages.
    fn render_all(&self, report: &PdfReport, state: &mut PageState) -> Result<()> {
        // ── Header ──
        self.draw_header(report, state);

        // ── Table of Contents (if multiple sections) ──
        if report.sections.len() > 1 {
            self.draw_toc(report, state);
        }

        // ── Sections ──
        for section in &report.sections {
            self.draw_section(section, state);
        }

        // ── Footer on every page ──
        for (page_num, &(page_idx, layer_idx)) in state.pages.iter().enumerate() {
            self.draw_footer(page_idx, layer_idx, report, page_num + 1);
        }

        Ok(())
    }

    fn draw_header(&self, report: &PdfReport, state: &mut PageState) {
        let layer = self
            .doc
            .get_page(state.current_page)
            .get_layer(state.current_layer);

        // Title
        layer.use_text(
            &report.title,
            HEADING_SIZE as f32,
            Mm(MARGIN_LEFT_MM as f32),
            Mm(state.cursor_y as f32),
            &self.font_bold,
        );
        state.cursor_y -= 5.0;

        // Subtitle
        if let Some(ref subtitle) = report.subtitle {
            if !subtitle.is_empty() {
                layer.use_text(
                    subtitle,
                    BODY_SIZE as f32,
                    Mm(MARGIN_LEFT_MM as f32),
                    Mm(state.cursor_y as f32),
                    &self.font_regular,
                );
                state.cursor_y -= 4.5;
            }
        }

        // Meta line
        let meta = format!(
            "{} | {} | {}",
            report.generated_at.format("%Y-%m-%d %H:%M UTC"),
            report.report_type.as_str(),
            report.report_type.classification()
        );
        layer.use_text(
            &meta,
            SMALL_SIZE as f32,
            Mm(MARGIN_LEFT_MM as f32),
            Mm(state.cursor_y as f32),
            &self.font_regular,
        );
        state.cursor_y -= 4.0;

        // Classification
        let classification = report.report_type.classification();
        layer.use_text(
            format!("CLASSIFICATION: {}", classification),
            SMALL_SIZE as f32,
            Mm(MARGIN_LEFT_MM as f32),
            Mm(state.cursor_y as f32),
            &self.font_bold,
        );
        state.cursor_y -= 3.5;

        // Horizontal rule
        self.draw_line(
            state,
            MARGIN_LEFT_MM,
            state.cursor_y,
            self.page_w.0 as f64 - MARGIN_RIGHT_MM,
        );
        state.cursor_y -= 3.0;

        self.ensure_space(state, 0.0);
    }

    fn draw_toc(&self, report: &PdfReport, state: &mut PageState) {
        self.ensure_space(state, 0.0);

        let layer = self
            .doc
            .get_page(state.current_page)
            .get_layer(state.current_layer);
        layer.use_text(
            "TABLE OF CONTENTS",
            BODY_SIZE as f32,
            Mm(MARGIN_LEFT_MM as f32),
            Mm(state.cursor_y as f32),
            &self.font_bold,
        );
        state.cursor_y -= 4.0;

        for (i, section) in report.sections.iter().enumerate() {
            let label = format!("{}. {}", i + 1, section.heading);
            layer.use_text(
                &label,
                BODY_SIZE as f32,
                Mm((MARGIN_LEFT_MM + 3.0) as f32),
                Mm(state.cursor_y as f32),
                &self.font_regular,
            );
            state.cursor_y -= 3.5;
            self.ensure_space(state, 0.0);
        }

        state.cursor_y -= 2.0;
        self.draw_line(
            state,
            MARGIN_LEFT_MM,
            state.cursor_y,
            self.page_w.0 as f64 - MARGIN_RIGHT_MM,
        );
        state.cursor_y -= 3.0;
        self.ensure_space(state, 0.0);
    }

    fn draw_section(
        &self,
        section: &apex_insights::pdf_report::ReportSection,
        state: &mut PageState,
    ) {
        self.ensure_space(state, 5.0);

        let layer = self
            .doc
            .get_page(state.current_page)
            .get_layer(state.current_layer);

        // Section heading
        layer.use_text(
            &section.heading,
            HEADING_SIZE as f32,
            Mm(MARGIN_LEFT_MM as f32),
            Mm(state.cursor_y as f32),
            &self.font_bold,
        );
        state.cursor_y -= 5.0;

        // Severity badge (if present)
        if let Some(ref severity) = section.severity {
            let badge = format!("[{}]", severity.as_str());
            layer.use_text(
                &badge,
                SMALL_SIZE as f32,
                Mm(MARGIN_LEFT_MM as f32),
                Mm(state.cursor_y as f32),
                &self.font_bold,
            );
            state.cursor_y -= 3.5;
        }

        // Body text — word-wrap at content width
        let max_chars = chars_fit(self.content_width_mm, BODY_SIZE);
        let line_spacing = 3.5;
        for line in word_wrap(&section.body, max_chars) {
            self.ensure_space(state, line_spacing);
            let layer = self
                .doc
                .get_page(state.current_page)
                .get_layer(state.current_layer);
            layer.use_text(
                &line,
                BODY_SIZE as f32,
                Mm(MARGIN_LEFT_MM as f32),
                Mm(state.cursor_y as f32),
                &self.font_regular,
            );
            state.cursor_y -= line_spacing;
        }
        state.cursor_y -= 1.5;

        // Evidence table
        if !section.evidence_items.is_empty() {
            self.draw_evidence_table(&section.evidence_items, state);
        }

        // Sources
        if !section.sources.is_empty() {
            self.draw_sources(&section.sources, state);
        }

        self.ensure_space(state, 2.0);
    }

    fn draw_evidence_table(
        &self,
        evidence: &[apex_insights::pdf_report::EvidenceItem],
        state: &mut PageState,
    ) {
        self.ensure_space(state, 4.0);

        let layer = self
            .doc
            .get_page(state.current_page)
            .get_layer(state.current_layer);
        layer.use_text(
            "EVIDENCE",
            BODY_SIZE as f32,
            Mm(MARGIN_LEFT_MM as f32),
            Mm(state.cursor_y as f32),
            &self.font_bold,
        );
        state.cursor_y -= 4.0;

        let indent = MARGIN_LEFT_MM + 3.0;
        let max_chars = chars_fit(self.content_width_mm - 3.0, BODY_SIZE);

        for item in evidence {
            let text = format!("- {}: {}", item.label, item.value);
            let confidence = if (item.confidence - 0.5).abs() > f64::EPSILON {
                format!(" [{:.0}%]", item.confidence * 100.0)
            } else {
                String::new()
            };
            let line = format!("{}{}", text, confidence);

            for wrapped in word_wrap(&line, max_chars) {
                self.ensure_space(state, 3.5);
                let layer = self
                    .doc
                    .get_page(state.current_page)
                    .get_layer(state.current_layer);
                layer.use_text(
                    &wrapped,
                    BODY_SIZE as f32,
                    Mm(indent as f32),
                    Mm(state.cursor_y as f32),
                    &self.font_regular,
                );
                state.cursor_y -= 3.5;
            }
        }
        state.cursor_y -= 1.5;
    }

    fn draw_sources(&self, sources: &[SourceRef], state: &mut PageState) {
        self.ensure_space(state, 4.0);

        let layer = self
            .doc
            .get_page(state.current_page)
            .get_layer(state.current_layer);
        layer.use_text(
            "SOURCES",
            BODY_SIZE as f32,
            Mm(MARGIN_LEFT_MM as f32),
            Mm(state.cursor_y as f32),
            &self.font_bold,
        );
        state.cursor_y -= 4.0;

        let indent = MARGIN_LEFT_MM + 3.0;
        let max_chars = chars_fit(self.content_width_mm - 3.0, SMALL_SIZE);

        for (i, source) in sources.iter().enumerate() {
            let text = format!("[{}] {} ({})", i + 1, source.title, source.url);
            for wrapped in word_wrap(&text, max_chars) {
                self.ensure_space(state, 3.0);
                let layer = self
                    .doc
                    .get_page(state.current_page)
                    .get_layer(state.current_layer);
                layer.use_text(
                    &wrapped,
                    SMALL_SIZE as f32,
                    Mm(indent as f32),
                    Mm(state.cursor_y as f32),
                    &self.font_regular,
                );
                state.cursor_y -= 3.0;
            }
        }
    }

    fn draw_footer(
        &self,
        page_idx: PdfPageIndex,
        layer_idx: PdfLayerIndex,
        report: &PdfReport,
        page_num: usize,
    ) {
        let layer = self.doc.get_page(page_idx).get_layer(layer_idx);

        // Page number
        let page_text = format!(
            "Page {} -- {} | {}",
            page_num,
            report.report_type.classification(),
            report.report_type.as_str(),
        );

        // Estimate text width (rough approximation since we can't measure)
        let est_width_mm = page_text.len() as f64 * avg_char_width_mm(SMALL_SIZE);
        let x_pos = self.page_w.0 as f64 - MARGIN_RIGHT_MM - est_width_mm;

        layer.use_text(
            &page_text,
            SMALL_SIZE as f32,
            Mm(x_pos.max(MARGIN_LEFT_MM) as f32),
            Mm((MARGIN_BOTTOM_MM - 2.0) as f32),
            &self.font_regular,
        );
    }

    /// Draw a horizontal line at `y` (in mm).
    fn draw_line(&self, state: &mut PageState, x1_mm: f64, y_mm: f64, x2_mm: f64) {
        let layer = self
            .doc
            .get_page(state.current_page)
            .get_layer(state.current_layer);
        let points = vec![
            (Point::new(Mm(x1_mm as f32), Mm(y_mm as f32)), false),
            (Point::new(Mm(x2_mm as f32), Mm(y_mm as f32)), false),
        ];
        let line = Line {
            points,
            is_closed: false,
        };
        layer.add_line(line);
    }

    /// Add a new page if the remaining space is less than `needed` (in mm).
    fn ensure_space(&self, state: &mut PageState, needed: f64) {
        if state.remaining_height() < needed {
            state.add_page(self.doc, self.page_w, self.page_h);
            // Re-draw header on continuation pages
            let layer = self
                .doc
                .get_page(state.current_page)
                .get_layer(state.current_layer);
            layer.use_text(
                "(continued)",
                SMALL_SIZE as f32,
                Mm((self.page_w.0 as f64 - MARGIN_RIGHT_MM - 15.0) as f32),
                Mm(state.cursor_y as f32),
                &self.font_regular,
            );
            state.cursor_y -= 3.5;
            self.draw_line(
                state,
                MARGIN_LEFT_MM,
                state.cursor_y,
                self.page_w.0 as f64 - MARGIN_RIGHT_MM,
            );
            state.cursor_y -= 3.0;
        }
    }
}

/// Simple word-wrap using character-count approximation.
fn word_wrap(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() || max_chars == 0 {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut current = String::with_capacity(max_chars);
        for word in paragraph.split(' ') {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() <= max_chars {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }

    if lines.is_empty() && !text.is_empty() {
        lines.push(text.to_string());
    }

    lines
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use apex_insights::pdf_report::{EvidenceItem, PageSize, PdfReport, ReportSection, ReportType};

    #[test]
    fn test_word_wrap_empty() {
        let lines = word_wrap("", 80);
        assert_eq!(lines, vec![""]);
    }

    #[test]
    fn test_word_wrap_short() {
        let lines = word_wrap("hello world", 80);
        assert_eq!(lines, vec!["hello world"]);
    }

    #[test]
    fn test_word_wrap_newlines() {
        let lines = word_wrap("line1\n\nline2", 80);
        assert_eq!(lines, vec!["line1", "", "line2"]);
    }

    #[test]
    fn test_render_empty_report() {
        let report = PdfReport::new("Test Report", ReportType::InsightSummary);
        let pdf = render_report_to_pdf(&report).unwrap();
        assert!(!pdf.is_empty(), "PDF must not be empty");
        // Should start with PDF magic bytes
        assert!(pdf.starts_with(b"%PDF-"), "expected PDF header");
    }

    #[test]
    fn test_render_report_with_section() {
        let mut report = PdfReport::new("Detailed Report", ReportType::EntityDossier);
        report.add_section(
            ReportSection::new("Section 1").with_body("This is the body content of section one."),
        );
        report.add_section(
            ReportSection::new("Section 2").with_body("This is the body content of section two."),
        );
        let pdf = render_report_to_pdf(&report).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_render_report_with_evidence() {
        let mut report = PdfReport::new("Evidence Report", ReportType::CompetitiveAnalysis);
        let mut section = ReportSection::new("Key Findings");
        section.add_evidence(EvidenceItem::new("Revenue", "$10M").with_confidence(0.85));
        section.add_evidence(EvidenceItem::new("Employees", "5,000").with_confidence(0.90));
        report.add_section(section);
        let pdf = render_report_to_pdf(&report).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_render_letter_size() {
        let report = PdfReport::new("Letter Test", ReportType::InsightSummary)
            .with_page_size(PageSize::Letter);
        let pdf = render_report_to_pdf(&report).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_page_dimensions_a4() {
        let (w, h) = page_dimensions_mm(&PageSize::A4);
        assert!((w.0 - 210.0).abs() < 1.0, "A4 width ~210 mm");
        assert!((h.0 - 297.0).abs() < 1.0, "A4 height ~297 mm");
    }

    #[test]
    fn test_page_dimensions_letter() {
        let (w, h) = page_dimensions_mm(&PageSize::Letter);
        assert!((w.0 - 216.0).abs() < 1.0, "Letter width ~216 mm");
        assert!((h.0 - 279.0).abs() < 1.0, "Letter height ~279 mm");
    }

    #[test]
    fn test_chars_fit() {
        let n = chars_fit(170.0, 9.0);
        assert!(n > 40, "should fit at least 40 chars at 9pt in 170mm");
    }

    #[test]
    fn test_avg_char_width() {
        let w = avg_char_width_mm(9.0);
        assert!((w - 3.175).abs() < 0.1, "avg char width ~3.175 mm at 9pt");
    }
}
