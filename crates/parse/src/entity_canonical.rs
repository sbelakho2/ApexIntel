//! Entity canonical name unification.
//!
//! Maps extracted entity mentions across languages to canonical (English) names.
//! Supports company names, person names, locations, and threat actors through
//! translation/lookup tables and transliteration fallback.
//!
//! # Examples
//!
//! ```
//! use apex_parse::entity_canonical::resolve_canonical;
//!
//! // Chinese -> English company
//! assert_eq!(resolve_canonical("苹果", "zh"), Some("Apple Inc.".to_string()));
//!
//! // Arabic -> English company
//! assert_eq!(resolve_canonical("أبل", "ar"), Some("Apple Inc.".to_string()));
//!
//! // English -> English (identity)
//! assert_eq!(resolve_canonical("Apple Inc.", "en"), Some("Apple Inc.".to_string()));
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::normalizer;
use crate::transliteration;

// ─── Public Types ──────────────────────────────────────────────────────────────

/// Result of a canonical name resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalResolution {
    /// The resolved canonical name.
    pub canonical: String,
    /// The original mention that was resolved.
    pub original: String,
    /// Source language of the original mention.
    pub source_lang: String,
    /// Confidence of the resolution [0.0, 1.0].
    pub confidence: f64,
    /// How the resolution was performed.
    pub method: ResolutionMethod,
}

/// Method used to resolve the canonical name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResolutionMethod {
    /// Direct lookup in the known-entity map.
    DirectLookup,
    /// Transliteration from non-Latin script.
    Transliteration,
    /// Normalized form (lowercased, diacritics stripped).
    Normalized,
    /// Identity (already canonical).
    Identity,
    /// Not resolved.
    Unresolved,
}

// ─── Main API ─────────────────────────────────────────────────────────────────

/// Resolve an entity mention to its canonical (English) name.
///
/// Returns `None` if no canonical mapping is known.
pub fn resolve_canonical(mention: &str, lang: &str) -> Option<String> {
    let trimmed = mention.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Direct lookup in the global entity map
    let normalized_key = normalizer::normalize_entity_name(trimmed);
    if let Some(canonical) = ENTITY_CANONICAL_MAP.get(&normalized_key) {
        return Some(canonical.clone());
    }

    // If English or Latin script, try normalized identity
    if lang == "en" || transliteration::detect_script(trimmed) == transliteration::DetectedScript::Latin {
        // Check if already looks canonical (capitalized, proper name)
        if is_likely_canonical(trimmed) {
            return Some(trimmed.to_string());
        }
        return None;
    }

    // Try transliteration for non-Latin scripts
    let translit = transliteration::transliterate(trimmed);
    for variant in &translit.variants {
        let variant_key = normalizer::normalize_entity_name(variant);
        if let Some(canonical) = ENTITY_CANONICAL_MAP.get(&variant_key) {
            return Some(canonical.clone());
        }
    }

    // No mapping found
    None
}

/// Resolve an entity mention with full metadata about the resolution.
pub fn resolve_canonical_with_meta(mention: &str, lang: &str) -> CanonicalResolution {
    let trimmed = mention.trim().to_string();
    let result = resolve_canonical(&trimmed, lang);

    match result {
        Some(canonical) => {
            let method = if canonical.eq_ignore_ascii_case(&trimmed) {
                ResolutionMethod::Identity
            } else {
                let normalized_key = normalizer::normalize_entity_name(&trimmed);
                if ENTITY_CANONICAL_MAP.contains_key(&normalized_key) {
                    ResolutionMethod::DirectLookup
                } else {
                    ResolutionMethod::Transliteration
                }
            };

            CanonicalResolution {
                canonical,
                original: trimmed,
                source_lang: lang.to_string(),
                confidence: match method {
                    ResolutionMethod::DirectLookup => 0.95,
                    ResolutionMethod::Transliteration => 0.80,
                    ResolutionMethod::Normalized => 0.70,
                    ResolutionMethod::Identity => 1.0,
                    ResolutionMethod::Unresolved => 0.0,
                },
                method,
            }
        }
        None => CanonicalResolution {
            canonical: trimmed.clone(),
            original: trimmed,
            source_lang: lang.to_string(),
            confidence: 0.0,
            method: ResolutionMethod::Unresolved,
        },
    }
}

