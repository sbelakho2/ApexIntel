//!
//! Multi-Agent Reasoning System for ApexIntel OSINT platform.
//!
//! Implements specialized agents for OSINT analysis:
//! - Investigator Agent: Deep analysis of specific entity
//! - Cross-Reference Agent: Entity correlation
//! - Threat Analyst Agent: Risk assessment
//! - Financial Analyst Agent: Company health
//! - Geopolitical Agent: Regional risk
//!
//! Each agent is specialized for a specific domain of OSINT analysis.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

use crate::advanced_prompting::{
    AdvancedPromptingEngine, AnalysisPerspective, CalibratedConfidence, ConfidenceLevel,
};
use crate::quality_control::{BiasCheck, HallucinationCheck, QualityControlEngine};
use crate::rag::KnowledgeBase;
use crate::LlmClient;

/// Agent types in the multi-agent system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    /// Deep analysis of specific entities
    Investigator,
    /// Entity correlation and linking
    CrossReference,
    /// Risk assessment
    ThreatAnalyst,
    /// Company financial health
    FinancialAnalyst,
    /// Regional and geopolitical risk
    Geopolitical,
}

impl AgentType {
    pub fn system_prompt(&self) -> &'static str {
        match self {
            Self::Investigator => {
                "You are an analytical OSINT processing system. \
                Your role is to extract and structure factual information from provided source data only. \
                Core rules: \
                - ONLY use information explicitly present in the provided source text \
                - NEVER fabricate data, names, figures, or relationships not in the sources \
                - When information is missing, state 'Not available from sources' \
                - Mark every claim with the specific source line that supports it \
                - Distinguish between verified facts, reported claims, and inferences \
                Output structured profiles with explicit source citations for every field. \
                /no_think"
            }
            Self::CrossReference => {
                "You are an analytical entity correlation system. \
                Your role is to identify connections between entities based ONLY on provided source data. \
                Core rules: \
                - ONLY correlate based on information present in the provided data \
                - NEVER invent connections or relationships not supported by evidence \
                - When entities cannot be reliably linked, state 'No verified connection found' \
                - Rate connection strength only with supporting evidence citations \
                - Flag ambiguous entity resolution as 'Uncertain' with explanation \
                /no_think"
            }
            Self::ThreatAnalyst => {
                "You are an analytical risk assessment system. \
                Your role is to evaluate threats based ONLY on provided evidence and established frameworks. \
                Core rules: \
                - ONLY assess threats supported by evidence in the provided data \
                - NEVER invent threat actors, vulnerabilities, or attack scenarios \
                - Use established frameworks (MITRE ATT&CK, NIST, ISO 31000) only when evidence supports mapping \
                - Always separate likelihood (evidence-based) from impact (analytical assessment) \
                - Label unsupported assessments as 'Insufficient evidence' \
                /no_think"
            }
            Self::FinancialAnalyst => {
                "You are an analytical financial processing system. \
                Your role is to analyze financial data based ONLY on provided information. \
                Core rules: \
                - ONLY analyze data explicitly present in the provided financial information \
                - NEVER fabricate revenue figures, ratios, ratings, or financial metrics \
                - When data is missing or incomplete, state 'Data unavailable' for that dimension \
                - Distinguish between verified filings, reported estimates, and projected figures \
                - Never assert creditworthiness or fraud without explicit evidence in the data \
                /no_think"
            }
            Self::Geopolitical => {
                "You are an analytical geopolitical assessment system. \
                Your role is to evaluate regional risks based ONLY on provided intelligence data. \
                Core rules: \
                - ONLY assess risks supported by provided intelligence and data sources \
                - NEVER fabricate political events, sanctions details, or conflict scenarios \
                - Cite specific source data for every risk assessment \
                - Label forward-looking scenarios as 'Projected' not 'Confirmed' \
                - When regional data is insufficient, state 'Insufficient regional intelligence' \
                /no_think"
            }
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Investigator => "investigator",
            Self::CrossReference => "cross_reference",
            Self::ThreatAnalyst => "threat_analyst",
            Self::FinancialAnalyst => "financial_analyst",
            Self::Geopolitical => "geopolitical",
        }
    }
}

/// Configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_type: AgentType,
    /// Custom system prompt override (uses default if None).
    pub system_prompt: Option<String>,
    /// Enable chain-of-thought reasoning.
    pub chain_of_thought: bool,
    /// Enable multi-perspective analysis.
    pub multi_perspective: bool,
    /// Custom perspectives to use.
    pub custom_perspectives: Option<Vec<AnalysisPerspective>>,
    /// Maximum analysis iterations.
    pub max_iterations: usize,
    /// Enable RAG grounding.
    pub use_rag: bool,
    /// Enable quality control checks.
    pub quality_control: bool,
}

