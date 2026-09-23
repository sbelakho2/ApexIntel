use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonNameScript {
    Latin,
    Arabic,
    Hebrew,
    Cyrillic,
    Cjk,
    Hangul,
    Mixed,
    Unknown,
}

const CONNECTOR_WORDS: &[&str] = &[
    "al", "bin", "ibn", "ben", "bint", "de", "del", "da", "dos", "van", "von", "der", "la", "le",
];

const BLOCKED_TERMS: &[&str] = &[
    // English company suffixes
    "holdings",
    "limited",
    "ltd",
    "inc",
    "corp",
    "corporation",
    "group",
    "company",
    "systems",
    "technologies",
    "technology",
    "industries",
    "industrial",
    "partners",
    "capital",
    "ventures",
    "manufacturing",
    "electronics",
    // Non-English company suffixes (≥4 chars, safe for substring match)
    "gmbh",  // German
    "sarl",  // French
    "spzoo", // Polish (sp. z o.o.)
];

/// Short company suffixes that must match as whole words only.
const BLOCKED_WORD_TERMS: &[&str] = &[
    "ag", "sa", "sas", "bv", "nv", "spa", "srl", "sl", "oo", "za", "ao", "kft", "rt", "as", "ab",
    "oy", "pty", "cc", "co", "plc", "llp", "lp", "llc", "jsc", "kda", "ykk", "kk",
];

const BLOCKED_PHRASES: &[&str] = &[
    "artificial intelligence",
    "computer science",
    "foreign policy",
    "middle east",
    "new york",
    "open source",
    "social media",
    "united states",
    "united kingdom",
    "european union",
    "north america",
    "south america",
    "african union",
];

/// Role/title words that, when present in a "name", strongly indicate the
/// string is a job title rather than a person name (e.g. "Director
/// Operations", "Head of Procurement").
const ROLE_LABEL_WORDS: &[&str] = &[
    "director",
    "manager",
    "officer",
    "president",
    "chairman",
    "chairperson",
    "secretary",
    "head",
    "chief",
    "lead",
    "leader",
    "supervisor",
    "coordinator",
    "administrator",
    "controller",
    "inspector",
    "auditor",
    "analyst",
    "specialist",
    "engineer",
    "architect",
    "consultant",
    "advisor",
    "adviser",
    "assistant",
    "deputy",
    "associate",
    "executive",
    "representative",
    "agent",
    "liaison",
    "principal",
    "partner",
    "senior",
    "junior",
    "vp",
    "ceo",
    "cfo",
    "cto",
    "coo",
    "cio",
    "cmo",
    "gm",
];

/// Role-domain / function words. These are not titles on their own, but when a
/// title word ("head", "vp", "director") co-occurs with one of these, the
/// string is overwhelmingly a role description, not a person name:
/// "Head of Procurement", "VP Global Supply Chain", "Director of Sourcing".
const ROLE_DOMAIN_WORDS: &[&str] = &[
    "procurement",
    "purchasing",
    "sourcing",
    "category",
    "commodity",
    "supply",
    "chain",
    "operations",
    "logistics",
    "quality",
    "compliance",
    "legal",
    "finance",
    "financial",
    "engineering",
    "technical",
    "technology",
    "global",
    "strategy",
    "business",
    "sales",
    "marketing",
    "commercial",
    "risk",
    "audit",
    "talent",
    "people",
    "human",
    "resources",
    "relations",
    "officer", // "Chief ... Officer"
];

/// Facility / infrastructure project name suffixes that indicate a string is
/// a project, facility, or place — not a person.
const FACILITY_SUFFIXES: &[&str] = &[
    "farm",
    "park",
    "plant",
    "station",
    "project",
    "phase",
    "unit",
    "field",
    "mine",
    "mill",
    "refinery",
    "terminal",
    "depot",
    "facility",
    "complex",
    "center",
    "centre",
    "tower",
    "dam",
    "reservoir",
    "pipeline",
    "grid",
    "line",
    "corridor",
    "zone",
    "district",
    "estate",
    "port",
    "airport",
    "harbor",
    "harbour",
    "warehouse",
    "factory",
    "works",
    "lab",
    "laboratory",
    "institute",
    "academy",
    "university",
    "school",
    "college",
    "hospital",
    "clinic",
    "mall",
    "plaza",
    "square",
    "boulevard",
    "avenue",
];

