//! Template Diversity Engine for Insight Titles.
//!
//! Generates semantically diverse, context-aware insight titles using
//! multiple strategies. Replaces the static 24-template round-robin system
//! with dynamic generation that adapts to signal context and entity state.
//!
//! # Strategies
//! - [`EventDriven`](TitleStrategy::EventDriven): Entity-action-context titles
//! - [`RiskExposure`](TitleStrategy::RiskExposure): Entity risk highlighting
//! - [`CompetitiveComparison`](TitleStrategy::CompetitiveComparison): Pairwise entity comparison
//! - [`TrendAnalysis`](TitleStrategy::TrendAnalysis): Sector/industry trend extraction
//! - [`DataDiscovery`](TitleStrategy::DataDiscovery): Data-driven discovery titles
//! - [`ImpactAssessment`](TitleStrategy::ImpactAssessment): Forward-looking impact analysis
//! - [`NarrativeArc`](TitleStrategy::NarrativeArc): Entity evolution/transformation arc
//! - [`SignalSynthesis`](TitleStrategy::SignalSynthesis): Multi-signal synthesis titles

use crate::entity_relevance::{EntityRegistry, SignalContext};
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

// ────────────────────────────────────────────
// Strategy enum
// ────────────────────────────────────────────

/// The strategy used to generate a diverse insight title.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TitleStrategy {
    /// "NVIDIA expands GPU capacity amid AI demand surge"
    EventDriven,
    /// "Foxconn's exposure to Apple supply chain shifts"
    RiskExposure,
    /// "TSMC's 3nm ramp outpaces Samsung's GAA efforts"
    CompetitiveComparison,
    /// "Supply chain concentration risk in semiconductor packaging"
    TrendAnalysis,
    /// "New regulatory filing reveals AMD's EUV procurement plans"
    DataDiscovery,
    /// "What Pegatron's Mexico expansion means for EMS margins"
    ImpactAssessment,
    /// "From notebook assembly to EV inverters: Hon Hai's diversification"
    NarrativeArc,
    /// "Three signals pointing to reshoring in PCB manufacturing"
    SignalSynthesis,
}

impl TitleStrategy {
    /// All available strategies, in a fixed order for deterministic iteration.
    const ALL: [TitleStrategy; 8] = [
        TitleStrategy::EventDriven,
        TitleStrategy::RiskExposure,
        TitleStrategy::CompetitiveComparison,
        TitleStrategy::TrendAnalysis,
        TitleStrategy::DataDiscovery,
        TitleStrategy::ImpactAssessment,
        TitleStrategy::NarrativeArc,
        TitleStrategy::SignalSynthesis,
    ];

    /// A human-readable label for the strategy.
    pub fn label(&self) -> &'static str {
        match self {
            Self::EventDriven => "Event-Driven",
            Self::RiskExposure => "Risk Exposure",
            Self::CompetitiveComparison => "Competitive Comparison",
            Self::TrendAnalysis => "Trend Analysis",
            Self::DataDiscovery => "Data Discovery",
            Self::ImpactAssessment => "Impact Assessment",
            Self::NarrativeArc => "Narrative Arc",
            Self::SignalSynthesis => "Signal Synthesis",
        }
    }
}

// ────────────────────────────────────────────
// Output type
// ────────────────────────────────────────────

/// A generated diverse title with metadata.
pub struct DiverseTitle {
    /// The generated title string.
    pub title: String,
    /// Which strategy produced this title.
    pub strategy: TitleStrategy,
    /// Semantic diversity score (0.0–1.0) relative to recent titles.
    pub diversity_score: f64,
    /// Entity names mentioned or relevant to this title.
    pub entities: Vec<String>,
}

// ────────────────────────────────────────────
// Title Generator
// ────────────────────────────────────────────

/// Generates diverse, context-aware insight titles.
///
/// Uses multiple strategies with weight-based selection and tracks recent
/// titles to avoid repetition. All state is internal — no global/static state.
///
/// # Thread safety
/// `TitleGenerator` is `Send + Sync` when used with `&self` read-only methods.
/// Mutation methods require `&mut self`.
pub struct TitleGenerator {
    /// Recently used titles (to avoid repetition).
    recent_titles: Vec<String>,
    /// Max recent titles to track.
    max_history: usize,
    /// Strategy weights that adjust based on recent usage.
    strategy_weights: HashMap<TitleStrategy, f64>,
    /// Count of how many times each strategy has been used (for LRU-like tracking).
    strategy_usage_count: HashMap<TitleStrategy, u64>,
    /// Total calls made (for recency tracking).
    total_calls: u64,
}

