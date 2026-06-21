//! Unified POI Role Classification
//!
//! Provides a single, consistent role classification function that replaces
//! the three incompatible implementations scattered across the codebase:
//! - `infer_role_family_advanced()` in `llm_entity_extractor.rs`
//! - `detect_role_domain()` in `parse/src/person.rs`
//! - `classify_title_to_role_family()` in `parse/src/ner.rs`
//!
//! All three must produce the same `RoleFamily` for the same input title
//! to prevent silent data corruption when different pipelines classify the
//! same person differently.
//!
//! # Priority ordering
//!
//! The classifier uses a **contextual priority model** rather than a fixed
//! category-first approach. For compound titles (e.g., "VP Procurement"),
//! the **functional role domain** (Procurement, Supply Chain) takes precedence
//! over organizational rank (VP, Director) because the business purpose of
//! this classifier is to route contacts to the correct functional stakeholder.
//!
//! Priority logic:
//! 1. Check for explicit functional domain keywords FIRST
//!    (Procurement/Supply Chain, Quality/Compliance, Security/Cyber,
//!     Engineering, Operations, Finance, Sales, Legal, HR, Government)
//! 2. Only classify as Executive if NO functional domain is detected
//!    AND the title contains executive-level rank indicators (CEO, President,
//!    Managing Director, etc. WITHOUT a functional qualifier)
//! 3. Specialized categories (Logistics, FreeZone, Certification, Industry,
//!    Distributor)
//! 4. Other (original title as fallback)
//!
//! This means "VP Procurement" → Procurement (not Executive),
//! "Director of Supply Chain" → Procurement, "Plant Manager" → Operations,
//! but "Chief Executive Officer" → Executive, "President" → Executive.

use apex_core::entities::RoleFamily;

/// Classify a job title string into a `RoleFamily`.
///
/// This is the **single canonical classifier** for the ApexIntel system.
/// All code paths that need to determine a POI's role family must call
/// this function or a thin wrapper around it.
///
/// # Examples
/// ```
/// assert_eq!(classify_role("VP Procurement"), RoleFamily::Procurement);
/// assert_eq!(classify_role("Chief Technology Officer"), RoleFamily::Executive);
/// assert_eq!(classify_role("Quality Assurance Manager"), RoleFamily::SupplierQuality);
/// assert_eq!(classify_role("Plant Manager"), RoleFamily::Operations);
/// assert_eq!(classify_role("Unknown Job Title XYZ"), RoleFamily::Other("unknown job title xyz".to_string()));
/// ```
pub fn classify_role(title: &str) -> RoleFamily {
    let t = title.trim().to_lowercase();
    if t.is_empty() {
        return RoleFamily::Other("unknown".to_string());
    }

    // ── 0. Buyer-relevant functional domains take priority over C-suite rank ──
    // "Chief Procurement Officer" → Procurement (not Executive), because the
    // business purpose is to route contacts to the correct buyer. Only
    // non-buyer C-suite titles (CFO, CTO, etc.) classify as Executive.
    if is_procurement(&t) {
        return RoleFamily::Procurement;
    }

    // ── 0b. True C-suite titles (non-buyer) → Executive ──
    // "Chief Financial Officer", "CTO", "COO" → Executive. Checked AFTER
    // procurement so "Chief Procurement Officer" routes to Procurement.
    if is_c_suite(&t) {
        return RoleFamily::Executive;
    }

    // ── 1. Quality / Compliance ──
    if is_quality(&t) {
        return RoleFamily::SupplierQuality;
    }

    // ── 3. Security / Cyber ──
    if is_security(&t) {
        return RoleFamily::Security;
    }

    // ── 4. Engineering / Technical ──
    if is_engineering(&t) {
        return RoleFamily::Engineering;
    }

    // ── 4b. Logistics / Port / Transport (specialized — before Operations) ──
    if t.contains("logistics") || t.contains("shipping") || t.contains("freight")
        || t.contains("transport") || t.contains("port operations")
    {
        return RoleFamily::PortLogistics;
    }

    // ── 5. Operations / Manufacturing ──
    // NOTE: pure "logistics/shipping/freight/transport" titles are handled by
    // the specialized PortLogistics check (step 4b above) before this general
    // operations matcher, so they classify correctly as PortLogistics.
    if is_operations(&t) {
        return RoleFamily::Operations;
    }

    // ── 6. Finance ──
    if is_finance(&t) {
        return RoleFamily::Finance;
    }

    // ── 7. Sales / Marketing ──
    if is_sales(&t) {
        return RoleFamily::Other("Sales/Marketing".to_string());
    }

    // ── 8. Legal ──
    if is_legal(&t) {
        return RoleFamily::Legal;
    }

    // ── 9. HR / People ──
    if is_hr(&t) {
        return RoleFamily::Other("Human Resources".to_string());
    }

    // ── 9b. Free Zone Authority (specialized — before Government) ──
    if t.contains("free zone") || t.contains("freezone") || t.contains("economic zone") {
        return RoleFamily::FreeZoneAuthority;
    }

    // ── 10. Government / Defense ──
    if is_government(&t) {
        return RoleFamily::Government;
    }

    // ── 11. Executive / C-Suite (only if NO functional domain matched) ──
    // This is checked AFTER all functional domains because compound titles
    // like "VP Procurement" or "Director of Supply Chain" should route to
    // Procurement, not Executive. Only pure rank titles (CEO, President,
    // Chairman without a functional qualifier) land here.
    if is_executive(&t) {
        return RoleFamily::Executive;
    }

    // ── 12. Free Zone / Certification (specialized) ──
    // ── 12. Certification / Inspection (specialized) ──
    // (Logistics/Port handled at step 4b; Free Zone at step 9b.)
    if t.contains("certification") || t.contains("inspector") || t.contains("auditor") {
        return RoleFamily::CertificationBody;
    }

    // ── 13. Industry / Trade Associations ──
    if t.contains("trade association") || t.contains("industry association")
        || t.contains("chamber of commerce")
    {
        return RoleFamily::IndustryAssociation;
    }

    // ── 14. Distributor / Reseller ──
    if t.contains("distributor") || t.contains("reseller") || t.contains("wholesale")
        || t.contains("dealer")
    {
        return RoleFamily::Distributor;
    }

    // ── Fallback ──
    RoleFamily::Other(t)
}

