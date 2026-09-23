//! # Threat Intelligence Module Tests
//!
//! Comprehensive tests for the Threat Intelligence Module (Phase 3.2).

#![allow(clippy::disallowed_methods)]

use apex_threat_intel::{
    error::ThreatIntelError,
    models::{ConfidenceLevel, GeoRegion, IndustrySector, PaginationParams, RiskScore, SeverityLevel},
    supply_chain_threats::{
        Component, ComponentRiskLevel, DisruptionScenario, ScenarioType,
        Severity, Supplier, SupplierCapacity, SupplierRiskScore, SupplierTier, SupplyChainThreatModel,
    },
    threat_actor_database::{
        ActorMotivation, ActorStatus, AttackPattern, Campaign, ThreatActor, ThreatActorDatabase,
    },
    attack_surface::{
        AttackSurfaceAnalyzer, ExposureType, Misconfiguration, MisconfigurationType,
        ShadowITAsset, ShadowITType, Vulnerability,
    },
    competitive_intelligence::{
        Competitor, CompetitiveIntelligenceEngine, FinancialHealth, ProfitabilityStatus,
        StrategicMove, StrategicMoveType, StrategicPrediction, PredictionType,
    },
    mitre_attck::{AttackTactic, AttackTechnique, AttckMatrix},
};

mod threat_actor_tests {
    use super::*;

    #[test]
    fn test_threat_actor_creation() {
        let actor = ThreatActor::new("APT-TEST", ActorMotivation::Espionage, ActorStatus::Active);
        assert_eq!(actor.alias, "APT-TEST");
        assert_eq!(actor.motivation, ActorMotivation::Espionage);
        assert_eq!(actor.status, ActorStatus::Active);
    }

    #[test]
    fn test_actor_builder_pattern() {
        let actor = ThreatActor::new("APT-BUILDER", ActorMotivation::Financial, ActorStatus::Active)
            .with_name("Builder Test Actor")
            .with_aliases(vec!["BT1".to_string(), "BT2".to_string()])
            .with_attributed_country("US")
            .with_target_sectors(vec![IndustrySector::Technology, IndustrySector::Healthcare])
            .with_sophistication(8);

        assert_eq!(actor.name, Some("Builder Test Actor".to_string()));
        assert_eq!(actor.aliases, vec!["BT1", "BT2"]);
        assert_eq!(actor.attributed_country, Some("US".to_string()));
        assert_eq!(actor.target_sectors.len(), 2);
        assert_eq!(actor.sophistication_level, 8);
    }

    #[test]
    fn test_actor_sector_targeting() {
        let actor = ThreatActor::new("SECTOR-TEST", ActorMotivation::Financial, ActorStatus::Active)
            .with_target_sectors(vec![IndustrySector::Technology]);

        assert!(actor.targets_sector(&IndustrySector::Technology));
        assert!(!actor.targets_sector(&IndustrySector::Automotive));
    }

    #[test]
    fn test_actor_has_technique() {
        let actor = ThreatActor::new("TECH-TEST", ActorMotivation::Financial, ActorStatus::Active);
        
        assert!(!actor.has_technique("T1195"));
    }

    #[test]
    fn test_actor_risk_score() {
        let actor = ThreatActor::new("RISK-TEST", ActorMotivation::Espionage, ActorStatus::Active)
            .with_target_sectors(vec![
                IndustrySector::Technology,
                IndustrySector::Defense,
                IndustrySector::Aerospace,
            ])
            .with_sophistication(9);

        let score = actor.calculate_risk_score();
        assert!(score.score > 0.0);
        assert!(!score.factors.is_empty());
    }

    #[test]
    fn test_database_with_known_actors() {
        let db = ThreatActorDatabase::with_known_actors();
        assert!(!db.list_actors(PaginationParams::default_page()).items.is_empty());
    }

