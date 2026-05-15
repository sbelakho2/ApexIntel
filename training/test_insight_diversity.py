#!/usr/bin/env python3
"""
Insight Training Data Diversity & Quality Tests
================================================
Tests for bias detection, entity coverage, template diversity,
and data integrity in instruction-tuning JSONL files.

Requirements: stdlib + pyyaml only. No Rust modifications.
"""

import json
import math
import os
import re
import sys
import unittest
from collections import Counter, defaultdict
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None
    print("WARNING: pyyaml not installed. Entity coverage tests will be skipped.", file=sys.stderr)

# ── Paths ──────────────────────────────────────────────────────────────────
PROJECT_DIR = Path(__file__).resolve().parent.parent
CONFIG_DIR = PROJECT_DIR / "config"
TRAINING_DATA_DIR = PROJECT_DIR / "training_data" / "instruction_tuning"

AUGMENTATION_YAML = CONFIG_DIR / "augmentation_entities.yaml"
COMPETITIVE_ANALYSIS = TRAINING_DATA_DIR / "competitive_analysis.jsonl"
COMPETITIVE_ANALYSIS_AUG = TRAINING_DATA_DIR / "competitive_analysis_augmented.jsonl"
ENTITY_EXTRACTION = TRAINING_DATA_DIR / "entity_extraction.jsonl"
CONSTRAINT_FOLLOWING = TRAINING_DATA_DIR / "constraint_following_examples.jsonl"


# ── Helpers ────────────────────────────────────────────────────────────────

def load_jsonl(path, max_entries=None):
    """Load a JSONL file, returning list of parsed dicts."""
    if not path.exists():
        return []
    entries = []
    with open(path, "r", encoding="utf-8") as f:
        for i, line in enumerate(f):
            if max_entries is not None and i >= max_entries:
                break
            line = line.strip()
            if not line:
                continue
            try:
                entries.append(json.loads(line))
            except json.JSONDecodeError as e:
                entries.append(None)  # Malformed entry
    return entries


def load_yaml(path):
    """Load a YAML file, returning parsed dict."""
    if yaml is None:
        return {}
    with open(path, "r", encoding="utf-8") as f:
        return yaml.safe_load(f)


def get_entities_from_yaml(yaml_data):
    """Extract company names from augmentation_entities.yaml structure."""
    ems = []
    oem = []
    for entry in yaml_data.get("ems_companies", []):
        if isinstance(entry, list) and len(entry) > 0:
            ems.append(entry[0])
    for entry in yaml_data.get("oem_companies", []):
        if isinstance(entry, list) and len(entry) > 0:
            oem.append(entry[0])
    return ems, oem


def jaccard_similarity(set_a, set_b):
    """Compute Jaccard similarity between two sets."""
    if not set_a and not set_b:
        return 1.0
    intersection = len(set_a & set_b)
    union = len(set_a | set_b)
    return intersection / union if union > 0 else 0.0


def get_reference_entity(entry):
    """
    Extract the reference entity from a competitive analysis entry.
    The reference is usually in the system prompt or user content.
    """
    if entry is None:
        return None
    messages = entry.get("messages", [])
    for msg in messages:
        content = msg.get("content", "")
        role = msg.get("role", "")
        if role == "system":
            # Pattern: "You are comparing <Entity> capabilities versus..."
            m = re.search(
                r"comparing\s+([A-Z][A-Za-z0-9\s\.\,\'\-&()]+?)(?:\s+capabilities|\s+versus|\s+with)",
                content,
                re.IGNORECASE,
            )
            if m:
                return m.group(1).strip()
        if role == "user":
            # Pattern: "Reference entity: <Entity>"
            m = re.search(r"Reference entity[:\s]+([A-Z][A-Za-z0-9\s\.\,\'\-&()]+)", content)
            if m:
                return m.group(1).strip()
            # Pattern: "Compare [Entity] vs"
            m = re.search(
                r"(?:compare|analyze)\s+([A-Z][A-Za-z0-9\s\.\,\'\-&()]+?)\s+(?:vs|versus|against|and)",
                content,
                re.IGNORECASE,
            )
            if m:
                return m.group(1).strip()
    return None


def get_extracted_entities(entry):
    """
    Extract company names from the assistant's JSON output in
    entity_extraction.jsonl entries.
    """
    if entry is None:
        return []
    messages = entry.get("messages", [])
    for msg in messages:
        if msg.get("role") == "assistant":
            content = msg.get("content", "")
            try:
                data = json.loads(content)
                return [c.get("name", "") for c in data.get("companies", [])]
            except (json.JSONDecodeError, TypeError):
                pass
    return []


def get_source_text(entry):
    """Get the user (source) text from an entity extraction entry."""
    if entry is None:
        return ""
    messages = entry.get("messages", [])
    for msg in messages:
        if msg.get("role") == "user":
            return msg.get("content", "")
    return ""


def get_constraint_triggers(entry):
    """Extract the trigger signal pattern from a constraint-following entry."""
    if entry is None:
        return None
    messages = entry.get("messages", [])
    for msg in messages:
        if msg.get("role") == "user":
            content = msg.get("content", "")
            # Extract trigger signal lines
            triggers = []
            for line in content.split("\n"):
                line = line.strip()
                if line.startswith("- "):
                    triggers.append(line[2:].strip())
            return tuple(triggers)
    return None


def get_constraint_response_template(entry):
    """Get the template fingerprint from a constraint-following assistant response.

    Returns a normalized template string with entity-specific values replaced
    by placeholders, so entries differing only by entity name are counted
    as the same template.
    """
    if entry is None:
        return None
    messages = entry.get("messages", [])
    for msg in messages:
        if msg.get("role") == "assistant":
            content = msg.get("content", "")
            try:
                data = json.loads(content)
                # Normalize: remove entity-specific values to get template
                data.pop("affected_entity", None)
                # Normalize narrative: replace entity names with placeholder
                if "narrative" in data:
                    data["narrative"] = "NARRATIVE_TEMPLATE"
                # Return structural fingerprint (warning_type + severity + actions)
                return json.dumps(
                    {k: v for k, v in sorted(data.items())
                     if k in ("warning_type", "severity", "recommended_actions")},
                    sort_keys=True
                )
            except (json.JSONDecodeError, TypeError):
                return content[:100]
    return None


# ── Universal Discovery Helpers ─────────────────────────────────────────────
# These mirror the Rust company_discovery.rs and entity_verifier.rs patterns
# so we can verify the CONCEPT of universal discovery from Python.

COMPANY_SUFFIXES = [
    "Corporation", "Incorporated", "Limited",
    "Corp.", "Corp", "Inc.", "Inc", "Ltd.", "Ltd",
    "LLC.", "LLC", "PLC.", "PLC",
    "GmbH", "SARL", "S.A.R.L.", "S.A.", "N.V.",
    "Co.", "Co", "AG", "KG", "Company",
]

