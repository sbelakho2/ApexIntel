#[cfg(test)]
mod tests {
    use apex_api::pdf_writer;
    use apex_insights::pdf_report::{PdfReport, ReportSection, ReportType};

    #[test]
    fn test_generate_and_inspect_pdf() {
        let mut report = PdfReport::new("Manual PDF Test", ReportType::InsightSummary);
        report.add_section(
            ReportSection::new("Test Section")
                .with_body("This is test body content that should appear in the PDF."),
        );
        let pdf = pdf_writer::render_report_to_pdf(&report).unwrap();

        // Save to file
        std::fs::write("/tmp/manual_test_pdf.pdf", &pdf).unwrap();

        // Print first 500 bytes as hex for inspection
        let preview: Vec<String> = pdf.iter().take(500).map(|b| format!("{:02X}", b)).collect();
        eprintln!("PDF size: {} bytes", pdf.len());
        eprintln!("PDF header: {:?}", &pdf[..std::cmp::min(50, pdf.len())]);
        eprintln!("First 500 bytes hex: {}", preview.join(" "));

        // Search for CLASSIFICATION text in raw bytes
        let search = b"CLASSIFICATION";
        let found = pdf.windows(search.len()).any(|w| w == search);
        eprintln!("Raw 'CLASSIFICATION' found: {}", found);

        // Search for hex-encoded CLASSIFICATION
        let hex_str: String = b"CLASSIFICATION"
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect();
        let hex_bytes = hex_str.as_bytes();
        let found_hex = pdf.windows(hex_bytes.len()).any(|w| w == hex_bytes);
        eprintln!("Hex-encoded '{}' found: {}", hex_str, found_hex);

        assert!(pdf.len() > 100, "PDF should be non-empty");
    }
}