    #[test]
    fn test_database_add_actor() {
        let mut db = ThreatActorDatabase::new();
        let id = db.add_actor(ThreatActor::new("NEW-ACTOR", ActorMotivation::Financial, ActorStatus::Active)).unwrap();
        
        assert!(db.get_actor(id).is_some());
        assert!(db.get_actor_by_alias("NEW-ACTOR").is_some());
    }

    #[test]
    fn test_database_duplicate_alias() {
        let mut db = ThreatActorDatabase::new();
        db.add_actor(ThreatActor::new("DUP-TEST", ActorMotivation::Financial, ActorStatus::Active)).unwrap();
        
        let result = db.add_actor(ThreatActor::new("DUP-TEST", ActorMotivation::Espionage, ActorStatus::Active));
        assert!(result.is_err());
    }

    #[test]
    fn test_database_search_actors() {
        let mut db = ThreatActorDatabase::new();
        let _ = db.add_actor(ThreatActor::new("APT-ALPHA", ActorMotivation::Espionage, ActorStatus::Active));
        let _ = db.add_actor(ThreatActor::new("FIN-BETA", ActorMotivation::Financial, ActorStatus::Active));

        let results = db.search_actors("APT");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].alias, "APT-ALPHA");
    }

    #[test]
    fn test_database_sector_filtering() {
        let mut db = ThreatActorDatabase::new();
        let _ = db.add_actor(
            ThreatActor::new("TECH-ACTOR", ActorMotivation::Financial, ActorStatus::Active)
                .with_target_sectors(vec![IndustrySector::Technology])
        );
        let _ = db.add_actor(
            ThreatActor::new("AUTO-ACTOR", ActorMotivation::Financial, ActorStatus::Active)
                .with_target_sectors(vec![IndustrySector::Automotive])
        );

        let tech_actors = db.get_actors_by_sector(&IndustrySector::Technology);
        assert_eq!(tech_actors.len(), 1);
    }

    #[test]
    fn test_database_status_filtering() {
        let mut db = ThreatActorDatabase::new();
        let _ = db.add_actor(ThreatActor::new("ACTIVE-1", ActorMotivation::Financial, ActorStatus::Active));
        let _ = db.add_actor(ThreatActor::new("DORMANT-1", ActorMotivation::Financial, ActorStatus::Dormant));

        let active = db.get_actors_by_status(ActorStatus::Active);
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn test_attack_pattern() {
        let pattern = AttackPattern::new("PAT-TEST", "Test Pattern")
            .with_description("A test attack pattern")
            .with_mitre_id("T1195")
            .with_mitigation(vec!["Mitigation 1".to_string()])
            .with_sectors(vec![IndustrySector::Technology]);

        assert_eq!(pattern.id, "PAT-TEST");
        assert_eq!(pattern.mitre_id, Some("T1195".to_string()));
        assert_eq!(pattern.applicable_sectors.len(), 1);
    }

    #[test]
    fn test_campaign() {
        let campaign = Campaign::new("Test Campaign", chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap());
        assert!(campaign.duration_days().is_none()); // No end date

        let campaign = campaign.with_end_date(chrono::NaiveDate::from_ymd_opt(2023, 6, 1).unwrap());
        assert_eq!(campaign.duration_days(), Some(151));
    }

    #[test]
    fn test_actor_reference() {
        use apex_threat_intel::threat_actor_database::ActorReference;
        let reference = ActorReference {
            source: "Test Source".to_string(),
            url: Some("https://example.com".to_string()),
            description: Some("Test description".to_string()),
            published_at: None,
        };

        assert_eq!(reference.source, "Test Source");
    }
}

mod attack_surface_tests {
    use super::*;
    #[test]
    fn test_exposure_creation() {
        let exposure = apex_threat_intel::attack_surface::ExternalExposure::new(
            uuid::Uuid::new_v4(),
            "api.example.com",
            ExposureType::PublicApi,
        )
        .with_severity(SeverityLevel::High)
        .with_port(443)
        .with_description("Exposed API endpoint");

        assert_eq!(exposure.asset_identifier, "api.example.com");
        assert_eq!(exposure.exposure_type, ExposureType::PublicApi);
        assert_eq!(exposure.severity, SeverityLevel::High);
    }

