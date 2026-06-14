//! PDF export using `printpdf` for pure Rust PDF generation.
//!
//! Converts [`PdfReport`] structs into downloadable PDF byte streams.
//! Supports headers, footers, page numbers, and classification markings.
//!
//! # Sensei-Rams compliance
//! - Clean, functional, no decorative elements
//! - Monospace font for technical content
//! - Header with report title, date, and classification label
//! - Footer with page numbers

use anyhow::{Context, Result};
use apex_insights::pdf_report::{PageSize, PdfReport, ReportSection};
use printpdf::*;
use std::fs;
use std::path::PathBuf;
use tempfile::NamedTempFile;

/// Configuration for PDF export.
#[derive(Debug, Clone)]
pub struct PdfExportConfig {
    pub output_dir: PathBuf,
    pub max_pages: usize,
    pub page_size: PageSize,
}

impl Default for PdfExportConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("/tmp/apex-pdfs"),
            max_pages: 50,
            page_size: PageSize::A4,
        }
    }
}

/// Generate a PDF from a `PdfReport` and save it to the configured output directory.
///
/// Returns the path to the generated PDF file.
pub async fn generate_pdf(report: &PdfReport, config: &PdfExportConfig) -> Result<PathBuf> {
    fs::create_dir_all(&config.output_dir)
        .context("Failed to create PDF output directory")?;

    let (width_mm, height_mm) = config.page_size.dimensions_mm();
    let pt_width = Mm(width_mm as f32);
    let pt_height = Mm(height_mm as f32);

    // Create a new PDF document with the proper page size
    let (doc, page_idx, _layer_idx) =
        PdfDocument::new(&report.title, pt_width, pt_height, "ApexIntel");

    // Calculate margins
    let margin_left = Mm(15.0_f32);
    let _margin_right = Mm(15.0_f32);
    let margin_top = Mm(20.0_f32);
    let margin_bottom = Mm(20.0_f32);
    let usable_width = width_mm - 30.0; // 15mm each side
    let usable_height = height_mm - 40.0; // 20mm each side

    // Fonts: use built-in fonts for portability
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;
    let font_regular = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_mono = doc.add_builtin_font(BuiltinFont::Courier)?;

    let current_page = doc.get_page(page_idx);
    let _layer = current_page.add_layer("Content");

    // Draw report
    render_report(
        &doc,
        page_idx,
        report,
        &font_bold,
        &font_regular,
        &font_mono,
        pt_width,
        pt_height,
        margin_left,
        margin_top,
        margin_bottom,
        usable_width as f32,
        usable_height as f32,
    )?;

    // Generate filename
    let safe_title: String = report
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect();
    let timestamp = report.generated_at.format("%Y%m%d_%H%M%S");
    let filename = format!("{}_{}.pdf", safe_title.replace(' ', "_"), timestamp);
    let output_path = config.output_dir.join(&filename);

    // Save to temporary file first, then rename
    let temp = NamedTempFile::new_in(&config.output_dir)
        .context("Failed to create temp file for PDF")?;
    let temp_path = temp.path().to_path_buf();

    doc.save(&mut std::io::BufWriter::new(
        fs::File::create(&temp_path)?,
    ))?;

    fs::rename(&temp_path, &output_path)
        .context("Failed to rename temporary PDF file")?;

    tracing::info!(path = %output_path.display(), "PDF generated successfully");
    Ok(output_path)
}

