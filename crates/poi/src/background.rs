//! Deep background intelligence synthesis for Points of Interest.
//!
//! Aggregates education history, work history, public records, and known
//! associates from a `PoiProfile`'s accumulated artifacts, then uses the LLM
//! to synthesise a cohesive background narrative.
//!
//! # Data flow
//! 1. `BackgroundBuilder::from_profile` extracts structured fields from
//!    `PoiProfile.artifacts` (searching for type hints like `"education"`,
//!    `"work_history"`, `"public_record"`, `"associates"`).
//! 2. `BackgroundBuilder::with_llm` enriches the raw data with an LLM
//!    narrative and risk flags.
//! 3. `PoiBackground` is the output: portable, serializable, storable.
//!
//! # Usage
//! ```rust,ignore
//! use apex_poi::background::BackgroundBuilder;
//! let bg = BackgroundBuilder::from_profile(&poi_profile)
//!     .build()
//!     .await?;
//! println!("{}", bg.narrative.unwrap_or_default());
//! ```

use anyhow::{Context, Result};
use apex_llm::LlmClient;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock};
use tracing::warn;

use crate::model::{PoiProfile, RoleHistoryEntry};

// ─────────────────────────────────────────────────────────────────────────────
// Regex helpers (compiled once at startup)
// ─────────────────────────────────────────────────────────────────────────────

/// Captures academic degree abbreviations, optionally followed by "in <Field>".
static RE_DEGREE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(B\.?Sc?\.?|B\.?A\.?|M\.?Sc?\.?|M\.?B\.?A\.?|Ph\.?D\.?|M\.?Eng\.?|LLB|LLM|BEng|MEng|DBA)\.?
        (?:\s+(?:in|of|d[eu])\s+([A-Za-z][A-Za-z ]{2,40}))?",
    ).unwrap()
});

/// Captures a 4-digit graduation/class year (1960-2030).
static RE_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(19[6-9]\d|20[0-3]\d)\b").unwrap());

/// Flexible date: DD/MM/YYYY, MM-DD-YYYY, "Month DD, YYYY", "DD Month YYYY".
static RE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(\d{1,2}[/\-]\d{1,2}[/\-]\d{2,4}|(?:January|February|March|April|May|June|July|August|September|October|November|December|Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\.?\s+\d{1,2},?\s+\d{4}|\d{1,2}\s+(?:January|February|March|April|May|June|July|August|September|October|November|December)\.?\s+\d{4})\b",
    ).unwrap()
});

// ─────────────────────────────────────────────────────────────────────────────
// Output types
// ─────────────────────────────────────────────────────────────────────────────

/// A single education record extracted from artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EducationRecord {
    pub institution: String,
    pub degree: Option<String>,
    pub field: Option<String>,
    pub year_graduation: Option<u32>,
    pub source_url: Option<String>,
}

/// A public record link — legal, regulatory, or government filing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicRecord {
    pub record_type: String,
    pub description: String,
    pub source_url: Option<String>,
    pub date_str: Option<String>,
}

/// A known associate with a relationship description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownAssociate {
    pub name: String,
    pub relationship: String,
    pub org: Option<String>,
    pub confidence: f64,
}

/// Risk flag raised by LLM or heuristic analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundRiskFlag {
    pub flag_type: String,
    pub description: String,
    pub severity: String,
    pub confidence: f64,
}

/// Full deep background intelligence record for a POI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiBackground {
    pub person_id: String,
    pub person_name: String,
    pub education_history: Vec<EducationRecord>,
    /// Full career / work history (incorporates `PoiProfile.role_history`).
    pub work_history: Vec<RoleHistoryEntry>,
    pub public_records: Vec<PublicRecord>,
    pub known_associates: Vec<KnownAssociate>,
    /// LLM-generated synthesis paragraph.
    pub narrative: Option<String>,
    /// Risk flags from LLM or heuristics.
    pub risk_flags: Vec<BackgroundRiskFlag>,
    /// Completeness score [0, 1] for this background record.
    pub completeness: f64,
    pub created_at: DateTime<Utc>,
}

