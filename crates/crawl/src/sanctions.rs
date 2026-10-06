//! Sanctions and export-control active screening module.
//!
//! Loads OFAC SDN, EU consolidated, and UN sanctions lists and performs
//! real-time fuzzy name matching against tracked entities.
//!
//! # Sources
//! - **OFAC SDN** — US Treasury Office of Foreign Assets Control Specially
//!   Designated Nationals and Blocked Persons List.
//!   Download: `https://www.treasury.gov/ofac/downloads/sdn.xml`
//! - **EU Consolidated** — European Union consolidated list of sanctions.
//!   Download: `https://webgate.ec.europa.eu/fsd/fsf/public/files/xmlFullSanctionsList_1_1/content`
//! - **BIS Entity List** — US Bureau of Industry and Security export denial list.
//!   CSV at `https://www.bis.doc.gov/index.php/documents/regulations-docs/2326-supplement-no-4-to-part-744/file`
//!
//! # Matching algorithm
//! Uses Jaro-Winkler similarity with a configurable threshold (default 0.92).
//! Also performs exact matching on identifier values (passport numbers, DUNS, etc.).
//!
//! # Usage
//! ```no_run
//! use apex_crawl::sanctions::SanctionsScreener;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let screener = SanctionsScreener::load_from_web().await?;
//! let hits = screener.screen_entity("Viktor Bout", &[]);
//! for h in &hits {
//!     println!("{} matched {} (score {:.2})", h.query_name, h.matched_name, h.similarity);
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// Source list for a match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SanctionsList {
    OfacSdn,
    OfacNs,
    EuConsolidated,
    UnSecurity,
    BisEntityList,
    BisDeniedPersons,
}

impl SanctionsList {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OfacSdn => "OFAC_SDN",
            Self::OfacNs => "OFAC_NS",
            Self::EuConsolidated => "EU_CONSOLIDATED",
            Self::UnSecurity => "UN_SECURITY_COUNCIL",
            Self::BisEntityList => "BIS_ENTITY_LIST",
            Self::BisDeniedPersons => "BIS_DENIED_PERSONS",
        }
    }
}

/// Type of sanctioned entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    Individual,
    Organization,
    Vessel,
    Aircraft,
    Unknown,
}

impl EntityType {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "individual" | "person" => Self::Individual,
            "organization" | "entity" | "legal entity" => Self::Organization,
            "vessel" | "ship" => Self::Vessel,
            "aircraft" | "plane" => Self::Aircraft,
            _ => Self::Unknown,
        }
    }
}

/// A match found during sanctions screening.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanctionsMatch {
    /// The query name that was screened.
    pub query_name: String,
    /// The matched name from the sanctions list.
    pub matched_name: String,
    /// All known aliases for this entry.
    pub aliases: Vec<String>,
    /// Similarity score [0.0, 1.0].
    pub similarity: f64,
    /// Whether this is an exact match (score == 1.0).
    pub is_exact: bool,
    /// Source list where the match was found.
    pub list: SanctionsList,
    /// Unique identifier in the sanctions list.
    pub entry_id: String,
    /// Sanction programs (e.g. "SDGT", "IRAN", "RUSSIA").
    pub programs: Vec<String>,
    /// Entity type.
    pub entity_type: EntityType,
    /// Nationalities / countries of concern.
    pub nationalities: Vec<String>,
    /// Identifier matches (passport, TIN, etc.).
    pub identifier_matches: Vec<IdentifierMatch>,
    /// Date this entry was added to the list.
    pub added_date: Option<DateTime<Utc>>,
}

/// An identifier-based match (passport number, company ID, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifierMatch {
    pub id_type: String,
    pub id_value: String,
    pub id_country: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal entry type
// ─────────────────────────────────────────────────────────────────────────────

/// A normalized sanctions list entry (source-agnostic).
#[derive(Debug, Clone)]
struct SanctionEntry {
    id: String,
    primary_name: String,
    aliases: Vec<String>,
    /// All names including aliases, normalised to lowercase for fast compare.
    searchable_names: Vec<String>,
    entity_type: EntityType,
    programs: Vec<String>,
    nationalities: Vec<String>,
    identifiers: Vec<(String, String, Option<String>)>, // (type, value, country)
    list: SanctionsList,
    added_date: Option<DateTime<Utc>>,
}