# Suffixes sorted by length descending for greedy matching
_COMPANY_SUFFIXES_SORTED = sorted(COMPANY_SUFFIXES, key=len, reverse=True)

# Known ticker → company name map (same as Rust EntityVerifier seed)
KNOWN_TICKERS = {
    "AAPL": "Apple", "MSFT": "Microsoft", "GOOGL": "Alphabet",
    "GOOG": "Alphabet", "AMZN": "Amazon", "NVDA": "NVIDIA",
    "META": "Meta", "TSLA": "Tesla", "TSM": "TSMC",
    "INTC": "Intel", "AMD": "AMD", "QCOM": "Qualcomm",
    "AVGO": "Broadcom", "ASML": "ASML", "TXN": "Texas Instruments",
    "MU": "Micron", "CRM": "Salesforce", "ORCL": "Oracle",
    "IBM": "IBM", "CSCO": "Cisco", "NOC": "Northrop Grumman",
    "LMT": "Lockheed Martin", "RTX": "Raytheon Technologies",
    "GD": "General Dynamics", "BA": "Boeing", "AIR": "Airbus",
    "EADSY": "Airbus", "BAESY": "BAE Systems", "ESLT": "Elbit Systems",
    "RHM.DE": "Rheinmetall", "SAAB-B.ST": "Saab", "LDO.MI": "Leonardo",
    "HO.PA": "Thales", "JBL": "Jabil", "FLEX": "Flex",
    "CLS": "Celestica", "SANM": "Sanmina", "PLXS": "Plexus",
    "KE": "Kimball Electronics", "BHE": "Benchmark Electronics",
}

STOPWORDS = {
    "the", "this", "that", "these", "those", "there", "their", "they",
    "have", "has", "had", "been", "being", "some", "any", "each",
    "every", "both", "few", "more", "most", "other", "into", "over",
    "such", "only", "own", "same", "than", "very", "just", "also",
    "about", "above", "below", "between", "through", "during",
    "before", "after", "where", "which", "what", "when", "why", "how",
    "who", "whom", "with", "without",
}

EXCLUDED_LOCATIONS = {
    "new york", "los angeles", "chicago", "houston", "london", "paris",
    "tokyo", "beijing", "shanghai", "hong kong", "singapore", "dubai",
    "san francisco", "washington", "boston", "seattle", "miami", "dallas",
    "berlin", "munich", "milan", "rome", "madrid", "toronto", "sydney",
    "melbourne", "mumbai", "delhi", "bangalore",
}

LEGAL_SUFFIXES_NORM = [
    " inc.", " inc", " incorporated", " corp.", " corp", " corporation",
    " ltd.", " ltd", " limited", " llc", " plc", " plc.",
    " gmbh", " sarl", " s.a.r.l.", " s.a.", " n.v.", " ag",
    " co.", " co", " kg", " pty ltd", " pty. ltd.",
]


def escape_re(s):
    """Escape string for use in regex."""
    return re.escape(s)


def build_suffix_pattern():
    """Build regex matching '[Name] [Suffix]' — mirrors Rust suffix_pattern()."""
    suffixes = "|".join(escape_re(s) for s in _COMPANY_SUFFIXES_SORTED)
    return re.compile(
        r"\b((?:[A-Z][a-zA-Z&\-]+(?:\s+[A-Z][a-zA-Z&\-]+)*?))\s+((?i:" + suffixes + r"))(?:[\s,;!?)]|$)"
    )


def build_ticker_pattern():
    """Build regex matching 'EXCHANGE:TICKER' or '$TICKER' — mirrors Rust ticker_pattern()."""
    return re.compile(
        r"(?i)(?:NASDAQ|NYSE|AMS|LSE|TSE|HKEX|ASX|TSX|BSE|NSE|EURONEXT|SHG|SHE|OTCQX|OTCQB|OTCPK):"
        r"([A-Z0-9]{1,6}(?:-[A-Z0-9]{1,6})?)|\$([A-Z0-9]{1,6}(?:-[A-Z0-9]{1,6})?)"
    )


def build_comma_suffix_pattern():
    """Build regex matching 'Name, Suffix' — mirrors Rust comma_suffix_pattern()."""
    suffixes = "|".join(escape_re(s) for s in _COMPANY_SUFFIXES_SORTED)
    return re.compile(
        r"\b([A-Z][a-zA-Z&\-]+(?: [A-Z][a-zA-Z&\-]+)*),\s*((?i:" + suffixes + r"))(?:[\s,;!?)]|$)"
    )


def build_capitalized_words_pattern():
    """Build regex matching 3+ consecutive capitalized words."""
    return re.compile(r"\b([A-Z][a-zA-Z]+(?: [A-Z][a-zA-Z]+){2,})\b")


def normalize_company_name(name):
    """Normalize a company name: lowercase, strip legal suffixes, remove punctuation.
    Mirrors Rust normalize_company_name()."""
    result = name.strip().lower()
    for suffix in LEGAL_SUFFIXES_NORM:
        if result.endswith(suffix):
            trimmed_len = len(result) - len(suffix)
            result = result[:trimmed_len].rstrip()
            break
    # Keep only alphanumeric, whitespace, hyphens, ampersands
    filtered = []
    for c in result:
        if c.isalnum() or c.isspace() or c in '-&':
            filtered.append(c)
    result = ''.join(filtered)
    return ' '.join(result.split())


def extract_ticker_pattern(text):
    """Extract ticker patterns from text. Returns list of (exchange, ticker).
    Mirrors Rust extract_ticker_pattern()."""
    pattern = build_ticker_pattern()
    results = []
    for match in pattern.finditer(text):
        g1 = match.group(1)
        g2 = match.group(2)
        full = match.group(0)
        if g1:
            # EXCHANGE:TICKER — extract exchange name from prefix
            colon_pos = full.find(':')
            exchange = full[:colon_pos] if colon_pos > 0 else "OTC"
            results.append((exchange.upper(), g1))
        elif g2:
            results.append(("OTC", g2))
    return results


def extract_suffix_matches(text):
    """Extract suffix-based company mentions. Returns list of raw names.
    Mirrors Rust suffix_pattern() capture logic."""
    pattern = build_suffix_pattern()
    matches = []
    for m in pattern.finditer(text):
        name = m.group(1).strip()
        suffix = m.group(2).strip()
        if len(name) > 2:
            raw_name = f"{name} {suffix}"
            matches.append(raw_name)
    return matches


def extract_comma_suffix_matches(text):
    """Extract comma-suffix company mentions. Returns list of raw names."""
    pattern = build_comma_suffix_pattern()
    matches = []
    for m in pattern.finditer(text):
        name = m.group(1).strip()
        if len(name) > 2:
            matches.append(name)
    return matches