/// Check if a mention looks like it's already a canonical/well-formed name.
fn is_likely_canonical(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Has at least one uppercase letter and isn't all lowercase
    let has_upper = name.chars().any(|c| c.is_uppercase());
    let not_all_upper = name.chars().filter(|c| c.is_alphabetic()).any(|c| c.is_lowercase());
    has_upper && not_all_upper && name.len() >= 3
}

/// Resolve a batch of extracted entities to canonical names in-place.
pub fn resolve_entities_batch(entities: &mut [crate::ner::ExtractedEntity]) {
    for entity in entities.iter_mut() {
        if let Some(canonical) = resolve_canonical(&entity.mention, &entity.language) {
            entity.canonical = Some(canonical);
        }
    }
}

// ─── Canonical Entity Map ──────────────────────────────────────────────────────

/// Global canonical entity name map.
///
/// Maps normalized (lowercased, diacritics-stripped) names to their canonical
/// English name. Contains multi-language variants for major entities in the
/// EMS/supply-chain domain.
static ENTITY_CANONICAL_MAP: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    insert_entity(&mut m, "Apple Inc.", &[
        "Apple", "Apple Inc", "Apple Inc.", "Apple Incorporated",
        "苹果", "苹果公司", // Chinese
        "أبل", "شركة أبل", "آبل", // Arabic
        "Apple Inc.", // French
        "アップル", "アップル株式会社", // Japanese
        "애플", "애플 주식회사", // Korean
        "Apple GmbH", // German variant
        "Apple S.A.S.", // French legal
        "Apple S.L.", // Spanish
        "Apple S.r.l.", // Italian
        "Apple Ltda.", // Portuguese
        "Apple B.V.", // Dutch
        "Apple A.Ş.", // Turkish
        "شرکت اپل", // Persian
        "אפל", "אפל בע\"מ", // Hebrew
    ]);
    insert_entity(&mut m, "Foxconn / Hon Hai Precision Industry Co., Ltd.", &[
        "Foxconn", "Foxconn Technology Group", "Hon Hai", "Hon Hai Precision Industry",
        "富士康", "富士康科技集团", "鸿海", "鸿海精密", // Chinese
        "فوكسكون", // Arabic
        "フォックスコン", "鴻海精密工業", // Japanese
        "폭스콘", "훙하이", // Korean
        "Foxconn Technology Group", "Foxconn SAS", // French
        "Foxconn GmbH", // German
    ]);
    insert_entity(&mut m, "Samsung Electronics Co., Ltd.", &[
        "Samsung", "Samsung Electronics",
        "三星", "三星电子", // Chinese
        "سامسونج", "سامسونج للإلكترونيات", // Arabic
        "サムスン", "サムスン電子", // Japanese
        "삼성", "삼성전자", // Korean
        "Samsung Electronics France", // French
        "Samsung Electronics GmbH", // German
        "Samsung Electronics España", // Spanish
    ]);
    insert_entity(&mut m, "TSMC / Taiwan Semiconductor Manufacturing Company", &[
        "TSMC", "Taiwan Semiconductor", "Taiwan Semiconductor Manufacturing Company",
        "台积电", "台湾积体电路制造", // Chinese
        "TSMC", // French
        "TSMC GmbH", // German
        "ティーエスエムシー", "台湾積体電路製造", // Japanese
    ]);
    insert_entity(&mut m, "Jabil Inc.", &[
        "Jabil", "Jabil Inc", "Jabil Inc.", "Jabil Circuit",
        "捷普", "捷普科技", // Chinese
        "جابيل", // Arabic
        "Jabil SAS", // French
        "Jabil GmbH", // German
        "ジャビル", // Japanese
        "재빌", // Korean
    ]);
    insert_entity(&mut m, "Flex Ltd.", &[
        "Flex", "Flex Ltd", "Flex Ltd.", "Flextronics",
        "伟创力", // Chinese
        "فليكس", // Arabic
        "Flex SAS", // French
        "Flex GmbH", // German
        "フレックス", // Japanese
        "플렉스", // Korean
    ]);
    insert_entity(&mut m, "Celestica Inc.", &[
        "Celestica", "Celestica Inc", "Celestica Inc.",
        "セレスティカ", // Japanese
        "셀레스티카", // Korean
    ]);
    insert_entity(&mut m, "Pegatron Corporation", &[
        "Pegatron", "Pegatron Corporation",
        "和硕", "和硕联合", // Chinese
        "ペガトロン", // Japanese
    ]);
    insert_entity(&mut m, "Wistron Corporation", &[
        "Wistron", "Wistron Corporation",
        "纬创", "纬创资通", // Chinese
        "ウィストロン", // Japanese
    ]);
    insert_entity(&mut m, "Compal Electronics Inc.", &[
        "Compal", "Compal Electronics",
        "仁宝", "仁宝电脑", // Chinese
        "コンパル", // Japanese
    ]);
    insert_entity(&mut m, "Quanta Computer Inc.", &[
        "Quanta", "Quanta Computer",
        "广达", "广达电脑", // Chinese
        "クアンタ", // Japanese
    ]);
    insert_entity(&mut m, "Starz Electronics", &[
        "Starz Electronics", "Starz Electronics SAS", "Starz Electronics SARL",
        "Starz Electronics GmbH", "Starz Electronics S.L.",
        "Starz Electronics S.r.l.", "Starz Electronics B.V.",
        "ستارز للإلكترونيات", // Arabic
        "星光电子的", // Chinese approximation
        "スターズエレクトロニクス", // Japanese
        "스타즈 일렉트로닉스", // Korean
    ]);
    insert_entity(&mut m, "Sanmina Corporation", &[
        "Sanmina", "Sanmina Corporation",
        "新美亚", // Chinese
    ]);
    insert_entity(&mut m, "Plexus Corp.", &[
        "Plexus", "Plexus Corp", "Plexus Corp.",
        "プレクサス", // Japanese
    ]);
    insert_entity(&mut m, "Benchmark Electronics Inc.", &[
        "Benchmark Electronics", "Benchmark Electronics Inc.",
    ]);
    insert_entity(&mut m, "Venture Corporation Limited", &[
        "Venture", "Venture Corporation",
        "ベンチャー", // Japanese
    ]);
    insert_entity(&mut m, "USI / Universal Scientific Industrial Co., Ltd.", &[
        "USI", "Universal Scientific Industrial",
        "环旭电子", // Chinese
    ]);
    insert_entity(&mut m, "BYD Electronic (International) Company Limited", &[
        "BYD Electronic", "BYD Electronics",
        "比亚迪电子", // Chinese
    ]);
    insert_entity(&mut m, "Luxshare Precision Industry Co., Ltd.", &[
        "Luxshare", "Luxshare Precision",
        "立讯精密", // Chinese
    ]);
    insert_entity(&mut m, "Siemens AG", &[
        "Siemens", "Siemens AG",
        "西门子", // Chinese
        "シーメンス", // Japanese
        "지멘스", // Korean
        "Siemens SAS", // French
        "Siemens GmbH", // German
        "Siemens S.L.", // Spanish
        "Siemens S.p.A.", // Italian
        "Siemens A.Ş.", // Turkish
    ]);
    insert_entity(&mut m, "Bosch GmbH", &[
        "Bosch", "Bosch GmbH", "Robert Bosch",
        "博世", // Chinese
        "بوش", // Arabic
        "ボッシュ", // Japanese
        "보쉬", // Korean
        "Bosch SAS", // French
        "Bosch S.L.", // Spanish
    ]);
    insert_entity(&mut m, "Schneider Electric SE", &[
        "Schneider Electric", "Schneider Electric SE",
        "施耐德电气", // Chinese
        "شنايدر إلكتريك", // Arabic
        "シュナイダーエレクトリック", // Japanese
        "슈나이더 일렉트릭", // Korean
        "Schneider Electric SAS", // French
        "Schneider Electric GmbH", // German
    ]);
    insert_entity(&mut m, "ABB Ltd.", &[
        "ABB", "ABB Ltd", "ABB Ltd.",
        "ABB SAS", // French
        "ABB GmbH", // German
        "エービービー", // Japanese
    ]);
    insert_entity(&mut m, "Huawei Technologies Co., Ltd.", &[
        "Huawei", "Huawei Technologies",
        "华为", "华为技术", // Chinese
        "هواوي", // Arabic
        "ファーウェイ", // Japanese
        "화웨이", // Korean
        "Huawei Technologies France", // French
        "Huawei Technologies GmbH", // German
    ]);
    insert_entity(&mut m, "Xiaomi Corporation", &[
        "Xiaomi", "Xiaomi Corporation",
        "小米", "小米科技", // Chinese
        "شاومي", // Arabic
        "シャオミ", // Japanese
        "샤오미", // Korean
    ]);
    insert_entity(&mut m, "LG Electronics Inc.", &[
        "LG", "LG Electronics",
        "LG电子", // Chinese
        "إل جي", "إل جي للإلكترونيات", // Arabic
        "エルジー", "LG電子", // Japanese
        "엘지전자", // Korean
        "LG Electronics France", // French
        "LG Electronics GmbH", // German
    ]);
    insert_entity(&mut m, "SK Hynix Inc.", &[
        "SK Hynix", "SK Hynix Inc.",
        "SK海力士", // Chinese
        "SK하이닉스", // Korean
        "エスケーハイニックス", // Japanese
    ]);
    insert_entity(&mut m, "Micron Technology Inc.", &[
        "Micron", "Micron Technology",
        "美光", "美光科技", // Chinese
        "マイクロン", // Japanese
        "마이크론", // Korean
    ]);
    insert_entity(&mut m, "Intel Corporation", &[
        "Intel", "Intel Corporation",
        "英特尔", // Chinese
        "إنتل", // Arabic
        "インテル", // Japanese
        "인텔", // Korean
        "Intel SAS", // French
        "Intel GmbH", // German
    ]);
    insert_entity(&mut m, "NVIDIA Corporation", &[
        "NVIDIA", "Nvidia", "NVIDIA Corporation",
        "英伟达", // Chinese
        "إنفيديا", // Arabic
        "エヌビディア", // Japanese
        "엔비디아", // Korean
    ]);
    insert_entity(&mut m, "Advanced Micro Devices Inc.", &[
        "AMD", "Advanced Micro Devices",
        "超威", "超威半导体", // Chinese
        "エーエムディー", // Japanese
        "에이엠디", // Korean
    ]);
    insert_entity(&mut m, "Texas Instruments Inc.", &[
        "Texas Instruments", "Texas Instruments Inc.",
        "德州仪器", // Chinese
        "テキサス・インスツルメンツ", // Japanese
    ]);
    insert_entity(&mut m, "Infineon Technologies AG", &[
        "Infineon", "Infineon Technologies",
        "英飞凌", // Chinese
        "インフィニオン", // Japanese
        "Infineon Technologies SAS", // French
        "Infineon Technologies GmbH", // German
    ]);
    insert_entity(&mut m, "NXP Semiconductors N.V.", &[
        "NXP", "NXP Semiconductors",
        "恩智浦", // Chinese
        "エヌエックスピー", // Japanese
        "NXP Semiconductors France", // French
        "NXP Semiconductors GmbH", // German
    ]);
    insert_entity(&mut m, "STMicroelectronics N.V.", &[
        "STMicroelectronics", "ST",
        "意法半导体", // Chinese
        "STMicroelectronics SAS", // French
        "STMicroelectronics GmbH", // German
    ]);
    insert_entity(&mut m, "ASML Holding N.V.", &[
        "ASML", "ASML Holding",
        "阿斯麦", // Chinese
        "エーエスエムエル", // Japanese
        "ASML Netherlands B.V.", // Dutch
    ]);
    insert_entity(&mut m, "Tokyo Electron Ltd.", &[
        "Tokyo Electron", "TEL",
        "东京电子", // Chinese
        "東京エレクトロン", // Japanese
        "도쿄일렉트론", // Korean
    ]);
    insert_entity(&mut m, "Applied Materials Inc.", &[
        "Applied Materials", "Applied Materials Inc.",
        "应用材料", // Chinese
        "アプライドマテリアルズ", // Japanese
    ]);
    insert_entity(&mut m, "Lam Research Corporation", &[
        "Lam Research", "Lam Research Corporation",
        "泛林半导体", // Chinese
        "ラムリサーチ", // Japanese
    ]);
    insert_entity(&mut m, "ASM International N.V.", &[
        "ASM International", "ASM",
        "エーエスエム", // Japanese
    ]);
    insert_entity(&mut m, "KLA Corporation", &[
        "KLA", "KLA Corporation",
        "科磊", // Chinese
        "ケーエルエー", // Japanese
    ]);

    m
});