impl TitleGenerator {
    /// Create a new `TitleGenerator` with default configuration.
    pub fn new() -> Self {
        let mut strategy_weights = HashMap::new();
        let mut strategy_usage_count = HashMap::new();
        for strategy in &TitleStrategy::ALL {
            strategy_weights.insert(*strategy, 1.0);
            strategy_usage_count.insert(*strategy, 0);
        }
        Self {
            recent_titles: Vec::new(),
            max_history: 50,
            strategy_weights,
            strategy_usage_count,
            total_calls: 0,
        }
    }

    /// Create a `TitleGenerator` with a custom max history size.
    pub fn with_max_history(max_history: usize) -> Self {
        let mut gen = Self::new();
        gen.max_history = max_history.max(1);
        gen
    }

    /// Returns a reference to the recent titles.
    pub fn recent_titles(&self) -> &[String] {
        &self.recent_titles
    }

    /// Returns the current strategy weights.
    pub fn strategy_weights(&self) -> &HashMap<TitleStrategy, f64> {
        &self.strategy_weights
    }

    /// Generate a diverse, context-aware insight title.
    ///
    /// Selects the least-recently-used strategy weighted by `strategy_weights`
    /// (biased toward unused strategies), then generates a title using that
    /// strategy. Updates `recent_titles` and `strategy_weights` after generation.
    ///
    /// * `context` — the signal context (text, entity hint, category, etc.)
    /// * `registry` — optional entity registry for entity-aware title generation
    pub fn generate(
        &mut self,
        context: &SignalContext,
        registry: Option<&EntityRegistry>,
    ) -> DiverseTitle {
        let strategy = self.select_strategy();
        let entities = self.extract_entities(context, registry);
        let entity_name = context
            .entity_hint
            .clone()
            .unwrap_or_else(|| "Entity".to_string());

        let title =
            self.generate_with_strategy(strategy, context, registry, &entity_name, &entities);

        let diversity_score = semantic_diversity_score(&title, &self.recent_titles);

        self.record_usage(strategy, &title);

        DiverseTitle {
            title,
            strategy,
            diversity_score,
            entities,
        }
    }

    /// Select a strategy using weighted random selection, biased toward
    /// less-frequently used strategies.
    fn select_strategy(&self) -> TitleStrategy {
        let total: f64 = self.strategy_weights.values().sum();
        if total <= 0.0 {
            return TitleStrategy::EventDriven;
        }

        let mut rng = rand::thread_rng();
        let mut threshold = rng.gen::<f64>() * total;

        for strategy in &TitleStrategy::ALL {
            let weight = self.strategy_weights.get(strategy).copied().unwrap_or(0.0);
            threshold -= weight;
            if threshold <= 0.0 {
                return *strategy;
            }
        }

        TitleStrategy::EventDriven
    }

    /// Generate a title using a specific strategy.
    fn generate_with_strategy(
        &self,
        strategy: TitleStrategy,
        context: &SignalContext,
        registry: Option<&EntityRegistry>,
        entity_name: &str,
        entities: &[String],
    ) -> String {
        match strategy {
            TitleStrategy::EventDriven => {
                self.generate_event_driven(context, entity_name, entities)
            }
            TitleStrategy::RiskExposure => {
                self.generate_risk_exposure(context, entity_name, entities)
            }
            TitleStrategy::CompetitiveComparison => {
                self.generate_competitive_comparison(context, entity_name, entities, registry)
            }
            TitleStrategy::TrendAnalysis => self.generate_trend_analysis(context, entity_name),
            TitleStrategy::DataDiscovery => self.generate_data_discovery(context, entity_name),
            TitleStrategy::ImpactAssessment => self.generate_impact_assessment(context, entity_name),
            TitleStrategy::NarrativeArc => {
                self.generate_narrative_arc(context, entity_name, registry)
            }
            TitleStrategy::SignalSynthesis => self.generate_signal_synthesis(context, entity_name),
        }
    }

    /// Extract entity names from context and registry.
    fn extract_entities(
        &self,
        context: &SignalContext,
        registry: Option<&EntityRegistry>,
    ) -> Vec<String> {
        let mut entities = Vec::new();

        // Add the entity hint if present
        if let Some(ref hint) = context.entity_hint {
            entities.push(hint.clone());
        }

        // Try to find additional entities from the registry
        if let Some(reg) = registry {
            // Look for entity names mentioned in the signal text
            let text_lower = context.text.to_lowercase();
            for name in reg.entity_names() {
                if text_lower.contains(&name.to_lowercase())
                    && !entities.iter().any(|e| e.eq_ignore_ascii_case(name))
                {
                    entities.push(name.clone());
                }
            }
        }

        if entities.is_empty() {
            entities.push(entity_fallback(context));
        }

        entities
    }

    // ── Individual strategy generators ──

