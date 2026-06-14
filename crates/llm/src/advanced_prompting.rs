//!
//! Advanced Prompting for ApexIntel OSINT platform.
//!
//! Implements advanced prompting techniques:
//! - Chain-of-thought reasoning prompts
//! - Self-consistency verification
//! - Multi-perspective analysis
//! - Confidence calibration
//!
//! Optimizes LLM outputs for complex OSINT analysis tasks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Confidence level for analysis outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceLevel {
    /// High confidence (>0.8)
    High,
    /// Medium confidence (0.5-0.8)
    Medium,
    /// Low confidence (<0.5)
    Low,
    /// Unknown or unassessed
    Unknown,
}

impl ConfidenceLevel {
    pub fn from_score(score: f64) -> Self {
        if score > 0.8 {
            Self::High
        } else if score > 0.5 {
            Self::Medium
        } else if score > 0.0 {
            Self::Low
        } else {
            Self::Unknown
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unknown => "unknown",
        }
    }
}

/// Configuration for chain-of-thought prompting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainOfThoughtConfig {
    /// Enable step-by-step reasoning.
    pub enabled: bool,
    /// Number of reasoning steps to generate.
    pub max_steps: usize,
    /// Include intermediate checkpoints.
    pub include_checkpoints: bool,
    /// Verify reasoning consistency.
    pub verify_consistency: bool,
}

impl Default for ChainOfThoughtConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_steps: 5,
            include_checkpoints: true,
            verify_consistency: true,
        }
    }
}

/// A reasoning step in chain-of-thought analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningStep {
    /// Step number (1-indexed).
    pub step: usize,
    /// Step description.
    pub description: String,
    /// Evidence or analysis for this step.
    pub evidence: String,
    /// Confidence in this step.
    pub confidence: f64,
    /// Dependencies on other steps.
    pub depends_on: Vec<usize>,
}

impl ReasoningStep {
    pub fn new(step: usize, description: &str, evidence: &str) -> Self {
        Self {
            step,
            description: description.to_string(),
            evidence: evidence.to_string(),
            confidence: 0.7,
            depends_on: vec![],
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_dependencies(mut self, deps: Vec<usize>) -> Self {
        self.depends_on = deps;
        self
    }
}

/// Configuration for self-consistency verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfConsistencyConfig {
    /// Number of reasoning paths to generate.
    pub num_paths: usize,
    /// Minimum agreement threshold for consensus.
    pub agreement_threshold: f64,
    /// Include dissenting views.
    pub include_dissent: bool,
}

impl Default for SelfConsistencyConfig {
    fn default() -> Self {
        Self {
            num_paths: 3,
            agreement_threshold: 0.66,
            include_dissent: true,
        }
    }
}

/// Self-consistency verification result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfConsistencyResult {
    /// Whether paths reached consensus.
    pub consensus_reached: bool,
    /// The consensus answer.
    pub consensus_answer: String,
    /// Agreement ratio (0.0-1.0).
    pub agreement_ratio: f64,
    /// All generated paths.
    pub paths: Vec<ReasoningPath>,
    /// Dissenting opinions.
    pub dissent: Vec<ReasoningPath>,
}

impl SelfConsistencyResult {
    pub fn evaluate_consensus(&mut self, include_dissent: bool) {
        let mut answer_counts: HashMap<String, usize> = HashMap::new();

        for path in &self.paths {
            *answer_counts.entry(path.answer.clone()).or_insert(0) += 1;
        }

        let total = self.paths.len();
        if total == 0 {
            self.consensus_reached = false;
            return;
        }

        let max_count = answer_counts.values().max().copied().unwrap_or(0);
        self.agreement_ratio = max_count as f64 / total as f64;

        self.consensus_answer = answer_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(ans, _)| ans)
            .unwrap_or_default();

        self.consensus_reached = self.agreement_ratio >= 0.66;

        // Collect dissenting paths
        if include_dissent {
            self.dissent = self.paths
                .iter()
                .filter(|p| p.answer != self.consensus_answer)
                .cloned()
                .collect();
        }
    }
}