    #[test]
    fn test_exposure_risk_score() {
        let exposure = apex_threat_intel::attack_surface::ExternalExposure::new(
            uuid::Uuid::new_v4(),
            "test.com",
            ExposureType::Database,
        )
        .with_severity(SeverityLevel::Critical);

        let score = exposure.calculate_risk_score();
        assert!(score > 0.6); // High due to critical + database exposure
    }

    #[test]
    fn test_vulnerability_cvss_severity() {
        let vuln = Vulnerability::new("Critical CVE")
            .with_cvss(9.5);

        assert_eq!(vuln.severity, SeverityLevel::Critical);
        assert_eq!(vuln.cvss_score, Some(9.5));
    }

    #[test]
    fn test_vulnerability_severity_from_cvss() {
        assert_eq!(SeverityLevel::from_cvss(9.5), SeverityLevel::Critical);
        assert_eq!(SeverityLevel::from_cvss(7.5), SeverityLevel::High);
        assert_eq!(SeverityLevel::from_cvss(5.0), SeverityLevel::Medium);
        assert_eq!(SeverityLevel::from_cvss(2.5), SeverityLevel::Low);
        assert_eq!(SeverityLevel::from_cvss(0.0), SeverityLevel::Info);
    }

    #[test]
    fn test_misconfiguration() {
        let misconfig = Misconfiguration::new(
            MisconfigurationType::WeakAuthentication,
            "Missing MFA on admin portal",
        );

        assert_eq!(misconfig.misconfiguration_type, MisconfigurationType::WeakAuthentication);
        assert_eq!(misconfig.severity, SeverityLevel::Critical); // Default for this type
    }

    #[test]
    fn test_shadow_it_asset() {
        let asset = ShadowITAsset::new(ShadowITType::CloudStorage, "Dropbox")
            .with_sensitivity(apex_threat_intel::attack_surface::DataSensitivity::Confidential);

        assert_eq!(asset.asset_type, ShadowITType::CloudStorage);
        assert_eq!(asset.data_sensitivity, apex_threat_intel::attack_surface::DataSensitivity::Confidential);
    }

    #[test]
    fn test_shadow_it_risk_score() {
        let asset = ShadowITAsset::new(ShadowITType::PersonalDevice, "Unknown Device");

        let score = asset.calculate_risk_score();
        assert!(score > 0.0);
    }

    #[test]
    fn test_attack_surface_analyzer() {
        let mut analyzer = AttackSurfaceAnalyzer::new();
        let org_id = uuid::Uuid::new_v4();
        let assessment_id = analyzer.create_assessment(org_id);

        let vuln = Vulnerability::new("Test Vulnerability")
            .with_cvss(8.5);
        analyzer.add_vulnerability(assessment_id, vuln).unwrap();

        let assessment = analyzer.get_assessment(assessment_id).unwrap();
        assert!(!assessment.vulnerabilities.is_empty());
    }

    #[test]
    fn test_critical_findings() {
        let mut analyzer = AttackSurfaceAnalyzer::new();
        let org_id = uuid::Uuid::new_v4();
        let assessment_id = analyzer.create_assessment(org_id);

        let vuln = Vulnerability::new("Critical CVE")
            .with_cvss(9.8);
        analyzer.add_vulnerability(assessment_id, vuln).unwrap();

        let critical = analyzer.get_all_critical_findings();
        assert!(!critical.is_empty());
    }

