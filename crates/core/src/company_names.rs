use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

static RE_NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w\s]").unwrap());
static RE_MULTI_WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Normalize a company name for cross-crate fuzzy matching and dedup.
pub fn normalize_company_name(name: &str) -> String {
    let stripped = strip_diacritics(name);
    let normalized_script = normalize_mixed_script_confusables(&stripped);
    let lower = normalized_script.to_lowercase().trim().to_string();
    let compact = RE_MULTI_WS
        .replace_all(&RE_NON_WORD.replace_all(&lower, " "), " ")
        .trim()
        .to_string();
    let compact = collapse_letter_sequences(&compact);

    let suffixes = [
        " inc", " ltd", " llc", " corp", " s a", " sa", " s a r l", " sarl", " gmbh", " ag",
        " sas", " co", " plc", " n v", " nv",
    ];

    let mut result = compact;
    loop {
        let prev_len = result.len();
        for suffix in &suffixes {
            let bare_suffix = suffix.trim();
            if result == bare_suffix {
                result.clear();
                break;
            }
            if result.ends_with(suffix) {
                result = result[..result.len() - suffix.len()].trim().to_string();
                break;
            }
        }
        if result.len() == prev_len {
            break;
        }
    }

    result
}

fn collapse_letter_sequences(input: &str) -> String {
    let mut collapsed = Vec::new();
    let mut letter_run = String::new();

    for token in input.split_whitespace() {
        if token.len() == 1 && token.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            letter_run.push_str(token);
            continue;
        }

        if !letter_run.is_empty() {
            collapsed.push(std::mem::take(&mut letter_run));
        }
        collapsed.push(token.to_string());
    }

    if !letter_run.is_empty() {
        collapsed.push(letter_run);
    }

    collapsed.join(" ")
}

fn strip_diacritics(input: &str) -> String {
    input.nfd().filter(|ch| !is_combining_mark(*ch)).collect()
}

fn normalize_mixed_script_confusables(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            'А' | 'а' => 'a',
            'В' | 'в' => 'b',
            'С' | 'с' => 'c',
            'ԁ' => 'd',
            'Е' | 'е' => 'e',
            'Ғ' | 'ғ' => 'f',
            'ɢ' | 'Ԍ' | 'ɡ' | 'Գ' | 'г' | 'Г' => 'g',
            'Н' | 'н' => 'h',
            'І' | 'і' => 'i',
            'Ј' | 'ј' => 'j',
            'К' | 'к' => 'k',
            'Լ' => 'l',
            'М' | 'м' => 'm',
            'Ν' | 'П' | 'п' | 'η' => 'n',
            'О' | 'о' => 'o',
            'Ο' | 'ο' | 'Ө' | 'ө' | 'Օ' => 'o',
            'Р' | 'р' => 'p',
            'ԛ' => 'q',
            'Γ' => 'r',
            'Т' | 'т' => 't',
            'τ' => 't',
            'Ս' => 'u',
            'ν' | 'ѵ' => 'v',
            'Ԝ' => 'w',
            'Х' | 'х' => 'x',
            'Υ' | 'υ' | 'Ү' | 'ү' => 'y',
            'Ζ' | 'z' | 'ᴢ' => 'z',
            'Β' => 'b',
            'Ι' => 'i',
            'Κ' => 'k',
            'Μ' => 'm',
            'Τ' => 't',
            'Ρ' => 'p',
            _ => ch,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::normalize_company_name;

    #[test]
    fn normalize_company_name_strips_suffixes_and_punctuation() {
        assert_eq!(
            normalize_company_name("Young Poong Electronics Co., Ltd."),
            "young poong electronics"
        );
        assert_eq!(
            normalize_company_name("STMicroelectronics N.V."),
            "stmicroelectronics"
        );
    }

    #[test]
    fn normalize_company_name_strips_diacritics() {
        assert_eq!(normalize_company_name("Cafe Societe S.A."), "cafe societe");
        assert_eq!(normalize_company_name("Café Société S.A."), "cafe societe");
    }

    #[test]
    fn normalize_company_name_normalizes_mixed_script_confusables() {
        let mixed = format!("{}cme", '\u{0410}');
        assert_eq!(normalize_company_name(&mixed), "acme");
    }

    #[test]
    fn normalize_company_name_collapses_letter_sequences() {
        assert_eq!(normalize_company_name("A.B.C. Corp."), "abc");
        assert_eq!(normalize_company_name("N.V."), "");
    }

    #[test]
    fn fuzz_company_name_normalize_no_panic() {
        let inputs = [
            "",
            "Rоsatом",
            "A\u{200B}B\u{200C}C",
            "شركة التقنية العالمية ذ.م.م",
            "株式会社テスト",
        ];

        for input in inputs {
            let normalized = normalize_company_name(input);
            assert!(normalized.is_ascii() || !normalized.is_empty() || input.is_empty());
        }
    }
}