impl AgentConfig {
    pub fn for_type(agent_type: AgentType) -> Self {
        let (cot, mp) = match agent_type {
            AgentType::Investigator => (true, false),
            AgentType::CrossReference => (true, false),
            AgentType::ThreatAnalyst => (true, true),
            AgentType::FinancialAnalyst => (true, true),
            AgentType::Geopolitical => (true, true),
        };

        Self {
            agent_type,
            system_prompt: None,
            chain_of_thought: cot,
            multi_perspective: mp,
            custom_perspectives: None,
            max_iterations: 3,
            use_rag: true,
            quality_control: true,
        }
    }

    pub fn with_custom_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = Some(prompt);
        self
    }

    pub fn with_cot(mut self, enabled: bool) -> Self {
        self.chain_of_thought = enabled;
        self
    }

    pub fn with_multi_perspective(
        mut self,
        enabled: bool,
        perspectives: Vec<AnalysisPerspective>,
    ) -> Self {
        self.multi_perspective = enabled;
        self.custom_perspectives = Some(perspectives);
        self
    }

    pub fn with_rag(mut self, enabled: bool) -> Self {
        self.use_rag = enabled;
        self
    }

    pub fn with_quality_control(mut self, enabled: bool) -> Self {
        self.quality_control = enabled;
        self
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self::for_type(AgentType::Investigator)
    }
}

/// Agent state during analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub agent_type: AgentType,
    pub iteration: usize,
    pub findings: Vec<Finding>,
    pub confidence: f64,
    pub sources_used: Vec<String>,
    pub issues_flagged: Vec<String>,
}

impl AgentState {
    pub fn new(agent_type: AgentType) -> Self {
        Self {
            agent_type,
            iteration: 0,
            findings: vec![],
            confidence: 0.0,
            sources_used: vec![],
            issues_flagged: vec![],
        }
    }

    pub fn add_finding(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    pub fn add_source(&mut self, source: String) {
        if !self.sources_used.contains(&source) {
            self.sources_used.push(source);
        }
    }

    pub fn flag_issue(&mut self, issue: String) {
        self.issues_flagged.push(issue);
    }

    pub fn update_confidence(&mut self, confidence: f64) {
        self.confidence = confidence;
    }

    pub fn increment_iteration(&mut self) {
        self.iteration += 1;
    }
}

/// A finding from agent analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Finding title.
    pub title: String,
    /// Finding description.
    pub description: String,
    /// Confidence in this finding.
    pub confidence: f64,
    /// Source(s) supporting this finding.
    pub sources: Vec<String>,
    /// Tags/categories.
    pub tags: Vec<String>,
}