/// A single reasoning path in self-consistency verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningPath {
    /// Path identifier.
    pub id: usize,
    /// Reasoning steps.
    pub steps: Vec<ReasoningStep>,
    /// Final answer.
    pub answer: String,
    /// Confidence in the final answer.
    pub confidence: f64,
    /// Alternative answers considered.
    pub alternatives: Vec<String>,
}

impl ReasoningPath {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            steps: vec![],
            answer: String::new(),
            confidence: 0.0,
            alternatives: vec![],
        }
    }

    pub fn add_step(&mut self, step: ReasoningStep) {
        self.steps.push(step);
    }

    pub fn set_answer(&mut self, answer: String, confidence: f64) {
        self.answer = answer;
        self.confidence = confidence;
    }

    pub fn add_alternative(&mut self, alt: String) {
        self.alternatives.push(alt);
    }

    pub fn overall_confidence(&self) -> f64 {
        if self.steps.is_empty() {
            return self.confidence;
        }

        let step_confidence: f64 = self.steps.iter().map(|s| s.confidence).sum::<f64>()
            / self.steps.len() as f64;
        
        (step_confidence * 0.6 + self.confidence * 0.4).clamp(0.0, 1.0)
    }
}

/// Configuration for multi-perspective analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiPerspectiveConfig {
    /// Enable multi-perspective analysis.
    pub enabled: bool,
    /// Perspectives to include.
    pub perspectives: Vec<AnalysisPerspective>,
    /// Minimum perspectives required.
    pub min_perspectives: usize,
}

impl Default for MultiPerspectiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            perspectives: vec![
                AnalysisPerspective::Optimistic,
                AnalysisPerspective::Pessimistic,
                AnalysisPerspective::Neutral,
                AnalysisPerspective::Contrarian,
            ],
            min_perspectives: 2,
        }
    }
}

/// Analysis perspective types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisPerspective {
    /// Optimistic outlook.
    Optimistic,
    /// Pessimistic outlook.
    Pessimistic,
    /// Neutral/balanced view.
    Neutral,
    /// Devil's advocate / contrarian view.
    Contrarian,
    /// Risk-focused analysis.
    RiskFocused,
    /// Opportunity-focused analysis.
    OpportunityFocused,
    /// Long-term view.
    LongTerm,
    /// Short-term view.
    ShortTerm,
}

impl AnalysisPerspective {
    pub fn prompt_suffix(&self) -> &'static str {
        match self {
            Self::Optimistic => "Consider the most favorable interpretation. What positive outcomes are likely? What strengths support this view?",
            Self::Pessimistic => "Consider the worst-case scenario. What negative outcomes are possible? What risks or weaknesses exist?",
            Self::Neutral => "Present a balanced, objective view. Consider both positive and negative evidence equally.",
            Self::Contrarian => "Challenge conventional wisdom. Why might the mainstream view be wrong? What evidence contradicts the consensus?",
            Self::RiskFocused => "Focus on identifying and analyzing risks. What could go wrong? How significant are the potential threats?",
            Self::OpportunityFocused => "Focus on identifying opportunities. What positive developments are possible? What could create value?",
            Self::LongTerm => "Consider the long-term trajectory. How might this evolve over years? What structural changes matter?",
            Self::ShortTerm => "Focus on immediate developments. What matters in the near term? What are the near-term catalysts?",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Optimistic => "optimistic",
            Self::Pessimistic => "pessimistic",
            Self::Neutral => "neutral",
            Self::Contrarian => "contrarian",
            Self::RiskFocused => "risk_focused",
            Self::OpportunityFocused => "opportunity_focused",
            Self::LongTerm => "long_term",
            Self::ShortTerm => "short_term",
        }
    }
}

/// Result from multi-perspective analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiPerspectiveResult {
    /// Results from each perspective.
    pub perspective_results: HashMap<AnalysisPerspective, PerspectiveResult>,
    /// Synthesis across perspectives.
    pub synthesis: String,
    /// Key agreements across perspectives.
    pub agreements: Vec<String>,
    /// Key disagreements across perspectives.
    pub disagreements: Vec<String>,
}

/// Result from a single perspective.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerspectiveResult {
    pub perspective: AnalysisPerspective,
    pub analysis: String,
    pub confidence: f64,
    pub key_findings: Vec<String>,
}

