//! Conversion of raw crawl artifacts into typed POI artifacts, plus
//! lookalike-domain generation used by threat-intel jobs.

use apex_core::entities::{ArtifactType, PoiArtifact};
use apex_crawl::person_scraper::RawPersonArtifact;
use apex_crawl::tor_client::DarkWebPersonIntel;
use chrono::Utc;
use uuid::Uuid;
#[cfg(feature = "llm")]
pub(crate) fn raw_to_poi_artifact(
    person_id: Uuid,
    raw: RawPersonArtifact,
    fallback_url: &str,
    now: chrono::DateTime<Utc>,
) -> Option<PoiArtifact> {
    let url = raw.url.clone().unwrap_or_else(|| fallback_url.to_string());
    if url.trim().is_empty() {
        return None;
    }

    let ts_utc = chrono::DateTime::from_timestamp(raw.ts_utc, 0).unwrap_or(now);
    let mut artifact = PoiArtifact::new(
        person_id,
        map_raw_artifact_type(&raw.artifact_type),
        url.clone(),
        ts_utc,
    );
    artifact.title = Some(raw.title);
    artifact.content_summary = Some(raw.content);
    artifact.source_domain = extract_domain(&url);
    artifact.language = raw.language;
    artifact.topics = raw
        .meta
        .get("topic")
        .map(|t| vec![t.clone()])
        .unwrap_or_default();
    artifact.sentiment_score = Some(raw.confidence as f64);
    artifact.key_phrases = raw.meta.keys().cloned().collect();
    artifact.provenance = serde_json::json!({
        "source": raw.source,
        "artifact_type": raw.artifact_type,
    });
    artifact.metadata = serde_json::json!(raw.meta);
    Some(artifact)
}

#[cfg(feature = "llm")]
pub(crate) fn is_high_quality_raw_artifact(raw: &RawPersonArtifact) -> bool {
    if !raw.confidence.is_finite() || raw.confidence < 0.55 {
        return false;
    }

    let title_ok = raw.title.trim().len() >= 8;
    let content_len = raw.content.trim().len();
    if !title_ok && content_len < 40 {
        return false;
    }

    if raw.source.starts_with("social_") {
        let credibility = raw.confidence >= 0.65;
        let has_engagement = raw
            .meta
            .get("engagement")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v >= 20.0)
            .unwrap_or(false);
        let has_url = raw
            .url
            .as_deref()
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);
        return (credibility || has_engagement) && has_url && content_len >= 60;
    }

    if raw.source.starts_with("darkweb_") {
        let has_evidence = raw.meta.contains_key("domain") || raw.meta.contains_key("source");
        let has_url = raw
            .url
            .as_deref()
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);
        return raw.confidence >= 0.70 && has_evidence && has_url;
    }

    true
}