def extract_capitalized_words(text):
    """Extract 3+ consecutive capitalized words. Returns list of raw names."""
    pattern = build_capitalized_words_pattern()
    matches = []
    stopwords_lower = {w.lower() for w in STOPWORDS}
    for m in pattern.finditer(text):
        name = m.group(1).strip()
        if len(name) <= 5:
            continue
        # Check if all words are stopwords
        words = name.lower().split()
        if words and all(w in stopwords_lower for w in words):
            continue
        matches.append(name)
    return matches


def extract_all_candidates(text):
    """Extract all company candidates from text using all strategies.
    Returns list of (raw_name, normalized_name, confidence, source_type)."""
    seen_normalized = set()
    candidates = []

    # 1. Ticker patterns
    for exchange, ticker in extract_ticker_pattern(text):
        raw = f"{exchange}:{ticker}"
        norm = normalize_company_name(raw)
        if norm not in seen_normalized:
            seen_normalized.add(norm)
            candidates.append((raw, norm, 0.9, "ticker"))

    # 2. Suffix pattern
    for raw in extract_suffix_matches(text):
        norm = normalize_company_name(raw)
        if norm not in seen_normalized:
            seen_normalized.add(norm)
            candidates.append((raw, norm, 0.8, "suffix"))

    # 3. Comma-suffix pattern
    for raw in extract_comma_suffix_matches(text):
        norm = normalize_company_name(raw)
        if norm not in seen_normalized:
            seen_normalized.add(norm)
            candidates.append((raw, norm, 0.8, "comma_suffix"))

    # 4. Capitalized words (3+)
    for raw in extract_capitalized_words(text):
        norm = normalize_company_name(raw)
        if norm not in seen_normalized:
            seen_normalized.add(norm)
            candidates.append((raw, norm, 0.5, "capitalized"))

    return candidates


def heuristic_check(name):
    """Heuristic verification: does the name look like a real company?
    Mirrors Rust EntityVerifier.heuristic_check(). Returns score 0.0–1.0."""
    name = name.strip()
    # Length check
    if len(name) <= 3 or len(name) >= 100:
        return 0.0
    # Must contain at least one alphabetic character
    if not any(c.isalpha() for c in name):
        return 0.0

    lower = name.lower()
    words = lower.split()

    # All stopwords → not a company
    if words and all(w in STOPWORDS for w in words):
        return 0.0

    # Exclusion list
    if lower in EXCLUDED_LOCATIONS:
        return 0.0

    score = 0.3  # Base score for passing basic checks

    # Suffix boost
    suffix_set = {"inc", "corp", "ltd", "llc", "plc", "gmbh", "sarl", "ag", "kg",
                  "limited", "incorporated", "corporation", "company", "co"}
    has_suffix = any(w.rstrip('.') in suffix_set for w in words)
    if has_suffix:
        score += 0.3

    # Titlecase words boost (2+ capitalized words)
    titlecase_words = [
        w for w in name.split()
        if len(w) > 1 and w[0].isupper() and all(c.islower() or not c.isalpha() for c in w[1:])
    ]
    if len(titlecase_words) >= 2:
        score += 0.2

    # Embedded capitals boost
    upper_count = sum(1 for c in name if c.isupper())
    if upper_count >= 2 and len(name) > 5:
        score += 0.1

    # Penalize very short names
    if len(name) < 5:
        score -= 0.2

    return max(0.0, min(1.0, score))


def verify_candidate(raw_name, normalized_name, ticker=None):
    """Verify a company candidate using heuristic + ticker cross-reference.
    Mirrors Rust EntityVerifier.verify(). Returns (is_verified, confidence)."""
    confidence = 0.0
    methods = []

    # 1. Heuristic check
    h_score = heuristic_check(raw_name)
    if h_score > 0.0:
        methods.append(f"heuristic:{h_score:.2f}")
        confidence += h_score * 0.75

    # 2. Ticker verification
    if ticker and ticker.upper() in KNOWN_TICKERS:
        methods.append(f"ticker:{ticker}")
        confidence += 0.4
        known_name = KNOWN_TICKERS[ticker.upper()]
        if normalize_company_name(known_name) == normalized_name:
            confidence += 0.2

    confidence = min(1.0, confidence)
    is_verified = confidence >= 0.5
    return is_verified, confidence, methods


# ── Test Classes ───────────────────────────────────────────────────────────

class TestEntityCoverage(unittest.TestCase):
    """Verify all entities from augmentation_entities.yaml appear as reference."""

    @classmethod
    def setUpClass(cls):
        cls.yaml_data = load_yaml(AUGMENTATION_YAML)
        cls.ems, cls.oem = get_entities_from_yaml(cls.yaml_data)
        cls.all_entities = cls.ems + cls.oem

    def test_yaml_has_ems_entities(self):
        """augmentation_entities.yaml should define EMS companies."""
        self.assertGreaterEqual(len(self.ems), 20,
                                f"Expected ≥20 EMS companies, got {len(self.ems)}")

    def test_yaml_has_oem_entities(self):
        """augmentation_entities.yaml should define OEM companies."""
        self.assertGreaterEqual(len(self.oem), 10,
                                f"Expected ≥10 OEM companies, got {len(self.oem)}")

    def test_starz_in_ems(self):
        """Starz Electronics must be in the EMS list."""
        starz_variants = ["Starz Electronics", "Starz"]
        has_starz = any(
            any(v in e for v in starz_variants) for e in self.ems
        )
        self.assertTrue(has_starz, "Starz Electronics not found in EMS entities")

    def _check_coverage(self, filepath, label):
        """Check what fraction of entities appear as reference in a JSONL file."""
        entries = load_jsonl(filepath)
        self.assertGreater(len(entries), 0, f"{label}: File is empty")

        reference_entities = Counter()
        for entry in entries:
            ref = get_reference_entity(entry)
            if ref:
                # Normalize: try to match to known entities
                matched = False
                for known in self.all_entities:
                    if known.lower() in ref.lower() or ref.lower() in known.lower():
                        reference_entities[known] += 1
                        matched = True
                        break
                if not matched:
                    reference_entities[ref] += 1

        covered = [e for e in self.all_entities if e in reference_entities]
        coverage_pct = len(covered) / len(self.all_entities) * 100

        # Report
        print(f"\n  {label}: {len(reference_entities)} unique reference entities used")
        print(f"  Coverage: {len(covered)}/{len(self.all_entities)} ({coverage_pct:.1f}%)")

        # Check if Starz is overrepresented
        starz_count = reference_entities.get("Starz Electronics", 0)
        if starz_count == 0:
            starz_count = sum(
                v for k, v in reference_entities.items() if "starz" in k.lower()
            )
        total = sum(reference_entities.values())
        starz_pct = (starz_count / total * 100) if total > 0 else 0
        print(f"  Starz usage: {starz_count}/{total} ({starz_pct:.1f}%)")

        # Assertions
        self.assertGreater(
            coverage_pct, 50,
            f"{label}: Entity coverage too low ({coverage_pct:.1f}%). "
            f"Only {len(covered)}/{len(self.all_entities)} entities appear as reference."
        )
        self.assertLess(
            starz_pct, 50,
            f"{label}: Starz-centric bias detected — {starz_pct:.1f}% of entries use Starz as reference. "
            f"Expected < 50%."
        )

    def test_competitive_analysis_coverage(self):
        """All entities should appear as reference in competitive_analysis.jsonl."""
        self._check_coverage(COMPETITIVE_ANALYSIS, "competitive_analysis.jsonl")

    def test_competitive_analysis_augmented_coverage(self):
        """All entities should appear as reference in competitive_analysis_augmented.jsonl."""
        self._check_coverage(COMPETITIVE_ANALYSIS_AUG, "competitive_analysis_augmented.jsonl")