/// Directional/cardinal prefixes common in infrastructure project names
/// ("West Bakr", "North Field", "South Phase").
const DIRECTIONAL_PREFIXES: &[&str] = &[
    "north", "south", "east", "west", "central", "upper", "lower", "greater", "metro", "old", "new",
];

/// A curated gazetteer of place names (regions, provinces, major cities)
/// from countries in ApexIntel's entity universe. A candidate whose
/// normalized name matches any entry here is NOT a person.
///
/// This is not exhaustive GeoNames — it targets the specific countries
/// whose place names appeared as junk POIs: Vietnam, Egypt, Morocco,
/// Tunisia, Czech Republic, plus common false-positive regions worldwide.
const PLACE_NAME_GAZETTEER: &[&str] = &[
    // ── Vietnam: all 63 provinces ──
    "an giang",
    "ba ria vung tau",
    "bac giang",
    "bac kan",
    "bac lieu",
    "bac ninh",
    "ben tre",
    "binh dinh",
    "binh duong",
    "binh phuoc",
    "binh thuan",
    "ca mau",
    "can tho",
    "cao bang",
    "da nang",
    "dak lak",
    "dak nong",
    "dien bien",
    "dong nai",
    "dong thap",
    "gia lai",
    "ha giang",
    "ha nam",
    "ha tinh",
    "hai duong",
    "hai phong",
    "hau giang",
    "hanoi",
    "ho chi minh",
    "hoa binh",
    "hung yen",
    "khanh hoa",
    "kien giang",
    "kon tum",
    "lai chau",
    "lam dong",
    "lang son",
    "lao cai",
    "long an",
    "nam dinh",
    "nghe an",
    "ninh binh",
    "ninh thuan",
    "phu tho",
    "phu yen",
    "quang binh",
    "quang nam",
    "quang ngai",
    "quang ninh",
    "quang tri",
    "soc trang",
    "son la",
    "tay ninh",
    "thai binh",
    "thai nguyen",
    "thanh hoa",
    "thua thien hue",
    "tien giang",
    "tra vinh",
    "tuyen quang",
    "vinh long",
    "vinh phuc",
    "yen bai",
    // ── Egypt: governorates ──
    "alexandria",
    "aswan",
    "asyut",
    "beheira",
    "beni suef",
    "cairo",
    "dakahlia",
    "damietta",
    "faiyum",
    "gharbia",
    "giza",
    "ismailia",
    "kafr el sheikh",
    "luxor",
    "matrouh",
    "minya",
    "monufia",
    "new valley",
    "north sinai",
    "port said",
    "qalyubia",
    "qena",
    "red sea",
    "sharqia",
    "sohag",
    "south sinai",
    "suez",
    // ── Morocco: regions ──
    "casablanca settat",
    "draa tafilalet",
    "fes meknes",
    "guelmim oud noun",
    "laayoune sakia el hamra",
    "marrakech safi",
    "oriental",
    "rabat sale kenitra",
    "souss massa",
    "tanger tetouan al hoceima",
    "beni mellal khenifra",
    "dakhla oued ed dahab",
    "casablanca",
    "rabat",
    "marrakech",
    "fez",
    "tangier",
    "agadir",
    "meknes",
    "oujda",
    "kenitra",
    "tetouan",
    "safi",
    "el jadida",
    "nador",
    "settat",
    "berkan",
    "khouribga",
    "mohammedia",
    "laayoune",
    "dakhla",
    // ── Tunisia: governorates ──
    "tunis",
    "ariana",
    "ben arous",
    "manouba",
    "nabeul",
    "zaghouan",
    "bizerte",
    "beja",
    "jendouba",
    "kef",
    "siliana",
    "sousse",
    "monastir",
    "mahdia",
    "sfax",
    "kairouan",
    "kasserine",
    "sidi bouzid",
    "gabes",
    "medenine",
    "tataouine",
    "gafsa",
    "tozeur",
    "kebili",
    "gabès",
    "médenine",
    "tatouine",
    "kairouan",
    // ── Czech Republic: regions ──
    "prague",
    "central bohemia",
    "south bohemia",
    "plzen",
    "karlovy vary",
    "usti nad labem",
    "liberec",
    "hradec kralove",
    "pardubice",
    "vysocina",
    "south moravia",
    "olomouc",
    "moravia silesia",
    "zin",
    "hradec kralove",
    "pardubice",
    // ── Major false-positive cities worldwide ──
    "sao paulo",
    "rio de janeiro",
    "buenos aires",
    "mexico city",
    "santiago",
    "bogota",
    "lima",
    "caracas",
    "havana",
    "istanbul",
    "ankara",
    "izmir",
    "bursa",
    "lagos",
    "nairobi",
    "addis ababa",
    "accra",
    "dakar",
    "abidjan",
    "riyadh",
    "dubai",
    "doha",
    "kuwait city",
    "muscat",
    "amman",
    "beirut",
    "damascus",
    "baghdad",
    "tehran",
    "mumbai",
    "delhi",
    "bangalore",
    "chennai",
    "hyderabad",
    "kolkata",
    "singapore",
    "jakarta",
    "manila",
    "bangkok",
    "kuala lumpur",
    "hong kong",
    "shanghai",
    "beijing",
    "shenzhen",
    "guangzhou",
    "tokyo",
    "osaka",
    "seoul",
    "taipei",
    "sydney",
    "melbourne",
    "auckland",
    "berlin",
    "munich",
    "frankfurt",
    "hamburg",
    "cologne",
    "stuttgart",
    "paris",
    "marseille",
    "lyon",
    "toulouse",
    "nice",
    "bordeaux",
    "milan",
    "rome",
    "naples",
    "turin",
    "florence",
    "madrid",
    "barcelona",
    "valencia",
    "seville",
    "amsterdam",
    "rotterdam",
    "brussels",
    "antwerp",
    "zurich",
    "geneva",
    "vienna",
    "prague",
    "warsaw",
    "stockholm",
    "oslo",
    "copenhagen",
    "helsinki",
    "moscow",
    "st petersburg",
    "kiev",
    "minsk",
];

