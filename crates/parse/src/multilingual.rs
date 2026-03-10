use whatlang::{detect, Lang};

use crate::normalizer;

/// Detect language and return ISO 639-1 code.
pub fn detect_language(text: &str) -> String {
    if text.trim().is_empty() {
        return "en".to_string();
    }
    detect(text)
        .map(|info| match info.lang() {
            Lang::Eng => "en",
            Lang::Fra => "fr",
            Lang::Ara => "ar",
            Lang::Deu => "de",
            Lang::Spa => "es",
            Lang::Ita => "it",
            Lang::Nld => "nl",
            Lang::Pol => "pl",
            Lang::Por => "pt",
            Lang::Tur => "tr",
            Lang::Cmn => "zh",
            Lang::Jpn => "ja",
            Lang::Kor => "ko",
            Lang::Heb => "he",
            _ => "en",
        })
        .unwrap_or("en")
        .to_string()
}

/// Region-aware procurement keyword sets.
pub fn procurement_keywords(lang: &str) -> Vec<&'static str> {
    match lang {
        "fr" => vec![
            "fournisseur",
            "achats",
            "approvisionnement",
            "portail fournisseur",
            "appel d'offres",
            "demande de devis",
            "qualité fournisseur",
            "PPAP",
            "IMDS",
        ],
        "de" => vec![
            "Lieferant",
            "Einkauf",
            "Beschaffung",
            "Lieferantenportal",
            "Ausschreibung",
            "Anfrage",
            "Lieferantenqualität",
        ],
        "ar" => vec![
            "مورد",
            "موردين",
            "مشتريات",
            "بوابة الموردين",
            "طلب عرض أسعار",
            "جودة الموردين",
            "عطاء",
            "مناقصة",
        ],
        "es" => vec![
            "proveedor",
            "compras",
            "abastecimiento",
            "portal de proveedores",
            "licitación",
            "solicitud de oferta",
        ],
        "it" => vec![
            "fornitore",
            "acquisti",
            "approvvigionamento",
            "portale fornitori",
            "gara",
            "richiesta di offerta",
        ],
        "nl" => vec![
            "leverancier",
            "inkoop",
            "leveranciersportaal",
            "aanbesteding",
            "offerteaanvraag",
        ],
        "zh" => vec![
            "供应商",
            "采购",
            "供应商门户",
            "招标",
            "询价",
            "供应商质量",
            "电子制造",
            "合同制造",
            "政府采购",
        ],
        "ja" => vec![
            "サプライヤー",
            "調達",
            "入札",
            "見積依頼",
            "品質管理",
            "電子製造",
            "受託製造",
        ],
        "ko" => vec![
            "공급업체",
            "조달",
            "입찰",
            "견적요청",
            "공급업체품질",
            "전자제조",
            "계약제조",
        ],
        _ => vec![
            "supplier",
            "procurement",
            "vendor registration",
            "rfq",
            "rfp",
            "supplier quality",
            "ppap",
            "sourcing",
        ],
    }
}

/// EMS/electronics manufacturing keywords.
pub fn ems_keywords(lang: &str) -> Vec<&'static str> {
    match lang {
        "fr" => vec![
            "sous-traitance électronique",
            "CMS",
            "assemblage",
            "circuits imprimés",
            "fabrication électronique",
            "câblage",
        ],
        "ar" => vec![
            "تصنيع إلكتروني",
            "تجميع",
            "لوحات الدوائر المطبوعة",
            "تعهيد التصنيع",
            "تركيب المكونات السطحية",
        ],
        "zh" => vec![
            "电子制造服务",
            "SMT贴片",
            "PCB组装",
            "合同制造",
            "电子组装",
            "代工生产",
        ],
        "ja" => vec!["電子製造サービス", "SMT実装", "基板組立", "受託製造"],
        "ko" => vec!["전자제조서비스", "SMT실장", "PCB조립", "위탁제조"],
        _ => vec![
            "EMS",
            "contract manufacturing",
            "SMT",
            "THT",
            "PCB assembly",
            "box build",
            "cable harness",
            "AOI",
            "ICT",
            "BGA",
            "conformal coating",
        ],
    }
}

/// Certification/quality keywords, localised per language.
pub fn certification_keywords(lang: &str) -> Vec<&'static str> {
    match lang {
        "fr" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "marquage CE",
            "RoHS",
            "REACH",
            "minéraux de conflit",
            "certification qualité",
            "système de management qualité",
            "accréditation",
            "COFRAC",
        ],
        "ar" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "علامة CE",
            "RoHS",
            "REACH",
            "المعادن المتنازع عليها",
            "شهادة الجودة",
            "نظام إدارة الجودة",
            "الاعتماد",
            "هيئة الاعتماد",
        ],
        "zh" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "CE认证",
            "RoHS",
            "REACH",
            "冲突矿物",
            "质量认证",
            "质量管理体系",
            "认可",
            "国家认证",
        ],
        "ja" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "CEマーキング",
            "RoHS",
            "REACH",
            "紛争鉱物",
            "品質認証",
            "品質管理システム",
            "認定",
            "JIS規格",
        ],
        "ko" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "CE인증",
            "RoHS",
            "REACH",
            "분쟁광물",
            "품질인증",
            "품질경영시스템",
            "인정",
            "KS인증",
        ],
        "de" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "CE-Kennzeichnung",
            "RoHS",
            "REACH",
            "Konfliktmineralien",
            "Qualitätszertifizierung",
            "Qualitätsmanagementsystem",
            "Akkreditierung",
            "DAkkS",
        ],
        "es" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "marcado CE",
            "RoHS",
            "REACH",
            "minerales de conflicto",
            "certificación de calidad",
            "sistema de gestión",
            "acreditación",
        ],
        "it" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "marcatura CE",
            "RoHS",
            "REACH",
            "minerali di conflitto",
            "certificazione qualità",
            "sistema qualità",
            "accreditamento",
        ],
        "pt" => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "NADCAP",
            "marcação CE",
            "RoHS",
            "REACH",
            "minerais de conflito",
            "certificação de qualidade",
            "sistema de gestão",
        ],
        _ => vec![
            "ISO 9001",
            "IATF 16949",
            "AS9100",
            "ISO 13485",
            "ISO 14001",
            "IPC-A-610",
            "J-STD-001",
            "NADCAP",
            "UL",
            "CE marking",
            "RoHS",
            "REACH",
            "conflict minerals",
        ],
    }
}