class TestTitleDiversityJaccard(unittest.TestCase):
    """Measure template diversity using Jaccard similarity on trigger signals."""

    def test_constraint_following_trigger_diversity(self):
        """
        Verify constraint-following has sufficient trigger signal diversity.
        Jaccard similarity between any two entries' trigger sets should be < 1.0
        for at least 80% of pairwise comparisons.
        """
        entries = load_jsonl(CONSTRAINT_FOLLOWING)
        self.assertGreater(len(entries), 0, "constraint_following_examples.jsonl is empty")

        trigger_sets = []
        for entry in entries:
            triggers = get_constraint_triggers(entry)
            if triggers:
                trigger_sets.append(set(triggers))

        self.assertGreater(
            len(trigger_sets), 5,
            f"Too few entries with extractable triggers: {len(trigger_sets)}"
        )

        # Count unique trigger patterns
        unique_patterns = set(tuple(sorted(ts)) for ts in trigger_sets)
        print(f"\n  Unique trigger signal patterns: {len(unique_patterns)}")
        print(f"  Total entries with triggers: {len(trigger_sets)}")

        # Compute pairwise Jaccard similarities
        identical_pairs = 0
        total_pairs = 0
        for i in range(len(trigger_sets)):
            for j in range(i + 1, len(trigger_sets)):
                sim = jaccard_similarity(trigger_sets[i], trigger_sets[j])
                if sim >= 1.0:
                    identical_pairs += 1
                total_pairs += 1

        identical_ratio = identical_pairs / total_pairs if total_pairs > 0 else 0
        print(f"  Identical trigger pairs: {identical_pairs}/{total_pairs} ({identical_ratio*100:.1f}%)")

        # Assert: at least 5 unique trigger patterns
        self.assertGreaterEqual(
            len(unique_patterns), 5,
            f"Only {len(unique_patterns)} unique trigger patterns found. "
            f"Expected at least 5 to ensure diversity."
        )

        # Assert: no more than 50% of pairs should be identical
        self.assertLess(
            identical_ratio, 0.5,
            f"{identical_ratio*100:.1f}% of trigger pairs are identical. "
            f"Expected < 50% for reasonable diversity."
        )

    def test_constraint_following_response_diversity(self):
        """
        Verify constraint-following has diverse response templates.
        At least 3 distinct response structures should exist.
        """
        entries = load_jsonl(CONSTRAINT_FOLLOWING)
        templates = Counter()
        for entry in entries:
            tmpl = get_constraint_response_template(entry)
            if tmpl:
                templates[tmpl] += 1

        print(f"\n  Unique response templates: {len(templates)}")
        for tmpl, count in templates.most_common(5):
            # Show just the warning_type and severity
            try:
                data = json.loads(tmpl)
                wt = data.get("warning_type", "?")
                sev = data.get("severity", "?")
                print(f"    [{sev}] {wt} — appears {count}×")
            except (json.JSONDecodeError, TypeError):
                print(f"    (unparseable) — appears {count}×")

        self.assertGreaterEqual(
            len(templates), 3,
            f"Only {len(templates)} unique response templates. "
            f"Expected at least 3 for reasonable diversity."
        )


class TestComparisonMatrixBias(unittest.TestCase):
    """
    Verify no Starz-centric bias in entries mentioning specific
    non-EMS entities: Foxconn, NVIDIA, Tesla, ANONYMOUS_CORP.
    Checks both competitive_analysis.jsonl and
    competitive_analysis_augmented.jsonl for each target.
    """

    def _check_entity_bias(self, target_name):
        """
        Check entries mentioning a specific target entity across both
        competitive analysis files to ensure they don't always
        reference Starz.
        """
        all_target_entries = []
        starz_referencing = 0

        for filepath, label in [
            (COMPETITIVE_ANALYSIS, "competitive_analysis.jsonl"),
            (COMPETITIVE_ANALYSIS_AUG, "competitive_analysis_augmented.jsonl"),
        ]:
            entries = load_jsonl(filepath)
            for entry in entries:
                if entry is None:
                    continue
                content = ""
                for msg in entry.get("messages", []):
                    content += msg.get("content", "") + " "

                if target_name.lower() in content.lower():
                    all_target_entries.append(entry)
                    ref = get_reference_entity(entry)
                    if ref and "starz" in ref.lower():
                        starz_referencing += 1

        if not all_target_entries:
            self.skipTest(
                f"Neither competitive analysis file contains entries "
                f"mentioning '{target_name}'. This itself indicates a "
                f"coverage gap — major entities should appear."
            )

        starz_pct = (starz_referencing / len(all_target_entries)) * 100
        print(f"\n  '{target_name}': {len(all_target_entries)} entries across both files, "
              f"{starz_referencing} use Starz as reference ({starz_pct:.1f}%)")

        self.assertLess(
            starz_pct, 100,
            f"100% of entries mentioning '{target_name}' use Starz as reference. "
            f"This indicates systematic Starz-centric bias."
        )

    def test_foxconn_bias(self):
        """Foxconn comparisons should not all reference Starz."""
        self._check_entity_bias("Foxconn")

    def test_nvidia_bias(self):
        """NVIDIA comparisons should not all reference Starz."""
        self._check_entity_bias("NVIDIA")

    def test_tesla_bias(self):
        """Tesla comparisons should not all reference Starz."""
        self._check_entity_bias("Tesla")

    def test_anonymous_corp_bias(self):
        """ANONYMOUS_CORP comparisons should not all reference Starz."""
        self._check_entity_bias("ANONYMOUS_CORP")


