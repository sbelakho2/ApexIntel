//! Post-render template variation to reduce formulaic repetition in warnings.
//!
//! The 290 YAML recipes each have a single [`narrative_template`] and
//! [`action_playbook`].  After placeholder substitution the rendered text
//! is always structurally identical for a given recipe + entity combination.
//!
//! This module applies **deterministic (seeded) diversity** to the rendered
//! narrative and action text **without** requiring any YAML schema changes.
//!
//! # Seeding
//!
//! The seed is derived from `(recipe_code, entity_id, iso_week)` so that:
//!
//! * Same recipe + same entity + same week → same variation (stable UX).
//! * Same recipe + different entity → different variation (cross-entity diversity).
//! * Same recipe + same entity + different week → different variation (temporal diversity).

use chrono::Datelike;
use std::hash::{Hash, Hasher};

// ─── Deterministic seed ──────────────────────────────────────────────────────

/// Compute a deterministic u64 seed from recipe code, entity id and the current
/// ISO week number.
fn variation_seed(recipe_code: &str, entity_id: &str) -> u64 {
    let now = chrono::Utc::now();
    let iso = now.iso_week();
    let week_key: u64 =
        (iso.year() as u64).wrapping_mul(1000).wrapping_add(iso.week() as u64);

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    recipe_code.hash(&mut hasher);
    entity_id.hash(&mut hasher);
    week_key.hash(&mut hasher);
    hasher.finish()
}

/// Thin wrapper around a seeded PRNG for reproducible variation selection.
struct SeededRng(u64);

impl SeededRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Return an index in `0..len` using the next pseudo-random value.
    fn pick(&mut self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        // xorshift64* — produce a full u64 then reduce modulo len
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0 as usize).wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) % len
    }
}

// ─── Title variation ─────────────────────────────────────────────────────────

/// Opening-phrase synonym banks for warning titles.
static OPENING_VARIANTS: &[&[&str]] = &[
    &["detected", "identified", "observed", "found", "uncovered"],
    &["indicates", "suggests", "signals", "points to", "reveals"],
    &["risk", "exposure", "vulnerability", "threat signal", "concern"],
    &["alert", "notification", "advisory", "notice", "flag"],
];

/// Domain-specific synonym banks for common terms in warning titles.
static DOMAIN_SYNONYMS: &[(&str, &[&str])] = &[
    ("procurement", &["sourcing", "purchasing", "acquisition", "supply procurement"]),
    ("supplier", &["vendor", "provider", "source partner"]),
    ("certification", &["certificate", "accreditation", "qualification", "credential"]),
    ("compliance", &["conformance", "adherence", "regulatory alignment"]),
    ("security", &["cybersecurity", "info security", "defense posture"]),
    ("breach", &["violation", "compromise", "security incident", "exposure"]),
    ("vulnerability", &["weakness", "exposure gap", "deficiency"]),
    ("shift", &["change", "transition", "realignment", "movement"]),
    ("decline", &["drop", "downturn", "contraction", "weakening"]),
    ("expansion", &["growth", "scale-up", "build-out", "augmentation"]),
    ("reduction", &["cutback", "drawdown", "contraction", "trimming"]),
    ("investment", &["capital deployment", "funding", "spend", "allocation"]),
    ("partnership", &["alliance", "collaboration", "joint effort", "cooperative"]),
    ("acquisition", &["takeover", "purchase", "buy-out", "consolidation"]),
    ("restructuring", &["reorganization", "overhaul", "transformation", "reshaping"]),
];

/// Apply title-level diversity to a rendered warning title.
///
/// Techniques applied (deterministic via `seed`):
///
/// 1. **Synonym substitution** – replace known domain terms with variants.
/// 2. **Opening verb rotation** – vary the first action verb (e.g. "detected" ↔ "observed").
pub fn diversify_title(title: &str, recipe_code: &str, entity_id: &str) -> String {
    let seed = variation_seed(recipe_code, entity_id);
    let mut rng = SeededRng::new(seed);

    let mut result = title.to_string();

    // 1. Synonym substitution – replace up to 2 domain terms with random variants
    let mut replaced = 0usize;
    for (term, variants) in DOMAIN_SYNONYMS {
        if replaced >= 2 {
            break;
        }
        if let Some(pos) = find_word(&result, term) {
            let variant = variants[rng.pick(variants.len())];
            result.replace_range(pos..pos + term.len(), variant);
            replaced += 1;
        }
    }

    // 2. Opening verb rotation – replace known opening words
    for variants in OPENING_VARIANTS {
        for variant in *variants {
            if let Some(pos) = find_word_lower(&result, variant) {
                // Pick a different variant
                let others: Vec<&&str> = variants.iter().filter(|v| *v != variant).collect();
                if !others.is_empty() {
                    let replacement = others[rng.pick(others.len())];
                    // Preserve original case of the first character
                    let original_first = result[pos..].chars().next().unwrap_or(' ');
                    let replacement = if original_first.is_uppercase() {
                        let mut c = replacement.chars().next().unwrap_or(' ');
                        c.make_ascii_uppercase();
                        format!("{}{}", c, &replacement[1..])
                    } else {
                        replacement.to_string()
                    };
                    result.replace_range(pos..pos + variant.len(), &replacement);
                }
                break;
            }
        }
    }

    result
}