    /// EventDriven: "[Entity] [verb] [noun] amid [context]"
    fn generate_event_driven(
        &self,
        context: &SignalContext,
        entity_name: &str,
        entities: &[String],
    ) -> String {
        let entity = entities.first().map(|s| s.as_str()).unwrap_or(entity_name);
        let verb = pick_random(VERBS);
        let noun = pick_random(NOUNS);
        let context_phrase = truncate_context(&context.text, 6);
        format!("{} {} {} amid {}", entity, verb, noun, context_phrase)
    }

    /// RiskExposure: "[Entity]'s exposure to [topic]"
    fn generate_risk_exposure(
        &self,
        context: &SignalContext,
        entity_name: &str,
        entities: &[String],
    ) -> String {
        let entity = entities.first().map(|s| s.as_str()).unwrap_or(entity_name);
        let risk_topic = match context.category.as_deref() {
            Some("supply_chain") => pick_random(SUPPLY_CHAIN_RISKS),
            Some("regulatory") => pick_random(REGULATORY_RISKS),
            Some("security") => pick_random(SECURITY_RISKS),
            Some("trade") => pick_random(TRADE_RISKS),
            _ => pick_random(GENERIC_RISKS),
        };
        format!("{}'s exposure to {}", entity, risk_topic)
    }

    /// CompetitiveComparison: "[Entity A]'s [metric] [comparison] [Entity B]'s [metric]"
    fn generate_competitive_comparison(
        &self,
        _context: &SignalContext,
        entity_name: &str,
        entities: &[String],
        registry: Option<&EntityRegistry>,
    ) -> String {
        let entity_a = entities.first().map(|s| s.as_str()).unwrap_or(entity_name);

        // Try to find a second entity for comparison
        let entity_b = if entities.len() > 1 {
            entities[1].clone()
        } else if let Some(reg) = registry {
            // Pick a different entity from the same category
            let names: Vec<String> = reg
                .entity_names()
                .filter(|name| !name.eq_ignore_ascii_case(entity_a))
                .take(1)
                .cloned()
                .collect();
            names.first().cloned().unwrap_or_else(|| "Competitor".to_string())
        } else {
            "Competitor".to_string()
        };

        let metric = pick_random(METRICS);
        let comparison = pick_random(COMPARISONS);
        let metric2 = pick_random(METRICS);

        format!(
            "{}'s {} {} {}'s {}",
            entity_a, metric, comparison, entity_b, metric2
        )
    }

    /// TrendAnalysis: "[Trend] in [sector/industry]"
    fn generate_trend_analysis(&self, context: &SignalContext, entity_name: &str) -> String {
        let trend = match context.category.as_deref() {
            Some("demand") => pick_random(DEMAND_TRENDS),
            Some("supply_chain") => pick_random(SUPPLY_CHAIN_TRENDS),
            Some("commodity") => pick_random(COMMODITY_TRENDS),
            Some("security") => pick_random(SECURITY_TRENDS),
            Some("regulatory") => pick_random(REGULATORY_TRENDS),
            _ => pick_random(GENERIC_TRENDS),
        };
        let sector = infer_sector(context, entity_name);
        format!("{} in {}", trend, sector)
    }

    /// DataDiscovery: "New [data_type] reveals [entity]'s [insight]"
    fn generate_data_discovery(&self, _context: &SignalContext, entity_name: &str) -> String {
        let data_type = pick_random(DATA_TYPES);
        let insight = pick_random(DISCOVERY_INSIGHTS);
        format!(
            "New {} reveals {}'s {}",
            data_type, entity_name, insight
        )
    }

    /// ImpactAssessment: "What [event] means for [entity/stakeholder]"
    fn generate_impact_assessment(&self, _context: &SignalContext, entity_name: &str) -> String {
        let event = pick_random(IMPACT_EVENTS);
        format!("What {} means for {}", event, entity_name)
    }

    /// NarrativeArc: "From [past_focus] to [current_focus]: [entity]'s [transformation]"
    fn generate_narrative_arc(
        &self,
        _context: &SignalContext,
        entity_name: &str,
        _registry: Option<&EntityRegistry>,
    ) -> String {
        let past = pick_random(PAST_FOCUS_AREAS);
        let current = pick_random(CURRENT_FOCUS_AREAS);
        let transformation = pick_random(TRANSFORMATION_TYPES);
        format!(
            "From {} to {}: {}'s {}",
            past, current, entity_name, transformation
        )
    }