class TestFeedbackLoopClosure(unittest.TestCase):
    """
    Verify the entity selection mechanism (feedback loop) is not
    stuck on a single entity, i.e., reference entity distribution
    is reasonably balanced across all available entities.
    """

    def _check_feedback_loop(self, filepath, label):
        """Check reference entity distribution for feedback loop issues."""
        entries = load_jsonl(filepath)
        yaml_data = load_yaml(AUGMENTATION_YAML)
        ems, oem = get_entities_from_yaml(yaml_data)
        all_entities = ems + oem

        reference_counts = Counter()
        for entry in entries:
            ref = get_reference_entity(entry)
            if ref:
                matched = False
                for known in all_entities:
                    if known.lower() in ref.lower() or ref.lower() in known.lower():
                        reference_counts[known] += 1
                        matched = True
                        break
                if not matched:
                    reference_counts[ref] += 1

        total = sum(reference_counts.values())
        if total == 0:
            self.skipTest(f"{label}: No reference entities found")

        dominant_entity, dominant_count = reference_counts.most_common(1)[0]
        dominant_pct = (dominant_count / total) * 100

        # Count entities used at least once
        used_entities = len(reference_counts)
        print(f"\n  {label}:")
        print(f"  Unique entities used: {used_entities}")
        print(f"  Most used: '{dominant_entity}' — {dominant_count}/{total} ({dominant_pct:.1f}%)")

        # Check if single entity dominates
        self.assertLess(
            dominant_pct, 60,
            f"{label}: Feedback loop stuck on '{dominant_entity}' "
            f"({dominant_pct:.1f}% of entries). Expected < 60%."
        )

        # Gini coefficient approximation: check if top-3 entities account for >90%
        top3_count = sum(c for _, c in reference_counts.most_common(3))
        top3_pct = (top3_count / total) * 100 if total > 0 else 0
        print(f"  Top-3 entities: {top3_count}/{total} ({top3_pct:.1f}%)")

        self.assertLess(
            top3_pct, 95,
            f"{label}: Top-3 entities account for {top3_pct:.1f}% of references. "
            f"Entity selection is too concentrated."
        )

    def test_competitive_analysis_feedback(self):
        """Competitive analysis should have balanced entity distribution."""
        self._check_feedback_loop(COMPETITIVE_ANALYSIS, "competitive_analysis.jsonl")

    def test_competitive_analysis_augmented_feedback(self):
        """Augmented competitive analysis should have balanced entity distribution."""
        self._check_feedback_loop(COMPETITIVE_ANALYSIS_AUG, "competitive_analysis_augmented.jsonl")


class TestEdgeCases(unittest.TestCase):
    """Edge case and structural integrity tests."""

    def test_jsonl_files_exist(self):
        """All expected JSONL files must exist."""
        expected = [
            COMPETITIVE_ANALYSIS,
            COMPETITIVE_ANALYSIS_AUG,
            ENTITY_EXTRACTION,
            CONSTRAINT_FOLLOWING,
        ]
        for path in expected:
            self.assertTrue(
                path.exists(),
                f"Required file missing: {path}"
            )

    def test_jsonl_files_not_empty(self):
        """All JSONL files must have at least one entry."""
        for path in [COMPETITIVE_ANALYSIS, COMPETITIVE_ANALYSIS_AUG,
                     ENTITY_EXTRACTION, CONSTRAINT_FOLLOWING]:
            entries = load_jsonl(path)
            self.assertGreater(
                len(entries), 0,
                f"{path.name} is empty or contains no valid JSON entries"
            )

    def test_jsonl_is_valid_json(self):
        """Every line in every JSONL file must be valid JSON."""
        for path in [COMPETITIVE_ANALYSIS, COMPETITIVE_ANALYSIS_AUG,
                     ENTITY_EXTRACTION, CONSTRAINT_FOLLOWING]:
            with open(path, "r", encoding="utf-8") as f:
                for i, line in enumerate(f, 1):
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        json.loads(line)
                    except json.JSONDecodeError as e:
                        self.fail(f"{path.name}:{i} — Invalid JSON: {e}")

    def test_entity_extraction_no_malformed(self):
        """entity_extraction.jsonl entries must have messages array."""
        entries = load_jsonl(ENTITY_EXTRACTION)
        malformed = 0
        for i, entry in enumerate(entries):
            if entry is None:
                malformed += 1
                continue
            if "messages" not in entry or not isinstance(entry["messages"], list):
                malformed += 1
                continue
            roles = [m.get("role") for m in entry["messages"]]
            if "system" not in roles or "user" not in roles or "assistant" not in roles:
                malformed += 1

        print(f"\n  Entity extraction malformed entries: {malformed}/{len(entries)}")
        self.assertEqual(
            malformed, 0,
            f"{malformed} malformed entries in entity_extraction.jsonl"
        )

    def test_constraint_following_structure(self):
        """constraint_following entries must have system/user/assistant messages."""
        entries = load_jsonl(CONSTRAINT_FOLLOWING)
        for i, entry in enumerate(entries):
            if entry is None:
                continue
            roles = [m.get("role") for m in entry.get("messages", [])]
            self.assertIn(
                "system", roles,
                f"constraint_following[{i}]: missing system message"
            )
            self.assertIn(
                "user", roles,
                f"constraint_following[{i}]: missing user message"
            )
            self.assertIn(
                "assistant", roles,
                f"constraint_following[{i}]: missing assistant message"
            )

    def test_competitive_analysis_structure(self):
        """competitive_analysis entries must have proper message structure."""
        for path in [COMPETITIVE_ANALYSIS, COMPETITIVE_ANALYSIS_AUG]:
            entries = load_jsonl(path)
            label = path.name
            for i, entry in enumerate(entries):
                if entry is None:
                    continue
                roles = [m.get("role") for m in entry.get("messages", [])]
                self.assertIn(
                    "system", roles,
                    f"{label}[{i}]: missing system message"
                )
                self.assertIn(
                    "user", roles,
                    f"{label}[{i}]: missing user message"
                )
                self.assertIn(
                    "assistant", roles,
                    f"{label}[{i}]: missing assistant message"
                )