    #[test]
    fn test_remediation_priority() {
        let mut analyzer = AttackSurfaceAnalyzer::new();
        let org_id = uuid::Uuid::new_v4();
        let assessment_id = analyzer.create_assessment(org_id);

        let vuln1 = Vulnerability::new("Critical").with_cvss(9.8);
        let vuln2 = Vulnerability::new("Medium").with_cvss(5.5);
        analyzer.add_vulnerability(assessment_id, vuln1).unwrap();
        analyzer.add_vulnerability(assessment_id, vuln2).unwrap();

        let priorities = analyzer.calculate_remediation_priority(assessment_id);
        assert!(!priorities.is_empty());
        assert!(priorities[0].priority_score >= priorities[1].priority_score);
    }
}

mod supply_chain_tests {
    use super::*;

    #[test]
    fn test_supplier_creation() {
        let supplier = Supplier::new("Test Supplier", "US", SupplierTier::Tier1, SupplierCapacity::default())
            .with_category("Electronics")
            .with_criticality(0.8);

        assert_eq!(supplier.name, "Test Supplier");
        assert_eq!(supplier.country_code, "US");
        assert_eq!(supplier.tier, SupplierTier::Tier1);
    }

    #[test]
    fn test_component_creation() {
        let component = Component::new("PART-001", "Critical Component")
            .with_risk_level(ComponentRiskLevel::Critical);

        assert_eq!(component.part_number, "PART-001");
        assert_eq!(component.risk_level, ComponentRiskLevel::Critical);
    }