impl Finding {
    pub fn new(title: &str, description: &str) -> Self {
        Self {
            title: title.to_string(),
            description: description.to_string(),
            confidence: 0.5,
            sources: vec![],
            tags: vec![],
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_sources(mut self, sources: Vec<String>) -> Self {
        self.sources = sources;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Complete agent analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub agent_type: AgentType,
    pub analysis: String,
    pub findings: Vec<Finding>,
    pub confidence: CalibratedConfidence,
    pub quality_checks: Option<QualityChecks>,
    pub sources_cited: Vec<String>,
    pub iterations: usize,
    pub execution_time_ms: u64,
}

impl AgentResult {
    pub fn summary(&self) -> String {
        format!(
            "{} Agent Result: {} findings, {:.0}% confidence",
            self.agent_type.as_str(),
            self.findings.len(),
            self.confidence.calibrated_score * 100.0
        )
    }
}

/// Quality check results from agent analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityChecks {
    pub hallucination: HallucinationCheck,
    pub bias: BiasCheck,
    pub coherence_score: f64,
    pub overall_passes: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Multi-Agent Coordinator
// ─────────────────────────────────────────────────────────────────────────────

/// Coordinator for managing multiple agents.
pub struct MultiAgentCoordinator {
    agents: HashMap<AgentType, AgentConfig>,
    prompting_engine: AdvancedPromptingEngine,
    quality_engine: QualityControlEngine,
    knowledge_base: Option<std::sync::Arc<KnowledgeBase>>,
}

impl MultiAgentCoordinator {
    pub fn new() -> Self {
        let mut agents = HashMap::new();

        // Register default agent configurations
        for agent_type in [
            AgentType::Investigator,
            AgentType::CrossReference,
            AgentType::ThreatAnalyst,
            AgentType::FinancialAnalyst,
            AgentType::Geopolitical,
        ] {
            agents.insert(agent_type, AgentConfig::for_type(agent_type));
        }

        Self {
            agents,
            prompting_engine: AdvancedPromptingEngine::new(),
            quality_engine: QualityControlEngine::with_default_config(),
            knowledge_base: None,
        }
    }

    /// Set a custom agent configuration.
    pub fn with_agent_config(mut self, config: AgentConfig) -> Self {
        self.agents.insert(config.agent_type, config);
        self
    }

    /// Set the knowledge base for RAG.
    pub fn with_knowledge_base(mut self, kb: std::sync::Arc<KnowledgeBase>) -> Self {
        self.knowledge_base = Some(kb);
        self
    }

    /// Get agent configuration.
    pub fn get_agent_config(&self, agent_type: AgentType) -> Option<&AgentConfig> {
        self.agents.get(&agent_type)
    }

    /// Get all registered agent types.
    pub fn registered_agents(&self) -> Vec<AgentType> {
        self.agents.keys().cloned().collect()
    }

    /// Build system prompt for an agent.
    pub fn build_system_prompt(&self, agent_type: AgentType) -> String {
        let Some(config) = self.agents.get(&agent_type) else {
            return agent_type.system_prompt().to_string();
        };

        if let Some(ref custom) = config.system_prompt {
            return custom.clone();
        }

        agent_type.system_prompt().to_string()
    }

    /// Build user prompt for a task with optional context.
    pub fn build_task_prompt(
        &self,
        agent_type: AgentType,
        task: &str,
        context: Option<&str>,
    ) -> String {
        let Some(config) = self.agents.get(&agent_type) else {
            return task.to_string();
        };
        let mut prompt = task.to_string();

        // Add context if available
        if let Some(ctx) = context {
            prompt = format!("Context:\n{}\n\nTask:\n{}", ctx, task);
        }

        // Add chain-of-thought instruction if enabled
        if config.chain_of_thought {
            prompt = self.prompting_engine.generate_cot_prompt(&prompt);
        }

        prompt
    }

    /// Analyze with a specific agent.
    pub async fn analyze_with_agent(
        &self,
        agent_type: AgentType,
        task: &str,
        context: Option<&str>,
        llm: &crate::OpenAiCompatibleClient,
    ) -> Result<AgentResult> {
        let start = std::time::Instant::now();
        let Some(config) = self.agents.get(&agent_type) else {
            return Err(anyhow::anyhow!(
                "Agent type {:?} not registered",
                agent_type
            ));
        };

        // Build prompts
        let system_prompt = self.build_system_prompt(agent_type);
        let user_prompt = self.build_task_prompt(agent_type, task, context);

        // Generate analysis
        let analysis = llm
            .generate_text(&system_prompt, &user_prompt)
            .await
            .context("Agent analysis generation failed")?;

        // Parse findings from analysis
        let findings = self.extract_findings(&analysis, agent_type);

        // Calculate confidence
        let confidence = self.estimate_confidence(&analysis, agent_type);

        // Run quality checks if enabled
        let quality_checks = if config.quality_control {
            let hallucination = self.quality_engine.detect_hallucinations(&analysis);
            let bias = self.quality_engine.detect_bias(&analysis);
            let coherence = self.quality_engine.check_coherence(&analysis);

            let h_risk = hallucination.risk_score;
            let b_score = bias.bias_score;
            let c_score = coherence.score;
            Some(QualityChecks {
                hallucination,
                bias,
                coherence_score: c_score,
                overall_passes: c_score >= 0.7 && h_risk <= 0.3 && b_score <= 0.5,
            })
        } else {
            None
        };

        let execution_time_ms = start.elapsed().as_millis() as u64;

        let result = AgentResult {
            agent_type,
            analysis: analysis.clone(),
            findings,
            confidence,
            quality_checks,
            sources_cited: self.extract_sources(&analysis),
            iterations: 1,
            execution_time_ms,
        };

        debug!(
            agent = %agent_type.as_str(),
            findings = %result.findings.len(),
            confidence = %result.confidence.calibrated_score,
            "Agent analysis completed"
        );

        Ok(result)
    }

    /// Run coordinated analysis with multiple agents.
    pub async fn coordinated_analysis(
        &self,
        task: &str,
        agents_to_use: &[AgentType],
        llm: &crate::OpenAiCompatibleClient,
    ) -> Result<CoordinatedAnalysisResult> {
        info!(
            task = %task,
            agents = ?agents_to_use,
            "Starting coordinated multi-agent analysis"
        );

        let start = std::time::Instant::now();
        let mut agent_results = HashMap::new();

        // Run each agent in sequence
        for agent_type in agents_to_use {
            let result = self
                .analyze_with_agent(*agent_type, task, None, llm)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(agent = %agent_type.as_str(), error = %e, "Agent analysis failed");
                    AgentResult {
                        agent_type: *agent_type,
                        analysis: format!("Analysis failed: {}", e),
                        findings: vec![],
                        confidence: CalibratedConfidence {
                            raw_score: 0.0,
                            calibrated_score: 0.0,
                            level: ConfidenceLevel::Unknown,
                            factors: vec![],
                            interval: (0.0, 1.0),
                            warnings: vec![format!("Analysis error: {}", e)],
                        },
                        quality_checks: None,
                        sources_cited: vec![],
                        iterations: 0,
                        execution_time_ms: 0,
                    }
                });

            agent_results.insert(*agent_type, result);
        }

        // Synthesize results
        let synthesis = self.synthesize_agent_results(&agent_results);

        let execution_time_ms = start.elapsed().as_millis() as u64;

        Ok(CoordinatedAnalysisResult {
            agent_results,
            synthesis,
            execution_time_ms,
        })
    }

    fn extract_findings(&self, analysis: &str, _agent_type: AgentType) -> Vec<Finding> {
        let mut findings = Vec::new();

        // Simple finding extraction based on section headers
        let lines: Vec<&str> = analysis.lines().collect();
        let mut current_finding: Option<(String, String)> = None;

        for line in lines {
            let trimmed = line.trim();

            // Look for finding indicators
            if trimmed.starts_with("# ") || trimmed.starts_with("**") && trimmed.ends_with(":**") {
                // Save previous finding
                if let Some((title, desc)) = current_finding.take() {
                    if !desc.is_empty() {
                        findings.push(Finding::new(&title, &desc));
                    }
                }

                let title = trimmed
                    .trim_start_matches('#')
                    .trim_start_matches("**")
                    .trim_end_matches(":**")
                    .trim()
                    .to_string();
                current_finding = Some((title, String::new()));
            } else if let Some((_, ref mut desc)) = current_finding {
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    if !desc.is_empty() {
                        desc.push('\n');
                    }
                    desc.push_str(trimmed);
                }
            }
        }

        // Save last finding
        if let Some((title, desc)) = current_finding {
            if !desc.is_empty() {
                findings.push(Finding::new(&title, &desc));
            }
        }

        // If no structured findings, treat entire analysis as one finding
        if findings.is_empty() && !analysis.trim().is_empty() {
            findings.push(Finding::new("Analysis", analysis));
        }

        findings
    }