    /// SignalSynthesis: "[N] signals pointing to [conclusion]"
    fn generate_signal_synthesis(&self, _context: &SignalContext, _entity_name: &str) -> String {
        let n = pick_random(&[2, 3, 4, 5]);
        let conclusion = pick_random(CONCLUSIONS);
        // Capitalize the first letter of the conclusion for a proper title
        let conclusion_capped = {
            let mut chars = conclusion.chars();
            match chars.next() {
                None => conclusion.to_string(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        };
        format!("{} signals pointing to {}", n, conclusion_capped)
    }

    /// Record a strategy usage and update weights.
    fn record_usage(&mut self, strategy: TitleStrategy, title: &str) {
        self.total_calls += 1;

        // Track recent titles (bounded)
        self.recent_titles.push(title.to_string());
        if self.recent_titles.len() > self.max_history {
            self.recent_titles.remove(0);
        }

        // Update usage count
        *self.strategy_usage_count.entry(strategy).or_insert(0) += 1;

        // Reduce weight of used strategy by 20%, redistribute to others
        self.update_weights(strategy);
    }

    /// Reduce the weight of the used strategy by 20% and redistribute to others.
    fn update_weights(&mut self, used: TitleStrategy) {
        let current_weight = self.strategy_weights.get(&used).copied().unwrap_or(1.0);
        let reduction = current_weight * 0.20;

        // Reduce the used strategy's weight
        if let Some(weight) = self.strategy_weights.get_mut(&used) {
            *weight = (*weight - reduction).max(0.01);
        }

        // Redistribute to other strategies
        let other_count = (TitleStrategy::ALL.len() - 1) as f64;
        if other_count > 0.0 {
            let redistribution = reduction / other_count;
            for strategy in &TitleStrategy::ALL {
                if *strategy != used {
                    if let Some(weight) = self.strategy_weights.get_mut(strategy) {
                        *weight += redistribution;
                    }
                }
            }
        }
    }
}

impl Default for TitleGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: TitleGenerator contains only Send + Sync types (Vec, HashMap, u64)
unsafe impl Send for TitleGenerator {}
unsafe impl Sync for TitleGenerator {}

// ────────────────────────────────────────────
// Semantic diversity scoring
// ────────────────────────────────────────────

/// Compute semantic diversity score for a title relative to recent titles.
///
/// Uses word-overlap Jaccard similarity. Returns `1 - max_similarity` with any
/// recent title. Returns `0.0` if identical to a recent title, `0.8+` if less
/// than 30% word overlap.
pub fn semantic_diversity_score(new_title: &str, recent: &[String]) -> f64 {
    if recent.is_empty() {
        return 1.0;
    }

    let new_tokens = tokenize_words(new_title);
    if new_tokens.is_empty() {
        return 0.0;
    }

    let mut max_similarity = 0.0_f64;
    for recent_title in recent {
        let recent_tokens = tokenize_words(recent_title);
        let similarity = jaccard_similarity(&new_tokens, &recent_tokens);
        if similarity > max_similarity {
            max_similarity = similarity;
        }
        // Early exit: if we find an identical title, similarity is 1.0
        if (similarity - 1.0).abs() < f64::EPSILON {
            return 0.0;
        }
    }

    (1.0 - max_similarity).clamp(0.0, 1.0)
}

/// Compute Jaccard similarity between two sets of tokens.
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let intersection_size = a.intersection(b).count() as f64;
    let union_size = a.union(b).count() as f64;

    if union_size == 0.0 {
        return 0.0;
    }

    intersection_size / union_size
}

/// Tokenize a string into lowercase words for similarity comparison.
fn tokenize_words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

// ────────────────────────────────────────────
// Entity overexposure penalty
// ────────────────────────────────────────────

/// Compute a diversity boost multiplier based on entity overexposure.
///
/// Returns a multiplier `0.5–2.0`. Higher when the same entity has generated
/// many recent insights (penalises entity overexposure). Uses
/// `time_since_last_insight` to determine recency.
///
/// * `time_since_last_insight` — how long ago the entity had its last insight.
///   Pass `None` if unknown (returns neutral 1.0).
pub fn diversity_boost(time_since_last_insight: Option<Duration>) -> f64 {
    let elapsed = time_since_last_insight.unwrap_or(Duration::from_secs(3600));
    let elapsed_secs = elapsed.as_secs_f64();

    // If the entity had a very recent insight (< 5 minutes), penalize heavily
    if elapsed_secs < 300.0 {
        0.5
    // Recent insight (5-30 minutes), moderate penalty
    } else if elapsed_secs < 1800.0 {
        0.75
    // Moderate recency (30 min - 2 hours), slight penalty
    } else if elapsed_secs < 7200.0 {
        1.0
    // Less recent (2-24 hours), slight boost
    } else if elapsed_secs < 86400.0 {
        1.25
    // Stale entity (> 24 hours), significant boost
    } else {
        1.5
    }
}

// ────────────────────────────────────────────
// Helper functions
// ────────────────────────────────────────────

/// Pick a random element from a slice.
fn pick_random<T>(items: &[T]) -> &T {
    if items.is_empty() {
        panic!("pick_random called on empty slice");
    }
    let mut rng = rand::thread_rng();
    let idx = rng.gen_range(0..items.len());
    &items[idx]
}

/// Truncate context text to a maximum number of words.
fn truncate_context(text: &str, max_words: usize) -> String {
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|w| w.len() > 2) // Skip very short words (a, an, of, etc.)
        .take(max_words)
        .collect();

