//! POI name transliteration for entity resolution.
//!
//! Generates multiple transliteration variants for Arabic, Hebrew,
//! Chinese, Japanese, and Korean names to improve entity resolution
//! matching across different romanization standards.

use serde::Serialize;
use std::collections::HashSet;

// ─── Transliteration result ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TransliterationResult {
    pub original: String,
    pub script: DetectedScript,
    pub variants: Vec<String>,
    /// Normalized form for matching (lowercase, no diacritics)
    pub normalized: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum DetectedScript {
    Arabic,
    Hebrew,
    Chinese,
    Japanese,
    Korean,
    Latin,
    Mixed,
    Unknown,
}

// ─── Script detection ───────────────────────────────────────────────────

/// Detect the primary script of a name.
pub fn detect_script(name: &str) -> DetectedScript {
    let mut arabic = 0;
    let mut hebrew = 0;
    let mut cjk = 0;
    let mut hangul = 0;
    let mut hiragana_katakana = 0;
    let mut latin = 0;
    let total = name.chars().filter(|c| !c.is_whitespace()).count();

    for ch in name.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let cp = ch as u32;
        if (0x0600..=0x06FF).contains(&cp)
            || (0x0750..=0x077F).contains(&cp)
            || (0xFB50..=0xFDFF).contains(&cp)
        {
            arabic += 1;
        } else if (0x0590..=0x05FF).contains(&cp) || (0xFB1D..=0xFB4F).contains(&cp) {
            hebrew += 1;
        } else if (0x4E00..=0x9FFF).contains(&cp) || (0x3400..=0x4DBF).contains(&cp) {
            cjk += 1;
        } else if (0xAC00..=0xD7AF).contains(&cp) || (0x1100..=0x11FF).contains(&cp) {
            hangul += 1;
        } else if (0x3040..=0x309F).contains(&cp) || (0x30A0..=0x30FF).contains(&cp) {
            hiragana_katakana += 1;
        } else if ch.is_ascii_alphabetic() || (0x00C0..=0x024F).contains(&cp) {
            latin += 1;
        }
    }

    if total == 0 {
        return DetectedScript::Unknown;
    }

    let threshold = total / 2;

    if arabic > threshold {
        DetectedScript::Arabic
    } else if hebrew > threshold {
        DetectedScript::Hebrew
    } else if hangul > threshold {
        DetectedScript::Korean
    } else if cjk > threshold && hiragana_katakana > 0 {
        DetectedScript::Japanese
    } else if cjk > threshold {
        DetectedScript::Chinese
    } else if latin > threshold {
        DetectedScript::Latin
    } else {
        DetectedScript::Mixed
    }
}

// ─── Arabic transliteration ─────────────────────────────────────────────

/// Common Arabic name transliteration variants.
fn arabic_variants(name: &str) -> Vec<String> {
    let mut variants = HashSet::new();

    // Common Arabic name mappings
    let patterns: &[(&str, &[&str])] = &[
        ("محمد", &["Mohammed", "Muhammad", "Mohamed", "Mohamad", "Muhammed"]),
        ("أحمد", &["Ahmed", "Ahmad", "Ahmet"]),
        ("علي", &["Ali", "Aly"]),
        ("حسن", &["Hassan", "Hasan", "Hasen"]),
        ("حسين", &["Hussein", "Hussain", "Husein", "Hossein"]),
        ("عبد", &["Abd", "Abdel", "Abdul", "Abdu"]),
        ("الله", &["Allah", "Alla", "Ellah"]),
        ("عبدالله", &["Abdullah", "Abdallah", "Abdellah"]),
        ("عبدالرحمن", &["Abdulrahman", "Abderrahman", "Abderrahmane"]),
        ("إبراهيم", &["Ibrahim", "Ibraheem", "Brahim"]),
        ("يوسف", &["Youssef", "Yusuf", "Yusef", "Yousuf"]),
        ("خالد", &["Khaled", "Khalid", "Khālid"]),
        ("عمر", &["Omar", "Umar", "Omer"]),
        ("طارق", &["Tarek", "Tariq", "Tarik"]),
        ("كريم", &["Karim", "Kareem", "Krim"]),
        ("سليم", &["Slim", "Salim", "Selim"]),
        ("منصف", &["Moncef", "Monsif", "Munsif"]),
        ("رضا", &["Ridha", "Rida", "Reda", "Reza"]),
    ];

    // Check exact matches against known names
    for (arabic, roman) in patterns {
        if name.contains(arabic) {
            for variant in *roman {
                variants.insert(variant.to_string());
            }
        }
    }

    // If no known patterns matched, apply rule-based transliteration
    if variants.is_empty() {
        let basic = basic_arabic_transliterate(name);
        variants.insert(basic);
    }

    variants.into_iter().collect()
}