/// Check if text contains any of the given keywords (case-insensitive).
/// Strips HTML tags and diacritics before matching (B107, B108).
pub fn contains_keywords(text: &str, keywords: &[&str]) -> Vec<String> {
    let cleaned = normalizer::strip_html_tags(text);
    let stripped = normalizer::strip_diacritics(&cleaned);
    let lower = normalizer::normalize_whitespace(&stripped).to_lowercase();
    keywords
        .iter()
        .filter(|kw| {
            let kw_lower = normalizer::strip_diacritics(&kw.to_lowercase());
            lower.contains(&kw_lower)
        })
        .map(|kw| kw.to_string())
        .collect()
}

/// Score text relevance based on keyword density.
pub fn keyword_relevance_score(text: &str, keywords: &[&str]) -> f64 {
    if keywords.is_empty() || text.is_empty() {
        return 0.0;
    }
    let matches = contains_keywords(text, keywords);
    matches.len() as f64 / keywords.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_english() {
        let text =
            "The company specializes in electronic manufacturing services for automotive industry.";
        assert_eq!(detect_language(text), "en");
    }

    #[test]
    fn test_detect_french() {
        let text = "L'entreprise est spécialisée dans la fabrication électronique pour l'industrie automobile.";
        assert_eq!(detect_language(text), "fr");
    }

    #[test]
    fn test_detect_arabic() {
        let text = "تتخصص الشركة في خدمات التصنيع الإلكتروني لصناعة السيارات";
        assert_eq!(detect_language(text), "ar");
    }

    #[test]
    fn test_detect_chinese() {
        let text = "该公司专注于汽车行业的电子制造服务";
        assert_eq!(detect_language(text), "zh");
    }

    #[test]
    fn test_detect_empty() {
        assert_eq!(detect_language(""), "en");
    }

    #[test]
    fn test_detect_gibberish_defaults() {
        assert_eq!(detect_language("@@@###$$$"), "en");
    }

    #[test]
    fn test_procurement_keywords_en() {
        let kws = procurement_keywords("en");
        assert!(kws.contains(&"procurement"));
        assert!(kws.contains(&"rfq"));
    }

    #[test]
    fn test_procurement_keywords_fr() {
        let kws = procurement_keywords("fr");
        assert!(kws.contains(&"fournisseur"));
        assert!(kws.contains(&"achats"));
    }

    #[test]
    fn test_procurement_keywords_zh() {
        let kws = procurement_keywords("zh");
        assert!(kws.contains(&"供应商"));
        assert!(kws.contains(&"采购"));
    }

    #[test]
    fn test_ems_keywords_en() {
        let kws = ems_keywords("en");
        assert!(kws.contains(&"EMS"));
        assert!(kws.contains(&"SMT"));
    }

    #[test]
    fn test_contains_keywords() {
        let text = "We provide SMT assembly and PCB manufacturing services.";
        let kws = vec!["SMT", "PCB", "BGA", "AOI"];
        let matches = contains_keywords(text, &kws);
        assert_eq!(matches.len(), 2);
        assert!(matches.contains(&"SMT".to_string()));
        assert!(matches.contains(&"PCB".to_string()));
    }

    #[test]
    fn test_keyword_relevance_score() {
        let text = "We provide SMT assembly and PCB manufacturing services.";
        let kws = vec!["SMT", "PCB", "BGA", "AOI"];
        let score = keyword_relevance_score(text, &kws);
        assert!((score - 0.5).abs() < 0.001); // 2 out of 4
    }

    #[test]
    fn test_keyword_relevance_empty() {
        assert_eq!(keyword_relevance_score("", &["test"]), 0.0);
        assert_eq!(keyword_relevance_score("text", &[]), 0.0);
    }

    // B104: Mixed language content edge case
    #[test]
    fn test_contains_keywords_mixed_language() {
        let text = "The supplier provides fournisseur services and 供应商 solutions.";
        let kws = vec!["supplier", "fournisseur", "供应商", "missing"];
        let found = contains_keywords(text, &kws);
        assert_eq!(found.len(), 3);
        assert!(found.contains(&"supplier".to_string()));
        assert!(found.contains(&"fournisseur".to_string()));
        assert!(found.contains(&"供应商".to_string()));
    }

    // B107: Keyword scoring ignores markup remnants
    #[test]
    fn test_keyword_score_strips_markup() {
        let text = "<div class='supplier'>We are an EMS <b>supplier</b></div>";
        let kws = vec!["supplier", "EMS"];
        let score = keyword_relevance_score(text, &kws);
        assert!((score - 1.0).abs() < 0.001); // both found
    }

    // B108: Diacritics normalization in matching
    #[test]
    fn test_contains_keywords_diacritics() {
        let text = "Le fournisseur offre des résistances électroniques.";
        let kws = vec!["resistances", "electroniques"];
        let found = contains_keywords(text, &kws);
        assert_eq!(found.len(), 2);
    }
}