    if words.is_empty() {
        // Fallback: take first max_words words regardless of length
        let fallback: Vec<&str> = text.split_whitespace().take(max_words).collect();
        if fallback.is_empty() {
            return "market conditions".to_string();
        }
        return fallback.join(" ");
    }

    words.join(" ")
}

/// Infer a sector/industry from context and entity name.
fn infer_sector(context: &SignalContext, entity_name: &str) -> String {
    if let Some(ref cat) = context.category {
        match cat.as_str() {
            "semiconductor" => return "semiconductor manufacturing".to_string(),
            "ems" => return "electronics manufacturing services".to_string(),
            "oem" => return "original equipment manufacturing".to_string(),
            "automotive" => return "automotive supply chain".to_string(),
            "defense" => return "defense contracting".to_string(),
            _ => {}
        }
    }

    // Fall back to entity name with generic sector
    format!("{} sector", entity_name)
}

/// Default entity name when none can be extracted.
fn entity_fallback(context: &SignalContext) -> String {
    context
        .entity_hint
        .clone()
        .unwrap_or_else(|| "Market".to_string())
}

// ────────────────────────────────────────────
// Vocabulary constants
// ────────────────────────────────────────────

const VERBS: &[&str] = &[
    "expands", "reduces", "shifts", "launches", "acquires", "partners", "invests", "divests",
    "restructures", "delays", "accelerates", "scales", "consolidates", "diversifies", "relocates",
];

const NOUNS: &[&str] = &[
    "capacity", "operations", "production", "procurement", "workforce", "supply chain",
    "manufacturing", "R&D", "distribution", "logistics", "footprint", "investment", "partnership",
    "portfolio", "headcount",
];

const SUPPLY_CHAIN_RISKS: &[&str] = &[
    "supply chain concentration risk",
    "single-source dependency",
    "logistics bottleneck exposure",
    "inventory overhang risk",
    "supplier consolidation trends",
    "cross-border logistics disruptions",
    "raw material price volatility",
];

const REGULATORY_RISKS: &[&str] = &[
    "evolving trade compliance requirements",
    "cross-border data transfer restrictions",
    "environmental compliance mandates",
    "export control classification",
    "sanctions regime expansion",
    "tariff policy uncertainty",
];

const SECURITY_RISKS: &[&str] = &[
    "supply chain cyber vulnerabilities",
    "IP theft and trade secret risks",
    "operational technology exposure",
    "third-party security posture",
    "ransomware supply chain attack surface",
];

const TRADE_RISKS: &[&str] = &[
    "US-China tariff escalation",
    "regionalization of supply chains",
    "friendshoring policy shifts",
    "critical mineral export controls",
    "trade barrier proliferation",
];

const GENERIC_RISKS: &[&str] = &[
    "market volatility and demand uncertainty",
    "competitive pressure on margins",
    "geopolitical instability",
    "technology obsolescence risk",
    "workforce and talent shortages",
    "rising operational costs",
];

const METRICS: &[&str] = &[
    "revenue growth", "market share", "gross margin", "R&D spend", "capex intensity",
    "capacity utilization", "inventory turnover", "operating margin", "employee productivity",
    "patent portfolio",
];

const COMPARISONS: &[&str] = &[
    "outpaces", "lags", "matches", "surpasses", "undercuts", "trails", "exceeds", "approaches",
    "narrows gap with", "widens lead over",
];

const DEMAND_TRENDS: &[&str] = &[
    "Rising procurement activity",
    "Supply-demand imbalance",
    "Capacity expansion wave",
    "Order book acceleration",
    "Inventory build cycle",
];

const SUPPLY_CHAIN_TRENDS: &[&str] = &[
    "Supply chain regionalization",
    "Nearshoring acceleration",
    "Supplier diversification push",
    "Logistics cost escalation",
    "Inventory optimization shift",
];

const COMMODITY_TRENDS: &[&str] = &[
    "Raw material price pressure",
    "Commodity supply constraint",
    "Critical material dependency",
    "Price volatility impact",
    "Alternative material exploration",
];

const SECURITY_TRENDS: &[&str] = &[
    "Cyber resilience gap",
    "Supply chain attack surface",
    "Security compliance burden",
    "Third-party risk exposure",
    "Operational technology vulnerability",
];

