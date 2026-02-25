# apex-parse

Text normalisation, HTML extraction, and structured data parsing for the crawl pipeline.

## Responsibilities

- **Normaliser** (`normalizer.rs`): Pure text-transformation functions.
  - `strip_html(text)` — remove HTML tags and decode entities.
  - `truncate_text(text, max_chars)` — Unicode-safe truncation at grapheme cluster boundaries.
  - `extract_emails(text)` — RFC-5322-aware email extraction.
  - `extract_phones(text)` — international phone number detection.
  - `parse_number(text)` — locale-tolerant numeric parsing (comma/period separators).
  - `extract_urls(text)` — URL extraction with schema validation.
  - `validate_date(text)` — ISO 8601 date string validation.
  - `extract_entity_name(text)` — capitalisation-based entity name extraction.
  - `remove_boilerplate(text)` — strip common navigation/footer phrases.
  - `dedup_lines(text)` — deduplicate adjacent repeated lines.
  - `normalize_whitespace(text)` — ASCII whitespace collapse.
  - `normalize_unicode_whitespace(text)` — Unicode whitespace collapse (U+200B, BOM, etc.).

## Design notes

- All functions are pure, `#[inline]`-friendly, and allocation-minimising where possible.
- Adversarial/fuzz tests in the `#[cfg(test)]` module cover null bytes, BOM, RTL override, 1 MB inputs, deeply nested HTML, and malformed inputs.

## Test coverage

```bash
cargo test -p apex-parse --lib
```