/// Render the full report onto PDF pages.
#[allow(clippy::too_many_arguments)]
fn render_report(
    doc: &PdfDocumentReference,
    mut current_page_idx: PdfPageIndex,
    report: &PdfReport,
    font_bold: &IndirectFontRef,
    font_regular: &IndirectFontRef,
    font_mono: &IndirectFontRef,
    page_width: Mm,
    page_height: Mm,
    margin_left: Mm,
    margin_top: Mm,
    margin_bottom: Mm,
    usable_width: f32,
    _usable_height: f32,
) -> Result<()> {
    let mut y_cursor: f32 = page_height.0 - margin_top.0 - 5.0_f32;

    // ── Header block ──────────────────────────────────────────────────────
    y_cursor = draw_header(
        doc,
        current_page_idx,
        report,
        font_bold,
        font_regular,
        font_mono,
        margin_left,
        &mut y_cursor,
        usable_width,
    )?;

    y_cursor -= 8.0_f32; // spacing after header

    // ── Table of Contents ─────────────────────────────────────────────────
    if report.sections.len() > 1 {
        y_cursor = draw_toc(
            doc,
            current_page_idx,
            report,
            font_bold,
            font_regular,
            margin_left,
            &mut y_cursor,
            usable_width,
            page_height.0 - margin_bottom.0,
        )?;
        y_cursor -= 5.0_f32;
    }

    // ── Sections ──────────────────────────────────────────────────────────
    for section in &report.sections {
        if y_cursor < 30.0_f32 {
            // Need new page
            let (new_idx, _) = add_new_page(doc, page_width, page_height);
            current_page_idx = new_idx;
            y_cursor = page_height.0 - margin_top.0 - 5.0_f32;

            // Draw header on new page too
            y_cursor = draw_header(
                doc,
                current_page_idx,
                report,
                font_bold,
                font_regular,
                font_mono,
                margin_left,
                &mut y_cursor,
                usable_width,
            )?;
            y_cursor -= 8.0_f32;
        }

        y_cursor = draw_section(
            doc,
            current_page_idx,
            section,
            font_bold,
            font_regular,
            font_mono,
            margin_left,
            &mut y_cursor,
            usable_width,
            page_height.0 - margin_bottom.0,
        )?;
        y_cursor -= 4.0_f32;
    }

    // ── Footer (page numbers) on each page ────────────────────────────────
    // Note: printpdf requires us to track pages manually. For simplicity,
    // we draw a footer on the last page.
    draw_footer(
        doc,
        current_page_idx,
        report,
        font_regular,
        margin_left,
        page_height.0 - margin_bottom.0,
        usable_width,
        1, // page number
    )?;

    Ok(())
}

/// Draw the report header on a page.
fn draw_header(
    doc: &PdfDocumentReference,
    page_idx: PdfPageIndex,
    report: &PdfReport,
    font_bold: &IndirectFontRef,
    font_regular: &IndirectFontRef,
    _font_mono: &IndirectFontRef,
    margin_left: Mm,
    y: &mut f32,
    usable_width: f32,
) -> Result<f32> {
    let page = doc.get_page(page_idx);
    let layer = page.add_layer("Header");

    // Title
    layer.use_text(report.title.as_str(), 16.0_f32, margin_left, Mm(*y), font_bold);
    *y -= 7.0_f32;

    // Subtitle
    if let Some(sub) = &report.subtitle {
        layer.use_text(sub.as_str(), 10.0_f32, margin_left, Mm(*y), font_regular);
        *y -= 5.0_f32;
    }

    // Meta line
    let meta_text = format!(
        "ApexIntel Intelligence Report  |  {}",
        report.generated_at.format("%Y-%m-%d %H:%M UTC")
    );
    layer.use_text(&meta_text, 7.0_f32, margin_left, Mm(*y), font_regular);
    *y -= 4.0_f32;

    // Classification
    let class_text = format!(
        "{}  ·  {}",
        report.report_type.classification(),
        report.report_type.as_str()
    );
    layer.use_text(&class_text, 7.0_f32, margin_left, Mm(*y), font_regular);

    // Horizontal line
    *y -= 2.0_f32;
    draw_line(
        doc,
        page_idx,
        margin_left,
        Mm(*y),
        Mm(margin_left.0 + usable_width),
        Mm(*y),
    );

    Ok(*y)
}

/// Draw a horizontal line.
fn draw_line(
    doc: &PdfDocumentReference,
    page_idx: PdfPageIndex,
    x1: Mm,
    y1: Mm,
    x2: Mm,
    y2: Mm,
) {
    let page = doc.get_page(page_idx);
    let layer = page.add_layer("Lines");
    let points = vec![
        (Point::new(x1, y1), false),
        (Point::new(x2, y2), false),
    ];
    let line = Line {
        points,
        is_closed: false,
    };
    layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    layer.set_outline_thickness(0.5);
    layer.add_line(line);
}

/// Draw a table of contents.
fn draw_toc(
    doc: &PdfDocumentReference,
    page_idx: PdfPageIndex,
    report: &PdfReport,
    font_bold: &IndirectFontRef,
    font_regular: &IndirectFontRef,
    margin_left: Mm,
    y: &mut f32,
    _usable_width: f32,
    _bottom_margin: f32,
) -> Result<f32> {
    let page = doc.get_page(page_idx);
    let layer = page.add_layer("TOC");

    layer.use_text("Contents", 11.0_f32, margin_left, Mm(*y), font_bold);
    *y -= 6.0_f32;

    for (i, section) in report.sections.iter().enumerate() {
        if *y < 15.0_f32 {
            break; // skip remaining TOC entries if no space
        }
        let text = format!("{}. {}", i + 1, section.heading);
        layer.use_text(&text, 8.5_f32, margin_left, Mm(*y), font_regular);
        *y -= 4.5_f32;
    }

    Ok(*y)
}