const REGULATORY_TRENDS: &[&str] = &[
    "Regulatory compliance cost",
    "Trade policy adaptation",
    "Environmental mandate impact",
    "Export control tightening",
    "Data governance evolution",
];

const GENERIC_TRENDS: &[&str] = &[
    "Market structure evolution",
    "Competitive dynamics shift",
    "Technology adoption curve",
    "Operational efficiency drive",
    "Strategic realignment wave",
];

const DATA_TYPES: &[&str] = &[
    "regulatory filing", "patent application", "job posting data", "supplier registration",
    "certification record", "trade data", "earnings transcript", "press release analysis",
    "web change detection", "social sentiment analysis",
];

const DISCOVERY_INSIGHTS: &[&str] = &[
    "strategic expansion plans",
    "technology investment priorities",
    "supply chain restructuring efforts",
    "geographic diversification strategy",
    "talent acquisition focus areas",
    "product development roadmap",
    "partnership and M&A targets",
];

const IMPACT_EVENTS: &[&str] = &[
    "the latest capacity expansion",
    "recent leadership changes",
    "new market entry",
    "supply chain realignment",
    "technology partnership",
    "regulatory filing implications",
    "shifting trade dynamics",
];

const PAST_FOCUS_AREAS: &[&str] = &[
    "traditional manufacturing",
    "cost optimization",
    "legacy products",
    "domestic markets",
    "vertical integration",
    "notebook assembly",
    "contract manufacturing",
    "component sourcing",
];

const CURRENT_FOCUS_AREAS: &[&str] = &[
    "EV components",
    "AI infrastructure",
    "advanced packaging",
    "green manufacturing",
    "digital transformation",
    "regional hubs",
    "value-added services",
    "vertical integration",
];

const TRANSFORMATION_TYPES: &[&str] = &[
    "strategic pivot",
    "capability evolution",
    "market repositioning",
    "diversification journey",
    "operational transformation",
    "technology modernization",
    "growth strategy",
];