/// Calibration settings for confidence estimation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationConfig {
    /// Enable confidence calibration.
    pub enabled: bool,
    /// Base uncertainty factor.
    pub base_uncertainty: f64,
    /// Adjust for evidence quality.
    pub adjust_for_evidence: bool,
    /// Adjust for source reliability.
    pub adjust_for_sources: bool,
    /// Penalize model complexity claims.
    pub penalize_overconfidence: bool,
}

impl Default for CalibrationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_uncertainty: 0.2,
            adjust_for_evidence: true,
            adjust_for_sources: true,
            penalize_overconfidence: true,
        }
    }
}

/// Calibrated confidence estimate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibratedConfidence {
    /// Raw confidence score (0.0-1.0).
    pub raw_score: f64,
    /// Calibrated confidence score (0.0-1.0).
    pub calibrated_score: f64,
    /// Confidence level.
    pub level: ConfidenceLevel,
    /// Calibration factors applied.
    pub factors: Vec<CalibrationFactor>,
    /// Confidence interval (lower, upper).
    pub interval: (f64, f64),
    /// Warnings about calibration.
    pub warnings: Vec<String>,
}

/// A factor that affects confidence calibration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationFactor {
    /// Factor name.
    pub name: String,
    /// Adjustment applied (-1.0 to +1.0).
    pub adjustment: f64,
    /// Reason for the adjustment.
    pub reason: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Advanced Prompting Engine
// ─────────────────────────────────────────────────────────────────────────────

/// Advanced prompting engine for OSINT analysis.
pub struct AdvancedPromptingEngine {
    cot_config: ChainOfThoughtConfig,
    sc_config: SelfConsistencyConfig,
    mp_config: MultiPerspectiveConfig,
    calib_config: CalibrationConfig,
}

impl AdvancedPromptingEngine {
    pub fn new() -> Self {
        Self {
            cot_config: ChainOfThoughtConfig::default(),
            sc_config: SelfConsistencyConfig::default(),
            mp_config: MultiPerspectiveConfig::default(),
            calib_config: CalibrationConfig::default(),
        }
    }

    pub fn with_cot_config(mut self, config: ChainOfThoughtConfig) -> Self {
        self.cot_config = config;
        self
    }

    pub fn with_sc_config(mut self, config: SelfConsistencyConfig) -> Self {
        self.sc_config = config;
        self
    }

    pub fn with_mp_config(mut self, config: MultiPerspectiveConfig) -> Self {
        self.mp_config = config;
        self
    }

    pub fn with_calib_config(mut self, config: CalibrationConfig) -> Self {
        self.calib_config = config;
        self
    }

    // ── Chain-of-thought prompt generation ──

    /// Generate a chain-of-thought prompt.
    pub fn generate_cot_prompt(&self, task: &str) -> String {
        let steps_instruction = if self.cot_config.include_checkpoints {
            "Break down your analysis into numbered steps. At each step, explicitly note your confidence level (high/medium/low) and what evidence supports your reasoning."
        } else {
            "Break down your analysis into clear, logical steps. Show your reasoning process."
        };

        format!(
            "Task: {}\n\n\
            Analysis Instructions:\n\
            {}\n\n\
            Number of steps: {}\n\n\
            Final Answer: [Provide your conclusion based on the reasoning above]\n\
            Confidence: [Rate your confidence in the final answer as High, Medium, or Low]\n\
            Key Assumptions: [List any assumptions made during analysis]\n\
            Sources Needed: [List what additional information would increase confidence]\n\n\
            /no_think",
            task, steps_instruction, self.cot_config.max_steps
        )
    }