// ── Category detection helpers ───────────────────────────────────────────────

/// Detect true C-suite titles that should ALWAYS classify as Executive,
/// regardless of any functional keyword they contain (e.g. "Chief Financial
/// Officer" → Executive, not Finance). This is narrower than `is_executive`
/// — it excludes VP, Director, General Manager, etc., so compound titles like
/// "VP Procurement" or "Director of Finance" still route to their functional
/// domain.
fn is_c_suite(t: &str) -> bool {
    // Exact CxO matches
    if t == "ceo" || t == "cto" || t == "cfo" || t == "coo" || t == "cio"
        || t == "ciso" || t == "cmo" || t == "cpo" || t == "cro" || t == "cso"
        || t == "chro" || t == "cao" || t == "cdo" || t == "cco"
    {
        return true;
    }
    // "Chief X Officer" pattern — the canonical C-suite form
    if t.starts_with("chief ") && t.contains("officer") {
        return true;
    }
    // "president" — but NOT "vice president" or "senior vice president"
    // (those are VP-level, handled by is_executive, not C-suite)
    if (t == "president" || t.starts_with("president ") || t.ends_with(" president"))
        && !t.contains("vice president") && !t.contains("svp") && !t.contains("evp")
    {
        return true;
    }
    // Chairman/Chairwoman/Chairperson and Managing/Executive Director
    let c_suite_keywords = [
        "chairman", "chairwoman", "chairperson",
        "managing director", "executive director",
    ];
    for kw in &c_suite_keywords {
        if t.contains(kw) {
            return true;
        }
    }
    false
}