/// Check whether a candidate "name" is actually a place name by consulting
/// the gazetteer. Returns true if it IS a place name (i.e., should be rejected).
pub fn is_place_name(name: &str) -> bool {
    let normalized = normalize_person_name(name).to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    // Direct match against gazetteer
    if PLACE_NAME_GAZETTEER.contains(&normalized.as_str()) {
        return true;
    }
    // Check each 2-word subsequence (handles "Bac Lieu Province" etc.)
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.len() >= 2 {
        for i in 0..words.len().saturating_sub(1) {
            let pair = format!("{} {}", words[i], words[i + 1]);
            if PLACE_NAME_GAZETTEER.contains(&pair.as_str()) {
                return true;
            }
        }
    }
    // Check if all words are in the gazetteer individually (handles
    // "Thai Nguyen" where both words appear separately)
    if words.len() >= 2 && words.iter().all(|w| w.len() >= 3) {
        let all_place_words = words.iter().all(|w| {
            PLACE_NAME_GAZETTEER
                .iter()
                .any(|entry| entry.split_whitespace().any(|ew| ew == *w))
        });
        if all_place_words {
            return true;
        }
    }
    false
}

/// Check whether a candidate "name" is actually a job title / role label.
/// Returns true if it looks like a role string (should be rejected as a name).
pub fn is_role_label(name: &str) -> bool {
    let normalized = normalize_person_name(name).to_lowercase();
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    let clean_words: Vec<String> = words
        .iter()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .collect();

    let role_word_count = clean_words
        .iter()
        .filter(|w| ROLE_LABEL_WORDS.contains(&w.as_str()))
        .count();
    if role_word_count == 0 {
        return false;
    }

    // A title word present is a strong signal. Two sub-cases:
    //  1) Title words are the majority of words ("Chief Financial Officer",
    //     "Senior Manager") → role.
    //  2) A title word co-occurs with a role-domain word ("Head of
    //     Procurement", "VP Global Supply Chain", "Director of Sourcing")
    //     → role, even when title words are not the majority.
    let has_domain_word = clean_words
        .iter()
        .any(|w| ROLE_DOMAIN_WORDS.contains(&w.as_str()));
    let role_is_majority = role_word_count as f64 / words.len() as f64 >= 0.5;
    role_is_majority || has_domain_word
}