    /// Parse a chain-of-thought response into structured steps.
    pub fn parse_cot_response(&self, response: &str) -> Vec<ReasoningStep> {
        let mut steps = Vec::new();
        let mut current_step = 0usize;

        // Simple step extraction based on numbered patterns
        let step_patterns = [
            r"(?m)^(?:Step\s+)?(\d+)[:\.]\s*(.+?)(?=Step\s+\d+|$)",
            r"(?m)^(?:First|Second|Third|Fourth|Fifth|Sixth|Seventh|Eighth|Ninth|Tenth)\s+[:\.]\s*(.+?)(?=(?:First|Second|Third|Fourth|Fifth|Sixth|Seventh|Eighth|Ninth|Tenth)\s+[:\.]|$)",
        ];

        for pattern in &step_patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                let matches: Vec<_> = re.captures_iter(response).collect();
                if !matches.is_empty() {
                    for cap in matches {
                        current_step += 1;
                        let description = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                        steps.push(ReasoningStep::new(
                            current_step,
                            description,
                            "", // Evidence would need separate parsing
                        ));
                    }
                    break;
                }
            }
        }

        // If no structured steps found, treat entire response as single step
        if steps.is_empty() && !response.trim().is_empty() {
            steps.push(ReasoningStep::new(1, "Analysis", response));
        }

        steps
    }

    // ── Self-consistency prompt generation ──

    /// Generate prompts for self-consistency verification.
    pub fn generate_sc_prompts(&self, task: &str) -> Vec<String> {
        let variations = if self.sc_config.num_paths >= 3 {
            vec![
                "Take a methodical, evidence-based approach.",
                "Consider the problem from first principles.",
                "Think critically and challenge assumptions.",
            ]
        } else {
            vec![
                "Analyze this problem carefully.",
                "Provide a thorough analysis.",
            ]
        };

        variations
            .iter()
            .take(self.sc_config.num_paths)
            .map(|variation| {
                format!(
                    "{}\n\nTask: {}\n\n\
                    Provide your analysis and final answer. Be explicit about your reasoning and confidence.\n\n\
                    /no_think",
                    variation, task
                )
            })
            .collect()
    }

    /// Create an empty self-consistency result structure.
    pub fn create_sc_result(&self) -> SelfConsistencyResult {
        SelfConsistencyResult {
            consensus_reached: false,
            consensus_answer: String::new(),
            agreement_ratio: 0.0,
            paths: vec![],
            dissent: vec![],
        }
    }

    // ── Multi-perspective prompt generation ──

    /// Generate prompts for each enabled perspective.
    pub fn generate_mp_prompts(&self, task: &str) -> HashMap<AnalysisPerspective, String> {
        let mut prompts = HashMap::new();

        for perspective in &self.mp_config.perspectives {
            let prompt = format!(
                "Analysis Task: {}\n\n\
                Perspective: {}\n\n\
                {}\n\n\
                Provide your analysis from this perspective. Be explicit about:\n\
                1. Key assumptions from this viewpoint\n\
                2. Evidence supporting this perspective\n\
                3. Key findings or conclusions\n\
                4. Confidence level and reasoning\n\n\
                /no_think",
                task,
                perspective.as_str(),
                perspective.prompt_suffix()
            );
            prompts.insert(*perspective, prompt);
        }

        prompts
    }

    /// Synthesize results from multiple perspectives.
    pub fn synthesize_perspectives(&self, results: &HashMap<AnalysisPerspective, PerspectiveResult>) -> MultiPerspectiveResult {
        let mut agreements = Vec::new();
        let mut disagreements = Vec::new();
        
        // Find key findings that appear across multiple perspectives
        let mut finding_counts: HashMap<String, Vec<AnalysisPerspective>> = HashMap::new();
        
        for result in results.values() {
            for finding in &result.key_findings {
                let normalized = finding.to_lowercase();
                finding_counts
                    .entry(normalized)
                    .or_default()
                    .push(result.perspective);
            }
        }

        for (finding, perspectives) in &finding_counts {
            if perspectives.len() >= 2 {
                agreements.push(finding.to_string());
            }
        }

        // Generate synthesis
        let synthesis = self.generate_synthesis(results, &agreements, &disagreements);

        // Identify disagreements (same topic, different conclusions)
        let perspectives: Vec<_> = results.keys().collect();
        for i in 0..perspectives.len() {
            for j in (i+1)..perspectives.len() {
                let persp1 = perspectives[i];
                let persp2 = perspectives[j];
                let Some(result1) = results.get(persp1) else { continue; };
                let Some(result2) = results.get(persp2) else { continue; };
                // Check for contradictory findings
                let common_topics = result1.key_findings.iter()
                    .filter(|f1| result2.key_findings.iter().any(|f2| {
                        f1.to_lowercase() != f2.to_lowercase() &&
                        (f1.to_lowercase().contains("risk") != f2.to_lowercase().contains("risk") ||
                         f1.to_lowercase().contains("opportunity") != f2.to_lowercase().contains("opportunity"))
                    }))
                    .count();

                if common_topics > 0 {
                    disagreements.push(format!(
                        "{} vs {}: different conclusions on {} shared topic(s)",
                        persp1.as_str(),
                        persp2.as_str(),
                        common_topics
                    ));
                }
            }
        }

        MultiPerspectiveResult {
            perspective_results: results.clone(),
            synthesis,
            agreements,
            disagreements,
        }
    }

    fn generate_synthesis(
        &self,
        results: &HashMap<AnalysisPerspective, PerspectiveResult>,
        agreements: &[String],
        disagreements: &[String],
    ) -> String {
        let mut synthesis = String::from("## Synthesis\n\n");

        synthesis.push_str("### Points of Agreement\n");
        if agreements.is_empty() {
            synthesis.push_str("No clear consensus emerged across perspectives.\n");
        } else {
            for agreement in agreements.iter().take(5) {
                synthesis.push_str(&format!("- {}\n", agreement));
            }
        }

        synthesis.push_str("\n### Points of Disagreement\n");
        if disagreements.is_empty() {
            synthesis.push_str("Perspectives are largely aligned.\n");
        } else {
            for disagreement in disagreements.iter().take(5) {
                synthesis.push_str(&format!("- {}\n", disagreement));
            }
        }

        // Overall assessment
        let avg_confidence: f64 = results.values().map(|r| r.confidence).sum::<f64>()
            / results.len().max(1) as f64;
        
        synthesis.push_str(&format!(
            "\n### Overall Assessment\n\
            Average confidence: {:.0}%\n\
            Perspective count: {}\n",
            avg_confidence * 100.0,
            results.len()
        ));

        synthesis
    }

    // ── Confidence calibration ──

    /// Calibrate a raw confidence score.
    pub fn calibrate_confidence(
        &self,
        raw_score: f64,
        evidence_quality: f64,
        source_reliability: f64,
        has_overconfident_claims: bool,
    ) -> CalibratedConfidence {
        let mut factors = Vec::new();
        let mut warnings = Vec::new();
        let mut calibrated = raw_score;

        // Base uncertainty adjustment
        if self.calib_config.base_uncertainty > 0.0 {
            let base_adj = -self.calib_config.base_uncertainty;
            factors.push(CalibrationFactor {
                name: "Base Uncertainty".to_string(),
                adjustment: base_adj,
                reason: format!("Applied base uncertainty factor: {}", self.calib_config.base_uncertainty),
            });
            calibrated += base_adj;
        }

        // Evidence quality adjustment
        if self.calib_config.adjust_for_evidence {
            let evidence_adj = (evidence_quality - 0.5) * 0.3;
            factors.push(CalibrationFactor {
                name: "Evidence Quality".to_string(),
                adjustment: evidence_adj,
                reason: format!(
                    "Evidence quality: {:.0}% (adjustment: {:+.2})",
                    evidence_quality * 100.0,
                    evidence_adj
                ),
            });
            calibrated += evidence_adj;
        }

        // Source reliability adjustment
        if self.calib_config.adjust_for_sources {
            let source_adj = (source_reliability - 0.5) * 0.2;
            factors.push(CalibrationFactor {
                name: "Source Reliability".to_string(),
                adjustment: source_adj,
                reason: format!(
                    "Source reliability: {:.0}% (adjustment: {:+.2})",
                    source_reliability * 100.0,
                    source_adj
                ),
            });
            calibrated += source_adj;
        }

        // Overconfidence penalty
        if self.calib_config.penalize_overconfidence && has_overconfident_claims {
            let penalty = -0.15;
            factors.push(CalibrationFactor {
                name: "Overconfidence Penalty".to_string(),
                adjustment: penalty,
                reason: "Detected potentially overconfident claims in analysis".to_string(),
            });
            calibrated += penalty;
            warnings.push("Analysis contains claims that may be overstated".to_string());
        }

        // Clamp to valid range
        calibrated = calibrated.clamp(0.0, 1.0);

        // Calculate confidence interval
        let interval_width = 0.1 + (1.0 - evidence_quality) * 0.2;
        let interval = (
            (calibrated - interval_width).max(0.0),
            (calibrated + interval_width).min(1.0),
        );

        CalibratedConfidence {
            raw_score,
            calibrated_score: calibrated,
            level: ConfidenceLevel::from_score(calibrated),
            factors,
            interval,
            warnings,
        }
    }

    /// Extract confidence mentions from text.
    pub fn extract_confidence_mentions(&self, text: &str) -> Vec<(String, f64)> {
        let mut mentions = Vec::new();
        
        let patterns = [
            (r"(?i)high confidence", 0.85),
            (r"(?i)very confident", 0.90),
            (r"(?i)moderate confidence", 0.65),
            (r"(?i)low confidence", 0.35),
            (r"(?i)uncertain", 0.30),
            (r"(?i)very uncertain", 0.20),
            (r"(?i)not confident", 0.25),
            (r"(?i)confident", 0.70),
            (r"(?i)certain", 0.95),
            (r"(?i)unsure", 0.40),
        ];

        for (pattern, value) in &patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                for m in re.find_iter(text) {
                    mentions.push((m.as_str().to_string(), *value));
                }
            }
        }

        mentions
    }

    /// Detect overconfident language patterns.
    pub fn detect_overconfidence(&self, text: &str) -> Vec<String> {
        let mut patterns = Vec::new();

        let overconfident_phrases = [
            "certainly",
            "definitely",
            "absolutely",
            "without doubt",
            "clearly",
            "obviously",
            "proven",
            "guaranteed",
            "always",
            "never",
            "impossible",
            "inevitable",
            "100%",
        ];

        let lower = text.to_lowercase();
        for phrase in &overconfident_phrases {
            if lower.contains(phrase) {
                patterns.push(format!("Overconfident phrase: '{}'", phrase));
            }
        }

        patterns
    }
}