fn is_executive(t: &str) -> bool {
    // Exact C-level matches
    if t == "ceo" || t == "cto" || t == "cfo" || t == "coo" || t == "cio"
        || t == "ciso" || t == "cmo" || t == "cpo" || t == "cro" || t == "cso"
        || t == "chro" || t == "cao" || t == "cdo" || t == "cco"
    {
        return true;
    }
    // Pattern matches for C-level with whitespace/punctuation
    let c_level_patterns = [
        "ceo ", " ceo", ".ceo", "/ceo",
        "cto ", " cto", ".cto", "/cto",
        "cfo ", " cfo", ".cfo", "/cfo",
        "coo ", " coo", ".coo", "/coo",
        "cio ", " cio", ".cio", "/cio",
        "ciso", "cmo ", " cmo", "cpo ", " cpo",
        "cro ", " cro", "cso ", " cso",
    ];
    for pat in &c_level_patterns {
        if t.contains(pat) { return true; }
    }

    // Executive title keywords
    let exec_keywords = [
        "chief ", "president", "chairman", "chairwoman", "chairperson",
        "managing director", "executive director", "board director",
        "vice president", "vp ", ".vp", "/vp", "senior vice president",
        "svp ", ".svp", "/svp", "executive vice president", "evp ",
        "owner", "founder", "co-founder", "managing partner", "partner",
        "general manager", "site director", "plant director",
        "division president", "group president",
    ];
    for kw in &exec_keywords {
        if t.contains(kw) { return true; }
    }

    false
}