impl SanctionEntry {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: impl Into<String>,
        primary_name: impl Into<String>,
        aliases: Vec<String>,
        entity_type: EntityType,
        programs: Vec<String>,
        nationalities: Vec<String>,
        identifiers: Vec<(String, String, Option<String>)>,
        list: SanctionsList,
        added_date: Option<DateTime<Utc>>,
    ) -> Self {
        let primary_name = primary_name.into();
        let mut searchable_names: Vec<String> = aliases.iter().map(|a| a.to_lowercase()).collect();
        searchable_names.push(primary_name.to_lowercase());
        Self {
            id: id.into(),
            primary_name,
            aliases,
            searchable_names,
            entity_type,
            programs,
            nationalities,
            identifiers,
            list,
            added_date,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Screener
// ─────────────────────────────────────────────────────────────────────────────

/// Sanctions screener loaded with one or more list sources.
#[derive(Debug)]
pub struct SanctionsScreener {
    entries: Vec<SanctionEntry>,
    /// Minimum Jaro-Winkler similarity to report as a match (default 0.92).
    threshold: f64,
    /// When this screener was last refreshed.
    pub last_refreshed: DateTime<Utc>,
    /// Lazy two-character token buckets: entry indices grouped by the
    /// 2-char prefixes of their name tokens. `screen_entity` only examines
    /// entries sharing a bucket with the query, which turns the former
    /// O(entities × list) full scans (30-minute timeout) into a small
    /// candidate set while preserving the token-gated matcher semantics
    /// (Jaro-Winkler >= 0.90 requires a shared 2-char prefix in practice;
    /// 1-char tokens bucket by their full token).
    token_buckets: std::sync::OnceLock<std::collections::HashMap<String, Vec<usize>>>,
    /// Lazy identifier index: uppercase identifier value -> entry indices.
    /// Identifier matches are name-independent, so they need their own
    /// candidate path through the token-bucket pruning.
    identifier_buckets: std::sync::OnceLock<std::collections::HashMap<String, Vec<usize>>>,
}

impl SanctionsScreener {
    /// Create an empty screener (no entries loaded).
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            threshold: 0.92,
            last_refreshed: Utc::now(),
            token_buckets: std::sync::OnceLock::new(),
            identifier_buckets: std::sync::OnceLock::new(),
        }
    }

    /// Set matching similarity threshold (0.0 to 1.0, default 0.92).
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold.clamp(0.5, 1.0);
        self
    }

    /// Load all known public sanctions lists from remote URLs.
    ///
    /// Downloads OFAC SDN XML + EU consolidated XML.  Network errors from
    /// individual sources are logged and skipped so partial loading succeeds.
    pub async fn load_from_web() -> Result<Self> {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(60),
            user_agent: Some(
                "ApexIntel/1.0 compliance-screening (+https://apexintel.io)".to_string(),
            ),
            ..crate::http::ExternalClientOptions::default()
        })
        .context("Build HTTP client for sanctions loading")?;

        let mut screener = Self::empty();

        // Load OFAC SDN XML
        match screener
            .load_ofac_sdn_xml(&client, "https://www.treasury.gov/ofac/downloads/sdn.xml")
            .await
        {
            Ok(count) => info!(count, "OFAC SDN entries loaded"),
            Err(e) => warn!(error = %e, "Failed to load OFAC SDN — using partial list"),
        }

        // Load EU consolidated XML (BulkDownload format)
        match screener.load_eu_consolidated_xml(
            &client,
            "https://webgate.ec.europa.eu/fsd/fsf/public/files/xmlFullSanctionsList_1_1/content",
        ).await {
            Ok(count) => info!(count, "EU consolidated sanctions entries loaded"),
            Err(e) => warn!(error = %e, "Failed to load EU consolidated list — using partial list"),
        }

        screener.last_refreshed = Utc::now();
        info!(total = screener.entries.len(), "Sanctions screener ready");
        Ok(screener)
    }

    /// Load OFAC SDN from a local XML file path.
    pub async fn load_ofac_sdn_file(&mut self, path: &str) -> Result<usize> {
        let content = tokio::fs::read(path)
            .await
            .with_context(|| format!("Read OFAC SDN file: {}", path))?;
        self.parse_ofac_sdn_xml(&content)
    }

    /// Load OFAC SDN from a remote URL.
    async fn load_ofac_sdn_xml(&mut self, client: &Client, url: &str) -> Result<usize> {
        debug!(url = %url, "Downloading OFAC SDN XML");
        let resp = client.get(url).send().await.context("Fetch OFAC SDN")?;
        if !resp.status().is_success() {
            anyhow::bail!("OFAC SDN download returned {}", resp.status());
        }
        let body = crate::http::read_capped_bytes(resp, crate::http::MAX_BULK_BODY_BYTES)
            .await
            .context("Read OFAC SDN body")?;
        self.parse_ofac_sdn_xml(&body)
    }

    /// Parse OFAC SDN XML content.
    fn parse_ofac_sdn_xml(&mut self, content: &[u8]) -> Result<usize> {
        use quick_xml::events::Event;
        use quick_xml::Reader;
        use std::str;

        let mut reader = Reader::from_reader(content);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::with_capacity(4096);
        let mut count = 0;

        // State machine for SDN XML traversal
        let mut in_entry = false;
        let mut in_aka = false;
        let mut in_id = false;
        let mut current_field = String::new();

        let mut uid = String::new();
        let mut first_name = String::new();
        let mut last_name = String::new();
        let mut sdn_type = String::new();
        let mut programs: Vec<String> = Vec::new();
        let mut aliases: Vec<String> = Vec::new();
        let mut current_aka_last = String::new();
        let mut current_aka_first = String::new();
        let mut identifiers: Vec<(String, String, Option<String>)> = Vec::new();
        let mut current_id_type = String::new();
        let mut current_id_number = String::new();
        let mut current_id_country: Option<String> = None;
        let mut nationalities: Vec<String> = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = str::from_utf8(e.name().as_ref()).unwrap_or("").to_string();
                    current_field = name.clone();
                    match name.as_str() {
                        "sdnEntry" => {
                            in_entry = true;
                            uid.clear();
                            first_name.clear();
                            last_name.clear();
                            sdn_type.clear();
                            programs.clear();
                            aliases.clear();
                            identifiers.clear();
                            nationalities.clear();
                        }
                        "aka" => {
                            in_aka = true;
                            current_aka_last.clear();
                            current_aka_first.clear();
                        }
                        "id" if in_entry => {
                            in_id = true;
                            current_id_type.clear();
                            current_id_number.clear();
                            current_id_country = None;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) => {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        buf.clear();
                        continue;
                    }
                    if in_entry {
                        match current_field.as_str() {
                            "uid" => uid = text,
                            "lastName" if !in_aka && !in_id => last_name = text,
                            "firstName" if !in_aka && !in_id => first_name = text,
                            "sdnType" => sdn_type = text,
                            "program" => programs.push(text),
                            "lastName" if in_aka => current_aka_last = text,
                            "firstName" if in_aka => current_aka_first = text,
                            "nationalityCountry" => nationalities.push(text),
                            "idType" if in_id => current_id_type = text,
                            "idNumber" if in_id => current_id_number = text,
                            "idCountry" if in_id => current_id_country = Some(text),
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = str::from_utf8(e.name().as_ref()).unwrap_or("").to_string();
                    match name.as_str() {
                        "aka" if in_entry => {
                            in_aka = false;
                            let alias = format_name(&current_aka_first, &current_aka_last);
                            if !alias.is_empty() {
                                aliases.push(alias);
                            }
                        }
                        "id" if in_entry => {
                            in_id = false;
                            if !current_id_number.is_empty() {
                                identifiers.push((
                                    current_id_type.clone(),
                                    current_id_number.clone(),
                                    current_id_country.clone(),
                                ));
                            }
                        }
                        "sdnEntry" if in_entry => {
                            in_entry = false;
                            let primary = format_name(&first_name, &last_name);
                            if !primary.is_empty() {
                                let entry = SanctionEntry::new(
                                    uid.clone(),
                                    primary,
                                    aliases.clone(),
                                    EntityType::from_str(&sdn_type),
                                    programs.clone(),
                                    nationalities.clone(),
                                    identifiers.clone(),
                                    SanctionsList::OfacSdn,
                                    None,
                                );
                                self.entries.push(entry);
                                count += 1;
                            }
                        }
                        _ => {}
                    }
                    current_field.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    warn!(error = %e, "OFAC SDN XML parse warning (continuing)");
                    break;
                }
                _ => {}
            }
            buf.clear();
        }

        Ok(count)
    }

    /// Load EU consolidated XML from a remote URL.
    async fn load_eu_consolidated_xml(&mut self, client: &Client, url: &str) -> Result<usize> {
        debug!(url = %url, "Downloading EU consolidated sanctions XML");
        let resp = client
            .get(url)
            .send()
            .await
            .context("Fetch EU consolidated")?;
        if !resp.status().is_success() {
            anyhow::bail!("EU consolidated download returned {}", resp.status());
        }
        let body = crate::http::read_capped_bytes(resp, crate::http::MAX_BULK_BODY_BYTES)
            .await
            .context("Read EU consolidated body")?;
        self.parse_eu_consolidated_xml(&body)
    }

    /// Parse EU consolidated XML (BulkDownload v1.1 format).
    fn parse_eu_consolidated_xml(&mut self, content: &[u8]) -> Result<usize> {
        use quick_xml::events::Event;
        use quick_xml::Reader;
        use std::str;

        let mut reader = Reader::from_reader(content);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::with_capacity(4096);
        let mut count = 0;
        let mut in_sanctioned = false;
        let mut current_name = String::new();
        let mut aliases: Vec<String> = Vec::new();
        let mut subject_type = String::new();
        let mut current_field = String::new();
        let mut entry_id = String::new();
        let mut in_name = false;
        let mut in_alias_name = false;
        let mut name_parts: Vec<String> = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let name = str::from_utf8(e.name().as_ref()).unwrap_or("").to_string();
                    current_field = name.clone();
                    match name.as_str() {
                        "sanctionEntity" | "entity" => {
                            in_sanctioned = true;
                            current_name.clear();
                            aliases.clear();
                            subject_type.clear();
                            name_parts.clear();
                            // Try to get logicalId attribute
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"logicalId"
                                    || attr.key.as_ref() == b"euReferenceNumber"
                                {
                                    entry_id = str::from_utf8(attr.value.as_ref())
                                        .unwrap_or("")
                                        .to_string();
                                }
                            }
                        }
                        "namePart" if in_sanctioned => {
                            in_name = true;
                        }
                        "aliasStrong" | "aliasWeak" if in_sanctioned => {
                            in_alias_name = true;
                            name_parts.clear();
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) => {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        buf.clear();
                        continue;
                    }
                    if in_sanctioned {
                        match current_field.as_str() {
                            "namePart" | "wholeName" | "firstName" | "lastName" | "middleName" => {
                                name_parts.push(text.clone());
                                if current_name.is_empty() {
                                    current_name = text;
                                }
                            }
                            "subjectType" | "entity_type" => subject_type = text,
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = str::from_utf8(e.name().as_ref()).unwrap_or("").to_string();
                    match name.as_str() {
                        "namePart" if in_name => {
                            in_name = false;
                            if !name_parts.is_empty() {
                                current_name = name_parts.join(" ");
                                name_parts.clear();
                            }
                        }
                        "aliasStrong" | "aliasWeak" if in_alias_name => {
                            in_alias_name = false;
                            if !name_parts.is_empty() {
                                aliases.push(name_parts.join(" "));
                                name_parts.clear();
                            }
                        }
                        "sanctionEntity" | "entity" if in_sanctioned => {
                            in_sanctioned = false;
                            if !current_name.is_empty() {
                                let entry = SanctionEntry::new(
                                    if entry_id.is_empty() {
                                        count.to_string()
                                    } else {
                                        entry_id.clone()
                                    },
                                    current_name.clone(),
                                    aliases.clone(),
                                    EntityType::from_str(&subject_type),
                                    vec![], // regulation_title not parsed from this XML format
                                    vec![],
                                    vec![],
                                    SanctionsList::EuConsolidated,
                                    None,
                                );
                                self.entries.push(entry);
                                count += 1;
                            }
                        }
                        _ => {}
                    }
                    current_field.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    warn!(error = %e, "EU consolidated XML parse warning (continuing)");
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        Ok(count)
    }

    // ── Screening ─────────────────────────────────────────────────────────────

    /// Entry indices whose names share a 2-char token bucket with any query
    /// token. Recall-safe for the token-gated matcher: a Jaro-Winkler token
    /// match >= 0.90 shares the first two characters except for very short
    /// tokens, which bucket by their full text.
    fn candidate_entry_indices(&self, query_tokens: &[String]) -> Vec<usize> {
        let buckets = self.token_buckets.get_or_init(|| {
            let mut map: std::collections::HashMap<String, Vec<usize>> =
                std::collections::HashMap::new();
            for (index, entry) in self.entries.iter().enumerate() {
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for name in &entry.searchable_names {
                    for token in normalize_name_tokens(name) {
                        let key: String = if token.chars().count() <= 2 {
                            token
                        } else {
                            token.chars().take(2).collect()
                        };
                        if seen.insert(key.clone()) {
                            map.entry(key).or_default().push(index);
                        }
                    }
                }
            }
            map
        });

        let mut indices: Vec<usize> = Vec::new();
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for token in query_tokens {
            let key: String = if token.chars().count() <= 2 {
                token.clone()
            } else {
                token.chars().take(2).collect()
            };
            if let Some(entries) = buckets.get(&key) {
                for index in entries {
                    if seen.insert(*index) {
                        indices.push(*index);
                    }
                }
            }
        }
        indices
    }

    /// Entry indices matching any supplied identifier value (name-independent).
    fn identifier_entry_indices(&self, identifiers: &[(String, String)]) -> Vec<usize> {
        if identifiers.is_empty() {
            return Vec::new();
        }
        let buckets = self.identifier_buckets.get_or_init(|| {
            let mut map: std::collections::HashMap<String, Vec<usize>> =
                std::collections::HashMap::new();
            for (index, entry) in self.entries.iter().enumerate() {
                for (_id_type, value, _country) in &entry.identifiers {
                    map.entry(value.to_uppercase()).or_default().push(index);
                }
            }
            map
        });
        let mut indices: Vec<usize> = Vec::new();
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for (_query_type, value) in identifiers {
            if let Some(entries) = buckets.get(&value.to_uppercase()) {
                for index in entries {
                    if seen.insert(*index) {
                        indices.push(*index);
                    }
                }
            }
        }
        indices
    }

    /// Screen an entity name (and optional identifiers) against all loaded sanctions lists.
    ///
    /// `name` — the entity name to screen (person or organisation).
    /// `identifiers` — optional list of (type, value) pairs (e.g. passport number).
    ///
    /// Returns a deduplicated, similarity-sorted `Vec<SanctionsMatch>`.
    pub fn screen_entity(
        &self,
        name: &str,
        identifiers: &[(String, String)],
    ) -> Vec<SanctionsMatch> {
        let query_lower = name.to_lowercase();
        let query_tokens = normalize_name_tokens(name);
        let mut candidate_indices = self.candidate_entry_indices(&query_tokens);
        candidate_indices.extend(self.identifier_entry_indices(identifiers));
        candidate_indices.sort_unstable();
        candidate_indices.dedup();

        let mut matches: Vec<SanctionsMatch> = Vec::new();

        for &entry_index in &candidate_indices {
            let entry = &self.entries[entry_index];
            // Check identifier exact matches first (highest confidence)
            let mut id_matches: Vec<IdentifierMatch> = Vec::new();
            for (_q_type, q_value) in identifiers {
                for (e_type, e_value, e_country) in &entry.identifiers {
                    if e_value.to_uppercase() == q_value.to_uppercase() {
                        id_matches.push(IdentifierMatch {
                            id_type: e_type.clone(),
                            id_value: e_value.clone(),
                            id_country: e_country.clone(),
                        });
                    }
                }
            }

            let has_id_match = !id_matches.is_empty();

            // Compute best name similarity across all names for this entry
            // using the token-gated matcher (name_match_score). Full-string
            // Jaro-Winkler alone matched on prefix similarity and produced
            // pair-level false-positive floods (2026-10-06 incident).
            let mut best_score: f64 = 0.0;
            let mut best_name = entry.primary_name.clone();

            for candidate_lower in &entry.searchable_names {
                let Some(score) = name_match_score(&query_lower, candidate_lower) else {
                    continue;
                };
                if score > best_score {
                    best_score = score;
                    // Find original (non-lowercase) name
                    if candidate_lower == &entry.primary_name.to_lowercase() {
                        best_name = entry.primary_name.clone();
                    } else if let Some(alias) = entry
                        .aliases
                        .iter()
                        .find(|a| a.to_lowercase() == *candidate_lower)
                    {
                        best_name = alias.clone();
                    }
                }
            }

            // Report if above threshold or exact identifier match
            if best_score >= self.threshold || has_id_match {
                let effective_score = if has_id_match {
                    1.0_f64.max(best_score)
                } else {
                    best_score
                };
                matches.push(SanctionsMatch {
                    query_name: name.to_string(),
                    matched_name: best_name,
                    aliases: entry.aliases.clone(),
                    similarity: effective_score.min(1.0),
                    is_exact: effective_score >= 1.0 || has_id_match,
                    list: entry.list.clone(),
                    entry_id: entry.id.clone(),
                    programs: entry.programs.clone(),
                    entity_type: entry.entity_type.clone(),
                    nationalities: entry.nationalities.clone(),
                    identifier_matches: id_matches,
                    added_date: entry.added_date,
                });
            }
        }

        // Sort: exact matches first, then by similarity descending
        matches.sort_unstable_by(|a, b| {
            b.is_exact.cmp(&a.is_exact).then(
                b.similarity
                    .partial_cmp(&a.similarity)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });

        // Deduplicate exact duplicates from same list with same entry_id
        matches.dedup_by(|a, b| a.list == b.list && a.entry_id == b.entry_id);

        matches
    }

    /// Screen multiple entities in batch.
    pub fn screen_batch(
        &self,
        entities: &[(String, Vec<(String, String)>)],
    ) -> Vec<(String, Vec<SanctionsMatch>)> {
        entities
            .iter()
            .map(|(name, ids)| {
                let hits = self.screen_entity(name, ids);
                (name.clone(), hits)
            })
            .collect()
    }

    /// Return the total number of entries loaded across all lists.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Return counts per list source.
    pub fn list_counts(&self) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();
        for entry in &self.entries {
            *counts.entry(entry.list.as_str().to_string()).or_insert(0) += 1;
        }
        counts
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// String similarity algorithms
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the Jaro-Winkler similarity between two strings.
///
/// Returns a score in [0.0, 1.0] where 1.0 is an exact match.
/// Case-sensitive — normalize to lowercase before calling.
pub fn jaro_winkler(s1: &str, s2: &str) -> f64 {
    if s1 == s2 {
        return 1.0;
    }
    if s1.is_empty() || s2.is_empty() {
        return 0.0;
    }

    let jaro = jaro_similarity(s1, s2);

    // Common prefix length (up to 4 chars)
    let prefix_len = s1
        .chars()
        .zip(s2.chars())
        .take(4)
        .take_while(|(a, b)| a == b)
        .count();

    // Winkler boost (p = 0.1)
    let p = 0.1;
    jaro + (prefix_len as f64 * p * (1.0 - jaro))
}

fn jaro_similarity(s1: &str, s2: &str) -> f64 {
    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();
    let len1 = s1_chars.len();
    let len2 = s2_chars.len();

    let match_window = (len1.max(len2) / 2).saturating_sub(1);

    let mut s1_matches = vec![false; len1];
    let mut s2_matches = vec![false; len2];
    let mut matches = 0usize;
    let mut transpositions = 0usize;

    for i in 0..len1 {
        let start = i.saturating_sub(match_window);
        let end = (i + match_window + 1).min(len2);
        for j in start..end {
            if !s2_matches[j] && s1_chars[i] == s2_chars[j] {
                s1_matches[i] = true;
                s2_matches[j] = true;
                matches += 1;
                break;
            }
        }
    }

    if matches == 0 {
        return 0.0;
    }

    let mut k = 0;
    for i in 0..len1 {
        if s1_matches[i] {
            while !s2_matches[k] {
                k += 1;
            }
            if s1_chars[i] != s2_chars[k] {
                transpositions += 1;
            }
            k += 1;
        }
    }

    let m = matches as f64;
    let t = transpositions as f64 / 2.0;

    (m / len1 as f64 + m / len2 as f64 + (m - t) / m) / 3.0
}

/// Token-level matching score: what fraction of query tokens appear in the candidate.
/// Normalize a name for matching: lowercase, punctuation removed, collapse
/// whitespace. "MOHAMMED, Ali" -> "mohammed ali".
fn normalize_name_tokens(name: &str) -> Vec<String> {
    name.to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Token-gated name match score (incident 2026-10-06).
///
/// Full-string Jaro-Winkler matched "Mohammed Khalil" against "MOHAMMED, Ali"
/// on prefix similarity alone, flooding sanctions warnings with pair-level
/// false positives. The gate is now explicit:
///
/// * every query token must match some candidate token at >= 0.90,
/// * single-token queries only match exactly (a lone common given name is
///   never a sanctions hit),
/// * a candidate carrying extra tokens (middle names, patronymics) scores
///   0.85 + 0.15 * coverage, so "Mohammed Khalil" still matches "Mohammed
///   Khalil Zadeh" without matching "Mohammed Ali".
///
/// Returns `None` when the gate fails, else the score in `0..=1`.
pub fn name_match_score(query: &str, candidate: &str) -> Option<f64> {
    let query_tokens = normalize_name_tokens(query);
    let candidate_tokens = normalize_name_tokens(candidate);
    if query_tokens.is_empty() || candidate_tokens.is_empty() {
        return None;
    }
    if query_tokens == candidate_tokens {
        return Some(1.0);
    }
    if query_tokens.len() == 1 {
        // A single-token query only matches single-token candidates exactly
        // (already handled above); anything else needs corroborating
        // identifiers, which the caller checks separately.
        return None;
    }

    let mut matched_candidates: Vec<bool> = vec![false; candidate_tokens.len()];
    let mut matched_query = 0usize;
    for query_token in &query_tokens {
        let mut best_index: Option<usize> = None;
        let mut best_score = 0.0_f64;
        for (index, candidate_token) in candidate_tokens.iter().enumerate() {
            if matched_candidates[index] {
                continue;
            }
            let score = jaro_winkler(query_token, candidate_token);
            if score > best_score {
                best_score = score;
                best_index = Some(index);
            }
        }
        if best_score >= 0.90 {
            matched_query += 1;
            if let Some(index) = best_index {
                matched_candidates[index] = true;
            }
        }
    }
    if matched_query != query_tokens.len() {
        return None;
    }
    let matched_candidate_count = matched_candidates
        .iter()
        .filter(|matched| **matched)
        .count();
    let coverage = matched_candidate_count as f64 / candidate_tokens.len() as f64;
    // Equal-length token sets already returned 1.0; here the candidate carries
    // extra tokens, so score slightly below an exact match but above the
    // containment floor.
    let containment = 0.85 + 0.15 * coverage;
    // Reordered full names ("putin vladimir" vs "vladimir putin") deserve the
    // full score even though token order differs.
    if query_tokens.len() == candidate_tokens.len() {
        Some(1.0_f64.max(containment))
    } else {
        Some(containment)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn format_name(first: &str, last: &str) -> String {
    match (first.trim().is_empty(), last.trim().is_empty()) {
        (true, true) => String::new(),
        (true, false) => last.trim().to_string(),
        (false, true) => first.trim().to_string(),
        (false, false) => format!("{} {}", first.trim(), last.trim()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_gate_rejects_prefix_similarity_false_positives() {
        // Production false-positive pairs from the 2026-10-06 sanctions flood:
        // full-string Jaro-Winkler matched on the shared given name alone.
        assert_eq!(name_match_score("MOHAMMED Khalil", "MOHAMMED, Ali"), None);
        assert_eq!(name_match_score("Mohammed Khalil", "Mohammed Hassan"), None);
        assert_eq!(name_match_score("Ahmed Ben Salah", "Ahmed Ben Ali"), None);
    }

    #[test]
    fn token_gate_accepts_containment_and_reordering() {
        // Middle names / patronymics are containment, not rejection.
        let contained = name_match_score("Mohammed Khalil", "Mohammed Khalil Zadeh")
            .expect("containment must match");
        assert!(contained >= 0.92, "containment score {contained}");

        // Reordered full names match at full score.
        let reordered =
            name_match_score("putin vladimir", "Vladimir Putin").expect("reorder must match");
        assert!(reordered >= 0.99, "reorder score {reordered}");

        // Transliteration-grade token similarity still matches.
        assert!(name_match_score("Mohamed Khalil", "Mohammed Khalil").is_some());

        // Exact equality is the perfect score.
        assert_eq!(
            name_match_score("Mohammed Khalil", "MOHAMMED KHALIL"),
            Some(1.0)
        );
    }

    #[test]
    fn single_token_queries_never_match_on_a_common_given_name() {
        assert_eq!(name_match_score("Mohammed", "Mohammed Khalil Zadeh"), None);
        assert_eq!(name_match_score("Mohammed", "Mohammed"), Some(1.0));
    }

    fn jaro_winkler_exact() {
        assert!((jaro_winkler("john smith", "john smith") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jaro_winkler_empty() {
        assert_eq!(jaro_winkler("", "hello"), 0.0);
        assert_eq!(jaro_winkler("hello", ""), 0.0);
    }

    #[test]
    fn jaro_winkler_similar_names() {
        // Typical sanctions name variant
        let score = jaro_winkler("viktor bout", "viktor beout");
        // Should be high but not 1.0
        assert!(score > 0.8, "Expected high similarity, got {}", score);
    }

    #[test]
    fn jaro_winkler_transposition() {
        let score = jaro_winkler("martha", "marhta");
        assert!(
            score > 0.9,
            "Expected high similarity for transposition, got {}",
            score
        );
    }

    #[test]
    fn jaro_winkler_very_different() {
        let score = jaro_winkler("apple", "orange");
        assert!(score < 0.7, "Expected low similarity, got {}", score);
    }

    #[test]
    fn format_name_both() {
        assert_eq!(format_name("John", "Smith"), "John Smith");
    }

    #[test]
    fn format_name_last_only() {
        assert_eq!(format_name("", "Organization"), "Organization");
    }

    #[test]
    fn screener_empty_no_matches() {
        let screener = SanctionsScreener::empty();
        let hits = screener.screen_entity("Any Entity", &[]);
        assert!(hits.is_empty());
    }

    #[test]
    fn screener_exact_name_match() {
        let mut screener = SanctionsScreener::empty();
        screener.entries.push(SanctionEntry::new(
            "TEST-001",
            "Viktor Bout",
            vec!["Viktor Anatoliyevich Bout".to_string()],
            EntityType::Individual,
            vec!["SDGT".to_string()],
            vec!["Russia".to_string()],
            vec![],
            SanctionsList::OfacSdn,
            None,
        ));

        // Exact match
        let hits = screener.screen_entity("Viktor Bout", &[]);
        assert!(!hits.is_empty());
        assert!(hits[0].is_exact);
        assert!((hits[0].similarity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn screener_alias_match() {
        let mut screener = SanctionsScreener::empty();
        screener.entries.push(SanctionEntry::new(
            "TEST-002",
            "Viktor Bout",
            vec!["Viktor Anatoliyevich Bout".to_string()],
            EntityType::Individual,
            vec!["SDGT".to_string()],
            vec![],
            vec![],
            SanctionsList::OfacSdn,
            None,
        ));

        let hits = screener.screen_entity("Viktor Anatoliyevich Bout", &[]);
        assert!(!hits.is_empty());
    }

    #[test]
    fn screener_below_threshold_no_match() {
        let mut screener = SanctionsScreener::empty();
        screener.entries.push(SanctionEntry::new(
            "TEST-003",
            "Kim Jong Un",
            vec![],
            EntityType::Individual,
            vec!["DPRK".to_string()],
            vec![],
            vec![],
            SanctionsList::OfacSdn,
            None,
        ));

        // Very different name
        let hits = screener.screen_entity("John Smith", &[]);
        assert!(hits.is_empty(), "Should not match dissimilar names");
    }

    #[test]
    fn screener_identifier_match() {
        let mut screener = SanctionsScreener::empty();
        screener.entries.push(SanctionEntry::new(
            "TEST-004",
            "Some Entity",
            vec![],
            EntityType::Individual,
            vec!["SDGT".to_string()],
            vec![],
            vec![(
                "Passport".to_string(),
                "AB123456".to_string(),
                Some("Russia".to_string()),
            )],
            SanctionsList::OfacSdn,
            None,
        ));

        // Different name but matching passport
        let hits = screener.screen_entity(
            "Different Name",
            &[("Passport".to_string(), "AB123456".to_string())],
        );
        assert!(!hits.is_empty());
        assert!(hits[0].is_exact);
    }

    #[test]
    fn list_counts_empty() {
        let screener = SanctionsScreener::empty();
        let counts = screener.list_counts();
        assert!(counts.is_empty());
    }
}