/// Check whether a candidate name looks like a facility or infrastructure
/// project name ("West Bakr Wind Farm", "North Field Phase 2").
/// Returns true if it matches facility/project patterns.
pub fn is_facility_or_project_name(name: &str) -> bool {
    let normalized = normalize_person_name(name).to_lowercase();
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    // Check for facility suffixes anywhere in the name
    if words
        .iter()
        .any(|w| FACILITY_SUFFIXES.contains(&w.trim_matches(|c: char| !c.is_alphanumeric())))
    {
        return true;
    }
    // Check for directional prefix + capitalized proper noun pattern
    // ("West Bakr", "North Field")
    if words.len() >= 2 {
        let first_lower = words[0].to_lowercase();
        if DIRECTIONAL_PREFIXES.contains(&first_lower.as_str()) {
            return true;
        }
    }
    // Check for phase/unit patterns
    if normalized.contains("phase ")
        || normalized.contains(" unit ")
        || normalized.contains(" stage ")
        || normalized.contains(" block ")
    {
        return true;
    }
    // Check for "X Wind" / "X Solar" / "X Power" energy-project patterns
    let energy_words = ["wind", "solar", "power", "energy", "gas", "thermal"];
    if words.iter().any(|w| energy_words.contains(w)) {
        // Only reject if there's also a proper-noun component (not "Solar Energy Inc")
        let generic_words = [
            "wind", "solar", "power", "energy", "gas", "thermal", "farm", "plant", "project",
            "group", "the", "and", "of",
        ];
        let has_proper_noun = words.iter().any(|w| !generic_words.contains(w));
        if has_proper_noun {
            return true;
        }
    }
    false
}

/// Comprehensive junk-name rejection. Combines all deterministic checks:
/// place names, role labels, facility/project names. Returns true if the
/// name should be REJECTED (it's not a person).
pub fn is_non_person_string(name: &str) -> bool {
    is_place_name(name) || is_role_label(name) || is_facility_or_project_name(name)
}

pub fn detect_person_name_script(value: &str) -> PersonNameScript {
    let mut latin = 0;
    let mut arabic = 0;
    let mut hebrew = 0;
    let mut cyrillic = 0;
    let mut cjk = 0;
    let mut hangul = 0;
    let total = value.chars().filter(|c| !c.is_whitespace()).count();

    if total == 0 {
        return PersonNameScript::Unknown;
    }

    for ch in value.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let cp = ch as u32;
        if (0x0600..=0x06FF).contains(&cp)
            || (0x0750..=0x077F).contains(&cp)
            || (0x08A0..=0x08FF).contains(&cp)
        {
            arabic += 1;
        } else if (0x0590..=0x05FF).contains(&cp) || (0xFB1D..=0xFB4F).contains(&cp) {
            hebrew += 1;
        } else if (0x0400..=0x052F).contains(&cp)
            || (0x2DE0..=0x2DFF).contains(&cp)
            || (0xA640..=0xA69F).contains(&cp)
        {
            cyrillic += 1;
        } else if (0x4E00..=0x9FFF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0x3040..=0x30FF).contains(&cp)
        {
            cjk += 1;
        } else if (0xAC00..=0xD7AF).contains(&cp) || (0x1100..=0x11FF).contains(&cp) {
            hangul += 1;
        } else if ch.is_alphabetic() || (0x00C0..=0x024F).contains(&cp) {
            latin += 1;
        }
    }

    let threshold = total / 2;
    if arabic > threshold {
        PersonNameScript::Arabic
    } else if hebrew > threshold {
        PersonNameScript::Hebrew
    } else if cyrillic > threshold {
        PersonNameScript::Cyrillic
    } else if hangul > threshold {
        PersonNameScript::Hangul
    } else if cjk > threshold {
        PersonNameScript::Cjk
    } else if latin > threshold {
        PersonNameScript::Latin
    } else {
        PersonNameScript::Mixed
    }
}