fn is_procurement(t: &str) -> bool {
    let keywords = [
        "procurement", "sourcing", "purchasing", "buyer", "supply chain",
        "supplier", "vendor", "category manager", "commodity manager",
        "contract manager", "tender", "strategic sourcing",
        "global sourcing", "direct procurement", "indirect procurement",
        "supplier quality engineer", "supplier development",
        "purchasing manager", "procurement director",
        // "acquisition" is deliberately excluded — it matches "Talent Acquisition"
        // (HR) and "M&A Acquisition" (Strategy). Only compound forms like
        // "acquisition specialist" in a procurement context are procurement.
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_quality(t: &str) -> bool {
    let keywords = [
        "quality", "qa engineer", "qc inspector", "testing", "inspection",
        "compliance", "audit", "regulatory", "iso ", "safety",
        "environmental", "sustainability", "esg",
        "quality assurance", "quality control", "quality management",
        "six sigma", "lean ", "continuous improvement",
        "hse ", "ehs ", "health safety environment",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_security(t: &str) -> bool {
    let keywords = [
        "security", "cyber", "infosec", "privacy", "data protection",
        "threat", "vulnerability", "penetration test", "red team",
        "blue team", "soc ", "security operations",
        "information security", "network security",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_engineering(t: &str) -> bool {
    let keywords = [
        "engineer", "developer", "architect", "scientist", "programmer",
        "technical lead", "r&d", "research and development",
        "software", "hardware", "firmware", "embedded",
        "design engineer", "process engineer", "industrial engineer",
        "manufacturing engineer", "test engineer", "systems engineer",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_operations(t: &str) -> bool {
    let keywords = [
        "operations", "manufacturing", "production",
        "warehouse", "distribution",
        "plant manager", "factory", "facilities", "maintenance",
        "production manager", "operations director",
        "shop floor", "assembly", "fabrication",
        "shift supervisor", "line manager",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_finance(t: &str) -> bool {
    let keywords = [
        "finance", "accounting", "treasury", "controller", "bookkeeper",
        "tax", "auditor", "financial", "investment", "portfolio",
        "risk manager", "actuary",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_sales(t: &str) -> bool {
    let keywords = [
        "sales", "marketing", "business development", "account manager",
        "customer", "commercial", "revenue", "brand", "product manager",
        "growth", "market", "channel", "partnership",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_legal(t: &str) -> bool {
    let keywords = [
        "legal", "counsel", "attorney", "lawyer", "paralegal",
        "intellectual property", "patent", "trademark", "litigation",
        "contract", "general counsel",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_hr(t: &str) -> bool {
    let keywords = [
        "hr ", "human resources", "talent", "recruiting", "people",
        "payroll", "compensation", "benefits", "learning and development",
        "organizational development",
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

fn is_government(t: &str) -> bool {
    let keywords = [
        "government", "minister", "ambassador", "regulatory",
        "public policy", "authority", "agency",
        "defense", "military", "intelligence", "diplomat",
        "civil service", "public sector", "administration",
        // NOTE: "free zone" is NOT here — it has a dedicated FreeZoneAuthority
        // classification checked before this function.
    ];
    for kw in &keywords {
        if t.contains(kw) { return true; }
    }
    false
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Executive (pure rank, no functional qualifier) ──
    #[test]
    fn test_classify_ceo() {
        assert_eq!(classify_role("Chief Executive Officer"), RoleFamily::Executive);
        assert_eq!(classify_role("CEO"), RoleFamily::Executive);
        assert_eq!(classify_role("CEO & Founder"), RoleFamily::Executive);
    }

    #[test]
    fn test_classify_cto_cfo_coo() {
        assert_eq!(classify_role("CTO"), RoleFamily::Executive);
        assert_eq!(classify_role("Chief Financial Officer"), RoleFamily::Executive);
        assert_eq!(classify_role("COO"), RoleFamily::Executive);
    }

    #[test]
    fn test_classify_vp_president() {
        // "Vice President of Engineering" — "engineering" matches functional domain first → Engineering
        // But "vice president" in is_executive matches, and engineering is checked before executive.
        // Actually: is_engineering contains "engineer" which matches "engineering" → Engineering
        assert_eq!(classify_role("Vice President of Engineering"), RoleFamily::Engineering);
        // "SVP Marketing" — "marketing" matches sales → Sales/Marketing
        assert_eq!(classify_role("SVP Marketing"), RoleFamily::Other("Sales/Marketing".to_string()));
        // "President" alone → Executive
        assert_eq!(classify_role("President"), RoleFamily::Executive);
    }

    #[test]
    fn test_classify_managing_director() {
        assert_eq!(classify_role("Managing Director"), RoleFamily::Executive);
        assert_eq!(classify_role("General Manager"), RoleFamily::Executive);
    }

    // ── Compound titles: functional domain takes priority over rank ──
    #[test]
    fn test_compound_title_functional_priority() {
        // VP Procurement → Procurement (NOT Executive)
        assert_eq!(classify_role("VP Procurement"), RoleFamily::Procurement);
        // Director of Supply Chain → Procurement
        assert_eq!(classify_role("Director of Supply Chain"), RoleFamily::Procurement);
        // VP Operations → Operations (ops keywords match before executive check)
        assert_eq!(classify_role("VP Operations"), RoleFamily::Operations);
        // VP of Finance → Finance
        assert_eq!(classify_role("VP of Finance"), RoleFamily::Finance);
        // SVP of Sales → Sales/Marketing
        assert_eq!(classify_role("SVP of Sales"), RoleFamily::Other("Sales/Marketing".to_string()));
        // Chief Procurement Officer → Procurement (procurement keyword before exec check)
        assert_eq!(classify_role("Chief Procurement Officer"), RoleFamily::Procurement);
    }

    // ── Procurement ──
    #[test]
    fn test_classify_procurement() {
        assert_eq!(classify_role("VP Procurement"), RoleFamily::Procurement);
        assert_eq!(classify_role("Head of Supply Chain"), RoleFamily::Procurement);
        assert_eq!(classify_role("Purchasing Manager"), RoleFamily::Procurement);
        assert_eq!(classify_role("Sourcing Director"), RoleFamily::Procurement);
        assert_eq!(classify_role("Category Manager"), RoleFamily::Procurement);
    }

    #[test]
    fn test_classify_buyer() {
        assert_eq!(classify_role("Senior Buyer"), RoleFamily::Procurement);
        assert_eq!(classify_role("Strategic Sourcing Lead"), RoleFamily::Procurement);
    }

    // ── Quality ──
    #[test]
    fn test_classify_quality() {
        assert_eq!(classify_role("Director of Quality Assurance"), RoleFamily::SupplierQuality);
        assert_eq!(classify_role("QA Engineer"), RoleFamily::SupplierQuality);
        assert_eq!(classify_role("Compliance Officer"), RoleFamily::SupplierQuality);
        assert_eq!(classify_role("ISO Auditor"), RoleFamily::SupplierQuality);
    }

    // ── Security ──
    #[test]
    fn test_classify_security() {
        assert_eq!(classify_role("CISO"), RoleFamily::Executive); // Exec takes priority
        assert_eq!(classify_role("Security Engineer"), RoleFamily::Security);
        assert_eq!(classify_role("Cyber Security Analyst"), RoleFamily::Security);
    }

    // ── Engineering ──
    #[test]
    fn test_classify_engineering() {
        assert_eq!(classify_role("Software Engineer"), RoleFamily::Engineering);
        assert_eq!(classify_role("Systems Architect"), RoleFamily::Engineering);
        assert_eq!(classify_role("R&D Scientist"), RoleFamily::Engineering);
    }

    // ── Operations ──
    #[test]
    fn test_classify_operations() {
        // VP Operations → Operations (functional domain "operations" takes priority)
        assert_eq!(classify_role("VP Operations"), RoleFamily::Operations);
        assert_eq!(classify_role("Plant Manager"), RoleFamily::Operations);
        assert_eq!(classify_role("Production Supervisor"), RoleFamily::Operations);
        assert_eq!(classify_role("Warehouse Manager"), RoleFamily::Operations);
    }

    // ── Finance ──
    #[test]
    fn test_classify_finance() {
        assert_eq!(classify_role("Finance Director"), RoleFamily::Finance);
        assert_eq!(classify_role("Controller"), RoleFamily::Finance);
        assert_eq!(classify_role("Tax Manager"), RoleFamily::Finance);
    }

    // ── Sales ──
    #[test]
    fn test_classify_sales() {
        assert_eq!(classify_role("Sales Director"), RoleFamily::Other("Sales/Marketing".to_string()));
        assert_eq!(classify_role("Marketing Manager"), RoleFamily::Other("Sales/Marketing".to_string()));
    }

    // ── Legal ──
    #[test]
    fn test_classify_legal() {
        assert_eq!(classify_role("General Counsel"), RoleFamily::Legal);
        assert_eq!(classify_role("Intellectual Property Attorney"), RoleFamily::Legal);
    }

    // ── HR ──
    #[test]
    fn test_classify_hr() {
        assert_eq!(classify_role("HR Director"), RoleFamily::Other("Human Resources".to_string()));
        assert_eq!(classify_role("Talent Acquisition Manager"), RoleFamily::Other("Human Resources".to_string()));
    }

    // ── Government ──
    #[test]
    fn test_classify_government() {
        assert_eq!(classify_role("Government Relations Director"), RoleFamily::Government);
        assert_eq!(classify_role("Minister of Trade"), RoleFamily::Government);
        assert_eq!(classify_role("Defense Attaché"), RoleFamily::Government);
    }

    // ── Specialized ──
    #[test]
    fn test_classify_logistics() {
        assert_eq!(classify_role("Logistics Manager"), RoleFamily::PortLogistics);
        assert_eq!(classify_role("Shipping Coordinator"), RoleFamily::PortLogistics);
    }

    #[test]
    fn test_classify_free_zone() {
        assert_eq!(classify_role("Free Zone Authority Director"), RoleFamily::FreeZoneAuthority);
    }

    #[test]
    fn test_classify_certification() {
        assert_eq!(classify_role("Lead Auditor"), RoleFamily::SupplierQuality); // quality first
        assert_eq!(classify_role("Certification Inspector"), RoleFamily::CertificationBody);
    }

    // ── Edge cases ──
    #[test]
    fn test_classify_empty_title() {
        assert_eq!(classify_role(""), RoleFamily::Other("unknown".to_string()));
    }

    #[test]
    fn test_classify_unknown_title() {
        assert_eq!(
            classify_role("Something Completely Unknown 123"),
            RoleFamily::Other("something completely unknown 123".to_string())
        );
    }

    #[test]
    fn test_classify_whitespace_handling() {
        assert_eq!(classify_role("  CEO  "), RoleFamily::Executive);
    }

    // ── Consistency test: same title must produce same result each time ──
    #[test]
    fn test_classify_deterministic() {
        let titles = vec![
            "VP Procurement",
            "Director of Quality Assurance",
            "Supply Chain Manager",
            "CISO",
            "Plant Manager",
            "Finance Director",
            "General Counsel",
            "Free Zone Authority Director",
        ];
        for title in titles {
            let first = classify_role(title);
            let second = classify_role(title);
            assert_eq!(first, second, "classify_role must be deterministic for '{}'", title);
        }
    }
}