    fn extract_sources(&self, analysis: &str) -> Vec<String> {
        let mut sources = Vec::new();

        // Look for common source patterns
        let source_patterns = [
            r"(?i)source:\s*([^\n]+)",
            r"(?i)cited:\s*([^\n]+)",
            r"(?i)according to\s+([^\n,]+)",
            r"\[([^\]]+)\]", // Bracketed references
        ];

        for pattern in &source_patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                for cap in re.captures_iter(analysis) {
                    if let Some(source) = cap.get(1) {
                        let source_str = source.as_str().trim().to_string();
                        if !source_str.is_empty() && !sources.contains(&source_str) {
                            sources.push(source_str);
                        }
                    }
                }
            }
        }

        sources
    }

    fn estimate_confidence(&self, analysis: &str, _agent_type: AgentType) -> CalibratedConfidence {
        // Extract confidence mentions
        let mentions = self.prompting_engine.extract_confidence_mentions(analysis);
        let overconfidence = self.prompting_engine.detect_overconfidence(analysis);

        // Calculate average stated confidence
        let raw_score = if mentions.is_empty() {
            0.5
        } else {
            mentions.iter().map(|(_, v)| v).sum::<f64>() / mentions.len() as f64
        };

        // Adjust for overconfidence detection
        let has_overconfident_claims = !overconfidence.is_empty();

        self.prompting_engine.calibrate_confidence(
            raw_score,
            0.6, // Default evidence quality
            0.7, // Default source reliability
            has_overconfident_claims,
        )
    }

    fn synthesize_agent_results(
        &self,
        results: &HashMap<AgentType, AgentResult>,
    ) -> AgentSynthesis {
        let mut all_findings = Vec::new();
        let mut all_sources = Vec::new();
        let mut total_confidence = 0.0;
        let mut agent_count = 0;
        let key_insights = Vec::new();
        let mut risk_factors = Vec::new();

        for (agent_type, result) in results {
            total_confidence += result.confidence.calibrated_score;
            agent_count += 1;

            // Collect sources
            for source in &result.sources_cited {
                if !all_sources.contains(source) {
                    all_sources.push(source.clone());
                }
            }

            // Extract key insights based on agent type
            match agent_type {
                AgentType::ThreatAnalyst => {
                    for finding in &result.findings {
                        if finding.tags.iter().any(|t| t.contains("risk")) {
                            risk_factors.push(finding.description.clone());
                        }
                    }
                }
                AgentType::FinancialAnalyst => {
                    for finding in &result.findings {
                        if finding.description.to_lowercase().contains("concern") {
                            risk_factors.push(finding.description.clone());
                        }
                    }
                }
                _ => {}
            }

            // Take top 3 findings from each agent
            for finding in result.findings.iter().take(3) {
                all_findings.push(Finding {
                    title: format!("[{}] {}", agent_type.as_str(), finding.title),
                    ..finding.clone()
                });
            }
        }

        // Generate synthesis
        let avg_confidence = if agent_count > 0 {
            total_confidence / agent_count as f64
        } else {
            0.0
        };

        // Sort findings by confidence
        all_findings.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));

        AgentSynthesis {
            summary: format!(
                "Coordinated analysis from {} agents with average {:.0}% confidence",
                agent_count,
                avg_confidence * 100.0
            ),
            all_findings,
            consolidated_sources: all_sources,
            key_insights,
            risk_factors,
            overall_confidence: avg_confidence,
        }
    }
}

