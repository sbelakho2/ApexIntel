use std::collections::HashMap;
use std::sync::Arc;

use apex_core::analysis::{assess_evidence_quality, EvidenceRecord, EvidenceStance};
use uuid::Uuid;

use crate::*;

#[cfg(feature = "llm")]
fn evidence_quality_from_signals(evidence_signals: &[EvidenceSignal]) -> f64 {
    let now = Utc::now();
    let records: Vec<EvidenceRecord> = evidence_signals
        .iter()
        .map(|signal| {
            let mut record =
                EvidenceRecord::new(signal.relevance_score as f64, EvidenceStance::Supports)
                    .with_source_url(signal.source_url.clone())
                    .with_source_type(signal.signal_type.clone());
            if let Some(date_context) = signal.date_context.as_deref() {
                if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(date_context) {
                    record = record.with_observed_at(parsed.with_timezone(&Utc));
                }
            }
            record
        })
        .collect();
    assess_evidence_quality(&records, now).overall_score
}

fn evidence_quality_from_urls(urls: &[String]) -> f64 {
    let now = Utc::now();
    let records: Vec<EvidenceRecord> = urls
        .iter()
        .map(|url| EvidenceRecord::new(0.6, EvidenceStance::Supports).with_source_url(url.clone()))
        .collect();
    assess_evidence_quality(&records, now).overall_score
}