fn basic_arabic_transliterate(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        let replacement = match ch {
            'ا' | 'أ' | 'إ' | 'آ' => "a",
            'ب' => "b",
            'ت' => "t",
            'ث' => "th",
            'ج' => "j",
            'ح' => "h",
            'خ' => "kh",
            'د' => "d",
            'ذ' => "dh",
            'ر' => "r",
            'ز' => "z",
            'س' => "s",
            'ش' => "sh",
            'ص' => "s",
            'ض' => "d",
            'ط' => "t",
            'ظ' => "z",
            'ع' => "a",
            'غ' => "gh",
            'ف' => "f",
            'ق' => "q",
            'ك' => "k",
            'ل' => "l",
            'م' => "m",
            'ن' => "n",
            'ه' => "h",
            'و' => "w",
            'ي' => "y",
            'ة' => "a",
            'ى' => "a",
            'ء' => "",
            ' ' => " ",
            _ => "",
        };
        result.push_str(replacement);
    }
    result
}

// ─── Hebrew transliteration ─────────────────────────────────────────────

fn hebrew_variants(name: &str) -> Vec<String> {
    let mut variants = HashSet::new();

    let patterns: &[(&str, &[&str])] = &[
        ("משה", &["Moshe", "Moše", "Moses"]),
        ("דוד", &["David", "Daveed", "Dávid"]),
        ("יעקב", &["Yaakov", "Yakov", "Jacob", "Yacob"]),
        ("אברהם", &["Avraham", "Abraham", "Avram"]),
        ("יצחק", &["Yitzhak", "Isaac", "Itzhak", "Yitzchak"]),
        ("בנימין", &["Binyamin", "Benjamin", "Benny"]),
        ("שמעון", &["Shimon", "Simon", "Simeon"]),
        ("חיים", &["Chaim", "Haim", "Hayim"]),
        ("נתן", &["Natan", "Nathan", "Netanel"]),
        ("אלי", &["Eli", "Ely"]),
        ("גל", &["Gal"]),
        ("עמית", &["Amit"]),
        ("רון", &["Ron"]),
        ("יוסי", &["Yossi", "Yosi"]),
    ];

    for (hebrew, roman) in patterns {
        if name.contains(hebrew) {
            for variant in *roman {
                variants.insert(variant.to_string());
            }
        }
    }

    if variants.is_empty() {
        let basic = basic_hebrew_transliterate(name);
        variants.insert(basic);
    }

    variants.into_iter().collect()
}

fn basic_hebrew_transliterate(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        let replacement = match ch {
            'א' => "a",
            'ב' => "b",
            'ג' => "g",
            'ד' => "d",
            'ה' => "h",
            'ו' => "v",
            'ז' => "z",
            'ח' => "ch",
            'ט' => "t",
            'י' => "y",
            'כ' | 'ך' => "k",
            'ל' => "l",
            'מ' | 'ם' => "m",
            'נ' | 'ן' => "n",
            'ס' => "s",
            'ע' => "a",
            'פ' | 'ף' => "p",
            'צ' | 'ץ' => "tz",
            'ק' => "k",
            'ר' => "r",
            'ש' => "sh",
            'ת' => "t",
            ' ' => " ",
            _ => "",
        };
        result.push_str(replacement);
    }
    result
}

// ─── Chinese/Japanese/Korean ─────────────────────────────────────────────

fn cjk_variants(name: &str, _script: DetectedScript) -> Vec<String> {
    let mut variants = HashSet::new();

    // Common CJK surname lookup
    let surname_map: &[(&str, &[&str])] = &[
        ("王", &["Wang", "Wong"]),
        ("李", &["Li", "Lee"]),
        ("张", &["Zhang", "Chang", "Cheung"]),
        ("刘", &["Liu", "Lau"]),
        ("陈", &["Chen", "Chan"]),
        ("杨", &["Yang", "Yeung"]),
        ("黄", &["Huang", "Wong"]),
        ("赵", &["Zhao", "Chiu"]),
        ("吴", &["Wu", "Ng", "Goh"]),
        ("周", &["Zhou", "Chow"]),
        ("林", &["Lin", "Lam"]),
        ("田中", &["Tanaka"]),
        ("山田", &["Yamada"]),
        ("佐藤", &["Sato", "Satoh"]),
        ("鈴木", &["Suzuki"]),
        ("高橋", &["Takahashi"]),
        ("김", &["Kim", "Gim"]),
        ("이", &["Lee", "Yi", "Rhee"]),
        ("박", &["Park", "Pak", "Bak"]),
        ("최", &["Choi", "Choe"]),
        ("정", &["Jung", "Chung", "Jeong"]),
    ];

    for (chars, roman) in surname_map {
        if name.contains(chars) {
            for variant in *roman {
                variants.insert(variant.to_string());
            }
        }
    }

    // Record detected script as metadata (not added to variants to avoid polluting entity resolution)
    // Script information is available via detect_script() if callers need it.

    variants.into_iter().collect()
}