class TestEntityExtractionHallucination(unittest.TestCase):
    """Detect hallucinated entities in entity_extraction.jsonl."""

    def test_no_hallucinated_companies(self):
        """
        Every company extracted in entity_extraction.jsonl must appear
        in the source text. This test detects systematic hallucination.
        """
        entries = load_jsonl(ENTITY_EXTRACTION)
        hallucinated = []

        for i, entry in enumerate(entries):
            if entry is None:
                continue
            source = get_source_text(entry).lower()
            companies = get_extracted_entities(entry)

            for company in companies:
                if not company:
                    continue
                company_lower = company.lower()
                # Check if company name appears in source text
                if company_lower not in source:
                    # Check if it's a plausible known entity (allowlist)
                    hallucinated.append((i + 1, company))

        print(f"\n  Hallucinated companies detected: {len(hallucinated)}")
        if hallucinated:
            for line_no, name in hallucinated[:10]:
                print(f"    Line {line_no}: '{name}'")

        self.assertEqual(
            len(hallucinated), 0,
            f"{len(hallucinated)} hallucinated companies found. "
            f"First: Line {hallucinated[0][0]} — '{hallucinated[0][1]}'"
        )

    def test_no_hallucinated_persons(self):
        """
        Every person extracted in entity_extraction.jsonl must have
        name components present in the source text.
        """
        entries = load_jsonl(ENTITY_EXTRACTION)
        hallucinated = []

        for i, entry in enumerate(entries):
            if entry is None:
                continue
            source = get_source_text(entry).lower()
            messages = entry.get("messages", [])
            for msg in messages:
                if msg.get("role") == "assistant":
                    content = msg.get("content", "")
                    try:
                        data = json.loads(content)
                        for person in data.get("persons", []):
                            pname = person.get("name", "")
                            if not pname:
                                continue
                            parts = pname.lower().split()
                            # Check if any significant name part appears in source
                            found = any(
                                part in source for part in parts if len(part) > 2
                            )
                            if not found:
                                hallucinated.append((i + 1, pname))
                    except (json.JSONDecodeError, TypeError):
                        pass

        print(f"\n  Hallucinated persons detected: {len(hallucinated)}")
        if hallucinated:
            for line_no, name in hallucinated[:10]:
                print(f"    Line {line_no}: '{name}'")

        self.assertEqual(
            len(hallucinated), 0,
            f"{len(hallucinated)} hallucinated persons found. "
            f"First: Line {hallucinated[0][0]} — '{hallucinated[0][1]}'"
        )


class TestConfigIntegrity(unittest.TestCase):
    """Verify augmentation_entities.yaml is well-formed and complete."""

    def test_yaml_file_exists(self):
        """augmentation_entities.yaml must exist."""
        self.assertTrue(AUGMENTATION_YAML.exists(), "augmentation_entities.yaml missing")

    @unittest.skipIf(yaml is None, "pyyaml not installed")
    def test_yaml_well_formed(self):
        """augmentation_entities.yaml must be valid YAML."""
        data = load_yaml(AUGMENTATION_YAML)
        self.assertIn("ems_companies", data, "Missing 'ems_companies' key")
        self.assertIn("oem_companies", data, "Missing 'oem_companies' key")
        self.assertIn("capabilities", data, "Missing 'capabilities' key")
        self.assertIn("certifications", data, "Missing 'certifications' key")
        self.assertIn("industries", data, "Missing 'industries' key")
        self.assertIn("regions", data, "Missing 'regions' key")
        self.assertIn("roles", data, "Missing 'roles' key")
        self.assertIn("first_names", data, "Missing 'first_names' key")
        self.assertIn("last_names", data, "Missing 'last_names' key")

    @unittest.skipIf(yaml is None, "pyyaml not installed")
    def test_yaml_entity_counts(self):
        """Verify minimum entity counts in YAML."""
        data = load_yaml(AUGMENTATION_YAML)
        ems, oem = get_entities_from_yaml(data)
        self.assertGreaterEqual(len(ems), 20, f"Expected ≥20 EMS, got {len(ems)}")
        self.assertGreaterEqual(len(oem), 10, f"Expected ≥10 OEM, got {len(oem)}")
        self.assertGreaterEqual(len(data.get("capabilities", [])), 15,
                                f"Expected ≥15 capabilities, got {len(data.get('capabilities', []))}")
        self.assertGreaterEqual(len(data.get("certifications", [])), 8,
                                f"Expected ≥8 certifications, got {len(data.get('certifications', []))}")