#[cfg(feature = "llm")]
pub(crate) fn dark_web_to_raw_artifacts(
    intel: &DarkWebPersonIntel,
    org_domain: Option<&str>,
) -> Vec<RawPersonArtifact> {
    let mut out = Vec::new();
    let domain_lc = org_domain.map(|d| d.to_ascii_lowercase());

    for rec in intel.contact_records.iter().take(20) {
        let email_match = rec
            .email
            .as_deref()
            .map(|e| {
                domain_lc
                    .as_deref()
                    .map(|d| e.to_ascii_lowercase().ends_with(&format!("@{d}")))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        if rec.confidence < 0.35 && !email_match {
            continue;
        }

        let mut meta = std::collections::HashMap::new();
        meta.insert("source".to_string(), "onion_contact".to_string());
        meta.insert("confidence".to_string(), format!("{:.2}", rec.confidence));
        if let Some(org) = &rec.org {
            meta.insert("org".to_string(), org.clone());
        }
        if let Some(email) = &rec.email {
            meta.insert("email".to_string(), email.clone());
        }
        if let Some(phone) = &rec.phone {
            meta.insert("phone".to_string(), phone.clone());
        }

        let mut artifact = RawPersonArtifact {
            source: "darkweb_contact".to_string(),
            artifact_type: "contact_leak".to_string(),
            title: format!("Onion contact signal for {}", intel.full_name),
            content: format!(
                "Name: {} | Email: {} | Phone: {} | Org: {}",
                rec.name.clone().unwrap_or_default(),
                rec.email.clone().unwrap_or_default(),
                rec.phone.clone().unwrap_or_default(),
                rec.org.clone().unwrap_or_default()
            ),
            url: Some(rec.source_url.clone()),
            ts_utc: rec.ts_scraped,
            language: None,
            confidence: if email_match {
                0.80
            } else {
                rec.confidence.max(0.70)
            },
            meta,
        };

        if email_match {
            artifact
                .meta
                .insert("domain_match".to_string(), "true".to_string());
        }
        out.push(artifact);
    }

    for rec in intel.breach_records.iter().take(20) {
        let email_lc = rec.email.to_ascii_lowercase();
        let domain_match = domain_lc
            .as_deref()
            .map(|d| email_lc.ends_with(&format!("@{d}")))
            .unwrap_or(false);
        if !domain_match {
            continue;
        }

        let mut meta = std::collections::HashMap::new();
        meta.insert("source".to_string(), rec.source.clone());
        meta.insert("domain".to_string(), rec.domain.clone());
        if let Some(date_posted) = &rec.date_posted {
            meta.insert("date_posted".to_string(), date_posted.clone());
        }

        out.push(RawPersonArtifact {
            source: "darkweb_breach".to_string(),
            artifact_type: "breach_credential".to_string(),
            title: format!("Onion breach match for {}", intel.full_name),
            content: format!("Breach email match for monitored domain: {}", rec.email),
            url: Some("https://onion.local/breach-match".to_string()),
            ts_utc: intel.ts_scraped,
            language: None,
            confidence: 0.82,
            meta,
        });
    }

    out
}

#[cfg(feature = "llm")]
pub(crate) fn map_raw_artifact_type(kind: &str) -> ArtifactType {
    match kind {
        "quote" => ArtifactType::PressQuote,
        "talk" => ArtifactType::SpeakerBio,
        "publication" => ArtifactType::Article,
        "social_mention" => ArtifactType::Article,
        "contact_leak" | "breach_credential" => ArtifactType::Other("security_intel".to_string()),
        "role" | "board_seat" => ArtifactType::RoleChange,
        "bio" => ArtifactType::SpeakerBio,
        "event_mention" => ArtifactType::Article,
        "award" => ArtifactType::Article,
        other => ArtifactType::Other(other.to_string()),
    }
}

#[cfg(feature = "llm")]
pub(crate) fn extract_domain(url: &str) -> Option<String> {
    let no_scheme = url.split("//").nth(1).unwrap_or(url);
    let host = no_scheme.split('/').next().unwrap_or("").trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Homoglyph mappings for lookalike generation.
/// Maps ASCII chars to visually similar Unicode chars.
pub(crate) const HOMOGLYPHS: &[(&str, &[&str])] = &[
    ("a", &["à", "á", "â", "ã", "ä", "å", "ɑ", "а"]),
    ("c", &["ç", "ć", "č", "с"]),
    ("d", &["đ", "ð"]),
    ("e", &["è", "é", "ê", "ë", "ε", "е"]),
    ("g", &["ğ", "ɡ"]),
    ("h", &["һ"]),
    ("i", &["ì", "í", "î", "ï", "ı", "і"]),
    ("l", &["ł", "ɫ", "1"]),
    ("n", &["ñ", "ŋ"]),
    ("o", &["ò", "ó", "ô", "õ", "ö", "ø", "0", "о"]),
    ("r", &["ŗ", "г"]),
    ("s", &["ş", "š", "ś", "ѕ"]),
    ("t", &["ţ", "ŧ"]),
    ("u", &["ù", "ú", "û", "ü", "µ"]),
    ("w", &["ŵ", "ω"]),
    ("y", &["ý", "ÿ", "ŷ", "у"]),
    ("z", &["ž", "ż", "ź"]),
];

/// TLD swap mappings — common typosquat TLD alternatives.
pub(crate) const TLD_SWAPS: &[(&str, &[&str])] = &[
    (
        ".com",
        &[".co", ".cm", ".corn", ".om", ".com.co", ".net", ".org"],
    ),
    (".net", &[".ner", ".met", ".org"]),
    (".org", &[".orq", ".og", ".net"]),
    (".co.uk", &[".co.ck", ".co.uk.com"]),
    (".de", &[".d3", ".de.com"]),
    (".fr", &[".f", ".fr.com"]),
    (".tn", &[".tn.com", ".rn"]),
];

/// Generate common typosquat/lookalike variants for a domain name.
/// Covers: transposition, character omission, character doubling,
/// homoglyph substitution, hyphen insertion, and TLD swaps.
pub(crate) fn generate_typosquat_variants(domain: &str) -> Vec<String> {
    // Handle multi-part TLDs (co.uk, com.au, co.jp, etc.)
    let multi_tlds = &[
        ".co.uk", ".com.au", ".co.jp", ".co.nz", ".com.br", ".com.mx", ".co.za", ".com.ar",
        ".com.tn", ".net.au", ".org.uk", ".ac.uk", ".gov.uk",
    ];
    let (sld_str, tld_str) = {
        let lower = domain.to_ascii_lowercase();
        let mut found = None;
        for mtld in multi_tlds {
            if lower.ends_with(mtld) {
                let base = &lower[..lower.len() - mtld.len()];
                found = Some((base.to_string(), mtld.to_string()));
                break;
            }
        }
        match found {
            Some((b, t)) => (b, t),
            None => {
                // Single TLD — split at last dot
                let parts: Vec<&str> = domain.rsplitn(2, '.').collect();
                if parts.len() < 2 {
                    return vec![];
                }
                (parts[1].to_string(), format!(".{}", parts[0]))
            }
        }
    };

    let sld: Vec<char> = sld_str.chars().collect();
    let n = sld.len();
    if n == 0 {
        return vec![];
    }
    let mut variants = std::collections::HashSet::new();

    // 1. Transposition: swap adjacent chars
    for i in 0..n.saturating_sub(1) {
        let mut v = sld.clone();
        v.swap(i, i + 1);
        let s: String = v.iter().collect();
        variants.insert(format!("{}{}", s, tld_str));
    }

    // 2. Omission: drop each char
    for i in 0..n {
        let s: String = sld
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, c)| c)
            .collect();
        if !s.is_empty() {
            variants.insert(format!("{}{}", s, tld_str));
        }
    }

    // 3. Character doubling (insert adjacent duplicate)
    for i in 0..n {
        let mut v: Vec<char> = sld.clone();
        v.insert(i, sld[i]);
        let s: String = v.iter().collect();
        variants.insert(format!("{}{}", s, tld_str));
    }

    // 4. Homoglyph substitution
    for (i, ch) in sld.iter().enumerate() {
        let lower_ch = ch.to_ascii_lowercase();
        for &(ascii, glyphs) in HOMOGLYPHS {
            if ascii.as_bytes() == [lower_ch as u8] {
                for &glyph in glyphs {
                    let mut v: Vec<char> = sld.clone();
                    v[i] = glyph.chars().next().unwrap_or(*ch);
                    let s: String = v.iter().collect();
                    variants.insert(format!("{}{}", s, tld_str));
                }
            }
        }
    }

    // 5. Hyphen insertion
    for i in 1..n {
        let (a, b): (String, String) = (sld[..i].iter().collect(), sld[i..].iter().collect());
        variants.insert(format!("{}-{}{}", a, b, tld_str));
    }

    // 6. TLD swap
    for &(tld_pattern, alts) in TLD_SWAPS {
        if tld_str == tld_pattern {
            for alt in alts {
                variants.insert(format!("{}{}", sld_str, alt));
            }
        }
    }

    // Cap at a generous limit — most domains will generate 30-80 variants
    variants.into_iter().take(100).collect()
}