// ─── Main API ───────────────────────────────────────────────────────────

/// Generate all transliteration variants for a name.
pub fn transliterate(name: &str) -> TransliterationResult {
    let script = detect_script(name);
    let variants = match script {
        DetectedScript::Arabic => arabic_variants(name),
        DetectedScript::Hebrew => hebrew_variants(name),
        DetectedScript::Chinese | DetectedScript::Japanese | DetectedScript::Korean => {
            cjk_variants(name, script)
        }
        DetectedScript::Latin => {
            // For Latin names, generate normalized variants
            vec![
                name.to_lowercase(),
                strip_diacritics(name),
            ]
        }
        _ => vec![name.to_string()],
    };

    let normalized = normalize_for_matching(name);

    TransliterationResult {
        original: name.to_string(),
        script,
        variants,
        normalized,
    }
}

/// Normalize a name for fuzzy matching: lowercase, remove diacritics,
/// collapse whitespace, strip titles.
pub fn normalize_for_matching(name: &str) -> String {
    let stripped = strip_diacritics(name);
    let lower = stripped.to_lowercase();

    // Remove common titles
    let titles = [
        "dr.", "dr ", "prof.", "prof ", "mr.", "mr ", "mrs.", "mrs ",
        "ms.", "ms ", "ing.", "ing ", "eng.", "eng ",
    ];
    let mut result = lower;
    for title in &titles {
        if result.starts_with(title) {
            result = result[title.len()..].to_string();
        }
    }

    // Collapse whitespace
    result.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

/// Strip common diacritics from Latin text.
fn strip_diacritics(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            'ş' => 's',
            'ğ' => 'g',
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'A',
            'É' | 'È' | 'Ê' | 'Ë' => 'E',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'O',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
            'Ñ' => 'N',
            'Ç' => 'C',
            _ => c,
        })
        .collect()
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_arabic() {
        assert_eq!(detect_script("محمد بن سالم"), DetectedScript::Arabic);
    }

    #[test]
    fn test_detect_hebrew() {
        assert_eq!(detect_script("משה דוד"), DetectedScript::Hebrew);
    }

    #[test]
    fn test_detect_chinese() {
        assert_eq!(detect_script("王大明"), DetectedScript::Chinese);
    }

    #[test]
    fn test_detect_korean() {
        assert_eq!(detect_script("김철수"), DetectedScript::Korean);
    }

    #[test]
    fn test_detect_latin() {
        assert_eq!(detect_script("John Smith"), DetectedScript::Latin);
    }

    #[test]
    fn test_arabic_transliteration() {
        let result = transliterate("محمد");
        assert_eq!(result.script, DetectedScript::Arabic);
        assert!(result.variants.iter().any(|v| v == "Mohammed" || v == "Muhammad" || v == "Mohamed"));
    }

    #[test]
    fn test_hebrew_transliteration() {
        let result = transliterate("דוד");
        assert_eq!(result.script, DetectedScript::Hebrew);
        assert!(result.variants.iter().any(|v| v == "David" || v == "Daveed"));
    }

    #[test]
    fn test_chinese_variants() {
        let result = transliterate("王");
        assert!(result.variants.iter().any(|v| v == "Wang" || v == "Wong"));
    }

    #[test]
    fn test_korean_variants() {
        let result = transliterate("김");
        assert!(result.variants.iter().any(|v| v == "Kim"));
    }

    #[test]
    fn test_normalize_strips_title() {
        assert_eq!(normalize_for_matching("Dr. Ahmed Ben Salah"), "ahmed ben salah");
    }

    #[test]
    fn test_normalize_strips_diacritics() {
        assert_eq!(normalize_for_matching("José García"), "jose garcia");
    }

    #[test]
    fn test_strip_diacritics() {
        assert_eq!(strip_diacritics("café résumé naïve"), "cafe resume naive");
    }
}