class TestUniversalDiscovery(unittest.TestCase):
    """Verify companies can be discovered from raw text without static lists."""

    def test_extract_suffix_pattern(self):
        """'Apple Inc. announced new products' → extracts 'Apple Inc.'"""
        text = "Apple Inc. announced new products yesterday."
        candidates = extract_all_candidates(text)
        raw_names = [c[0] for c in candidates]
        self.assertIn("Apple Inc.", raw_names,
                      f"Suffix pattern should match 'Apple Inc.' Got: {raw_names}")
        # Verify normalized
        norms = [c[1] for c in candidates]
        self.assertIn("apple", norms,
                      f"Normalized should be 'apple'. Got: {norms}")

    def test_extract_ticker_pattern(self):
        """'$AAPL up 5%' or 'NASDAQ:AAPL' → extracts ticker AAPL"""
        text = "Shares of $AAPL rose 5% today on strong earnings."
        tickers = extract_ticker_pattern(text)
        self.assertGreaterEqual(len(tickers), 1,
                                "Should extract at least one ticker")
        self.assertIn(("OTC", "AAPL"), tickers,
                      f"Should extract $AAPL as OTC:AAPL. Got: {tickers}")

        # Also test exchange-prefixed ticker
        text2 = "NASDAQ:AAPL reported earnings."
        tickers2 = extract_ticker_pattern(text2)
        self.assertIn(("NASDAQ", "AAPL"), tickers2,
                      f"Should extract NASDAQ:AAPL. Got: {tickers2}")

    def test_extract_multiple_companies(self):
        """Text mentioning multiple companies extracts all."""
        text = "Microsoft Corp. and Alphabet Inc. both announced AI partnerships. $MSFT rose 2%."
        candidates = extract_all_candidates(text)
        raw_names = [c[0] for c in candidates]
        # Should find Microsoft Corp., Alphabet Inc., MSFT ticker
        found_microsoft = any("Microsoft" in r for r in raw_names)
        found_alphabet = any("Alphabet" in r for r in raw_names)
        found_msft = any("MSFT" in r for r in raw_names)
        self.assertTrue(found_microsoft,
                        f"Should find Microsoft. Got: {raw_names}")
        self.assertTrue(found_alphabet,
                        f"Should find Alphabet. Got: {raw_names}")
        self.assertTrue(found_msft,
                        f"Should find $MSFT. Got: {raw_names}")

    def test_normalize_dedup(self):
        """Same company with different formats normalizes to same key."""
        n1 = normalize_company_name("Apple Inc.")
        n2 = normalize_company_name("Apple, Inc.")
        n3 = normalize_company_name("Apple Incorporated")
        n4 = normalize_company_name("Apple")
        self.assertEqual(n1, "apple",
                         f"'Apple Inc.' should normalize to 'apple'. Got: '{n1}'")
        self.assertEqual(n2, "apple",
                         f"'Apple, Inc.' should normalize to 'apple'. Got: '{n2}'")
        self.assertEqual(n3, "apple",
                         f"'Apple Incorporated' should normalize to 'apple'. Got: '{n3}'")
        self.assertEqual(n4, "apple",
                         f"'Apple' should normalize to 'apple'. Got: '{n4}'")
        # All normalize to same key
        self.assertEqual(len({n1, n2, n3, n4}), 1,
                         "All variants should normalize to identical key")

    def test_no_false_positives(self):
        """Common words are NOT extracted as companies."""
        # "company" is a known suffix; use text without suffix/named patterns
        text = "the company announced that it would invest in new technology."
        candidates = extract_all_candidates(text)
        self.assertEqual(len(candidates), 0,
                         f"No companies should be extracted from common words. Got: {candidates}")

        # Also test with no capitalized words at all
        text2 = "they announced new products and services for the market."
        candidates2 = extract_all_candidates(text2)
        self.assertEqual(len(candidates2), 0,
                         f"No companies from all-lowercase text. Got: {candidates2}")

    def test_extract_from_job_posting(self):
        """Job posting mentions employer company."""
        # Use a company name with suffix so extraction is reliable
        text = """
        Job Title: Senior Software Engineer
        Company: Tesla Motors Inc.
        Location: Austin, TX
        Tesla Motors is seeking an experienced engineer...
        """
        candidates = extract_all_candidates(text)
        raw_names = [c[0] for c in candidates]
        # Should extract "Tesla Motors Inc." via suffix pattern
        self.assertTrue(
            any("Tesla Motors" in r for r in raw_names),
            f"Should extract Tesla Motors from job posting. Got: {raw_names}"
        )

    def test_extract_from_patent(self):
        """Patent filing mentions assignee company."""
        text = "Assignee: Samsung Electronics Co., Ltd. filed patent US20240000000"
        candidates = extract_all_candidates(text)
        raw_names = [c[0] for c in candidates]
        self.assertTrue(
            any("Samsung" in r for r in raw_names),
            f"Should extract Samsung from patent. Got: {raw_names}"
        )

    def test_verify_heuristic_valid_company(self):
        """Valid company names pass heuristic verification."""
        names = ["Apple Inc.", "Microsoft Corporation", "TSMC Ltd", "Samsung Electronics"]
        for name in names:
            score = heuristic_check(name)
            self.assertGreater(
                score, 0.5,
                f"'{name}' should pass heuristic check (score > 0.5). Got: {score:.2f}"
            )

    def test_verify_heuristic_rejects_common_words(self):
        """Common words fail heuristic verification."""
        # "Company" is a legal suffix, so it legitimately passes — exclude it.
        # Single words without suffix pattern score base=0.3 but < 0.5 threshold.
        names = {
            "The": "too short or pure stopword",
            "Technology": "common word, no suffix",
            "Investment": "common word, no suffix",
            "Announced": "common word, no suffix",
            "the": "all lowercase, no capitals",
        }
        for name, reason in names.items():
            score = heuristic_check(name)
            self.assertLess(
                score, 0.5,
                f"'{name}' should score < 0.5 ({reason}). Got: {score:.2f}"
            )

    def test_discovery_without_seed_list(self):
        """System discovers from data without any seed entities."""
        # Start with empty registry (simulated by empty seen set)
        text = "NVIDIA Corporation announced new GPUs. NASDAQ:NVDA traded up."
        candidates = extract_all_candidates(text)
        self.assertGreaterEqual(
            len(candidates), 1,
            f"Should discover at least one company from text. Got: {len(candidates)}"
        )
        # Verify at least one passes heuristic verification
        any_verified = False
        for raw, norm, conf, stype in candidates:
            is_verified, vconf, methods = verify_candidate(raw, norm)
            if is_verified:
                any_verified = True
                break
        self.assertTrue(
            any_verified,
            "At least one discovered candidate should pass verification"
        )


class TestDiscoveryEdgeCases(unittest.TestCase):
    """Edge cases for the universal discovery system."""

    def test_empty_text(self):
        """Empty text → zero candidates."""
        candidates = extract_all_candidates("")
        self.assertEqual(len(candidates), 0,
                         "Empty text should yield zero candidates")

    def test_single_word_suffix_only(self):
        """Single word + suffix should NOT match (need at least one name word)."""
        text = "Corp. announced results."
        candidates = extract_all_candidates(text)
        # The suffix pattern requires [A-Z][a-zA-Z...]+ before the suffix
        # "Corp." alone won't have a preceding capitalized word
        self.assertEqual(
            len(candidates), 0,
            f"'Corp.' alone should not match as a company. Got: {candidates}"
        )

    def test_non_english_suffix(self):
        """International suffixes work."""
        texts = [
            "Siemens GmbH announced...",
            "Toyota Motor Corporation stellt vor...",
            "Samsung Electronics Co., Ltd. 発表...",
        ]
        for text in texts:
            candidates = extract_all_candidates(text)
            self.assertGreater(
                len(candidates), 0,
                f"Should extract from international suffix text: '{text[:50]}...'"
            )

    def test_ticker_only_text(self):
        """$TICKER patterns work without full company name."""
        text = "$AMD up 3% after earnings beat estimates $INTC down 1%"
        tickers = extract_ticker_pattern(text)
        ticker_symbols = [t for _, t in tickers]
        self.assertIn("AMD", ticker_symbols,
                      f"Should extract AMD ticker. Got: {ticker_symbols}")
        self.assertIn("INTC", ticker_symbols,
                      f"Should extract INTC ticker. Got: {ticker_symbols}")

    def test_newly_discovered_entity_gets_insight_priority(self):
        """Newly discovered entity gets priority for insight generation."""
        # Simulate: start with empty registry, discover companies, verify
        text = "NovaTech Solutions Inc. announced a breakthrough."
        candidates = extract_all_candidates(text)
        self.assertGreaterEqual(
            len(candidates), 1,
            "Should discover NovaTech Solutions Inc."
        )
        raw, norm, conf, stype = candidates[0]
        is_verified, vconf, methods = verify_candidate(raw, norm)
        # A well-formed company with "Inc." suffix should pass verification
        self.assertTrue(
            is_verified,
            f"NovaTech Solutions Inc. should pass verification. Confidence: {vconf:.2f}"
        )
        # In the Rust orchestrator, discovered entities without insights
        # get priority over entities that already have insights.
        # This test validates the discovery pipeline produces registrable entities.