    #[test]
    fn test_scenario_risk_exposure() {
        let scenario = DisruptionScenario::new("Test Scenario", ScenarioType::NaturalDisaster)
            .with_probability(0.5)
            .with_impact(Severity::Critical);

        let exposure = scenario.risk_exposure();
        assert!((exposure - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_scenario_builder() {
        let scenario = DisruptionScenario::new("Earthquake", ScenarioType::NaturalDisaster)
            .with_probability(0.2)
            .with_impact(Severity::High)
            .with_regions(vec![GeoRegion::EastAsia])
            .with_duration(90)
            .with_financial_impact(100_000_000, 500_000_000);

        assert_eq!(scenario.scenario_name, "Earthquake");
        assert_eq!(scenario.probability, 0.2);
        assert_eq!(scenario.affected_regions, vec![GeoRegion::EastAsia]);
    }

    #[test]
    fn test_geo_concentration_risk() {
        let mut model = SupplyChainThreatModel::new();

        model.add_supplier(Supplier::new("US Supplier", "US", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::NorthAmerica));
        model.add_supplier(Supplier::new("CN Supplier 1", "CN", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::EastAsia));
        model.add_supplier(Supplier::new("CN Supplier 2", "CN", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::EastAsia));

        let risks = model.calculate_geo_concentration();
        assert!(!risks.is_empty());

        // East Asia should have higher concentration
        let ea_risk = risks.iter().find(|r| r.region == GeoRegion::EastAsia);
        assert!(ea_risk.is_some());
    }

    #[test]
    fn test_single_manufacturer_identification() {
        let mut model = SupplyChainThreatModel::new();
        
        model.add_supplier(Supplier::new("Primary Mfr", "CN", SupplierTier::Tier2, SupplierCapacity::default()));
        model.add_component(Component::new("PART-001", "Single Source Part"));

        let risks = model.identify_single_manufacturer();
        assert_eq!(risks.len(), 1);
    }

    #[test]
    fn test_resilience_calculation() {
        let mut model = SupplyChainThreatModel::new();

        model.add_supplier(Supplier::new("US Supplier", "US", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::NorthAmerica));
        model.add_supplier(Supplier::new("EU Supplier", "DE", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::Europe));
        model.add_supplier(Supplier::new("APAC Supplier", "JP", SupplierTier::Tier1, SupplierCapacity::default())
            .with_region(GeoRegion::AsiaPacific));

        let resilience = model.calculate_resilience();
        assert!(resilience.overall_score >= 0.0);
        assert!(resilience.geographic_diversity_score > 0.0);
    }

    #[test]
    fn test_scenario_generation() {
        let mut model = SupplyChainThreatModel::new();
        model.generate_standard_scenarios();

        assert!(!model.scenarios().is_empty());

        let natural_disasters = model.get_scenarios_by_type(ScenarioType::NaturalDisaster);
        assert!(!natural_disasters.is_empty());
    }

    #[test]
    fn test_tier_filtering() {
        let mut model = SupplyChainThreatModel::new();

        model.add_supplier(Supplier::new("T1 Supplier", "US", SupplierTier::Tier1, SupplierCapacity::default()));
        model.add_supplier(Supplier::new("T2 Supplier", "CN", SupplierTier::Tier2, SupplierCapacity::default()));

        let tier1 = model.get_suppliers_by_tier(SupplierTier::Tier1);
        assert_eq!(tier1.len(), 1);
    }

    #[test]
    fn test_supplier_risk_score_calculation() {
        let mut risk = SupplierRiskScore::new(uuid::Uuid::new_v4());
        risk.financial_risk = 0.6;
        risk.operational_risk = 0.7;
        risk.geopolitical_risk = 0.5;
        risk.cyber_risk = 0.3;
        risk.concentration_risk = 0.4;
        risk.dependency_score = 0.6;

        risk.calculate_overall();

        let expected = 0.6 * 0.20 + 0.7 * 0.20 + 0.5 * 0.25 + 0.3 * 0.15 + 0.4 * 0.10 + 0.6 * 0.10;
        assert!((risk.overall_score - expected).abs() < 0.01);
    }
}

mod competitive_intelligence_tests {
    use super::*;

    #[test]
    fn test_competitor_creation() {
        let competitor = Competitor::new("Test Corp", IndustrySector::Technology)
            .with_market_share(15.5)
            .with_capabilities(vec!["Manufacturing".to_string(), "R&D".to_string()]);

        assert_eq!(competitor.name, "Test Corp");
        assert_eq!(competitor.industry, IndustrySector::Technology);
        assert_eq!(competitor.market_position.market_share_percent, 15.5);
    }

    #[test]
    fn test_strategic_move() {
        let move_ = StrategicMove::new(StrategicMoveType::Acquisition, "Acquisition of TechCo")
            .with_confidence(ConfidenceLevel::High)
            .with_impact(0.8);

        assert_eq!(move_.move_type, StrategicMoveType::Acquisition);
        assert_eq!(move_.confidence, ConfidenceLevel::High);
        assert!((move_.impact_assessment.competitive_impact - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_strategic_prediction() {
        let prediction = StrategicPrediction::new(PredictionType::MarketEntry, "Competitor X enters market")
            .with_probability(0.75);

        assert_eq!(prediction.prediction_type, PredictionType::MarketEntry);
        assert!((prediction.probability - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_financial_health_score() {
        let health = FinancialHealth {
            credit_rating: Some("AAA".to_string()),
            revenue_growth_yoy: 0.25,
            profit_margin: 0.15,
            debt_to_equity: None,
            cash_reserves: Some(100_000_000),
            burn_rate: None,
            funding_rounds: Vec::new(),
            profitability_status: ProfitabilityStatus::Profitable,
        };

        let score = health.overall_score();
        assert!(score > 0.5);
    }

    #[test]
    fn test_engine_add_competitor() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        let id = engine.add_competitor(
            Competitor::new("Competitor A", IndustrySector::Electronics)
                .with_market_share(20.0)
        );

        assert!(engine.get_competitor(id).is_some());
    }

    #[test]
    fn test_top_competitors() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        engine.add_competitor(
            Competitor::new("Small", IndustrySector::Technology).with_market_share(5.0)
        );
        engine.add_competitor(
            Competitor::new("Large", IndustrySector::Technology).with_market_share(30.0)
        );
        engine.add_competitor(
            Competitor::new("Medium", IndustrySector::Technology).with_market_share(15.0)
        );

        let top = engine.get_top_competitors(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "Large");
    }

    #[test]
    fn test_most_threatening_competitors() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        engine.add_competitor(
            Competitor::new("Low Threat", IndustrySector::Technology).with_market_share(10.0)
        );
        engine.add_competitor(
            Competitor::new("High Threat", IndustrySector::Technology).with_market_share(25.0)
        );

        let threatening = engine.get_most_threatening(2);
        assert_eq!(threatening.len(), 2);
    }

    #[test]
    fn test_pricing_intelligence() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        engine.add_pricing_intelligence(apex_threat_intel::competitive_intelligence::PricingIntelligence {
            product_id: None,
            product_name: "Widget A".to_string(),
            competitor_id: None,
            competitor_name: Some("Competitor X".to_string()),
            price: 99.99,
            currency: "USD".to_string(),
            unit: "unit".to_string(),
            effective_date: chrono::Utc::now(),
            price_type: apex_threat_intel::competitive_intelligence::PriceType::List,
            region: None,
            discount_available: Some(10.0),
            volume_tier_pricing: None,
            confidence: ConfidenceLevel::High,
            source: "Web scraping".to_string(),
        });

        let avg = engine.get_average_price("Widget A");
        assert!(avg.is_some());
    }

    #[test]
    fn test_threat_assessment_generation() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        engine.add_competitor(
            Competitor::new("Threat Competitor", IndustrySector::Technology)
                .with_market_share(30.0)
        );

        let assessment = engine.generate_threat_assessment(uuid::Uuid::new_v4());
        
        assert!(!assessment.competitors.is_empty());
    }

    #[test]
    fn test_technology_positioning() {
        let mut engine = CompetitiveIntelligenceEngine::new();
        
        let mut competitor = Competitor::new("Tech Leader", IndustrySector::Technology);
        competitor.technologies.push(apex_threat_intel::competitive_intelligence::TechnologyStack {
            category: apex_threat_intel::competitive_intelligence::TechnologyCategory::AI_ML,
            technologies: vec![
                apex_threat_intel::competitive_intelligence::TechnologyItem {
                    name: "Advanced AI".to_string(),
                    version: None,
                    vendor: None,
                    adoption_status: apex_threat_intel::competitive_intelligence::AdoptionStatus::Core,
                    integration_depth: apex_threat_intel::competitive_intelligence::IntegrationDepth::Critical,
                    strategic_importance: apex_threat_intel::competitive_intelligence::StrategicImportance::Critical,
                }
            ],
            maturity_level: apex_threat_intel::competitive_intelligence::TechnologyMaturity::Growth,
            investment_level: apex_threat_intel::competitive_intelligence::InvestmentLevel::High,
        });

        engine.add_competitor(competitor);

        let analysis = engine.analyze_technology_positioning();
        assert!(!analysis.is_empty());
    }
}

mod mitre_attck_tests {
    use super::*;

    #[test]
    fn test_attck_matrix() {
        let matrix = AttckMatrix::new("v14.0");
        assert_eq!(matrix.version, "v14.0");
        assert!(!matrix.techniques.is_empty());
    }

    #[test]
    fn test_tactic_properties() {
        let tactic = AttackTactic::InitialAccess;
        assert_eq!(tactic.as_str(), "initial-access");
        assert_eq!(tactic.id(), "TA0001");
        assert!(tactic.description().contains("foothold"));
    }

    #[test]
    fn test_find_by_tactic() {
        let matrix = AttckMatrix::new("v14.0");
        let initial_access = matrix.find_by_tactic(AttackTactic::InitialAccess);
        assert!(!initial_access.is_empty());
    }

    #[test]
    fn test_find_by_id() {
        let matrix = AttckMatrix::new("v14.0");
        let supply_chain = matrix.find_by_id("T1195");
        assert!(supply_chain.is_some());
        assert_eq!(supply_chain.unwrap().name, "Supply Chain Compromise");
    }

    #[test]
    fn test_find_by_id_pattern() {
        let matrix = AttckMatrix::new("v14.0");
        let supply_chain_variants = matrix.find_by_id_pattern("T1195");
        assert!(!supply_chain_variants.is_empty());
    }

    #[test]
    fn test_technique_builder() {
        let technique = AttackTechnique::new("T9999", "Test Technique")
            .with_tactics(vec![AttackTactic::InitialAccess])
            .with_description("A test technique")
            .with_detection("Monitor for suspicious activity")
            .with_mitigation("Implement security controls");

        assert_eq!(technique.id, "T9999");
        assert_eq!(technique.tactics, vec![AttackTactic::InitialAccess]);
        assert!(technique.detection.is_some());
        assert!(technique.mitigation.is_some());
    }
}

mod model_tests {
    use super::*;

    #[test]
    fn test_industry_sector_parsing() {
        assert_eq!(IndustrySector::from_str("automotive"), IndustrySector::Automotive);
        assert_eq!(IndustrySector::from_str("Automotive"), IndustrySector::Automotive);
        assert_eq!(IndustrySector::from_str("pharma"), IndustrySector::Pharmaceuticals);
        assert_eq!(IndustrySector::from_str("custom"), IndustrySector::Other("custom".to_string()));
    }

    #[test]
    fn test_confidence_level_from_f64() {
        assert_eq!(ConfidenceLevel::from_f64(0.9), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::from_f64(0.7), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::from_f64(0.3), ConfidenceLevel::Low);
        assert_eq!(ConfidenceLevel::from_f64(0.0), ConfidenceLevel::Unknown);
    }

    #[test]
    fn test_severity_from_cvss() {
        assert_eq!(SeverityLevel::from_cvss(9.5), SeverityLevel::Critical);
        assert_eq!(SeverityLevel::from_cvss(7.5), SeverityLevel::High);
        assert_eq!(SeverityLevel::from_cvss(5.0), SeverityLevel::Medium);
        assert_eq!(SeverityLevel::from_cvss(2.5), SeverityLevel::Low);
        assert_eq!(SeverityLevel::from_cvss(0.0), SeverityLevel::Info);
    }

    #[test]
    fn test_risk_score_bounds() {
        let rs = RiskScore::new(1.5, ConfidenceLevel::High);
        assert_eq!(rs.score, 1.0);

        let rs = RiskScore::new(-0.5, ConfidenceLevel::High);
        assert_eq!(rs.score, 0.0);
    }

    #[test]
    fn test_pagination() {
        let params = PaginationParams::new(10, 50);
        assert_eq!(params.offset, 10);
        assert_eq!(params.limit, 50);

        // Test cap at 1000
        let params = PaginationParams::new(0, 2000);
        assert_eq!(params.limit, 1000);
    }

    #[test]
    fn test_paginated_response() {
        use apex_threat_intel::models::PaginatedResponse;
        
        let items = vec![1, 2, 3, 4, 5];
        let response = PaginatedResponse::new(items, 100, 0, 10);
        
        assert_eq!(response.items.len(), 5);
        assert_eq!(response.total, 100);
        assert!(response.has_more);
    }
}

mod error_tests {
    use super::*;

    #[test]
    fn test_threat_actor_not_found() {
        let e = ThreatIntelError::threat_actor_not_found("APT29");
        assert!(e.to_string().contains("APT29"));
    }

    #[test]
    fn test_error_with_hint() {
        let e = ThreatIntelError::configuration("missing API key")
            .with_hint("set THREAT_INTEL_API_KEY");
        assert!(e.hint().is_some());
    }

    #[test]
    fn test_validation_error() {
        let e = ThreatIntelError::validation("invalid input");
        assert!(e.to_string().contains("invalid input"));
    }

    #[test]
    fn test_supply_chain_risk_error() {
        let e = ThreatIntelError::supply_chain_risk("Supplier not found");
        assert!(e.to_string().contains("Supplier not found"));
    }

    #[test]
    fn test_competitive_intelligence_error() {
        let e = ThreatIntelError::competitive_intelligence("Competitor data unavailable");
        assert!(e.to_string().contains("Competitor data unavailable"));
    }
}
