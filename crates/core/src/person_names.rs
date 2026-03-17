use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonNameScript {
    Latin,
    Arabic,
    Hebrew,
    Cyrillic,
    Cjk,
    Hangul,
    Mixed,
    Unknown,
}

const CONNECTOR_WORDS: &[&str] = &[
    "al", "bin", "ibn", "ben", "bint", "de", "del", "da", "dos", "van", "von", "der", "la", "le",
];

const BLOCKED_TERMS: &[&str] = &[
    // English company suffixes
    "holdings",
    "limited",
    "ltd",
    "inc",
    "corp",
    "corporation",
    "group",
    "company",
    "systems",
    "technologies",
    "technology",
    "industries",
    "industrial",
    "partners",
    "capital",
    "ventures",
    "manufacturing",
    "electronics",
    // Non-English company suffixes (≥4 chars, safe for substring match)
    "gmbh",           // German
    "sarl",           // French
    "spzoo",          // Polish (sp. z o.o.)
];

/// Short company suffixes that must match as whole words only.
const BLOCKED_WORD_TERMS: &[&str] = &[
    "ag", "sa", "sas", "bv", "nv", "spa", "srl", "sl",
    "oo", "za", "ao", "kft", "rt", "as", "ab", "oy",
    "pty", "cc", "co", "plc", "llp", "lp", "llc", "jsc",
    "kda", "ykk", "kk",
];

const BLOCKED_PHRASES: &[&str] = &[
    "artificial intelligence",
    "computer science",
    "foreign policy",
    "middle east",
    "new york",
    "open source",
    "social media",
    "united states",
];

pub fn detect_person_name_script(value: &str) -> PersonNameScript {
    let mut latin = 0;
    let mut arabic = 0;
    let mut hebrew = 0;
    let mut cyrillic = 0;
    let mut cjk = 0;
    let mut hangul = 0;
    let total = value.chars().filter(|c| !c.is_whitespace()).count();

    if total == 0 {
        return PersonNameScript::Unknown;
    }

    for ch in value.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let cp = ch as u32;
        if (0x0600..=0x06FF).contains(&cp)
            || (0x0750..=0x077F).contains(&cp)
            || (0x08A0..=0x08FF).contains(&cp)
        {
            arabic += 1;
        } else if (0x0590..=0x05FF).contains(&cp) || (0xFB1D..=0xFB4F).contains(&cp) {
            hebrew += 1;
        } else if (0x0400..=0x052F).contains(&cp)
            || (0x2DE0..=0x2DFF).contains(&cp)
            || (0xA640..=0xA69F).contains(&cp)
        {
            cyrillic += 1;
        } else if (0x4E00..=0x9FFF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0x3040..=0x30FF).contains(&cp)
        {
            cjk += 1;
        } else if (0xAC00..=0xD7AF).contains(&cp) || (0x1100..=0x11FF).contains(&cp) {
            hangul += 1;
        } else if ch.is_alphabetic() || (0x00C0..=0x024F).contains(&cp) {
            latin += 1;
        }
    }

    let threshold = total / 2;
    if arabic > threshold {
        PersonNameScript::Arabic
    } else if hebrew > threshold {
        PersonNameScript::Hebrew
    } else if cyrillic > threshold {
        PersonNameScript::Cyrillic
    } else if hangul > threshold {
        PersonNameScript::Hangul
    } else if cjk > threshold {
        PersonNameScript::Cjk
    } else if latin > threshold {
        PersonNameScript::Latin
    } else {
        PersonNameScript::Mixed
    }
}

pub fn normalize_person_name(value: &str) -> String {
    value
        .nfkd()
        .filter(|ch| !is_combining_mark(*ch))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub fn looks_like_person_name(value: &str) -> bool {
    let normalized = normalize_person_name(value);
    let cleaned = normalized.trim_matches(|c: char| !c.is_alphanumeric() && !c.is_alphabetic());
    if cleaned.is_empty() || cleaned.len() > 80 || cleaned.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }

    let lowered = cleaned.to_lowercase();
    if BLOCKED_PHRASES.contains(&lowered.as_str())
        || BLOCKED_TERMS.iter().any(|term| lowered.contains(term))
        || lowered
            .split_whitespace()
            .any(|word| BLOCKED_WORD_TERMS.contains(&word))
    {
        return false;
    }

    let script = detect_person_name_script(cleaned);
    match script {
        PersonNameScript::Cjk | PersonNameScript::Hangul => {
            let chars: Vec<char> = cleaned.chars().filter(|c| !c.is_whitespace()).collect();
            !chars.is_empty() && chars.len() <= 6 && chars.iter().all(|c| c.is_alphabetic())
        }
        PersonNameScript::Unknown => false,
        _ => {
            let parts: Vec<&str> = cleaned
                .split_whitespace()
                .map(|part| {
                    part.trim_matches(|c: char| {
                        !c.is_alphabetic() && c != '-' && c != '\'' && c != '’' && c != '·'
                    })
                })
                .filter(|part| !part.is_empty())
                .collect();

            if parts.len() < 2 || parts.len() > 5 {
                return false;
            }

            let total_chars: usize = parts.iter().map(|part| part.chars().count()).sum();
            if total_chars < 4 {
                return false;
            }

            parts.iter().all(|part| token_is_person_like(part, script))
        }
    }
}

fn token_is_person_like(token: &str, script: PersonNameScript) -> bool {
    if token.is_empty() {
        return false;
    }

    let lowered = token.to_lowercase();
    if CONNECTOR_WORDS.contains(&lowered.as_str()) {
        return true;
    }

    if !token
        .chars()
        .all(|c| c.is_alphabetic() || c == '-' || c == '\'' || c == '’' || c == '·')
    {
        return false;
    }

    match script {
        PersonNameScript::Latin | PersonNameScript::Cyrillic | PersonNameScript::Mixed => token
            .chars()
            .next()
            .map(|first| first.is_uppercase())
            .unwrap_or(false),
        PersonNameScript::Arabic | PersonNameScript::Hebrew => token.chars().count() >= 2,
        PersonNameScript::Cjk | PersonNameScript::Hangul => true,
        PersonNameScript::Unknown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_name_is_person_like() {
        assert!(looks_like_person_name("Ahmed Ben Ali"));
    }

    #[test]
    fn arabic_name_is_person_like() {
        assert!(looks_like_person_name("محمد بن سالم"));
    }

    #[test]
    fn hebrew_name_is_person_like() {
        assert!(looks_like_person_name("משה דוד"));
    }

    #[test]
    fn cyrillic_name_is_person_like() {
        assert!(looks_like_person_name("Алексей Иванов"));
    }

    #[test]
    fn cjk_name_is_person_like() {
        assert!(looks_like_person_name("王小明"));
    }

    #[test]
    fn company_suffix_is_rejected() {
        assert!(!looks_like_person_name("Atlas Holdings"));
    }

    #[test]
    fn normalize_person_name_strips_diacritics() {
        assert_eq!(normalize_person_name("José García"), "Jose Garcia");
    }
}