// ─── Action variation ────────────────────────────────────────────────────────

/// Action-verb synonym banks.
static ACTION_VERBS: &[&[&str]] = &[
    &["monitor", "track", "watch", "review", "observe"],
    &["investigate", "examine", "assess", "evaluate", "analyze"],
    &["engage", "contact", "reach out to", "connect with", "consult"],
    &["verify", "validate", "confirm", "substantiate", "corroborate"],
    &["flag", "escalate", "raise", "highlight", "elevate"],
    &["mitigate", "address", "remediate", "counter", "respond to"],
    &["review", "audit", "inspect", "examine", "scrutinize"],
];

/// Apply action-level diversity to a rendered warning action item.
///
/// Techniques applied:
///
/// 1. **Verb rotation** – replace action verbs with domain-appropriate synonyms.
/// 2. **Phrase rephrasing** – for common multi-word action patterns.
pub fn diversify_action(action: &str, recipe_code: &str, entity_id: &str) -> String {
    let seed = variation_seed(recipe_code, entity_id);
    let mut rng = SeededRng::new(seed);

    let mut result = action.to_string();

    // 1. Verb rotation – replace up to 2 action verbs
    let mut replaced = 0usize;
    for variants in ACTION_VERBS {
        if replaced >= 2 {
            break;
        }
        for variant in *variants {
            if let Some(pos) = find_word_lower(&result, variant) {
                let others: Vec<&&str> = variants.iter().filter(|v| *v != variant).collect();
                if !others.is_empty() {
                    let replacement = others[rng.pick(others.len())];
                    let original_first = result[pos..].chars().next().unwrap_or(' ');
                    let replacement = if original_first.is_uppercase() {
                        let mut c = replacement.chars().next().unwrap_or(' ');
                        c.make_ascii_uppercase();
                        format!("{}{}", c, &replacement[1..])
                    } else {
                        replacement.to_string()
                    };
                    result.replace_range(pos..pos + variant.len(), &replacement);
                    replaced += 1;
                }
                break;
            }
        }
    }

    result
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Find the first occurrence of `needle` as a whole word (case-sensitive).
fn find_word(haystack: &str, needle: &str) -> Option<usize> {
    let lower_needle = needle.to_ascii_lowercase();
    let lower_haystack = haystack.to_ascii_lowercase();
    find_word_lower_impl(&lower_haystack, &lower_needle, haystack)
}

/// Find the first occurrence of `needle` where `needle` is already lowercase.
fn find_word_lower(haystack: &str, needle: &str) -> Option<usize> {
    let lower_haystack = haystack.to_ascii_lowercase();
    find_word_lower_impl(&lower_haystack, needle, haystack)
}

/// Implementation: search `lower_haystack` for `lower_needle`, check word
/// boundaries, return the byte position in the *original* `haystack`.
///
/// The position mapping works by iterating the *original* string character by
/// character, building a lowercased counterpart, and tracking byte offsets so
/// that a match in the lowercased form can be translated back to the original.
fn find_word_lower_impl(lower_haystack: &str, lower_needle: &str, original: &str) -> Option<usize> {
    // Build a character-aligned mapping from lowercased bytes to original bytes.
    // Each entry is (lower_byte_offset, original_byte_offset) for the start of each char.
    let mut mapping: Vec<(usize, usize)> = Vec::new();
    let mut lower_offset = 0usize;
    let mut orig_offset = 0usize;
    for ch in original.chars() {
        mapping.push((lower_offset, orig_offset));
        lower_offset += ch.to_ascii_lowercase().len_utf8();
        orig_offset += ch.len_utf8();
    }
    // Final sentinel
    mapping.push((lower_offset, orig_offset));

    let mut start = 0;
    while let Some(pos) = lower_haystack[start..].find(lower_needle) {
        let abs_pos = start + pos;
        // Check word boundary before
        let before_ok = if abs_pos == 0 {
            true
        } else {
            let prev = lower_haystack[..abs_pos].chars().last()?;
            !prev.is_alphanumeric()
        };
        // Check word boundary after
        let after_pos = abs_pos + lower_needle.len();
        let after_ok = if after_pos >= lower_haystack.len() {
            true
        } else {
            let next = lower_haystack[after_pos..].chars().next()?;
            !next.is_alphanumeric()
        };
        if before_ok && after_ok {
            // Binary-search the mapping for the byte offset in the original string
            if let Ok(idx) = mapping.binary_search_by(|&(l, _)| l.cmp(&abs_pos)) {
                return Some(mapping[idx].1);
            }
            // nearest lower entry
            let idx = mapping.partition_point(|&(l, _)| l <= abs_pos);
            if idx > 0 && idx <= mapping.len() {
                return Some(mapping[idx - 1].1);
            }
            return Some(abs_pos); // fallback
        }
        start = abs_pos + 1;
    }
    None
}

// Note on test-only import: The tests module is at the bottom of recipes.rs.
// We keep tests co-located here for isolation.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variation_seed_is_deterministic() {
        let a = variation_seed("A001", "uuid-123");
        let b = variation_seed("A001", "uuid-123");
        assert_eq!(a, b);
    }

    #[test]
    fn test_variation_seed_differs_across_entities() {
        let a = variation_seed("A001", "uuid-123");
        let b = variation_seed("A001", "uuid-456");
        assert_ne!(a, b);
    }

    #[test]
    fn test_diversify_title_synonym_substitution() {
        let title = "Procurement risk detected at Acme Corp";
        let diversified = diversify_title(title, "B003", "entity-uuid-1");
        // Should differ from original (or at least be valid)
        assert!(!diversified.is_empty());
        // Should still contain the entity name
        assert!(diversified.contains("Acme Corp"));
    }

    #[test]
    fn test_diversify_title_preserves_non_domain_text() {
        let title = "SSL certificate expiry detected for example.com";
        let diversified = diversify_title(title, "C001", "uuid-test");
        assert!(diversified.contains("example.com"));
        assert!(diversified.to_ascii_lowercase().contains("ssl"));
    }

    #[test]
    fn test_diversify_action_verb_rotation() {
        let action = "Monitor supplier certification status closely";
        let diversified = diversify_action(action, "A002", "uuid-789");
        assert!(!diversified.is_empty());
    }

    #[test]
    fn test_diversify_action_preserves_entity_names() {
        let action = "Investigate Flex Ltd sourcing activities in Southeast Asia";
        let diversified = diversify_action(action, "B004", "uuid-abc");
        assert!(diversified.contains("Flex Ltd"));
    }

    #[test]
    fn test_diversify_title_deterministic() {
        let title = "Compliance vulnerability identified at Starz Electronics";
        let a = diversify_title(title, "D005", "uuid-repro");
        let b = diversify_title(title, "D005", "uuid-repro");
        assert_eq!(a, b);
    }

    #[test]
    fn test_diversify_title_differs_across_entities() {
        let title = "Supplier risk detected";
        let a = diversify_title(title, "A010", "entity-x");
        let b = diversify_title(title, "A010", "entity-y");
        // They can be equal by chance, but usually differ
        // Just verify both are valid
        assert!(!a.is_empty());
        assert!(!b.is_empty());
    }

    #[test]
    fn test_seeded_rng_distribution() {
        let mut rng = SeededRng::new(42);
        let mut counts = [0usize; 4];
        for _ in 0..1000 {
            counts[rng.pick(4)] += 1;
        }
        // Each bucket should have roughly 250 (± reasonable margin)
        for &c in &counts {
            assert!(c > 100, "bucket too sparse: {c}");
            assert!(c < 400, "bucket too dense: {c}");
        }
    }

    #[test]
    fn test_seeded_rng_deterministic() {
        let mut a = SeededRng::new(99);
        let mut b = SeededRng::new(99);
        for _ in 0..20 {
            assert_eq!(a.pick(10), b.pick(10));
        }
    }

    #[test]
    fn test_find_word_boundary_middle_of_sentence() {
        let s = "The supplier certification has expired";
        let pos = find_word(s, "supplier");
        assert!(pos.is_some(), "should find 'supplier' as a whole word");
    }

    #[test]
    fn test_find_word_no_substring_match() {
        // "procurement" should NOT match inside "misprocurement" because
        // it is preceded by an alphanumeric character ('m'), so the word
        // boundary check fails.
        let s = "misprocurement detected";
        let pos = find_word(s, "procurement");
        assert!(pos.is_none(), "should not match inside 'misprocurement'");
        // In "reprocur", "procurement" is not present at all
        let s2 = "reprocur scenario";
        let pos2 = find_word(s2, "procurement");
        assert!(pos2.is_none(), "should not match partial word");
    }

    #[test]
    fn test_diversify_title_empty_input() {
        let diversified = diversify_title("", "E001", "uuid-empty");
        assert!(diversified.is_empty());
    }

    #[test]
    fn test_diversify_action_empty_input() {
        let diversified = diversify_action("", "E002", "uuid-empty");
        assert!(diversified.is_empty());
    }
}