impl PoiBackground {
    /// Completeness as the fraction of non-empty sections.
    fn compute_completeness(&self) -> f64 {
        let sections = 5.0_f64;
        let mut filled = 0.0_f64;
        if !self.education_history.is_empty() {
            filled += 1.0;
        }
        if !self.work_history.is_empty() {
            filled += 1.0;
        }
        if !self.public_records.is_empty() {
            filled += 1.0;
        }
        if !self.known_associates.is_empty() {
            filled += 1.0;
        }
        if self.narrative.is_some() {
            filled += 1.0;
        }
        filled / sections
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder
// ─────────────────────────────────────────────────────────────────────────────

/// Builds a `PoiBackground` from a `PoiProfile` with optional LLM enrichment.
pub struct BackgroundBuilder<'a> {
    profile: &'a PoiProfile,
    llm: Option<Arc<dyn LlmClient>>,
}

impl<'a> BackgroundBuilder<'a> {
    pub fn from_profile(profile: &'a PoiProfile) -> Self {
        Self { profile, llm: None }
    }

    /// Attach an LLM for narrative synthesis and risk flagging.
    pub fn with_llm(mut self, llm: Arc<dyn LlmClient>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Build the background record.
    pub async fn build(self) -> Result<PoiBackground> {
        let education_history = self.extract_education();
        let public_records = self.extract_public_records();
        let known_associates = self.extract_associates();
        let work_history = self.profile.role_history.clone();

        // LLM enrichment
        let (narrative, risk_flags) = if let Some(ref llm) = self.llm {
            match self
                .generate_narrative_and_flags(
                    llm.as_ref(),
                    &education_history,
                    &public_records,
                    &known_associates,
                )
                .await
            {
                Ok(result) => result,
                Err(e) => {
                    warn!(person=%self.profile.name, error=%e, "Background LLM enrichment failed");
                    (None, vec![])
                }
            }
        } else {
            (None, vec![])
        };

        let mut bg = PoiBackground {
            person_id: self.profile.person_id.clone(),
            person_name: self.profile.name.clone(),
            education_history,
            work_history,
            public_records,
            known_associates,
            narrative,
            risk_flags,
            completeness: 0.0,
            created_at: Utc::now(),
        };
        bg.completeness = bg.compute_completeness();
        Ok(bg)
    }

    // ── Extractors ────────────────────────────────────────────────────────────

    fn extract_education(&self) -> Vec<EducationRecord> {
        self.profile
            .artifacts
            .iter()
            .filter(|a| {
                let t = a.artifact_type.to_lowercase();
                t.contains("education") || t.contains("degree") || t.contains("university")
            })
            .map(|a| {
                let summary = &a.content_summary;

                // Parse degree + optional field.
                let (degree, field) = RE_DEGREE
                    .captures(summary)
                    .map(|c| {
                        (
                            Some(c.get(1).unwrap().as_str().to_string()),
                            c.get(2).map(|m| m.as_str().trim().to_string()),
                        )
                    })
                    .unwrap_or((None, None));

                // Parse any 4-digit year present in the summary.
                let year_graduation = RE_YEAR
                    .find(summary)
                    .and_then(|m| m.as_str().parse::<u32>().ok())
                    .filter(|&y| y >= 1960 && y <= 2030);

                EducationRecord {
                    institution: a.title.clone(),
                    degree,
                    field,
                    year_graduation,
                    source_url: a.source_url.clone(),
                }
            })
            .collect()
    }

    fn extract_public_records(&self) -> Vec<PublicRecord> {
        self.profile
            .artifacts
            .iter()
            .filter(|a| {
                let t = a.artifact_type.to_lowercase();
                t.contains("public_record")
                    || t.contains("legal")
                    || t.contains("regulatory")
                    || t.contains("filing")
                    || t.contains("sanction")
            })
            .map(|a| {
                // Parse a date string from the content summary.
                let date_str = RE_DATE
                    .find(&a.content_summary)
                    .map(|m| m.as_str().to_string());

                PublicRecord {
                    record_type: a.artifact_type.clone(),
                    description: a.content_summary.clone(),
                    source_url: a.source_url.clone(),
                    date_str,
                }
            })
            .collect()
    }

    fn extract_associates(&self) -> Vec<KnownAssociate> {
        self.profile
            .artifacts
            .iter()
            .filter(|a| {
                let t = a.artifact_type.to_lowercase();
                t.contains("associate") || t.contains("network") || t.contains("connection")
            })
            .map(|a| {
                // Compute confidence from signal keywords in the content summary.
                let lower = a.content_summary.to_lowercase();
                let confidence = if lower.contains("confirmed")
                    || lower.contains("verified")
                    || lower.contains("known")
                {
                    0.9
                } else if lower.contains("likely") || lower.contains("associated") {
                    0.7
                } else if lower.contains("possible") || lower.contains("suspected") {
                    0.5
                } else if lower.contains("rumored") || lower.contains("alleged") {
                    0.3
                } else {
                    0.5
                };

                KnownAssociate {
                    name: a.title.clone(),
                    relationship: a.artifact_type.clone(),
                    org: None,
                    confidence,
                }
            })
            .collect()
    }

    // ── LLM enrichment ────────────────────────────────────────────────────────

    async fn generate_narrative_and_flags(
        &self,
        llm: &dyn LlmClient,
        education: &[EducationRecord],
        records: &[PublicRecord],
        associates: &[KnownAssociate],
    ) -> Result<(Option<String>, Vec<BackgroundRiskFlag>)> {
        let edu_text = if education.is_empty() {
            "No education records on file.".to_string()
        } else {
            education
                .iter()
                .map(|e| format!("- {}", e.institution))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let work_text = if self.profile.role_history.is_empty() {
            "No work history available.".to_string()
        } else {
            self.profile
                .role_history
                .iter()
                .take(8)
                .map(|r| format!("- {} at {} (since {})", r.title, r.org, r.start_ts))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let records_text = if records.is_empty() {
            "No public records flagged.".to_string()
        } else {
            records
                .iter()
                .take(5)
                .map(|r| format!("- [{}] {}", r.record_type, r.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let assoc_text = if associates.is_empty() {
            "No known associates on file.".to_string()
        } else {
            associates
                .iter()
                .take(5)
                .map(|a| format!("- {} ({})", a.name, a.relationship))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let system = "You are an OSINT background intelligence analyst. \
            Synthesise the provided background data into:\n\
            1. A concise 2-3 sentence narrative paragraph.\n\
            2. A JSON array of risk flags (may be empty []). Each flag: \
            { \"flag_type\": str, \"description\": str, \"severity\": \"low|medium|high\", \"confidence\": float }\n\
            Respond ONLY with a JSON object: { \"narrative\": \"...\", \"risk_flags\": [...] }";

        let user = format!(
            "Subject: {} — {} at {}\n\
            Education:\n{}\n\
            Work history:\n{}\n\
            Public records:\n{}\n\
            Known associates:\n{}",
            self.profile.name,
            self.profile.current_role,
            self.profile.org,
            edu_text,
            work_text,
            records_text,
            assoc_text,
        );

        let json_str = llm
            .generate_json(system, &user)
            .await
            .context("Background LLM call failed")?;

        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).context("Failed to parse LLM background JSON")?;

        let narrative = parsed
            .get("narrative")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let risk_flags: Vec<BackgroundRiskFlag> = parsed
            .get("risk_flags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        Some(BackgroundRiskFlag {
                            flag_type: item.get("flag_type")?.as_str()?.to_string(),
                            description: item.get("description")?.as_str()?.to_string(),
                            severity: item.get("severity")?.as_str()?.to_string(),
                            confidence: item
                                .get("confidence")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.5),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok((narrative, risk_flags))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Convenience function
// ─────────────────────────────────────────────────────────────────────────────

/// Build background for a profile without LLM narration.
pub fn build_background_sync(profile: &PoiProfile) -> PoiBackground {
    let b = BackgroundBuilder::from_profile(profile);
    let education_history = b.extract_education();
    let public_records = b.extract_public_records();
    let known_associates = b.extract_associates();
    let work_history = profile.role_history.clone();
    let mut bg = PoiBackground {
        person_id: profile.person_id.clone(),
        person_name: profile.name.clone(),
        education_history,
        work_history,
        public_records,
        known_associates,
        narrative: None,
        risk_flags: vec![],
        completeness: 0.0,
        created_at: Utc::now(),
    };
    bg.completeness = bg.compute_completeness();
    bg
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        InfluenceProfile, PoiArtifact, PoiProfile, PriorityVector, PsychProfile, RoleFamily,
        RoleHistoryEntry,
    };

    fn make_profile(
        artifacts: Vec<PoiArtifact>,
        role_history: Vec<RoleHistoryEntry>,
    ) -> PoiProfile {
        PoiProfile {
            person_id: "p-001".into(),
            name: "Alice Test".into(),
            name_variants: vec![],
            org: "Test Corp".into(),
            org_id: None,
            current_role: "CEO".into(),
            role_family: RoleFamily::Executive,
            region: "North America".into(),
            country_code: "US".into(),
            public_bio: "A founder.".into(),
            public_email: None,
            artifacts,
            priority_vector: PriorityVector::zero(),
            psychological: PsychProfile::default_profile(),
            influence: InfluenceProfile {
                influence_score: 0.0,
                graph_centrality: 0.0,
                public_recurrence: 0.0,
                role_seniority_score: 0.0,
                network_size: 0,
            },
            engagement: None,
            role_history,
            last_updated_utc: 0,
            profile_completeness: 0.0,
        }
    }

    #[test]
    fn education_extraction_picks_up_education_type() {
        let artifacts = vec![
            PoiArtifact {
                artifact_type: "education".into(),
                title: "MIT".into(),
                content_summary: "BS Computer Science".into(),
                source_url: None,
                ts_utc: 0,
            },
            PoiArtifact {
                artifact_type: "news".into(),
                title: "Some article".into(),
                content_summary: "Content.".into(),
                source_url: None,
                ts_utc: 0,
            },
        ];

        let profile = make_profile(artifacts, vec![]);
        let bg = build_background_sync(&profile);
        assert_eq!(bg.education_history.len(), 1);
        assert_eq!(bg.education_history[0].institution, "MIT");
    }

    #[test]
    fn public_records_extracted_from_sanction_artifacts() {
        let artifacts = vec![PoiArtifact {
            artifact_type: "sanction_record".into(),
            title: "OFAC listing".into(),
            content_summary: "Listed under SDN program.".into(),
            source_url: Some("https://ofac.treasury.gov".into()),
            ts_utc: 0,
        }];

        let profile = make_profile(artifacts, vec![]);
        let bg = build_background_sync(&profile);
        assert_eq!(bg.public_records.len(), 1);
        assert!(bg.public_records[0].description.contains("SDN"));
    }

    #[test]
    fn completeness_increases_with_populated_sections() {
        let profile = make_profile(vec![], vec![]);
        let empty_bg = build_background_sync(&profile);
        assert!(empty_bg.completeness < 0.5);

        let artifacts = vec![
            PoiArtifact {
                artifact_type: "education".into(),
                title: "Oxford".into(),
                content_summary: "".into(),
                source_url: None,
                ts_utc: 0,
            },
            PoiArtifact {
                artifact_type: "public_record".into(),
                title: "Court filing".into(),
                content_summary: "Civil case.".into(),
                source_url: None,
                ts_utc: 0,
            },
        ];
        let profile2 = make_profile(artifacts, vec![]);
        let bg2 = build_background_sync(&profile2);
        assert!(bg2.completeness > empty_bg.completeness);
    }

    #[test]
    fn work_history_inherits_from_profile_role_history() {
        let role = RoleHistoryEntry {
            org: "IBM".into(),
            title: "Engineer".into(),
            role_family: RoleFamily::Engineering,
            start_ts: 1000000,
            end_ts: None,
        };
        let profile = make_profile(vec![], vec![role.clone()]);
        let bg = build_background_sync(&profile);
        assert_eq!(bg.work_history.len(), 1);
        assert_eq!(bg.work_history[0].org, "IBM");
    }
}