pub fn normalize_person_name(value: &str) -> String {
    value
        .nfkd()
        .filter(|ch| !is_combining_mark(*ch))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

pub fn looks_like_person_name(value: &str) -> bool {
    let normalized = normalize_person_name(value);
    let cleaned = normalized.trim_matches(|c: char| !c.is_alphanumeric() && !c.is_alphabetic());
    if cleaned.is_empty() || cleaned.len() > 80 || cleaned.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }

    // Deterministic junk rejection: place names, role labels, facility/project names
    if is_non_person_string(cleaned) {
        return false;
    }

    let lowered = cleaned.to_lowercase();
    if BLOCKED_PHRASES.contains(&lowered.as_str())
        || BLOCKED_TERMS.iter().any(|term| lowered.contains(term))
        || lowered
            .split_whitespace()
            .any(|word| BLOCKED_WORD_TERMS.contains(&word))
    {
        return false;
    }

    let script = detect_person_name_script(cleaned);
    match script {
        PersonNameScript::Cjk | PersonNameScript::Hangul => {
            let chars: Vec<char> = cleaned.chars().filter(|c| !c.is_whitespace()).collect();
            !chars.is_empty() && chars.len() <= 6 && chars.iter().all(|c| c.is_alphabetic())
        }
        PersonNameScript::Unknown => false,
        _ => {
            let parts: Vec<&str> = cleaned
                .split_whitespace()
                .map(|part| {
                    part.trim_matches(|c: char| {
                        !c.is_alphabetic() && c != '-' && c != '\'' && c != '’' && c != '·'
                    })
                })
                .filter(|part| !part.is_empty())
                .collect();

            if parts.len() < 2 || parts.len() > 5 {
                return false;
            }

            let total_chars: usize = parts.iter().map(|part| part.chars().count()).sum();
            if total_chars < 4 {
                return false;
            }

            parts.iter().all(|part| token_is_person_like(part, script))
        }
    }
}

