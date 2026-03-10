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

    let suffixes = [
        " inc", " ltd", " llc", " corp", " s a", " sa", " s a r l", " sarl", " gmbh", " ag",
        " sas", " co", " plc", " n v", " nv",
    ];

    let mut result = compact;
    loop {
        let prev_len = result.len();
        for suffix in &suffixes {
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
            'Е' | 'е' => 'e',
            'Н' | 'н' => 'h',
            'І' | 'і' => 'i',
            'К' | 'к' => 'k',
            'М' | 'м' => 'm',
            'О' | 'о' => 'o',
            'Р' | 'р' => 'p',
            'Т' | 'т' => 't',
            'Х' | 'х' => 'x',
            'Υ' | 'υ' => 'y',
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
}