pub(super) async fn run_recipe_fire(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    #[cfg(feature = "llm")]
    let insight_llm_client = {
        let base_url =
            std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
        let api_key = std::env::var("LLM_API_KEY").ok();
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
        let mut config = apex_llm::inference::InferenceConfig::default();
        config.model = model;
        config.max_tokens = 2048;
        config.timeout = std::time::Duration::from_secs(180);
        InferenceLlmClient::new(base_url, api_key, config)
    };

    let seed_recipes = load_default_seed_recipes().unwrap_or_default();
    let engine_recipes: Vec<Recipe> = seed_recipes
        .iter()
        .filter(|sr| !sr.narrative_template.is_empty() && !sr.signals.is_empty())
        .map(|sr| seed_recipe_to_engine_recipe(sr))
        .collect();

    if engine_recipes.is_empty() {
        run.skip("recipe_fire: no seed recipes with signals/templates available");
        return run;
    }

    let engine = RecipeEngine::load(engine_recipes);

    let since = Utc::now() - chrono::Duration::days(30);
    let obs_counts = match store.get_obs_type_counts_per_entity(since).await {
        Ok(v) => v,
        Err(e) => {
            run.fail(&format!(
                "recipe_fire: failed to fetch observation counts: {e}"
            ));
            return run;
        }
    };

    let warn_counts = store
        .get_warning_type_counts_per_entity(since)
        .await
        .unwrap_or_default();
    let ce_features = store
        .get_competitor_event_features(since)
        .await
        .unwrap_or_default();
    let wc_features = store
        .get_webchange_jsonb_features(since)
        .await
        .unwrap_or_default();
    let wc_kw_features = store
        .get_webchange_keyword_features(since)
        .await
        .unwrap_or_default();
    let person_feats = store
        .get_person_features_per_company()
        .await
        .unwrap_or_default();
    let cert_feats = store
        .get_certification_features_per_company()
        .await
        .unwrap_or_default();
    let cap_feats = store
        .get_capability_features_per_company()
        .await
        .unwrap_or_default();
    let site_feats = store
        .get_site_features_per_company()
        .await
        .unwrap_or_default();
    let graph_feats = store.get_graph_edge_features().await.unwrap_or_default();

    if obs_counts.is_empty() && warn_counts.is_empty() {
        run.skip("recipe_fire: no observations or warnings in last 30 days");
        return run;
    }

    let mut entity_maps: HashMap<Uuid, FeatureMap> = HashMap::new();

    for (entity_id, obs_type, count) in &obs_counts {
        let fm = entity_maps.entry(*entity_id).or_default();
        let count_f = *count as f64;
        fm.insert(format!("{obs_type}.count"), count_f);
        fm.insert(format!("{obs_type}.any"), count_f);

        match obs_type.as_str() {
            "lookalike_domain" => {
                for k in &[
                    "LookalikeDomain.count",
                    "LookalikeDomain.active",
                    "LookalikeDomain.any",
                    "Security.risk",
                    "Security.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "dns_posture" => {
                for k in &[
                    "DNSPosture.degraded",
                    "DNSPosture.count",
                    "Security.count",
                    "Compliance.cybersecurity",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "kev_match" => {
                for k in &[
                    "KEV.match",
                    "KEV.count",
                    "Security.vulnerability",
                    "Security.risk",
                    "Security.count",
                    "Compliance.risk",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            "SocialPost" => {
                for k in &[
                    "SocialPost.count",
                    "SocialPost.sentiment",
                    "SocialPost.any",
                    "News.count",
                    "News.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += count_f;
                }
            }
            _ => {}
        }
    }

    for (entity_id, wtype, count) in &warn_counts {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        fm.insert(format!("Warning.{wtype}"), c);

        match wtype.as_str() {
            "certification_update" => {
                for k in &[
                    "CertificationUpdate.count",
                    "CertificationUpdate.any",
                    "Certification.count",
                    "Certification.any",
                    "Compliance.certification",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "hiring_signal" => {
                for k in &[
                    "JobPost.count",
                    "JobPost.any",
                    "Demand.hiring",
                    "Demand.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "ma_activity" => {
                for k in &[
                    "Competitor.ma_activity",
                    "Competitor.acquisition",
                    "Company.acquisition",
                    "Company.M_A",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "expansion" => {
                for k in &[
                    "Company.expansion",
                    "Company.investment",
                    "Facility.new",
                    "Facility.count",
                    "Production.site.change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "technology" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Patent.count",
                    "Patent.any",
                    "Innovation.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "supply_chain_disruption" | "supply_chain" => {
                for k in &[
                    "SupplyChain.disruption",
                    "SupplyChain.count",
                    "Supplier.risk",
                    "Supplier.count",
                    "Material.shortage",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "geopolitical_risk" | "geopolitical" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Sanctions.count",
                    "Sanctions.risk",
                    "Trade.restriction",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "competitive" => {
                for k in &["Competitor.count", "Competitor.activity", "Industry.trend"] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "compliance" => {
                for k in &[
                    "Compliance.count",
                    "Compliance.risk",
                    "Regulatory.count",
                    "Regulatory.change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "market_intelligence" => {
                for k in &[
                    "Market.count",
                    "Market.intelligence",
                    "Industry.count",
                    "Industry.trend",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "procurement" => {
                for k in &[
                    "Procurement.count",
                    "Procurement.any",
                    "Tender.count",
                    "Tender.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "relationship" => {
                for k in &["Relationship.count", "Relationship.any", "Connection.count"] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "talent_movement" | "talent_migration" | "key_hire" => {
                for k in &[
                    "PersonMention.role_change",
                    "PersonMention.count",
                    "RoleChange.competitor_destination",
                    "RoleChange.count",
                    "SocialSignal.leadership_change",
                    "JobPost.executive.new_function",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "patent" | "patent_filing" | "ip_filing" => {
                for k in &[
                    "PatentPublished.competitor.cluster",
                    "PatentPublished.technology_overlap",
                    "PatentPublished.litigation.filed",
                    "PatentPublished.university_collab",
                    "Patent.count",
                    "Patent.any",
                    "IP.count",
                    "IP.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "leadership_change" | "executive_change" => {
                for k in &[
                    "SocialSignal.leadership_change",
                    "SocialSignal.ip_dispute",
                    "PersonMention.role_change",
                    "RoleChange.competitor_destination",
                    "JobPost.executive.new_function",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            other => {
                let capitalized = capitalize_first(other);
                *fm.entry(format!("{capitalized}.count")).or_default() += c;
                *fm.entry(format!("{capitalized}.any")).or_default() += c;
            }
        }
    }

    for (entity_id, signal_type, keyword, count) in &ce_features {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        if !signal_type.is_empty() {
            *fm.entry(format!("Competitor.{signal_type}")).or_default() += c;
            *fm.entry("Competitor.count".into()).or_default() += c;
        }
        if !keyword.is_empty() {
            *fm.entry(format!("Competitor.{keyword}")).or_default() += c;
        }
    }

    for (entity_id, source_id, signal_type, count) in &wc_features {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;

        match source_id.as_str() {
            "ofac_sanctions" | "eu_sanctions" | "un_sanctions" => {
                for k in &[
                    "Sanctions.count",
                    "Sanctions.any",
                    "Sanctions.list",
                    "Sanctions.screening.match",
                    "Sanctions.risk",
                    "OFAC.count",
                    "OFAC.any",
                    "OFAC.match",
                    "Compliance.sanctions",
                    "Trade.count",
                    "Trade.any",
                    "Embargo.count",
                    "Embargo.any",
                    "ExportControl.count",
                    "ExportControl.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "uspto_patents" | "epo_patents" | "wipo_patents" => {
                for k in &[
                    "Patent.count",
                    "Patent.any",
                    "Patent.recent",
                    "Patent.filing",
                    "Patent.competitor",
                    "Technology.patent",
                    "Technology.count",
                    "Technology.any",
                    "IP.count",
                    "IP.any",
                    "PatentPublished.competitor.cluster",
                    "PatentPublished.technology_overlap",
                    "PatentPublished.litigation.filed",
                    "PatentPublished.university_collab",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "sam_gov" | "ted_eu" | "dgmarket" => {
                for k in &[
                    "Tender.count",
                    "Tender.any",
                    "Tender.public",
                    "Tender.posted",
                    "Tender.public.posted",
                    "Tender.framework_agreement",
                    "Tender.sector",
                    "Tender.region",
                    "Procurement.count",
                    "Procurement.any",
                    "Contract.count",
                    "Contract.any",
                    "Government.count",
                    "Government.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "sec_edgar" | "sec_filings" => {
                for k in &[
                    "Filing.count",
                    "Filing.any",
                    "Filing.recent",
                    "Company.filing",
                    "Company.count",
                    "Company.any",
                    "CompanyProfile.count",
                    "CompanyProfile.any",
                    "CompanyProfile.revenue",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "defense_news" | "jane_defence" | "janes_defence" => {
                for k in &[
                    "Defense.count",
                    "Defense.any",
                    "Security.defense",
                    "Security.count",
                    "Security.any",
                    "News.count",
                    "News.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "bloomberg_global" | "ft_global" | "nyt_us" | "axios_us" | "politico_us"
            | "the_hill" | "afp_global" => {
                for k in &[
                    "News.count",
                    "News.any",
                    "News.geopolitical",
                    "News.industry",
                    "News.competitor",
                    "PressRelease.count",
                    "PressRelease.any",
                    "Industry.count",
                    "Industry.any",
                    "Market.count",
                    "Market.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "foreign_affairs" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Geopolitical.any",
                    "News.geopolitical",
                    "News.count",
                    "News.any",
                    "Trade.count",
                    "Trade.any",
                    "Regional.risk.high",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {
                if !source_id.is_empty() {
                    *fm.entry(format!("WebChange.{source_id}")).or_default() += c;
                }
            }
        }

        match signal_type.as_str() {
            "hiring_signal" => {
                for k in &[
                    "JobPost.count",
                    "JobPost.any",
                    "JobPost.role_family",
                    "Demand.hiring",
                    "JobPost.executive.new_function",
                    "SocialSignal.leadership_change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "certification_update" => {
                for k in &[
                    "CertificationUpdate.count",
                    "CertificationUpdate.any",
                    "CertificationUpdate.new",
                    "Certification.count",
                    "Certification.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "technology" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Innovation.count",
                    "Innovation.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "supply_chain_disruption" => {
                for k in &[
                    "SupplyChain.count",
                    "SupplyChain.disruption",
                    "Supplier.risk",
                    "Supplier.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "geopolitical_risk" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Security.risk",
                    "Security.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {}
        }
    }

    for (entity_id, keyword, count) in &wc_kw_features {
        let fm = entity_maps.entry(*entity_id).or_default();
        let c = *count as f64;
        match keyword.as_str() {
            "patent" => {
                for k in &[
                    "Patent.count",
                    "Patent.any",
                    "Patent.recent",
                    "IP.count",
                    "IP.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "innovation" | "breakthrough" | "next-generation" | "new technology" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Technology.emerging",
                    "Innovation.count",
                    "Innovation.any",
                    "Industry40.count",
                    "Industry40.any",
                    "Digital.count",
                    "Digital.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "r&d" | "research and development" => {
                for k in &[
                    "Technology.count",
                    "Technology.any",
                    "Engineering.count",
                    "Engineering.any",
                    "Competitor.R_D",
                    "Product.development.early",
                    "SocialSignal.research_partnership",
                    "PatentPublished.university_collab",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "career" | "hiring" | "job opening" | "join our team" | "open position" => {
                for k in &[
                    "JobPost.count",
                    "JobPost.any",
                    "JobPost.volume",
                    "Demand.hiring",
                    "Demand.count",
                    "Demand.any",
                    "JobPost.executive.new_function",
                    "SocialSignal.leadership_change",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "certification" | "accreditation" | "iso 9001" | "iso 14001" | "iso 13485"
            | "iso 27001" | "as9100" | "iatf 16949" => {
                for k in &[
                    "CertificationUpdate.count",
                    "CertificationUpdate.any",
                    "CertificationUpdate.new",
                    "Certification.count",
                    "Certification.any",
                    "Certification.new",
                    "Compliance.count",
                    "Compliance.any",
                    "Compliance.certification",
                    "Audit.count",
                    "Audit.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
                match keyword.as_str() {
                    "iatf 16949" => {
                        *fm.entry("CertificationUpdate.new.IATF_16949".into())
                            .or_default() += c;
                    }
                    "as9100" => {
                        *fm.entry("CertificationUpdate.new.AS9100".into())
                            .or_default() += c;
                    }
                    "iso 13485" => {
                        *fm.entry("CertificationUpdate.new.ISO_13485".into())
                            .or_default() += c;
                    }
                    "iso 27001" => {
                        *fm.entry("CertificationUpdate.new.ISO_27001".into())
                            .or_default() += c;
                    }
                    _ => {}
                }
            }
            "compliance" | "audit" => {
                for k in &[
                    "Compliance.count",
                    "Compliance.any",
                    "Compliance.risk",
                    "Regulatory.count",
                    "Regulatory.any",
                    "Audit.count",
                    "Audit.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "sanctions" => {
                for k in &[
                    "Sanctions.count",
                    "Sanctions.any",
                    "Sanctions.list",
                    "OFAC.count",
                    "OFAC.any",
                    "Compliance.sanctions",
                    "Trade.count",
                    "Trade.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "tariff" => {
                for k in &[
                    "Tariff.count",
                    "Tariff.any",
                    "Tariff.change.announced",
                    "Tariff.reduction",
                    "Trade.count",
                    "Trade.any",
                    "Trade.restriction",
                    "Customs.count",
                    "Customs.any",
                    "Import.count",
                    "Import.any",
                    "ImportData.count",
                    "ImportData.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "shortage" | "allocation" | "lead time" => {
                for k in &[
                    "SupplyChain.count",
                    "SupplyChain.any",
                    "SupplyChain.lead_time.increase",
                    "Supplier.lead_time.increase",
                    "Supplier.capacity.reduced",
                    "Material.shortage",
                    "Material.count",
                    "Material.any",
                    "Commodity.shortage",
                    "Commodity.count",
                    "Commodity.any",
                    "Semiconductor.lead_time.surge",
                    "Inventory.count",
                    "Inventory.any",
                    "Component.count",
                    "Component.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "supply chain disruption" => {
                for k in &[
                    "SupplyChain.disruption",
                    "SupplyChain.count",
                    "SupplyChain.any",
                    "Supplier.risk",
                    "Supplier.count",
                    "Supplier.any",
                    "Logistics.disruption",
                    "Logistics.count",
                    "Logistics.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "geopolitical" => {
                for k in &[
                    "Geopolitical.risk",
                    "Geopolitical.count",
                    "Geopolitical.any",
                    "Security.risk",
                    "Security.count",
                    "Security.any",
                    "Regional.risk.high",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "acquisition" | "acquired" | "joint venture" | "strategic partnership" => {
                for k in &[
                    "Company.acquisition",
                    "Company.count",
                    "Company.any",
                    "Competitor.acquisition",
                    "Competitor.ma_activity",
                    "Post_MA.integration",
                    "Post_MA.integration.issues",
                    "Partnership.strategic",
                    "PressRelease.acquisition",
                    "PressRelease.JV_announced",
                    "NewEntrant.count",
                    "NewEntrant.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "investment in" | "new facility" | "new manufacturing" | "grand opening"
            | "groundbreaking" => {
                for k in &[
                    "Company.expansion",
                    "Company.investment",
                    "Company.count",
                    "Company.any",
                    "Competitor.expansion",
                    "Competitor.factory",
                    "Facility.new",
                    "Facility.count",
                    "Facility.any",
                    "Production.site.change",
                    "Production.count",
                    "Production.any",
                    "PressRelease.expansion",
                    "Infrastructure.count",
                    "Infrastructure.any",
                    "Growth.count",
                    "Growth.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "product launch" => {
                for k in &[
                    "Product.count",
                    "Product.any",
                    "Product.development.early",
                    "Product.supply_chain.new",
                    "Competitor.product",
                    "Competitor.count",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {
                if !keyword.is_empty() {
                    let kw_clean = keyword.replace(' ', "_");
                    *fm.entry(format!("WebChange.kw_{kw_clean}")).or_default() += c;
                }
            }
        }
    }

    for (company_id, role_family, influence, pain, change_risk, count) in &person_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;
        *fm.entry("POI.count".into()).or_default() += c;
        *fm.entry("POI.any".into()).or_default() += c;

        if *influence > 0.7 {
            for k in &[
                "POI.influence.broad",
                "POI.influence.expanding",
                "POI.influence.external",
                "POI.influence.chain",
                "POI.strategic.influence",
                "POI.visibility.high",
                "POI.spec.influence",
                "POI.stakeholder.map.complete",
            ] {
                *fm.entry(k.to_string()).or_default() += c;
            }
        }

        if *pain > 0.6 {
            for k in &[
                "POI.pain_index.high",
                "POI.pain.cost.expressed",
                "POI.pain.delivery.expressed",
                "POI.pain.quality.expressed",
                "POI.pain.flexibility.expressed",
                "POI.frustration.supplier",
                "POI.frustration.internal",
                "POI.lead_time.concern",
                "POI.reliability.priority",
            ] {
                *fm.entry(k.to_string()).or_default() += c;
            }
        }
        if *pain > 0.8 {
            *fm.entry("POI.pain_index.very_high".into()).or_default() += c;
        }

        if *change_risk > 0.5 {
            for k in &[
                "POI.role_change.CPO.new",
                "POI.scope.expanded",
                "POI.promotion.detected",
                "POI.milestone.career",
                "POI.project.new",
                "POI.team.building",
                "Decision.count",
                "Decision.any",
                "Decision.imminent",
                "Decision.budget.allocated",
                "Decision.committee.formed",
                "PersonMention.role_change",
                "PersonMention.count",
                "RoleChange.competitor_destination",
                "RoleChange.count",
                "SocialSignal.leadership_change",
                "JobPost.executive.new_function",
            ] {
                *fm.entry(k.to_string()).or_default() += c;
            }
        }

        match role_family.as_str() {
            "C-Suite" => {
                for k in &[
                    "POI.strategic.influence",
                    "POI.visibility.high",
                    "POI.thought_leadership",
                    "POI.media.presence",
                    "POI.network.broad",
                    "POI.social.active",
                    "Decision.executive",
                    "Succession.identified",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "Government" | "Agency Head" => {
                for k in &[
                    "POI.region.visiting",
                    "Government.count",
                    "Government.any",
                    "Regulatory.count",
                    "Regulatory.any",
                    "POI.knowledge.gap",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            "Industry Association" | "Industry Analyst" => {
                for k in &[
                    "POI.thought_leadership",
                    "POI.media.presence",
                    "POI.network.broad",
                    "Industry.count",
                    "Industry.any",
                    "ConferenceAgenda.count",
                    "ConferenceAgenda.any",
                    "Event.count",
                    "Event.any",
                ] {
                    *fm.entry(k.to_string()).or_default() += c;
                }
            }
            _ => {}
        }

        for k in &[
            "POI.location.nearby",
            "POI.operations.experience",
            "Multi_POI.count",
            "Multi_POI.any",
            "Trigger.count",
            "Trigger.any",
        ] {
            *fm.entry(k.to_string()).or_default() += c;
        }
    }

    for (company_id, standard, count) in &cert_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;
        *fm.entry("Certification.count".into()).or_default() += c;
        *fm.entry("Certification.any".into()).or_default() += c;
        *fm.entry("CertificationUpdate.count".into()).or_default() += c;
        *fm.entry("CertificationUpdate.any".into()).or_default() += c;

        let std_upper = standard.to_uppercase();
        if std_upper.contains("IATF") || std_upper.contains("16949") {
            *fm.entry("CertificationUpdate.new.IATF_16949".into())
                .or_default() += c;
            *fm.entry("Tender.sector=automotive".into()).or_default() += c;
        }
        if std_upper.contains("AS9100")
            || std_upper.contains("AS 9100")
            || std_upper.contains("EN 9100")
        {
            *fm.entry("CertificationUpdate.new.AS9100".into())
                .or_default() += c;
            *fm.entry("Tender.sector=aerospace".into()).or_default() += c;
        }
        if std_upper.contains("13485") {
            *fm.entry("CertificationUpdate.new.ISO_13485".into())
                .or_default() += c;
            *fm.entry("Tender.sector=medical".into()).or_default() += c;
        }
        if std_upper.contains("14001") {
            *fm.entry("Environmental.count".into()).or_default() += c;
            *fm.entry("ESG.count".into()).or_default() += c;
        }
        if std_upper.contains("27001") {
            *fm.entry("CertificationUpdate.new.ISO_27001".into())
                .or_default() += c;
            *fm.entry("Security.count".into()).or_default() += c;
            *fm.entry("Compliance.cybersecurity".into()).or_default() += c;
        }
    }

    for (company_id, capability, count) in &cap_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;
        *fm.entry("Capability.count".into()).or_default() += c;
        *fm.entry("Capability.any".into()).or_default() += c;
        *fm.entry("Technology.count".into()).or_default() += c;

        let cap_lower = capability.to_lowercase();
        if cap_lower.contains("smt") || cap_lower.contains("pcb") {
            *fm.entry("Competitor.careers.SMT".into()).or_default() += c;
            *fm.entry("Competitor.capability_page.changed".into())
                .or_default() += c;
        }
        if cap_lower.contains("medical") {
            *fm.entry("Tender.sector=medical".into()).or_default() += c;
        }
        if cap_lower.contains("automotive") {
            *fm.entry("Tender.sector=automotive".into()).or_default() += c;
        }
        if cap_lower.contains("aerospace")
            || cap_lower.contains("avionics")
            || cap_lower.contains("satellite")
        {
            *fm.entry("Tender.sector=aerospace".into()).or_default() += c;
            *fm.entry("Defense.count".into()).or_default() += c;
        }
        if cap_lower.contains("iot") || cap_lower.contains("embedded") {
            *fm.entry("Industry40.count".into()).or_default() += c;
            *fm.entry("Digital.count".into()).or_default() += c;
        }
        if cap_lower.contains("prototype") || cap_lower.contains("testing") {
            *fm.entry("Product.development.early".into()).or_default() += c;
        }
    }

    for (company_id, country_code, site_type, count) in &site_feats {
        let fm = entity_maps.entry(*company_id).or_default();
        let c = *count as f64;

        *fm.entry("Production.count".into()).or_default() += c;
        *fm.entry("Production.any".into()).or_default() += c;
        *fm.entry("Facility.count".into()).or_default() += c;
        *fm.entry("Facility.any".into()).or_default() += c;

        if !country_code.is_empty() {
            *fm.entry(format!("Geographic.{country_code}")).or_default() += c;
            *fm.entry("SupplyChain.geo.concentrated".into()).or_default() += c;
        }
        if !site_type.is_empty() {
            *fm.entry(format!("Facility.{site_type}")).or_default() += c;
            *fm.entry("Production.site.change".into()).or_default() += c;
        }
    }

    for (source_id, edge_type, count) in &graph_feats {
        let fm = entity_maps.entry(*source_id).or_default();
        let c = *count as f64;

        *fm.entry("Connection.count".into()).or_default() += c;
        *fm.entry("Relationship.count".into()).or_default() += c;
        *fm.entry("Ecosystem.count".into()).or_default() += c;

        match edge_type.as_str() {
            "CompanyCompany" => {
                *fm.entry("Competitor.count".into()).or_default() += c;
                *fm.entry("Vendor.count".into()).or_default() += c;
                *fm.entry("Vendor.any".into()).or_default() += c;
                *fm.entry("Supplier.count".into()).or_default() += c;
                *fm.entry("Customer.count".into()).or_default() += c;
                *fm.entry("Customer.any".into()).or_default() += c;
                *fm.entry("Partnership.strategic".into()).or_default() += c;
                *fm.entry("Distributor.count".into()).or_default() += c;
                *fm.entry("Distributor.any".into()).or_default() += c;
            }
            "CompanyPerson" => {
                *fm.entry("POI.count".into()).or_default() += c;
                *fm.entry("POI.any".into()).or_default() += c;
                *fm.entry("Alumni.count".into()).or_default() += c;
                *fm.entry("Alumni.any".into()).or_default() += c;
                *fm.entry("Alumni.connection".into()).or_default() += c;
                *fm.entry("Connection.mutual".into()).or_default() += c;
            }
            "leads" => {
                *fm.entry("POI.influence.chain".into()).or_default() += c;
                *fm.entry("Relationship.new".into()).or_default() += c;
                *fm.entry("Relationship.strengthened".into()).or_default() += c;
            }
            _ => {}
        }
    }

    tracing::info!(
        entities = entity_maps.len(),
        obs_rows = obs_counts.len(),
        warn_rows = warn_counts.len(),
        ce_rows = ce_features.len(),
        wc_rows = wc_features.len(),
        kw_rows = wc_kw_features.len(),
        person_rows = person_feats.len(),
        cert_rows = cert_feats.len(),
        cap_rows = cap_feats.len(),
        site_rows = site_feats.len(),
        graph_rows = graph_feats.len(),
        "recipe_fire: built feature maps"
    );

    let entity_id_strs: Vec<(String, FeatureMap)> = entity_maps
        .into_iter()
        .map(|(id, fm)| (id.to_string(), fm))
        .collect();
    let entity_refs: Vec<(&str, &FeatureMap)> = entity_id_strs
        .iter()
        .map(|(id, fm)| (id.as_str(), fm))
        .collect();

    let candidates = engine.evaluate_batch(&entity_refs);

    let all_entity_uuids: Vec<Uuid> = candidates
        .iter()
        .filter_map(|c| Uuid::parse_str(&c.entity_id).ok())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let company_names: HashMap<String, (String, Option<String>, Option<String>)> = match store
        .get_company_names_by_ids(&all_entity_uuids)
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|(id, name, region, company_type)| (id.to_string(), (name, region, company_type)))
            .collect(),
        Err(e) => {
            tracing::warn!("recipe_fire: failed to load company names: {e}");
            HashMap::new()
        }
    };

    let poi_names: HashMap<String, String> = match store
        .get_person_names_by_company_ids(&all_entity_uuids)
        .await
    {
        Ok(rows) => {
            let mut m: HashMap<String, String> = HashMap::new();
            for (org_id, name) in rows {
                m.entry(org_id.to_string()).or_insert(name);
            }
            m
        }
        Err(e) => {
            tracing::warn!("recipe_fire: failed to load POI names: {e}");
            HashMap::new()
        }
    };

    #[cfg(feature = "llm")]
    let (entity_evidence, entity_contexts): (
        HashMap<String, Vec<EvidenceSignal>>,
        HashMap<String, EntityContext>,
    ) = {
        let mut evidence_map: HashMap<String, Vec<EvidenceSignal>> = HashMap::new();
        let mut context_map: HashMap<String, EntityContext> = HashMap::new();

        for (entity_id, (name, region, company_type)) in &company_names {
            let industry_tags: Vec<String> = Vec::new();
            context_map.insert(
                entity_id.clone(),
                EntityContext {
                    name: name.clone(),
                    region: region.clone().unwrap_or_default(),
                    entity_type: company_type.clone(),
                    is_competitor: false,
                    industry_tags,
                    certifications: Vec::new(),
                    capabilities: Vec::new(),
                    key_persons: Vec::new(),
                    recent_changes: Vec::new(),
                    threat_score: None,
                    overlap_score: None,
                    strategic_relevance: None,
                    revenue_estimate_usd: None,
                    employee_estimate: None,
                    competitor_names: Vec::new(),
                    sites_summary: Vec::new(),
                    competitor_events: Vec::new(),
                    domain: None,
                },
            );
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(Some(row)) = sqlx::query_as::<_, (Option<f64>, Option<f64>, Option<f64>, Option<i64>, Option<i32>, Option<String>, Option<Vec<String>>, Option<bool>)>(
                "SELECT threat_score, overlap_score, strategic_relevance, revenue_estimate_usd, employee_estimate, domain, industry_tags, (metadata->>'is_competitor')::boolean FROM companies WHERE id = $1"
            )
            .bind(entity_uuid)
            .fetch_optional(&store.pool)
            .await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    ctx.threat_score = row.0;
                    ctx.overlap_score = row.1;
                    ctx.strategic_relevance = row.2;
                    ctx.revenue_estimate_usd = row.3;
                    ctx.employee_estimate = row.4;
                    ctx.domain = row.5;
                    if let Some(tags) = row.6 {
                        ctx.industry_tags = tags;
                    }
                    ctx.is_competitor = row.7.unwrap_or(false);
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(edges) = store.get_edges_from(*entity_uuid, "company").await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for edge in edges.iter().take(8) {
                        if let Some((name, region, _ctype)) =
                            company_names.get(&edge.target_id.to_string())
                        {
                            let region_str = region.as_deref().unwrap_or("");
                            ctx.competitor_names
                                .push(format!("{} ({})", name, region_str));
                        }
                    }
                }
            }
            if let Ok(edges) = store.get_edges_to(*entity_uuid, "company").await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for edge in edges.iter().take(8) {
                        if let Some((name, region, _ctype)) =
                            company_names.get(&edge.source_id.to_string())
                        {
                            let region_str = region.as_deref().unwrap_or("");
                            let entry = format!("{} ({})", name, region_str);
                            if !ctx.competitor_names.contains(&entry) {
                                ctx.competitor_names.push(entry);
                            }
                        }
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(sites) = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>, Option<Vec<String>>)>(
                "SELECT name, city, country_code, site_type, capabilities FROM sites WHERE company_id = $1 LIMIT 6"
            )
            .bind(entity_uuid)
            .fetch_all(&store.pool)
            .await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for (sname, city, cc, stype, caps) in sites {
                        let location = city.unwrap_or_else(|| cc.unwrap_or_default());
                        let type_str = stype.unwrap_or_default();
                        let caps_str = caps.map(|c| c.join(", ")).unwrap_or_default();
                        let summary = if caps_str.is_empty() {
                            format!("{} ({}) in {}", sname, type_str, location)
                        } else {
                            format!("{} ({}) in {} – capabilities: {}", sname, type_str, location, caps_str)
                        };
                        ctx.sites_summary.push(summary);
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            let entity_id_str = entity_uuid.to_string();
            if let Ok(obs) = store.get_observations_by_entity(*entity_uuid, 5).await {
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for o in obs {
                        if o.observation_type == "CompetitorEvent"
                            || o.observation_type == "JobPost"
                            || o.observation_type == "SocialPost"
                        {
                            let excerpt = o
                                .value
                                .as_object()
                                .and_then(|obj| {
                                    obj.get("excerpt").or(obj.get("title")).or(obj.get("text"))
                                })
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let source = o
                                .value
                                .as_object()
                                .and_then(|obj| obj.get("source").or(obj.get("url")))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !excerpt.is_empty() {
                                let evt_line = crate::truncate_text(
                                    &format!("{}: {}", o.observation_type, &excerpt),
                                    200,
                                )
                                .to_string();
                                let sig = EvidenceSignal {
                                    title: format!(
                                        "{}: {}",
                                        o.observation_type,
                                        crate::truncate_text(&excerpt, 80)
                                    ),
                                    description: excerpt,
                                    source_url: source,
                                    signal_type: o.observation_type.clone(),
                                    extracted_facts: extract_facts_from_text(&o.value.to_string()),
                                    date_context: Some(o.ts_utc.format("%Y-%m-%d").to_string()),
                                    relevance_score: 0.75,
                                };
                                evidence_map
                                    .entry(entity_id_str.clone())
                                    .or_default()
                                    .push(sig);
                                ctx.competitor_events.push(evt_line);
                            }
                        }
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            if let Ok(certs) = store.get_certifications_for_company(*entity_uuid).await {
                let entity_id_str = entity_uuid.to_string();
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for cert in certs.iter().take(10) {
                        let valid_info = cert
                            .valid_until
                            .map(|d| format!(" (valid until {})", d))
                            .unwrap_or_default();
                        let cert_str = format!("{}{}", cert.standard, valid_info);
                        ctx.certifications.push(cert_str.clone());

                        let sig = EvidenceSignal {
                            title: format!("{} Certification", cert.standard),
                            description: format!(
                                "Holds {} certification{}. Issuing body: {}. Scope: {}",
                                cert.standard,
                                valid_info,
                                cert.issuing_body.as_deref().unwrap_or("Unknown"),
                                cert.scope.as_deref().unwrap_or("General")
                            ),
                            source_url: cert.evidence_url.clone().unwrap_or_default(),
                            signal_type: "certification".to_string(),
                            extracted_facts: vec![
                                format!("Standard: {}", cert.standard),
                                cert.valid_until
                                    .map(|d| format!("Valid until: {}", d))
                                    .unwrap_or_default(),
                            ]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect(),
                            date_context: cert.valid_until.map(|d| d.to_string()),
                            relevance_score: 0.5,
                        };
                        evidence_map
                            .entry(entity_id_str.clone())
                            .or_default()
                            .push(sig);
                    }
                }
            }
        }

        for entity_uuid in all_entity_uuids.iter() {
            if let Ok(caps) = store.list_capabilities(Some(*entity_uuid), 15, 0).await {
                let entity_id_str = entity_uuid.to_string();
                if let Some(ctx) = context_map.get_mut(&entity_id_str) {
                    for cap in caps.iter().take(10) {
                        let proof_info = cap
                            .proof_grade
                            .as_deref()
                            .map(|g| format!(" ({})", g))
                            .unwrap_or_default();
                        ctx.capabilities
                            .push(format!("{}{}", cap.capability, proof_info));

                        let sig = EvidenceSignal {
                            title: format!("Capability: {}", cap.capability),
                            description: format!(
                                "Manufacturing capability: {}. Proof level: {}",
                                cap.capability,
                                cap.proof_grade.as_deref().unwrap_or("Claimed")
                            ),
                            source_url: cap
                                .evidence_urls
                                .as_ref()
                                .and_then(|u| u.first().cloned())
                                .unwrap_or_default(),
                            signal_type: "capability".to_string(),
                            extracted_facts: vec![format!("Capability: {}", cap.capability)],
                            date_context: cap
                                .last_confirmed
                                .map(|d| d.format("%Y-%m-%d").to_string()),
                            relevance_score: 0.4,
                        };
                        evidence_map
                            .entry(entity_id_str.clone())
                            .or_default()
                            .push(sig);
                    }
                }
            }
        }

        for (entity_id, poi_name) in &poi_names {
            if let Some(ctx) = context_map.get_mut(entity_id) {
                ctx.key_persons.push(poi_name.clone());
            }
        }

        match store
            .get_warnings_by_entity_ids(&all_entity_uuids, 500)
            .await
        {
            Ok(rows) => {
                for w in rows {
                    if let Some(entity_ids) = &w.entity_ids {
                        let description = w.description.clone().unwrap_or_default();
                        let extracted =
                            extract_facts_from_text(&format!("{} {}", w.title, description));

                        let sig = EvidenceSignal {
                            title: w.title.clone(),
                            description: description.clone(),
                            source_url: w
                                .source_urls
                                .as_ref()
                                .and_then(|urls| urls.first().cloned())
                                .unwrap_or_default(),
                            signal_type: "warning".to_string(),
                            extracted_facts: extracted,
                            date_context: Some(w.ts_utc.format("%Y-%m-%d").to_string()),
                            relevance_score: 0.8
                                + (w.severity.as_str() == "critical")
                                    .then_some(0.2)
                                    .unwrap_or(0.0) as f32,
                        };

                        for eid in entity_ids {
                            evidence_map
                                .entry(eid.to_string())
                                .or_default()
                                .push(sig.clone());
                            if let Some(ctx) = context_map.get_mut(&eid.to_string()) {
                                let change = format!("{}: {}", w.warning_type, w.title);
                                if ctx.recent_changes.len() < 5 {
                                    ctx.recent_changes.push(change);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("recipe_fire: failed to load warning evidence: {e}");
            }
        }

        let unique_entity_uuids: std::collections::HashSet<Uuid> =
            all_entity_uuids.iter().cloned().collect();
        for entity_uuid in unique_entity_uuids.iter() {
            if let Ok(obs_rows) = store.get_observations_by_entity(*entity_uuid, 10).await {
                for obs in obs_rows {
                    let (title, description) = if let Some(obj) = obs.value.as_object() {
                        let title = obj
                            .get("title")
                            .or_else(|| obj.get("headline"))
                            .or_else(|| obj.get("role"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("{} update", obs.observation_type));
                        let desc = obj
                            .get("description")
                            .or_else(|| obj.get("text"))
                            .or_else(|| obj.get("summary"))
                            .or_else(|| obj.get("content"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        (title, desc)
                    } else if let Some(s) = obs.value.as_str() {
                        (obs.observation_type.clone(), s.to_string())
                    } else {
                        continue;
                    };

                    if description.is_empty() && !title.contains(':') {
                        continue;
                    }

                    let source_url = obs
                        .provenance
                        .as_object()
                        .and_then(|p| p.get("source_url").or_else(|| p.get("url")))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    let extracted = extract_facts_from_text(&format!("{} {}", title, description));

                    let sig = EvidenceSignal {
                        title,
                        description,
                        source_url,
                        signal_type: obs.observation_type.clone(),
                        extracted_facts: extracted,
                        date_context: Some(obs.ts_utc.format("%Y-%m-%d").to_string()),
                        relevance_score: 0.5,
                    };
                    evidence_map
                        .entry(entity_uuid.to_string())
                        .or_default()
                        .push(sig);
                }
            }
        }

        tracing::info!(
            unique_entities = unique_entity_uuids.len(),
            entities_with_evidence = evidence_map.len(),
            total_evidence_items = evidence_map.values().map(|v| v.len()).sum::<usize>(),
            entities_with_context = context_map.len(),
            "recipe_fire: loaded enriched evidence for LLM"
        );

        (evidence_map, context_map)
    };

    #[cfg(not(feature = "llm"))]
    let entity_evidence_urls = collect_entity_evidence_urls(&store, &all_entity_uuids).await;

    let mut dedup_map: HashMap<(String, String), usize> = HashMap::new();
    for (idx, c) in candidates.iter().enumerate() {
        let key = (c.recipe_code.clone(), c.entity_id.clone());
        let dominated = match dedup_map.get(&key) {
            Some(&prev_idx) => {
                let prev = &candidates[prev_idx];
                c.confidence * c.impact > prev.confidence * prev.impact
            }
            None => true,
        };
        if dominated {
            dedup_map.insert(key, idx);
        }
    }
    let deduped_idxs: std::collections::HashSet<usize> = dedup_map.values().copied().collect();

    let total_candidates = candidates.len();
    let deduped_count = total_candidates - deduped_idxs.len();
    if deduped_count > 0 {
        tracing::info!(
            total = total_candidates,
            deduped = deduped_count,
            "recipe_fire: per-run dedup removed duplicate candidates"
        );
    }

    let mut insights_inserted: u64 = 0;
    let mut warnings_inserted: u64 = 0;
    let mut skipped_low_conf: u64 = 0;
    let mut skipped_dedup: u64 = 0;
    let mut skipped_cross_run: u64 = 0;

    let cross_run_dedup: std::collections::HashSet<(String, String)> = {
        let rows = sqlx::query(
            "SELECT DISTINCT unnest(tags) AS tag, unnest(entity_ids)::text AS eid \
             FROM insights WHERE created_at > NOW() - INTERVAL '4 days'",
        )
        .fetch_all(&store.pool)
        .await;
        match rows {
            Ok(rows) => {
                use sqlx::Row as _;
                rows.into_iter()
                    .filter_map(|r| {
                        let tag: String = r.try_get("tag").ok()?;
                        let eid: String = r.try_get("eid").ok()?;
                        Some((tag, eid))
                    })
                    .collect()
            }
            Err(e) => {
                tracing::warn!("recipe_fire: failed to load cross-run dedup set: {e}");
                std::collections::HashSet::new()
            }
        }
    };

    #[cfg(feature = "llm")]
    let mut llm_entity_cat_done: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    #[cfg(feature = "llm")]
    let mut entity_run_count: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    for (idx, c) in candidates.iter().enumerate() {
        if !deduped_idxs.contains(&idx) {
            skipped_dedup += 1;
            continue;
        }

        if c.confidence < 0.45 {
            skipped_low_conf += 1;
            continue;
        }

        if cross_run_dedup.contains(&(c.recipe_code.clone(), c.entity_id.clone())) {
            skipped_cross_run += 1;
            continue;
        }

        let entity_uuid = Uuid::parse_str(&c.entity_id).ok();
        let entity_ids = entity_uuid.map(|u| vec![u]);
        let mut tags = vec![c.category.clone(), c.recipe_code.clone()];

        let mut slots: HashMap<String, String> = HashMap::new();
        let mut entity_label = String::new();
        let mut entity_region = String::new();
        let mut entity_type: Option<String> = None;
        if let Some((name, region, company_type)) = company_names.get(&c.entity_id) {
            entity_label = name.clone();
            entity_type = company_type.clone();
            slots.insert("company_name".into(), name.clone());
            slots.insert("supplier_name".into(), name.clone());
            slots.insert("vendor_name".into(), name.clone());
            slots.insert("customer_name".into(), name.clone());
            slots.insert("target_company".into(), name.clone());
            slots.insert("entity_name".into(), name.clone());
            slots.insert("competitor_name".into(), name.clone());
            if let Some(r) = region {
                entity_region = r.clone();
                slots.insert("region".into(), r.clone());
                slots.insert("location".into(), r.clone());
                slots.insert("country".into(), r.clone());
            }
        }
        if let Some(poi) = poi_names.get(&c.entity_id) {
            slots.insert("poi_name".into(), poi.clone());
            slots.insert("mentor_name".into(), poi.clone());
        }

        let mut signal_details: Vec<String> = Vec::new();
        if let Some((_, fm)) = entity_id_strs.iter().find(|(id, _)| id == &c.entity_id) {
            if let Some(v) = fm.get("POI.count") {
                slots.insert("poi_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} person(s) of interest tracked", *v as i64));
                }
            }
            if let Some(v) = fm.get("JobPost.count") {
                slots.insert("job_count".into(), (*v as i64).to_string());
                slots.insert("job_delta".into(), format!("{:.0}", v));
                slots.insert("npi_jobs".into(), (*v as i64).to_string());
                slots.insert("eng_jobs".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} job posting(s) observed", *v as i64));
                }
            }
            if let Some(v) = fm.get("Patent.count") {
                slots.insert("patent_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} patent(s) filed", *v as i64));
                }
            }
            if let Some(v) = fm.get("Tender.count") {
                slots.insert("tender_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} tender(s) identified", *v as i64));
                }
            }
            if let Some(v) = fm.get("Certification.count") {
                slots.insert("cert_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} certification(s) on record", *v as i64));
                }
            }
            if let Some(v) = fm.get("Competitor.count") {
                slots.insert("competitor_count".into(), (*v as i64).to_string());
                if *v > 0.0 {
                    signal_details.push(format!("{} competitor signal(s)", *v as i64));
                }
            }
            if let Some(v) = fm.get("NewsArticle.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} news article(s) referenced", *v as i64));
                }
            }
            if let Some(v) = fm.get("WebChange.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} web change(s) detected", *v as i64));
                }
            }
            if let Some(v) = fm.get("LookalikeDomain.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} lookalike domain(s) detected", *v as i64));
                }
            }
            if let Some(v) = fm.get("DNSPosture.degraded") {
                if *v > 0.0 {
                    signal_details.push("DNS posture degradation detected".to_string());
                }
            }
            if let Some(v) = fm.get("KEV.match") {
                if *v > 0.0 {
                    signal_details.push("known exploited vulnerability match detected".to_string());
                }
            }
            if let Some(v) = fm.get("KEV.count") {
                if *v > 0.0 {
                    signal_details.push(format!(
                        "{} KEV-linked vulnerability match(es) detected",
                        *v as i64
                    ));
                }
            }
            if let Some(v) = fm.get("Filing.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} regulatory filing(s)", *v as i64));
                }
            }
            if let Some(v) = fm.get("SanctionEntry.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} sanction entry(ies)", *v as i64));
                }
            }
            if let Some(v) = fm.get("TradeShow.count") {
                if *v > 0.0 {
                    signal_details.push(format!("{} trade show participation(s)", *v as i64));
                }
            }
            let signal_count = c.evidence_ids.len();
            slots.insert("signal_count".into(), signal_count.to_string());
        }

        #[cfg(not(feature = "llm"))]
        let _rendered_narrative = clean_rendered_text(&resolve_evidence_placeholders(
            &c.narrative_template,
            &slots,
        ));
        #[cfg(not(feature = "llm"))]
        let rendered_action =
            clean_rendered_text(&resolve_evidence_placeholders(&c.action_template, &slots));

        #[cfg(not(feature = "llm"))]
        let analytical_narrative = build_analytical_narrative(
            &entity_label,
            &entity_region,
            entity_type.as_deref(),
            &c.category,
            &signal_details,
            c.confidence,
            &c.evidence_ids,
        );

        #[cfg(feature = "llm")]
        {
            let ec_key = (c.entity_id.clone(), c.category.clone());
            if !llm_entity_cat_done.insert(ec_key) {
                tracing::debug!(
                    entity = %entity_label,
                    category = %c.category,
                    recipe = %c.recipe_code,
                    "recipe_fire: skipping duplicate entity+category LLM call"
                );
                skipped_dedup += 1;
                continue;
            }

            let run_count = entity_run_count.entry(c.entity_id.clone()).or_insert(0);
            if *run_count >= 2 {
                tracing::debug!(
                    entity = %entity_label,
                    category = %c.category,
                    "recipe_fire: per-run entity cap reached (F9), skipping"
                );
                skipped_dedup += 1;
                continue;
            }
            *run_count += 1;
        }

        #[cfg(feature = "llm")]
        let llm_output = {
            let mut evidence_signals: Vec<EvidenceSignal> = entity_evidence
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_default();

            for sig in &mut evidence_signals {
                sig.relevance_score = calculate_relevance(
                    &sig.title,
                    &sig.description,
                    &sig.signal_type,
                    &c.category,
                );
            }

            evidence_signals.sort_by(|a, b| {
                b.relevance_score
                    .partial_cmp(&a.relevance_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            evidence_signals.truncate(12);

            let entity_ctx = entity_contexts
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_else(|| EntityContext {
                    name: entity_label.clone(),
                    region: entity_region.clone(),
                    entity_type: entity_type.clone(),
                    is_competitor: false,
                    industry_tags: Vec::new(),
                    certifications: Vec::new(),
                    capabilities: Vec::new(),
                    key_persons: poi_names.get(&c.entity_id).cloned().into_iter().collect(),
                    recent_changes: Vec::new(),
                    threat_score: None,
                    overlap_score: None,
                    strategic_relevance: None,
                    revenue_estimate_usd: None,
                    employee_estimate: None,
                    competitor_names: Vec::new(),
                    sites_summary: Vec::new(),
                    competitor_events: Vec::new(),
                    domain: None,
                });

            tracing::info!(
                entity = %entity_label,
                category = %c.category,
                evidence_count = evidence_signals.len(),
                certs = entity_ctx.certifications.len(),
                caps = entity_ctx.capabilities.len(),
                "recipe_fire: generating LLM insight"
            );

            match generate_llm_insight(
                &insight_llm_client,
                &entity_ctx,
                &c.category,
                &evidence_signals,
            )
            .await
            {
                Ok((headline, narrative, recommendation, llm_confidence)) => {
                    let source_urls = ranked_source_urls(&evidence_signals, 6);
                    let sources_footer = format_sources_footer(&evidence_signals, 6);
                    let summary = if sources_footer.is_empty() {
                        format!("{}\n\n{}", narrative, recommendation)
                    } else {
                        format!("{}\n\n{}\n\n{}", narrative, recommendation, sources_footer)
                    };
                    Some((
                        headline,
                        summary,
                        recommendation,
                        llm_confidence,
                        source_urls,
                        evidence_signals,
                    ))
                }
                Err(e) => {
                    tracing::warn!(
                        entity = %entity_label,
                        category = %c.category,
                        error = %e,
                        "recipe_fire: LLM insight generation failed; skipping insight (no template fallback)"
                    );
                    None
                }
            }
        };

        #[cfg(feature = "llm")]
        let (title, summary, warning_action, stored_confidence, llm_source_urls, evidence_quality) =
            match llm_output {
                Some((
                    title,
                    summary,
                    warning_action,
                    llm_confidence,
                    llm_source_urls,
                    evidence_signals,
                )) => {
                    let evidence_quality = evidence_quality_from_signals(&evidence_signals);
                    let stored_confidence =
                        (0.75 * c.confidence + 0.15 * llm_confidence + 0.10 * evidence_quality)
                            .clamp(0.0, 1.0);
                    (
                        title,
                        summary,
                        warning_action,
                        stored_confidence,
                        llm_source_urls,
                        evidence_quality,
                    )
                }
                None => continue,
            };

        #[cfg(feature = "llm")]
        let maybe_evidence_urls = (!llm_source_urls.is_empty()).then_some(llm_source_urls);

        #[cfg(not(feature = "llm"))]
        let stored_confidence = {
            let evidence_urls = entity_evidence_urls
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_default();
            let evidence_quality = evidence_quality_from_urls(&evidence_urls);
            (0.85 * c.confidence + 0.15 * evidence_quality).clamp(0.0, 1.0)
        };
        #[cfg(not(feature = "llm"))]
        let (title, summary) = {
            if !should_emit_fallback_insight(
                &signal_details,
                &rendered_action,
                c.confidence,
                c.evidence_ids.len(),
            ) {
                tracing::warn!(
                    recipe = %c.recipe_code,
                    category = %c.category,
                    entity = %entity_label,
                    confidence = c.confidence,
                    evidence_count = c.evidence_ids.len(),
                    signal_details = ?signal_details,
                    "recipe_fire: skipping low-signal fallback insight"
                );
                continue;
            }

            let title = build_analytical_title(
                &entity_label,
                &entity_region,
                &c.category,
                &signal_details,
                entity_type.as_deref(),
            );
            let evidence_urls = entity_evidence_urls
                .get(&c.entity_id)
                .cloned()
                .unwrap_or_default();
            let summary = build_fallback_summary(
                &analytical_narrative,
                &rendered_action,
                &signal_details,
                &evidence_urls,
                &entity_label,
                &entity_region,
                entity_type.as_deref(),
                &c.category,
                &c.severity,
                c.confidence,
                c.evidence_ids.len(),
            );
            (title, summary)
        };

        #[cfg(not(feature = "llm"))]
        let maybe_evidence_urls = entity_evidence_urls
            .get(&c.entity_id)
            .cloned()
            .filter(|urls| !urls.is_empty());

        #[cfg(feature = "llm")]
        tags.push(format!(
            "evidence:{}",
            if evidence_quality >= 0.75 {
                "high"
            } else if evidence_quality >= 0.5 {
                "moderate"
            } else {
                "emerging"
            }
        ));

        #[cfg(not(feature = "llm"))]
        if let Some(urls) = maybe_evidence_urls.as_ref() {
            let evidence_quality = evidence_quality_from_urls(urls);
            tags.push(format!(
                "evidence:{}",
                if evidence_quality >= 0.75 {
                    "high"
                } else if evidence_quality >= 0.5 {
                    "moderate"
                } else {
                    "emerging"
                }
            ));
        }

        if !passes_shared_insight_quality_gate(&title, &summary, Some(c.category.as_str())) {
            tracing::warn!(
                recipe = %c.recipe_code,
                category = %c.category,
                title = %title,
                "recipe_fire: shared insight quality gate rejected summary"
            );
            continue;
        }

        match store
            .insert_insight(
                &title,
                &summary,
                Some(c.category.as_str()),
                None,
                Some(stored_confidence),
                maybe_evidence_urls,
                entity_ids.clone(),
                Some(tags),
            )
            .await
        {
            Ok(_) => insights_inserted += 1,
            Err(e) => {
                tracing::warn!(recipe = %c.recipe_code, "recipe_fire: insert_insight failed: {e}")
            }
        }

        #[cfg(feature = "llm")]
        if (c.severity == "warning" || c.severity == "critical") && stored_confidence >= 0.75 {
            let warn_title = format!("[{}] {}", c.recipe_code, title);
            let _ = store
                .insert_warning(
                    &c.category,
                    &warn_title,
                    Some(&warning_action),
                    &c.severity,
                    None,
                    Some(&c.recipe_code),
                    entity_ids.clone(),
                    None,
                    Some(stored_confidence),
                )
                .await;
            warnings_inserted += 1;
        }

        #[cfg(not(feature = "llm"))]
        if (c.severity == "warning" || c.severity == "critical") && c.confidence >= 0.75 {
            let warn_title = format!("[{}] {}", c.recipe_code, title);
            let _ = store
                .insert_warning(
                    &c.category,
                    &warn_title,
                    Some(&rendered_action),
                    &c.severity,
                    None,
                    Some(&c.recipe_code),
                    entity_ids.clone(),
                    None,
                    Some(c.confidence),
                )
                .await;
            warnings_inserted += 1;
        }
    }

    tracing::info!(
        total_candidates = total_candidates,
        skipped_low_conf = skipped_low_conf,
        skipped_dedup = skipped_dedup,
        skipped_cross_run = skipped_cross_run,
        inserted = insights_inserted,
        warnings = warnings_inserted,
        "recipe_fire: run complete"
    );

    run.succeed(
        insights_inserted,
        &format!(
            "recipe_fire: {} candidate(s), inserted {} insight(s), {} warning(s) (skipped {} low-conf, {} dedup, {} cross-run)",
            total_candidates,
            insights_inserted,
            warnings_inserted,
            skipped_low_conf,
            skipped_dedup,
            skipped_cross_run,
        ),
    );
    run
}