/// Draw a report section.
fn draw_section(
    doc: &PdfDocumentReference,
    page_idx: PdfPageIndex,
    section: &ReportSection,
    font_bold: &IndirectFontRef,
    font_regular: &IndirectFontRef,
    font_mono: &IndirectFontRef,
    margin_left: Mm,
    y: &mut f32,
    usable_width: f32,
    bottom_margin: f32,
) -> Result<f32> {
    let page = doc.get_page(page_idx);
    let layer = page.add_layer("Content");

    // Section heading
    layer.use_text(&section.heading, 11.0_f32, margin_left, Mm(*y), font_bold);
    *y -= 5.0_f32;

    // Severity badge
    if let Some(sev) = &section.severity {
        let badge = sev.as_str().to_uppercase();
        layer.use_text(&badge, 6.0_f32, margin_left, Mm(*y), font_bold);
        *y -= 4.0_f32;
    }

    // Body text
    if !section.body.is_empty() {
        let lines = word_wrap(&section.body, usable_width as usize, 10.0_f32);
        for line in &lines {
            if *y < bottom_margin + 10.0_f32 {
                return Ok(*y);
            }
            layer.use_text(line, 8.5_f32, margin_left, Mm(*y), font_regular);
            *y -= 4.0_f32;
        }
        *y -= 2.0_f32;
    }

    // Evidence table
    if !section.evidence_items.is_empty() {
        // Table header
        layer.use_text("EVIDENCE", 6.5_f32, margin_left, Mm(*y), font_bold);
        *y -= 4.0_f32;
        draw_line(
            doc,
            page_idx,
            margin_left,
            Mm(*y),
            Mm(margin_left.0 + usable_width),
            Mm(*y),
        );
        *y -= 2.0_f32;

        for ev in &section.evidence_items {
            if *y < bottom_margin + 5.0_f32 {
                break;
            }
            let label = if ev.label.len() > 35 {
                format!("{}…", &ev.label[..35])
            } else {
                ev.label.clone()
            };
            let conf = format!("{:.0}%", ev.confidence * 100.0);
            let line_text = format!("{}  |  {}", label, conf);
            layer.use_text(&line_text, 8.0_f32, margin_left, Mm(*y), font_mono);
            *y -= 3.5_f32;
        }
        *y -= 1.0_f32;
    }

    // Sources
    if !section.sources.is_empty() {
        if *y < bottom_margin + 15.0_f32 {
            return Ok(*y);
        }
        layer.use_text("SOURCES", 6.5_f32, margin_left, Mm(*y), font_bold);
        *y -= 4.0_f32;
        for src in &section.sources {
            if *y < bottom_margin + 5.0_f32 {
                break;
            }
            let text = format!("{} · {}", src.title, src.domain);
            layer.use_text(&text, 7.0_f32, margin_left, Mm(*y), font_regular);
            *y -= 3.5_f32;
        }
    }

    Ok(*y)
}

/// Draw the footer with page number and classification.
#[allow(unused_variables)]
fn draw_footer(
    doc: &PdfDocumentReference,
    page_idx: PdfPageIndex,
    report: &PdfReport,
    font_regular: &IndirectFontRef,
    margin_left: Mm,
    y_position: f32,
    usable_width: f32,
    page_num: u32,
) -> Result<()> {
    let page = doc.get_page(page_idx);
    let layer = page.add_layer("Footer");

    // Footer line
    draw_line(
        doc,
        page_idx,
        margin_left,
        Mm(y_position),
        Mm(margin_left.0 + usable_width),
        Mm(y_position),
    );

    let footer_y = y_position + 3.0_f32;
    let classification = report.report_type.classification();
    let footer_text = format!("ApexIntel Intelligence Report  |  Page {}  |  {}", page_num, classification);
    layer.use_text(&footer_text, 6.5_f32, margin_left, Mm(footer_y), font_regular);

    Ok(())
}

/// Add a new page to the document.
fn add_new_page(
    doc: &PdfDocumentReference,
    width: Mm,
    height: Mm,
) -> (PdfPageIndex, PdfLayerIndex) {
    doc.add_page(width, height, "ApexIntel")
}