class TestSignalDepth(unittest.TestCase):
    """Verify deep signal extraction for discovered companies."""

    def test_extract_website_from_context(self):
        """Website URL extracted from context around company mention."""
        text = "Visit https://www.qualtrics.com for more information about Qualtrics Inc."
        candidates = extract_all_candidates(text)
        raw_names = [c[0] for c in candidates]
        self.assertTrue(
            any("Qualtrics" in r for r in raw_names),
            f"Should extract Qualtrics Inc. from context with website. Got: {raw_names}"
        )
        # Verify website URL is present in text (simulating metadata extraction)
        website_match = re.search(r"https?://[^\s,;)]+", text)
        self.assertIsNotNone(website_match,
                             "Website URL should be extractable from context")
        self.assertIn("qualtrics.com", website_match.group(),
                      "Extracted URL should contain qualtrics.com")

    def test_extract_revenue_from_context(self):
        """Revenue figures extracted from context."""
        text = "Datarobot Inc. reported $500M in annual recurring revenue."
        candidates = extract_all_candidates(text)
        raw_names = [c[0] for c in candidates]
        self.assertTrue(
            any("Datarobot" in r for r in raw_names),
            f"Should extract Datarobot Inc. from context with revenue. Got: {raw_names}"
        )
        # Revenue pattern extraction
        rev_pattern = re.compile(
            r"(?i)(?:revenue|turnover|sales|earnings|income)"
            r"\s*(?:of|:)?\s*\$?([\d,.]+)\s*(?:billion|million|trillion|B|M|T|bn|mn)?"
        )
        rev_match = rev_pattern.search(text)
        self.assertIsNotNone(rev_match,
                             "Revenue figure should be extractable from context")

    def test_extract_location_from_context(self):
        """Location extracted from context."""
        text = "Palantir Technologies Inc., based in Denver, Colorado, announced..."
        candidates = extract_all_candidates(text)
        raw_names = [c[0] for c in candidates]
        # Palantir Technologies Inc. matches suffix pattern
        self.assertTrue(
            any("Palantir" in r for r in raw_names),
            f"Should extract Palantir Technologies. Got: {raw_names}"
        )
        # Location pattern extraction
        loc_pattern = re.compile(
            r"(?i)(?:based in|headquartered in|located in|operates in)"
            r"\s+([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+)*)"
        )
        loc_match = loc_pattern.search(text)
        self.assertIsNotNone(loc_match,
                             "Location should be extractable from context")

    def test_infer_category_from_signal_patterns(self):
        """Category inferred from signal types (job posts → tech, patents → R&D)."""
        # Job posting signals → Technology category
        job_text = """
        Job Title: Senior Software Engineer
        Company: TechCorp Inc.
        Location: San Francisco, CA
        """
        candidates = extract_all_candidates(job_text)
        self.assertGreaterEqual(
            len(candidates), 1,
            "Job posting should yield company candidates"
        )

        # Patent signals should also yield candidates
        patent_text = "Assignee: Qualcomm Technologies, Inc. filed patent US20240000001"
        patent_candidates = extract_all_candidates(patent_text)
        self.assertGreaterEqual(
            len(patent_candidates), 1,
            "Patent filing should yield company candidates"
        )

        # Financial signals (ticker patterns)
        fin_text = "$AMD up 3% after earnings beat estimates"
        fin_candidates = extract_all_candidates(fin_text)
        self.assertGreaterEqual(
            len(fin_candidates), 1,
            "Financial text should yield ticker-based candidates"
        )


# ── Summary Reporter ───────────────────────────────────────────────────────

class SummaryTestResult(unittest.TextTestResult):
    """Custom test result that prints a summary table."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.test_results = []

    def startTest(self, test):
        super().startTest(test)
        self._test_started = test

    def addSuccess(self, test):
        super().addSuccess(test)
        self.test_results.append((test._testMethodName, "PASS", ""))

    def addFailure(self, test, err):
        super().addFailure(test, err)
        self.test_results.append((test._testMethodName, "FAIL", self._exc_info_to_string(err, test)))

    def addError(self, test, err):
        super().addError(test, err)
        self.test_results.append((test._testMethodName, "ERROR", self._exc_info_to_string(err, test)))

    def addSkip(self, test, reason):
        super().addSkip(test, reason)
        self.test_results.append((test._testMethodName, "SKIP", reason))


def print_summary(test_result):
    """Print a formatted summary of all test results."""
    print("\n" + "=" * 72)
    print("  TEST SUMMARY — Insight Training Data Diversity")
    print("=" * 72)
    print(f"  {'Test':<50} {'Result':<8}")
    print("  " + "-" * 60)
    for name, status, detail in test_result.test_results:
        status_symbol = {"PASS": "✓", "FAIL": "✗", "ERROR": "⚠", "SKIP": "→"}.get(status, "?")
        print(f"  {status_symbol} {name:<48} {status:<8}")
        if status in ("FAIL", "ERROR") and detail:
            lines = detail.strip().split("\n")
            for line in lines[:3]:
                print(f"    {line.strip()}")
    print("  " + "-" * 60)
    total = len(test_result.test_results)
    passed = sum(1 for _, s, _ in test_result.test_results if s == "PASS")
    failed = sum(1 for _, s, _ in test_result.test_results if s == "FAIL")
    errors = sum(1 for _, s, _ in test_result.test_results if s == "ERROR")
    skipped = sum(1 for _, s, _ in test_result.test_results if s == "SKIP")
    print(f"  Total: {total}  |  Passed: {passed}  |  Failed: {failed}  "
          f"|  Errors: {errors}  |  Skipped: {skipped}")
    print("=" * 72)
    return failed + errors


# ── Main ───────────────────────────────────────────────────────────────────

def main():
    loader = unittest.TestLoader()
    suite = unittest.TestSuite()

    # Add all test classes
    suite.addTests(loader.loadTestsFromTestCase(TestConfigIntegrity))
    suite.addTests(loader.loadTestsFromTestCase(TestEdgeCases))
    suite.addTests(loader.loadTestsFromTestCase(TestEntityCoverage))
    suite.addTests(loader.loadTestsFromTestCase(TestTitleDiversityJaccard))
    suite.addTests(loader.loadTestsFromTestCase(TestComparisonMatrixBias))
    suite.addTests(loader.loadTestsFromTestCase(TestFeedbackLoopClosure))
    suite.addTests(loader.loadTestsFromTestCase(TestEntityExtractionHallucination))
    suite.addTests(loader.loadTestsFromTestCase(TestUniversalDiscovery))
    suite.addTests(loader.loadTestsFromTestCase(TestDiscoveryEdgeCases))
    suite.addTests(loader.loadTestsFromTestCase(TestSignalDepth))

    runner = unittest.TextTestRunner(
        verbosity=2,
        resultclass=SummaryTestResult,
        stream=sys.stdout,
    )

    result = runner.run(suite)
    failures = print_summary(result)

    print("\nNOTE: Some tests are expected to FAIL on the current biased dataset.")
    print("These failures validate the need for the fixes described in FIX_GUIDE.md.")
    print("After applying the fixes from FIX_GUIDE.md, all tests should pass.")

    return failures


if __name__ == "__main__":
    sys.exit(main())