impl Default for MultiAgentCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of coordinated multi-agent analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatedAnalysisResult {
    pub agent_results: HashMap<AgentType, AgentResult>,
    pub synthesis: AgentSynthesis,
    pub execution_time_ms: u64,
}

/// Synthesis across all agent results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSynthesis {
    pub summary: String,
    pub all_findings: Vec<Finding>,
    pub consolidated_sources: Vec<String>,
    pub key_insights: Vec<String>,
    pub risk_factors: Vec<String>,
    pub overall_confidence: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Specialized Agent Implementations
// ─────────────────────────────────────────────────────────────────────────────

/// Investigator Agent - Deep entity analysis.
pub struct InvestigatorAgent {
    /// Reserved for future LLM pipeline integration.
    _config: AgentConfig,
}

impl InvestigatorAgent {
    pub fn new() -> Self {
        Self {
            _config: AgentConfig::for_type(AgentType::Investigator),
        }
    }

    pub fn with_config(config: AgentConfig) -> Self {
        Self { _config: config }
    }

    /// Generate investigation report structure.
    pub fn investigation_structure(entity_name: &str) -> String {
        format!(
            "Investigation Report: {}\n\n\
            ## Entity Profile\n\
            - Legal Name:\n\
            - Trade Names / Aliases:\n\
            - Registration:\n\
            - Industry:\n\
            - Headquarters:\n\n\
            ## Financial Overview\n\
            - Revenue/Size:\n\
            - Profitability:\n\
            - Credit Rating:\n\n\
            ## Key Relationships\n\
            - Parent/Subsidiaries:\n\
            - Key Partners:\n\
            - Beneficial Owners:\n\n\
            ## Risk Indicators\n\
            - Red Flags:\n\
            - Legal Issues:\n\
            - Sanctions/PEP Status:\n\n\
            ## Source Evaluation\n\
            - Primary Sources:\n\
            - Reliability Assessment:",
            entity_name
        )
    }

    pub fn extraction_prompt(entity_name: &str, source_text: &str) -> String {
        format!(
            "Extract structured information about '{}' from the following text. \
            Create a detailed entity profile including all identifiable facts.\n\n\
            Source Text:\n{}\n\n\
            Focus on: Legal name, aliases, registration, relationships, financial indicators, risk factors. \
            Format as structured sections with source attributions.\n\n\
            /no_think",
            entity_name, source_text
        )
    }
}

impl Default for InvestigatorAgent {
    fn default() -> Self {
        Self::new()
    }
}

/// Cross-Reference Agent - Entity correlation.
pub struct CrossReferenceAgent {
    /// Reserved for future LLM pipeline integration.
    _config: AgentConfig,
}

impl CrossReferenceAgent {
    pub fn new() -> Self {
        Self {
            _config: AgentConfig::for_type(AgentType::CrossReference),
        }
    }