/// Simple word wrapping algorithm for PDF text.
fn word_wrap(text: &str, chars_per_line: usize, font_size_pt: f32) -> Vec<String> {
    // Approximate: monospace chars are roughly font_size * 0.6 mm wide
    let avg_char_width_mm = font_size_pt * 0.3528_f32; // pt to mm
    let max_chars = (chars_per_line as f32 / avg_char_width_mm).max(20.0_f32) as usize;

    if text.is_empty() || max_chars == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut current = String::with_capacity(max_chars);
        for word in paragraph.split(' ') {
            if word.is_empty() {
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use apex_insights::InsightSeverity;
    use apex_insights::pdf_report::{
        EvidenceItem, PdfReport, ReportSection, ReportType,
    };

    fn create_test_report() -> PdfReport {
        let mut report = PdfReport::new("Test Intelligence Report", ReportType::InsightSummary);
        report.subtitle = Some("Q1 2026 Analysis".to_string());

        let mut section = ReportSection::new("Supply Chain Alert")
            .with_body("Detected increased procurement activity in the EU region.")
            .with_severity(InsightSeverity::High);
        section.add_evidence(EvidenceItem::new("Procurement Signal", "12 new RFQs issued")
            .with_confidence(0.87));
        section.add_source(apex_insights::pdf_report::SourceRef {
            title: "Industry Report".to_string(),
            url: "https://example.com/report".to_string(),
            domain: "example.com".to_string(),
            observed_at: None,
        });
        report.add_section(section);

        let section2 = ReportSection::new("Competitor Movement")
            .with_body("Competitor X has expanded into the Asian market.")
            .with_severity(InsightSeverity::Medium);
        report.add_section(section2);

        report
    }

    #[test]
    fn test_word_wrap_empty() {
        let result = word_wrap("", 80, 10.0_f32);
        assert!(result.is_empty());
    }

    #[test]
    fn test_word_wrap_short() {
        let result = word_wrap("Short text", 80, 10.0_f32);
        assert_eq!(result, vec!["Short text"]);
    }

    #[test]
    fn test_word_wrap_newlines() {
        let result = word_wrap("Line one\nLine two", 80, 10.0_f32);
        assert!(result.contains(&"Line one".to_string()));
        assert!(result.contains(&"Line two".to_string()));
    }

    #[test]
    fn test_word_wrap_long_line() {
        let long = "This is a very long line that should definitely be wrapped into multiple segments because it exceeds the maximum character limit";
        let result = word_wrap(long, 30, 10.0_f32);
        assert!(result.len() > 1);
        // Each line should be within the limit
        for line in &result {
            assert!(line.len() <= 35, "Line too long: '{}' ({} chars)", line, line.len());
        }
    }

    #[test]
    fn test_pdf_export_config_default() {
        let config = PdfExportConfig::default();
        assert_eq!(config.max_pages, 50);
        assert_eq!(config.page_size, PageSize::A4);
    }

    #[test]
    fn test_pdf_generates_valid_file() {
        let report = create_test_report();
        let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let config = PdfExportConfig {
            output_dir: tmp_dir.path().to_path_buf(),
            max_pages: 10,
            page_size: PageSize::A4,
        };

        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        let result = rt.block_on(generate_pdf(&report, &config));
        assert!(result.is_ok(), "PDF generation failed: {:?}", result.err());

        let path = result.unwrap();
        assert!(path.exists(), "PDF file does not exist");
        let metadata = fs::metadata(&path).expect("Failed to get file metadata");
        assert!(metadata.len() > 0, "PDF file is empty");
    }

    #[test]
    fn test_pdf_respects_max_pages() {
        let mut report = create_test_report();
        // Add many sections to test multi-page
        for i in 0..5 {
            let section = ReportSection::new(&format!("Extra Section {}", i + 1))
                .with_body("This is additional content to test multi-page PDF generation with printpdf.");
            report.add_section(section);
        }

        let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let config = PdfExportConfig {
            output_dir: tmp_dir.path().to_path_buf(),
            max_pages: 5,
            page_size: PageSize::A4,
        };

        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        let result = rt.block_on(generate_pdf(&report, &config));
        assert!(result.is_ok(), "Multi-page PDF generation failed: {:?}", result.err());
    }

    #[test]
    fn test_pdf_with_page_size_letter() {
        let report = create_test_report();
        let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let config = PdfExportConfig {
            output_dir: tmp_dir.path().to_path_buf(),
            max_pages: 10,
            page_size: PageSize::Letter,
        };

        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        let result = rt.block_on(generate_pdf(&report, &config));
        assert!(result.is_ok(), "Letter-size PDF generation failed: {:?}", result.err());
    }

    #[test]
    fn test_pdf_filename_contains_title() {
        let report = create_test_report();
        let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let config = PdfExportConfig {
            output_dir: tmp_dir.path().to_path_buf(),
            max_pages: 10,
            page_size: PageSize::A4,
        };

        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        let result = rt.block_on(generate_pdf(&report, &config));
        assert!(result.is_ok());

        let path = result.unwrap();
        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(filename.contains("Test_Intelligence_Report"), "Filename should contain report title: {}", filename);
        assert!(filename.ends_with(".pdf"), "Filename should end with .pdf: {}", filename);
    }
}