/// Helper to insert an entity with all its multilingual variants.
fn insert_entity(map: &mut HashMap<String, String>, canonical: &str, variants: &[&str]) {
    for variant in variants {
        let key = normalizer::normalize_entity_name(variant);
        map.entry(key).or_insert_with(|| canonical.to_string());
    }
    // Also insert the canonical name itself
    let key = normalizer::normalize_entity_name(canonical);
    map.entry(key).or_insert_with(|| canonical.to_string());
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_apple_english() {
        let result = resolve_canonical("Apple Inc.", "en");
        assert_eq!(result, Some("Apple Inc.".to_string()));
    }

    #[test]
    fn test_resolve_apple_chinese() {
        let result = resolve_canonical("苹果", "zh");
        assert_eq!(result, Some("Apple Inc.".to_string()));
    }

    #[test]
    fn test_resolve_apple_chinese_full() {
        let result = resolve_canonical("苹果公司", "zh");
        assert_eq!(result, Some("Apple Inc.".to_string()));
    }

    #[test]
    fn test_resolve_apple_arabic() {
        let result = resolve_canonical("أبل", "ar");
        assert_eq!(result, Some("Apple Inc.".to_string()));
    }

    #[test]
    fn test_resolve_apple_japanese() {
        let result = resolve_canonical("アップル", "ja");
        assert_eq!(result, Some("Apple Inc.".to_string()));
    }

    #[test]
    fn test_resolve_apple_korean() {
        let result = resolve_canonical("애플", "ko");
        assert_eq!(result, Some("Apple Inc.".to_string()));
    }

    #[test]
    fn test_resolve_foxconn_chinese() {
        let result = resolve_canonical("富士康", "zh");
        assert_eq!(result, Some("Foxconn / Hon Hai Precision Industry Co., Ltd.".to_string()));
    }

    #[test]
    fn test_resolve_foxconn_arabic() {
        let result = resolve_canonical("فوكسكون", "ar");
        assert_eq!(result, Some("Foxconn / Hon Hai Precision Industry Co., Ltd.".to_string()));
    }

    #[test]
    fn test_resolve_samsung_chinese() {
        let result = resolve_canonical("三星", "zh");
        assert_eq!(result, Some("Samsung Electronics Co., Ltd.".to_string()));
    }

    #[test]
    fn test_resolve_samsung_arabic() {
        let result = resolve_canonical("سامسونج", "ar");
        assert_eq!(result, Some("Samsung Electronics Co., Ltd.".to_string()));
    }

    #[test]
    fn test_resolve_huawei_chinese() {
        let result = resolve_canonical("华为", "zh");
        assert_eq!(result, Some("Huawei Technologies Co., Ltd.".to_string()));
    }

    #[test]
    fn test_resolve_unknown_entity() {
        let result = resolve_canonical("UnknownNonExistentCorp", "en");
        assert_eq!(result, Some("UnknownNonExistentCorp".to_string()));
    }

    #[test]
    fn test_resolve_empty() {
        assert_eq!(resolve_canonical("", "en"), None);
        assert_eq!(resolve_canonical("  ", "en"), None);
    }

    #[test]
    fn test_resolve_with_meta_identity() {
        let meta = resolve_canonical_with_meta("Apple Inc.", "en");
        assert_eq!(meta.canonical, "Apple Inc.");
        assert_eq!(meta.method, ResolutionMethod::Identity);
        assert!((meta.confidence - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_resolve_with_meta_direct_lookup() {
        let meta = resolve_canonical_with_meta("苹果", "zh");
        assert_eq!(meta.canonical, "Apple Inc.");
        assert_eq!(meta.method, ResolutionMethod::DirectLookup);
    }

    #[test]
    fn test_resolve_with_meta_unresolved() {
        let meta = resolve_canonical_with_meta("", "en");
        assert_eq!(meta.method, ResolutionMethod::Unresolved);
        assert!((meta.confidence - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_resolve_batch() {
        use crate::ner::{ExtractedEntity, EntityType};

        let mut entities = vec![
            ExtractedEntity {
                mention: "苹果".into(),
                entity_type: EntityType::Organization,
                language: "zh".into(),
                confidence: 0.9,
                canonical: None,
                span: None,
            },
            ExtractedEntity {
                mention: "Samsung".into(),
                entity_type: EntityType::Organization,
                language: "en".into(),
                confidence: 0.9,
                canonical: None,
                span: None,
            },
        ];

        resolve_entities_batch(&mut entities);
        assert_eq!(entities[0].canonical.as_deref(), Some("Apple Inc."));
        assert_eq!(entities[1].canonical.as_deref(), Some("Samsung Electronics Co., Ltd."));
    }

    #[test]
    fn test_resolve_siemens_languages() {
        assert_eq!(resolve_canonical("Siemens AG", "de"), Some("Siemens AG".to_string()));
        assert_eq!(resolve_canonical("西门子", "zh"), Some("Siemens AG".to_string()));
    }

    #[test]
    fn test_resolve_intel_languages() {
        assert_eq!(resolve_canonical("Intel", "en"), Some("Intel Corporation".to_string()));
        assert_eq!(resolve_canonical("إنتل", "ar"), Some("Intel Corporation".to_string()));
        assert_eq!(resolve_canonical("英特尔", "zh"), Some("Intel Corporation".to_string()));
    }

    #[test]
    fn test_entity_map_contains_major_players() {
        // Key EMS companies must be in the map
        let keys = [
            "Jabil Inc.", "Flex Ltd.", "Celestica Inc.", "Pegatron Corporation",
            "Wistron Corporation", "Compal Electronics Inc.", "Quanta Computer Inc.",
            "Sanmina Corporation", "Plexus Corp.", "Benchmark Electronics Inc.",
        ];
        for key in &keys {
            let normalized = normalizer::normalize_entity_name(key);
            assert!(
                ENTITY_CANONICAL_MAP.contains_key(&normalized),
                "Missing canonical entry for: {key}"
            );
        }
    }

    #[test]
    fn test_is_likely_canonical() {
        assert!(is_likely_canonical("Apple Inc."));
        assert!(is_likely_canonical("John Smith"));
        assert!(!is_likely_canonical("apple"));
        assert!(!is_likely_canonical(""));
        assert!(!is_likely_canonical("A"));
    }
}
