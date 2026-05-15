#!/usr/bin/env python3
"""Fix eval JSONL system prompts to include explicit field name requirements.

The model was trained with correct field names but drifts at inference because
system prompts don't explicitly list the required JSON schema. This script
updates eval JSONL files to include explicit field requirements.
"""
import json
import os
import shutil
from pathlib import Path

WORK = Path(__file__).resolve().parent.parent
EVAL_DIR = os.environ.get("EVAL_DIR", str(WORK / "training_data" / "evaluation"))

# Enhanced system prompts with explicit schema requirements
ENHANCED_PROMPTS = {
    "company_dossier_eval.jsonl": (
        "You are generating a company intelligence dossier for an EMS competitive "
        "intelligence platform. Synthesize all provided entity data into a structured "
        "dossier. Return valid JSON with these required top-level keys:\n"
        "- profile_section: company overview with name, country, type, size\n"
        "- capability_assessment: analysis of manufacturing capabilities and gaps\n"
        "- certification_analysis: certification coverage and compliance status\n"
        "- risk_assessment: identified risks with severity ratings\n"
        "- opportunity_analysis: concrete business opportunities\n"
        "- competitive_position: market positioning summary\n"
        "Every assessment must be grounded in the provided data — never fabricate metrics or claims."
    ),
    "memo_quality_eval.jsonl": (
        "You are writing a weekly strategy memo for an EMS General Manager. Given a set "
        "of intelligence insights, produce a structured JSON memo. Return valid JSON with "
        "these required top-level keys:\n"
        "- executive_summary: concise overview of the week's key findings\n"
        "- total_insights: integer count of all insights\n"
        "- critical_count: integer count of critical-severity insights\n"
        "- warning_count: integer count of warning-severity insights\n"
        "- info_count: integer count of info-severity insights\n"
        "- top_actions: list of prioritized recommended actions\n"
        "- regional_sections: breakdown by geographic region\n"
        "- security_summary: cybersecurity and compliance highlights\n"
        "Be concise and actionable."
    ),
    "warning_generation_eval.jsonl": (
        "You are generating real-time intelligence warnings for an EMS competitive "
        "intelligence platform. Given trigger signals, produce a structured warning. "
        "Return valid JSON with these required top-level keys:\n"
        "- warning_type: category of warning (e.g., supply_disruption, competitor_move)\n"
        "- severity: one of 'critical', 'warning', or 'info' (exactly these values)\n"
        "- affected_entity: the company or entity affected\n"
        "- narrative: detailed explanation grounded in the provided signals\n"
        "- recommended_actions: list of concrete actions to take\n"
        "Ground all analysis in the provided signals."
    ),
    "poi_synthesis_eval.jsonl": (
        "You are synthesizing professional intelligence about a Person of Interest (POI) "
        "for an EMS competitive intelligence platform. Return valid JSON with these "
        "required top-level keys:\n"
        "- person_id: unique identifier\n"
        "- name: full name of the person\n"
        "- org: organization affiliation\n"
        "- current_role: current job title/role\n"
        "- role_family: category of role (e.g., executive, engineering, procurement)\n"
        "- priority_vector: dict with keys cost, quality, speed, resilience, compliance, security, confidence (0-1 scores)\n"
        "- psychological_profile: decision style and behavioral tendencies\n"
        "- influence_assessment: scope and nature of influence\n"
        "- what_changed: recent changes in role, responsibilities, or behavior\n"
        "- what_it_implies: strategic implications of the changes\n"
        "- how_to_approach: engagement recommendations\n"
        "Only cite provided artifacts — never fabricate."
    ),
    "competitive_analysis_eval.jsonl": (
        "You are comparing capabilities of two EMS companies for strategic positioning. "
        "Produce a structured competitive analysis. Return valid JSON with these required "
        "top-level keys:\n"
        "- capability_comparison: side-by-side analysis of manufacturing capabilities\n"
        "- certification_comparison: certification coverage comparison\n"
        "- scale_comparison: comparison of scale, revenue, headcount\n"
        "- advantages: key advantages of the primary company\n"
        "- gaps: capability or certification gaps\n"
        "- recommendations: concrete strategic recommendations\n"
        "Base all analysis solely on provided data — no fabrication."
    ),
    "supply_chain_risk_eval.jsonl": (
        "Analyze the supply chain risk described below. Return valid JSON with these "
        "required top-level keys:\n"
        "- risk_summary: concise description of the risk\n"
        "- affected_components: list of affected supply chain components\n"
        "- severity: one of 'critical', 'high', 'medium', or 'low'\n"
        "- mitigation_options: list of specific mitigation strategies\n"
        "Also include impact_assessment, timeline, and alternative_suppliers where applicable."
    ),
    "compliance_eval.jsonl": (
        "Assess the trade compliance risk for the described scenario. Return valid JSON "
        "with these required top-level keys:\n"
        "- risk_level: one of 'critical', 'high', 'medium', or 'low'\n"
        "- entities_of_concern: list of flagged entities\n"
        "- applicable_regulations: relevant laws and regulations\n"
        "- red_flags: identified compliance red flags\n"
        "- recommended_actions: concrete compliance actions to take"
    ),
    "entity_extraction_eval.jsonl": (
        "Extract structured entities from this text. Identify all companies, persons "
        "(with roles), capabilities, certifications, locations, and industries. Return "
        "valid JSON with these top-level keys:\n"
        "- companies: list of objects with 'name' and 'type' fields\n"
        "- persons: list of objects with 'name', 'role', and 'org' fields\n"
        "- capabilities: list of capability strings (e.g., 'SMT Assembly', 'PCB Fabrication')\n"
        "- certifications: list of certification strings (e.g., 'ISO 9001:2015', 'AS9100D')\n"
        "- locations: list of location objects\n"
        "- industries: list of industry strings\n"
        "Extract entity names exactly as they appear in the text."
    ),
    "recipe_quality_eval.jsonl": (
        "You are an OSINT analyst generating insight recipes for an EMS competitive "
        "intelligence platform. Return valid JSON with these required top-level keys:\n"
        "- id: unique recipe identifier\n"
        "- code: short code name\n"
        "- name: descriptive recipe name\n"
        "- description: what this recipe detects\n"
        "- signals: list of observable signal objects\n"
        "- transforms: list of data transformations\n"
        "- statistical_test: the statistical method used\n"
        "- insight_template: narrative template for the insight\n"
        "- action_template: narrative template for recommended action\n"
        "- applicability: geographic/industry scope\n"
        "- severity: one of 'critical', 'warning', or 'info'\n"
        "- category: one of 'demand', 'supply_chain', 'competitor', 'security', 'poi'\n"
        "Ground all signals in observable public data."
    ),
}

def fix_file(filename):
    """Update system prompts in an eval JSONL file."""
    path = os.path.join(EVAL_DIR, filename)
    if not os.path.exists(path):
        print(f"  SKIP: {filename} not found")
        return 0

    if filename not in ENHANCED_PROMPTS:
        print(f"  SKIP: {filename} no enhanced prompt defined")
        return 0

    new_system = ENHANCED_PROMPTS[filename]

    # Backup
    backup = path + ".bak3"
    shutil.copy2(path, backup)

    updated = 0
    lines = []
    with open(path) as f:
        for line in f:
            item = json.loads(line)
            if isinstance(item.get("input"), dict):
                item["input"]["system"] = new_system
                updated += 1
            elif isinstance(item.get("input"), str):
                # Convert string input to dict with system+user
                item["input"] = {"system": new_system, "user": item["input"]}
                updated += 1
            lines.append(json.dumps(item, ensure_ascii=False))

    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")

    print(f"  OK: {filename} — {updated} examples updated")
    return updated

def main():
    total = 0
    for filename in sorted(ENHANCED_PROMPTS.keys()):
        total += fix_file(filename)
    print(f"\nTotal: {total} examples updated across {len(ENHANCED_PROMPTS)} files")

if __name__ == "__main__":
    main()