    /// Generate correlation analysis prompt.
    pub fn correlation_prompt(entity_a: &str, entity_b: &str) -> String {
        format!(
            "Analyze potential connections between the following entities:\n\n\
            Entity A: {}\n\
            Entity B: {}\n\n\
            Investigate:\n\
            1. Direct connections (shared officers, addresses, accounts)\n\
            2. Indirect connections (via intermediaries, shell companies)\n\
            3. Timing correlations (registration dates, transactions)\n\
            4. Pattern analysis (similar structures, behaviors)\n\n\
            Rate the connection strength (None, Weak, Moderate, Strong, Confirmed). \
            Provide evidence supporting your assessment.\n\n\
            /no_think",
            entity_a, entity_b
        )
    }

    /// Generate entity resolution prompt.
    pub fn resolution_prompt(name_variants: &[String]) -> String {
        format!(
            "Resolve the following entity name variants. Determine if they refer to the same entity.\n\n\
            Variants:\n{}\n\n\
            Analysis required:\n\
            1. Name normalization and matching\n\
            2. Location/business alignment\n\
            3. Registration/incorporation records\n\
            4. Confidence in resolution\n\n\
            Provide resolution decision (Same Entity / Different Entities / Uncertain) \
            with supporting evidence.\n\n\
            /no_think",
            name_variants.iter()
                .enumerate()
                .map(|(i, n)| format!("{}. {}", i + 1, n))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

impl Default for CrossReferenceAgent {
    fn default() -> Self {
        Self::new()
    }
}

/// Threat Analyst Agent - Risk assessment.
pub struct ThreatAnalystAgent {
    /// Reserved for future LLM pipeline integration.
    _config: AgentConfig,
}

impl ThreatAnalystAgent {
    pub fn new() -> Self {
        Self {
            _config: AgentConfig::for_type(AgentType::ThreatAnalyst),
        }
    }

    /// Generate threat assessment framework prompt.
    pub fn threat_assessment_prompt(target: &str, threat_domain: &str) -> String {
        format!(
            "Conduct a comprehensive threat assessment for:\n\n\
            Target: {}\n\
            Domain: {}\n\n\
            Assessment Framework:\n\
            1. Threat Identification\n\
               - What threats are relevant?\n\
               - Who are the threat actors?\n\
               - What are their capabilities and intentions?\n\n\
            2. Vulnerability Analysis\n\
               - What weaknesses exist?\n\
               - Which are most exposed?\n\n\
            3. Impact Assessment\n\
               - What would be the consequences?\n\
               - How severe would impacts be?\n\n\
            4. Risk Calculation\n\
               - Likelihood (1-5):\n\
               - Severity (1-5):\n\
               - Risk Score (Likelihood × Severity):\n\n\
            5. Mitigation Recommendations\n\
               - Priority actions:\n\
               - Resource requirements:\n\n\
            /no_think",
            target, threat_domain
        )
    }

    /// Generate supply chain risk assessment prompt.
    pub fn supply_chain_risk_prompt(company: &str, component: &str) -> String {
        format!(
            "Assess supply chain risk for:\n\n\
            Company: {}\n\
            Component/Subsystem: {}\n\n\
            Analysis Required:\n\
            1. Single Points of Failure\n\
               - Sole-source dependencies?\n\
               - Geographic concentration?\n\n\
            2. Tier-N Exposure\n\
               - Direct supplier risks\n\
               - Sub-tier supplier risks\n\n\
            3. Concentration Risk\n\
               - Geographic\n\
               - Supplier\n\
               - Subcomponent\n\n\
            4. Disruption Scenarios\n\
               - Natural disaster\n\
               - Political instability\n\
               - Financial failure\n\
               - Quality incident\n\n\
            5. Mitigation Options\n\
               - Dual sourcing\n\
               - Inventory buffers\n\
               - Alternative designs\n\n\
            Risk Rating: [Critical/High/Medium/Low]\n\n\
            /no_think",
            company, component
        )
    }
}

impl Default for ThreatAnalystAgent {
    fn default() -> Self {
        Self::new()
    }
}

/// Financial Analyst Agent - Company health.
pub struct FinancialAnalystAgent {
    /// Reserved for future LLM pipeline integration.
    _config: AgentConfig,
}

impl FinancialAnalystAgent {
    pub fn new() -> Self {
        Self {
            _config: AgentConfig::for_type(AgentType::FinancialAnalyst),
        }
    }

    /// Generate financial health assessment prompt.
    pub fn financial_health_prompt(company: &str, financials: &str) -> String {
        format!(
            "Assess the financial health of {}.\n\n\
            Financial Data:\n{}\n\n\
            Assessment Dimensions:\n\
            1. Profitability Analysis\n\
               - Revenue trend\n\
               - Margin analysis\n\
               - Quality of earnings\n\n\
            2. Liquidity Assessment\n\
               - Working capital\n\
               - Cash position\n\
               - Credit facilities\n\n\
            3. Leverage Analysis\n\
               - Debt levels\n\
               - Debt service capacity\n\
               - Covenant compliance\n\n\
            4. Creditworthiness\n\
               - Rating implications\n\
               - Default probability\n\
               - Recovery prospects\n\n\
            5. Stress Testing\n\
               - Revenue shock sensitivity\n\
               - Rate rise impact\n\
               - Liquidity stress scenarios\n\n\
            Overall Health Rating: [Strong/Acceptable/Weak/Critical]\n\n\
            /no_think",
            company, financials
        )
    }

    /// Generate fraud indicator analysis prompt.
    pub fn fraud_indicator_prompt(company: &str, indicators: &[String]) -> String {
        format!(
            "Analyze potential fraud indicators for {}:\n\n\
            Observed Indicators:\n{}\n\n\
            Analysis Framework:\n\
            1. Red Flag Categorization\n\
               - Financial statement anomalies\n\
               - Corporate governance issues\n\
               - Related party concerns\n\
               - Behavioral indicators\n\n\
            2. Pattern Analysis\n\
               - Benign vs. concerning patterns\n\
               - Clustering of indicators\n\
               - Historical comparison\n\n\
            3. Risk Assessment\n\
               - Indicator severity\n\
               - Cumulative risk\n\
               - Correlation with known schemes\n\n\
            4. Recommended Actions\n\
               - Due diligence deepening\n\
               - Monitoring enhancements\n\
               - Escalation requirements\n\n\
            Fraud Risk Level: [Low/Medium/High/Critical]\n\n\
            /no_think",
            company,
            indicators
                .iter()
                .enumerate()
                .map(|(i, ind)| format!("{}. {}", i + 1, ind))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

impl Default for FinancialAnalystAgent {
    fn default() -> Self {
        Self::new()
    }
}

/// Geopolitical Agent - Regional risk.
pub struct GeopoliticalAgent {
    /// Reserved for future LLM pipeline integration.
    _config: AgentConfig,
}

impl GeopoliticalAgent {
    pub fn new() -> Self {
        Self {
            _config: AgentConfig::for_type(AgentType::Geopolitical),
        }
    }

    /// Generate regional risk assessment prompt.
    pub fn regional_risk_prompt(region: &str, business_activity: &str) -> String {
        format!(
            "Assess geopolitical risk for operations in:\n\n\
            Region: {}\n\
            Business Activity: {}\n\n\
            Assessment Framework:\n\
            1. Political Stability\n\
               - Government stability\n\
               - Policy continuity\n\
               - Institutional strength\n\n\
            2. Security Environment\n\
               - Conflict risk\n\
               - Crime/terrorism\n\
               - Infrastructure security\n\n\
            3. Regulatory Environment\n\
               - Rule of law\n\
               - Enforcement consistency\n\
               - Regulatory changes\n\n\
            4. Trade & Sanctions\n\
               - Trade restrictions\n\
               - Sanctions exposure\n\
               - Export controls\n\n\
            5. Human Rights & ESG\n\
               - Labor standards\n\
               - Environmental regulations\n\
               - Human rights record\n\n\
            Risk Rating: [Low/Medium/High/Critical]\n\
            Key Recommendations:\n\n\
            /no_think",
            region, business_activity
        )
    }

    /// Generate sanctions compliance assessment.
    pub fn sanctions_assessment_prompt(company: &str, country: &str) -> String {
        format!(
            "Assess sanctions compliance risk for {} regarding {}:\n\n\
            Analysis Required:\n\
            1. Direct Sanctions Exposure\n\
               - Listed parties\n\
               - SDN vs. sectoral sanctions\n\
               - Investment restrictions\n\n\
            2. Indirect Exposure\n\
               - Joint ventures\n\
               - Supply chain links\n\
               - Customer relationships\n\
               - Beneficial ownership\n\n\
            3. Red Flag Indicators\n\
               - Shell company structures\n\
               - Round-tripping patterns\n\
               - Unusual payment terms\n\n\
            4. Compliance Adequacy\n\
               - Screening procedures\n\
               - OFAC/sanctions program coverage\n\
               - Audit trail completeness\n\n\
            5. Risk Mitigation\n\
               - Controls enhancements\n\
               - Ongoing monitoring\n\
               - Legal review requirements\n\n\
            Compliance Risk: [Low/Medium/High/Critical]\n\n\
            /no_think",
            company, country
        )
    }
}

impl Default for GeopoliticalAgent {
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
    fn agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.agent_type, AgentType::Investigator);
        assert!(config.chain_of_thought);
        assert!(config.quality_control);
    }

    #[test]
    fn agent_types_have_system_prompts() {
        for agent_type in [
            AgentType::Investigator,
            AgentType::CrossReference,
            AgentType::ThreatAnalyst,
            AgentType::FinancialAnalyst,
            AgentType::Geopolitical,
        ] {
            let prompt = agent_type.system_prompt();
            assert!(!prompt.is_empty());
            assert!(prompt.contains("analytical"));
        }
    }

    #[test]
    fn finding_builder() {
        let finding = Finding::new("Test Finding", "Test description")
            .with_confidence(0.8)
            .with_sources(vec!["Source A".to_string()])
            .with_tags(vec!["risk".to_string()]);

        assert_eq!(finding.title, "Test Finding");
        assert_eq!(finding.confidence, 0.8);
        assert_eq!(finding.sources.len(), 1);
        assert_eq!(finding.tags.len(), 1);
    }

    #[test]
    fn agent_state_tracking() {
        let mut state = AgentState::new(AgentType::ThreatAnalyst);

        state.add_finding(Finding::new("Risk 1", "Description"));
        state.add_source("Reuters".to_string());
        state.flag_issue("Unverified claim".to_string());
        state.update_confidence(0.75);

        assert_eq!(state.findings.len(), 1);
        assert_eq!(state.sources_used.len(), 1);
        assert_eq!(state.issues_flagged.len(), 1);
        assert_eq!(state.confidence, 0.75);
    }

    #[test]
    fn investigator_structure_generation() {
        let structure = InvestigatorAgent::investigation_structure("Acme Corp");
        assert!(structure.contains("Acme Corp"));
        assert!(structure.contains("Entity Profile"));
        assert!(structure.contains("Risk Indicators"));
    }

    #[test]
    fn cross_reference_correlation() {
        let prompt = CrossReferenceAgent::correlation_prompt("Company A", "Company B");
        assert!(prompt.contains("Company A"));
        assert!(prompt.contains("Company B"));
        assert!(prompt.contains("Investigate"));
    }

    #[test]
    fn threat_assessment_framework() {
        let prompt = ThreatAnalystAgent::threat_assessment_prompt("Project X", "Cybersecurity");
        assert!(prompt.contains("Project X"));
        assert!(prompt.contains("Cybersecurity"));
        assert!(prompt.contains("Threat Identification"));
    }

    #[test]
    fn financial_health_rating() {
        let prompt = FinancialAnalystAgent::financial_health_prompt(
            "Test Corp",
            "Revenue: $1B, Profit: $100M",
        );
        assert!(prompt.contains("Test Corp"));
        assert!(prompt.contains("Profitability"));
        assert!(prompt.contains("Health Rating"));
    }

    #[test]
    fn regional_risk_assessment() {
        let prompt = GeopoliticalAgent::regional_risk_prompt("Southeast Asia", "Manufacturing");
        assert!(prompt.contains("Southeast Asia"));
        assert!(prompt.contains("Manufacturing"));
        assert!(prompt.contains("Political Stability"));
    }

    #[test]
    fn multi_agent_coordinator_registration() {
        let coordinator = MultiAgentCoordinator::new();
        let agents = coordinator.registered_agents();

        assert!(agents.contains(&AgentType::Investigator));
        assert!(agents.contains(&AgentType::CrossReference));
        assert!(agents.contains(&AgentType::ThreatAnalyst));
        assert!(agents.contains(&AgentType::FinancialAnalyst));
        assert!(agents.contains(&AgentType::Geopolitical));
    }

    #[test]
    fn finding_confidence_bounds() {
        let finding = Finding::new("Test", "Description").with_confidence(1.5); // Over 1.0

        assert!(finding.confidence <= 1.0);

        let finding2 = Finding::new("Test", "Description").with_confidence(-0.5); // Under 0.0

        assert!(finding2.confidence >= 0.0);
    }

    #[test]
    fn agent_result_summary() {
        let result = AgentResult {
            agent_type: AgentType::Investigator,
            analysis: "Test".to_string(),
            findings: vec![
                Finding::new("Finding 1", "Desc 1"),
                Finding::new("Finding 2", "Desc 2"),
            ],
            confidence: CalibratedConfidence {
                raw_score: 0.8,
                calibrated_score: 0.75,
                level: ConfidenceLevel::High,
                factors: vec![],
                interval: (0.6, 0.9),
                warnings: vec![],
            },
            quality_checks: None,
            sources_cited: vec![],
            iterations: 1,
            execution_time_ms: 100,
        };

        let summary = result.summary();
        assert!(summary.contains("investigator"));
        assert!(summary.contains("2 findings"));
        assert!(summary.contains("75%"));
    }
}