fn token_is_person_like(token: &str, script: PersonNameScript) -> bool {
    if token.is_empty() {
        return false;
    }

    let lowered = token.to_lowercase();
    if CONNECTOR_WORDS.contains(&lowered.as_str()) {
        return true;
    }

    if !token
        .chars()
        .all(|c| c.is_alphabetic() || c == '-' || c == '\'' || c == '’' || c == '·')
    {
        return false;
    }

    match script {
        PersonNameScript::Latin | PersonNameScript::Cyrillic | PersonNameScript::Mixed => token
            .chars()
            .next()
            .map(|first| first.is_uppercase())
            .unwrap_or(false),
        PersonNameScript::Arabic | PersonNameScript::Hebrew => token.chars().count() >= 2,
        PersonNameScript::Cjk | PersonNameScript::Hangul => true,
        PersonNameScript::Unknown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_name_is_person_like() {
        assert!(looks_like_person_name("Ahmed Ben Ali"));
    }

    #[test]
    fn arabic_name_is_person_like() {
        assert!(looks_like_person_name("محمد بن سالم"));
    }

    #[test]
    fn hebrew_name_is_person_like() {
        assert!(looks_like_person_name("משה דוד"));
    }

    #[test]
    fn cyrillic_name_is_person_like() {
        assert!(looks_like_person_name("Алексей Иванов"));
    }

    #[test]
    fn cjk_name_is_person_like() {
        assert!(looks_like_person_name("王小明"));
    }

    #[test]
    fn company_suffix_is_rejected() {
        assert!(!looks_like_person_name("Atlas Holdings"));
    }

    #[test]
    fn normalize_person_name_strips_diacritics() {
        assert_eq!(normalize_person_name("José García"), "Jose Garcia");
    }

    // ── Junk rejection tests ──

    #[test]
    fn rejects_vietnamese_provinces() {
        assert!(!looks_like_person_name("Bac Lieu"));
        assert!(!looks_like_person_name("Ca Mau"));
        assert!(!looks_like_person_name("Quang Ninh"));
        assert!(!looks_like_person_name("Thai Nguyen"));
        assert!(!looks_like_person_name("Tuyen Quang"));
        assert!(!looks_like_person_name("Cao Bang"));
    }

    #[test]
    fn rejects_place_names_worldwide() {
        assert!(!looks_like_person_name("Sao Paulo"));
        assert!(!looks_like_person_name("Casablanca"));
        assert!(!looks_like_person_name("Istanbul"));
    }

    #[test]
    fn rejects_role_strings_as_names() {
        assert!(!looks_like_person_name("Director Operations"));
        assert!(!looks_like_person_name("Head of Procurement"));
        assert!(!looks_like_person_name("Chief Financial Officer"));
        assert!(!looks_like_person_name("VP Global Supply Chain"));
        assert!(!looks_like_person_name("Senior Manager"));
    }

    #[test]
    fn rejects_project_facility_names() {
        assert!(!looks_like_person_name("West Bakr Lekela"));
        assert!(!looks_like_person_name("West Bakr"));
        assert!(!looks_like_person_name("North Field Phase 2"));
        assert!(!looks_like_person_name("Beni Haroun Dam"));
        assert!(!looks_like_person_name("Noor Solar Plant"));
    }

    #[test]
    fn accepts_real_person_names() {
        assert!(looks_like_person_name("Ahmed Ben Ali"));
        assert!(looks_like_person_name("Jennifer Samproni"));
        assert!(looks_like_person_name("Christopher Kubasik"));
        assert!(looks_like_person_name("Wong Chee Kheong"));
        assert!(looks_like_person_name("Nicolas Denis"));
        assert!(looks_like_person_name("Jure Sola"));
    }

    #[test]
    fn is_non_person_string_detects_all_junk_classes() {
        assert!(is_non_person_string("Bac Lieu"));
        assert!(is_non_person_string("West Bakr Wind Farm"));
        assert!(is_non_person_string("Head of Procurement"));
        assert!(!is_non_person_string("David Smith"));
        assert!(!is_non_person_string("Jennifer Samproni"));
    }

    // ── Regression tests for junk classes seen in production ──

    #[test]
    fn rejects_exact_junk_from_role_profile_migration() {
        // These are the exact "names" the 20260624 cleanup migration deleted.
        // They must never be re-ingested. The deterministic filter catches the
        // obvious junk (role strings stored as names). Ambiguous transliterations
        // like "Nguyen Hong Sam" or "Aioliki Aderes" that genuinely look like
        // names are left to the post-LLM backstop, not this filter.
        assert!(!looks_like_person_name("Chief Procurement Officer"));
        assert!(!looks_like_person_name("VP Global Procurement"));
        assert!(!looks_like_person_name("VP Global Supply Chain"));
        assert!(!looks_like_person_name("VP Procurement"));
        assert!(!looks_like_person_name("VP Supply Chain Management"));
    }

    #[test]
    fn rejects_role_domain_combinations() {
        // Title word + domain word → role string, even when title is not the
        // majority of words.
        assert!(!looks_like_person_name("Director of Sourcing"));
        assert!(!looks_like_person_name("VP Global Procurement"));
        assert!(!looks_like_person_name("Head of Purchasing"));
        assert!(!looks_like_person_name("Manager Category Management"));
        assert!(!looks_like_person_name("Chief Financial Officer"));
    }

    #[test]
    fn does_not_reject_real_names_containing_role_words_incidentally() {
        // A person whose surname happens to coincide with a title/domain word
        // must still pass when the overall string is clearly a name.
        assert!(looks_like_person_name("Harrison Ford"));
        assert!(looks_like_person_name("Aria Strategy"));
    }
}