const CONCLUSIONS: &[&str] = &[
    "PCB manufacturing reshoring",
    "semiconductor packaging consolidation",
    "EMS margin compression",
    "supply chain regionalization",
    "defense electronics build-up",
    "automotive electronics boom",
    "green manufacturing shift",
    "AI-driven supply chain optimization",
];

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> SignalContext {
        SignalContext {
            text: "NVIDIA announced expansion of AI GPU manufacturing capacity in Taiwan"
                .to_string(),
            entity_hint: Some("NVIDIA".to_string()),
            category: Some("semiconductor".to_string()),
            source_url: Some("https://example.com/news".to_string()),
            timestamp: 1_700_000_000,
        }
    }

    fn sample_registry() -> EntityRegistry {
        let mut registry = EntityRegistry::empty();
        registry.register(
            crate::entity_relevance::EntityProfile::new("NVIDIA")
                .with_category(crate::entity_relevance::EntityCategory::Semiconductor),
        );
        registry.register(
            crate::entity_relevance::EntityProfile::new("TSMC")
                .with_category(crate::entity_relevance::EntityCategory::Semiconductor),
        );
        registry
    }

    #[test]
    fn test_generate_returns_different_titles_consecutive_calls() {
        let mut gen = TitleGenerator::new();
        let context = sample_context();
        let registry = sample_registry();

        // 10 consecutive calls
        let mut strategies_used = std::collections::HashSet::new();
        for _ in 0..10 {
            let result = gen.generate(&context, Some(&registry));
            strategies_used.insert(result.strategy);
            assert!(!result.title.is_empty(), "Title should not be empty");
            assert!(
                result.diversity_score >= 0.0 && result.diversity_score <= 1.0,
                "Diversity score should be in [0, 1]"
            );
        }

        // Should have used at least 2 different strategies (highly likely with weighted random)
        assert!(
            strategies_used.len() >= 2,
            "Expected at least 2 different strategies in 10 calls, got {}",
            strategies_used.len()
        );
    }

    #[test]
    fn test_semantic_diversity_score_identical_returns_zero() {
        let title = "NVIDIA expands GPU capacity amid AI demand".to_string();
        let recent = vec!["NVIDIA expands GPU capacity amid AI demand".to_string()];
        let score = semantic_diversity_score(&title, &recent);
        assert!(
            (score - 0.0).abs() < 0.01,
            "Identical titles should score ~0.0, got {}",
            score
        );
    }

    #[test]
    fn test_semantic_diversity_score_different_returns_high() {
        let title = "NVIDIA expands GPU capacity amid AI demand".to_string();
        let recent = vec!["TSMC reports quarterly earnings beat estimates".to_string()];
        let score = semantic_diversity_score(&title, &recent);
        assert!(
            score > 0.7,
            "Completely different titles should score > 0.7, got {}",
            score
        );
    }

    #[test]
    fn test_semantic_diversity_score_empty_recent_returns_one() {
        let title = "Any title".to_string();
        let score = semantic_diversity_score(&title, &[]);
        assert!(
            (score - 1.0).abs() < 0.01,
            "Empty recent list should return 1.0, got {}",
            score
        );
    }

    #[test]
    fn test_diversity_boost_penalizes_recent_entity() {
        // Entity with very recent insight → low boost
        let recent_boost = diversity_boost(Some(Duration::from_secs(60))); // 1 minute ago
        // Entity with old insight → higher boost
        let old_boost = diversity_boost(Some(Duration::from_secs(86400 * 2))); // 2 days ago

        assert!(
            recent_boost < old_boost,
            "Recently insighed entity should have lower boost than old one: recent={}, old={}",
            recent_boost,
            old_boost
        );
    }

    #[test]
    fn test_diversity_boost_unknown_entity_returns_neutral() {
        let boost = diversity_boost(None);
        assert!(
            boost >= 0.5 && boost <= 2.0,
            "Diversity boost should be in [0.5, 2.0], got {}",
            boost
        );
    }

    #[test]
    fn test_diversity_boost_range() {
        let values = [
            diversity_boost(Some(Duration::from_secs(0))),
            diversity_boost(Some(Duration::from_secs(600))),
            diversity_boost(Some(Duration::from_secs(3600))),
            diversity_boost(Some(Duration::from_secs(86400))),
            diversity_boost(Some(Duration::from_secs(86400 * 7))),
        ];
        for v in &values {
            assert!(
                *v >= 0.5 && *v <= 2.0,
                "Boost value {} out of range [0.5, 2.0]",
                v
            );
        }
    }

    #[test]
    fn test_strategy_rotation_uses_all_strategies() {
        let mut gen = TitleGenerator::new();
        let context = sample_context();
        let registry = sample_registry();

        // Call generate enough times to use all strategies
        let mut strategies_used = std::collections::HashSet::new();
        for _ in 0..60 {
            let result = gen.generate(&context, Some(&registry));
            strategies_used.insert(result.strategy);
        }

        assert_eq!(
            strategies_used.len(),
            8,
            "Expected all 8 strategies to be used, got {}: {:?}",
            strategies_used.len(),
            strategies_used
        );
    }

    #[test]
    fn test_strategy_weights_redistribute_after_use() {
        let mut gen = TitleGenerator::new();

        // Capture initial weights
        let initial_weight = gen.strategy_weights[&TitleStrategy::EventDriven];

        // Use EventDriven strategy 3 times
        for _ in 0..3 {
            gen.update_weights(TitleStrategy::EventDriven);
        }

        let final_weight = gen.strategy_weights[&TitleStrategy::EventDriven];

        // EventDriven weight should have decreased
        assert!(
            final_weight < initial_weight,
            "Used strategy weight should decrease: initial={}, final={}",
            initial_weight,
            final_weight
        );

        // Other strategies should have increased
        let other_weight = gen.strategy_weights[&TitleStrategy::RiskExposure];
        assert!(
            other_weight > 1.0,
            "Unused strategy weight should increase above 1.0, got {}",
            other_weight
        );
    }

    #[test]
    fn test_strategies_all_have_unique_labels() {
        let mut labels = std::collections::HashSet::new();
        for strategy in &TitleStrategy::ALL {
            assert!(
                labels.insert(strategy.label()),
                "Duplicate label: {}",
                strategy.label()
            );
        }
        assert_eq!(labels.len(), 8);
    }

    #[test]
    fn test_event_driven_title_contains_entity() {
        let mut gen = TitleGenerator::new();
        let context = SignalContext {
            text: "TSMC investing in Arizona fab expansion".to_string(),
            entity_hint: Some("TSMC".to_string()),
            category: Some("semiconductor".to_string()),
            source_url: None,
            timestamp: 1_700_000_000,
        };
        let registry = sample_registry();
        let result = gen.generate(&context, Some(&registry));
        assert!(!result.title.is_empty(), "Event-driven title should not be empty");
        assert!(
            result.entities.iter().any(|e| e == "TSMC"),
            "Entities should include TSMC: {:?}",
            result.entities
        );
    }

    #[test]
    fn test_risk_exposure_title_format() {
        let gen = TitleGenerator::new();
        let context = SignalContext {
            text: "Supply chain disruption risk for electronics manufacturing".to_string(),
            entity_hint: Some("Foxconn".to_string()),
            category: Some("supply_chain".to_string()),
            source_url: None,
            timestamp: 1_700_000_000,
        };
        let entities: Vec<String> = vec!["Foxconn".to_string()];
        let title = gen.generate_with_strategy(TitleStrategy::RiskExposure, &context, None, "Foxconn", &entities);
        assert!(!title.is_empty());
        // Risk exposure titles follow "{entity}'s exposure to {topic}" pattern
        assert!(
            title.contains("'s exposure to"),
            "Risk title should contain \"'s exposure to\": {}",
            title
        );
    }

    #[test]
    fn test_signal_synthesis_title_contains_number() {
        let mut gen = TitleGenerator::new();
        let context = sample_context();
        let result = gen.generate(&context, None);
        if result.strategy == TitleStrategy::SignalSynthesis {
            assert!(
                result.title.chars().any(|c| c.is_ascii_digit()),
                "Signal synthesis title should contain a number: {}",
                result.title
            );
        }
    }

    #[test]
    fn test_title_generator_default_history() {
        let gen = TitleGenerator::new();
        assert_eq!(gen.max_history, 50);
    }

    #[test]
    fn test_title_generator_custom_history() {
        let gen = TitleGenerator::with_max_history(10);
        assert_eq!(gen.max_history, 10);
    }

    #[test]
    fn test_recent_titles_bounded() {
        let mut gen = TitleGenerator::with_max_history(5);
        let context = sample_context();

        // Generate 10 titles (should only keep last 5)
        for _ in 0..10 {
            gen.generate(&context, None);
        }

        assert!(
            gen.recent_titles().len() <= 5,
            "Recent titles should be bounded by max_history"
        );
    }

    #[test]
    fn test_semantic_diversity_score_partial_overlap() {
        let title = "NVIDIA expands GPU capacity amid AI demand surge".to_string();
        let recent = vec!["NVIDIA expands GPU manufacturing capacity".to_string()];
        let score = semantic_diversity_score(&title, &recent);
        // Partial overlap should give a moderate score
        assert!(
            score > 0.0 && score < 1.0,
            "Partial overlap should give score between 0 and 1, got {}",
            score
        );
    }

    #[test]
    fn test_jaccard_similarity_identical() {
        let set_a: HashSet<String> =
            ["nvidia", "expands", "capacity"].iter().map(|s| s.to_string()).collect();
        let set_b: HashSet<String> =
            ["nvidia", "expands", "capacity"].iter().map(|s| s.to_string()).collect();
        let sim = jaccard_similarity(&set_a, &set_b);
        assert!((sim - 1.0).abs() < 0.01, "Identical sets should have J=1.0");
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        let set_a: HashSet<String> =
            ["nvidia", "expands"].iter().map(|s| s.to_string()).collect();
        let set_b: HashSet<String> =
            ["tsmc", "reports"].iter().map(|s| s.to_string()).collect();
        let sim = jaccard_similarity(&set_a, &set_b);
        assert!((sim - 0.0).abs() < 0.01, "Disjoint sets should have J=0.0");
    }

    #[test]
    fn test_tokenize_words_basic() {
        let tokens = tokenize_words("NVIDIA expands GPU capacity");
        assert_eq!(tokens.len(), 4);
        assert!(tokens.contains("nvidia"));
        assert!(tokens.contains("expands"));
        assert!(tokens.contains("gpu"));
        assert!(tokens.contains("capacity"));
    }

    #[test]
    fn test_tokenize_words_ignores_punctuation() {
        let tokens = tokenize_words("NVIDIA's GPU: capacity!");
        assert!(tokens.contains("nvidia's"));
        assert!(tokens.contains("gpu"));
        assert!(tokens.contains("capacity"));
    }

    #[test]
    fn test_truncate_context_short_text() {
        let result = truncate_context("market conditions", 10);
        assert_eq!(result, "market conditions");
    }

    #[test]
    fn test_truncate_context_long_text() {
        let result = truncate_context("the quick brown fox jumps over the lazy dog", 3);
        assert_eq!(result, "the quick brown");
    }

    #[test]
    fn test_truncate_context_empty_fallback() {
        let result = truncate_context("a an of", 3);
        // All words are <= 2 chars, so we fall back
        assert_eq!(result, "a an of");
    }

    #[test]
    fn test_generate_titles_are_reasonably_formatted() {
        let mut gen = TitleGenerator::new();
        let context = sample_context();

        for _ in 0..20 {
            let result = gen.generate(&context, None);
            // Titles should not be empty
            assert!(!result.title.is_empty(), "Generated title should not be empty");
            // Titles should not exceed reasonable length (256 chars)
            assert!(
                result.title.len() <= 256,
                "Title too long: {} chars: {}",
                result.title.len(),
                result.title
            );
            // Titles should contain at least one uppercase letter
            assert!(
                result.title.chars().any(|c| c.is_uppercase()),
                "Title should contain uppercase: {}",
                result.title
            );
        }
    }
}