impl Default for AdvancedPromptingEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cot_prompt_includes_steps() {
        let engine = AdvancedPromptingEngine::new();
        let prompt = engine.generate_cot_prompt("Analyze company X");
        
        assert!(prompt.contains("numbered steps") || prompt.contains("Number of steps"));
        assert!(prompt.contains("Confidence"));
        assert!(prompt.contains("/no_think"));
    }

    #[test]
    fn cot_response_parsing() {
        let engine = AdvancedPromptingEngine::new();
        let response = "Step 1: Initial data review\nStep 2: Market analysis\nStep 3: Conclusion";
        
        let steps = engine.parse_cot_response(response);
        assert!(!steps.is_empty());
    }

    #[test]
    fn sc_prompts_diverse() {
        let engine = AdvancedPromptingEngine::new();
        let prompts = engine.generate_sc_prompts("Analyze this situation");
        
        assert_eq!(prompts.len(), 3);
        // Each prompt should have different instruction
        assert!(prompts[0] != prompts[1]);
    }

    #[test]
    fn sc_result_evaluates_consensus() {
        let engine = AdvancedPromptingEngine::new();
        
        let mut result = engine.create_sc_result();
        
        // Add paths with same answer
        let mut path1 = ReasoningPath::new(1);
        path1.set_answer("Growth".to_string(), 0.8);
        result.paths.push(path1);
        
        let mut path2 = ReasoningPath::new(2);
        path2.set_answer("Growth".to_string(), 0.7);
        result.paths.push(path2);
        
        result.evaluate_consensus(true);
        
        assert!(result.consensus_reached);
        assert_eq!(result.consensus_answer, "Growth");
        assert_eq!(result.agreement_ratio, 1.0);
    }

    #[test]
    fn sc_result_no_consensus() {
        let engine = AdvancedPromptingEngine::new();
        
        let mut result = engine.create_sc_result();
        
        let mut path1 = ReasoningPath::new(1);
        path1.set_answer("Growth".to_string(), 0.8);
        result.paths.push(path1);
        
        let mut path2 = ReasoningPath::new(2);
        path2.set_answer("Decline".to_string(), 0.7);
        result.paths.push(path2);
        
        result.evaluate_consensus(true);
        
        assert!(!result.consensus_reached);
        assert!(!result.dissent.is_empty());
    }

    #[test]
    fn mp_prompts_all_perspectives() {
        let engine = AdvancedPromptingEngine::new();
        let prompts = engine.generate_mp_prompts("Analyze company X");
        
        assert!(prompts.contains_key(&AnalysisPerspective::Optimistic));
        assert!(prompts.contains_key(&AnalysisPerspective::Pessimistic));
        assert!(prompts.contains_key(&AnalysisPerspective::Contrarian));
    }

    #[test]
    fn mp_synthesis_finds_agreements() {
        let engine = AdvancedPromptingEngine::new();
        
        let mut results = HashMap::new();
        results.insert(
            AnalysisPerspective::Optimistic,
            PerspectiveResult {
                perspective: AnalysisPerspective::Optimistic,
                analysis: "Strong growth expected".to_string(),
                confidence: 0.7,
                key_findings: vec!["Revenue growth".to_string(), "Market expansion".to_string()],
            },
        );
        results.insert(
            AnalysisPerspective::Pessimistic,
            PerspectiveResult {
                perspective: AnalysisPerspective::Pessimistic,
                analysis: "Risks exist".to_string(),
                confidence: 0.6,
                key_findings: vec!["Revenue growth".to_string(), "Competition".to_string()],
            },
        );
        
        let synthesis = engine.synthesize_perspectives(&results);
        
        assert!(!synthesis.agreements.is_empty());
        assert!(synthesis.agreements.iter().any(|a| a.contains("revenue")));
    }

    #[test]
    fn confidence_calibration() {
        let engine = AdvancedPromptingEngine::new();
        
        let result = engine.calibrate_confidence(0.9, 0.8, 0.9, false);
        
        assert!(result.calibrated_score < result.raw_score);
        assert!(!result.factors.is_empty());
    }

    #[test]
    fn confidence_calibration_with_overconfidence() {
        let engine = AdvancedPromptingEngine::new();
        
        let result = engine.calibrate_confidence(0.95, 0.5, 0.5, true);
        
        assert!(result.calibrated_score < 0.95);
        assert!(result.warnings.iter().any(|w| w.contains("overstated")));
    }

    #[test]
    fn confidence_level_from_score() {
        assert_eq!(ConfidenceLevel::from_score(0.9), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::from_score(0.6), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::from_score(0.3), ConfidenceLevel::Low);
        assert_eq!(ConfidenceLevel::from_score(-0.1), ConfidenceLevel::Unknown);
    }

    #[test]
    fn extract_confidence_mentions() {
        let engine = AdvancedPromptingEngine::new();
        let text = "We are highly confident in this analysis. The evidence is clear.";
        
        let mentions = engine.extract_confidence_mentions(text);
        assert!(!mentions.is_empty());
    }

    #[test]
    fn detect_overconfidence() {
        let engine = AdvancedPromptingEngine::new();
        let text = "This is clearly the best approach. It is guaranteed to succeed.";
        
        let patterns = engine.detect_overconfidence(text);
        assert!(!patterns.is_empty());
        assert!(patterns.iter().any(|p| p.contains("clearly")));
        assert!(patterns.iter().any(|p| p.contains("guaranteed")));
    }

    #[test]
    fn reasoning_step_with_confidence() {
        let step = ReasoningStep::new(1, "Initial analysis", "Data reviewed")
            .with_confidence(0.9);
        
        assert_eq!(step.confidence, 0.9);
    }

    #[test]
    fn reasoning_path_confidence() {
        let mut path = ReasoningPath::new(1);
        path.add_step(ReasoningStep::new(1, "Step 1", "Evidence 1").with_confidence(0.8));
        path.add_step(ReasoningStep::new(2, "Step 2", "Evidence 2").with_confidence(0.7));
        path.set_answer("Conclusion".to_string(), 0.75);
        
        let overall = path.overall_confidence();
        assert!(overall > 0.0 && overall <= 1.0);
    }

    #[test]
    fn perspective_prompt_suffixes() {
        assert!(AnalysisPerspective::Optimistic.prompt_suffix().contains("favorable"));
        assert!(AnalysisPerspective::Pessimistic.prompt_suffix().contains("worst-case"));
        assert!(AnalysisPerspective::Contrarian.prompt_suffix().contains("wrong"));
    }

    #[test]
    fn calibration_interval() {
        let engine = AdvancedPromptingEngine::new();
        
        let result = engine.calibrate_confidence(0.7, 0.9, 0.9, false);
        
        assert!(result.interval.0 < result.calibrated_score);
        assert!(result.calibrated_score < result.interval.1);
    }
}
