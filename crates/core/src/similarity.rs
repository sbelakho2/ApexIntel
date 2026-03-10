use std::collections::HashSet;
use std::hash::Hash;

pub fn jaccard_similarity<T>(left: &HashSet<T>, right: &HashSet<T>) -> f64
where
    T: Eq + Hash,
{
    if left.is_empty() && right.is_empty() {
        return 0.0;
    }

    let union = left.union(right).count();
    if union == 0 {
        return 0.0;
    }

    shared_member_count(left, right) as f64 / union as f64
}

pub fn shared_member_count<T>(left: &HashSet<T>, right: &HashSet<T>) -> usize
where
    T: Eq + Hash,
{
    left.intersection(right).count()
}

pub fn trigram_similarity(left: &str, right: &str) -> f64 {
    let left_trigrams = trigrams(left);
    let right_trigrams = trigrams(right);
    jaccard_similarity(&left_trigrams, &right_trigrams)
}

fn trigrams(input: &str) -> HashSet<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut set = HashSet::new();

    if chars.len() < 3 {
        let padded: Vec<char> = format!(" {} ", input).chars().collect();
        for window in padded.windows(3) {
            set.insert(window.iter().collect());
        }
        return set;
    }

    for window in chars.windows(3) {
        set.insert(window.iter().collect());
    }

    set
}

#[cfg(test)]
mod tests {
    use std::iter::FromIterator;

    use super::{jaccard_similarity, shared_member_count, trigram_similarity};

    #[test]
    fn jaccard_similarity_returns_zero_for_empty_sets() {
        let left = std::collections::HashSet::<String>::new();
        let right = std::collections::HashSet::<String>::new();

        assert_eq!(jaccard_similarity(&left, &right), 0.0);
    }

    #[test]
    fn shared_member_count_returns_intersection_size() {
        let left = std::collections::HashSet::from_iter(["alice", "bob", "carol"]);
        let right = std::collections::HashSet::from_iter(["bob", "carol", "dave"]);

        assert_eq!(shared_member_count(&left, &right), 2);
        assert!((jaccard_similarity(&left, &right) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn trigram_similarity_handles_short_strings_with_padding() {
        let sim = trigram_similarity("ab", "ab");

        assert!((sim - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn trigram_similarity_preserves_relative_overlap() {
        let similar = trigram_similarity("starz electronics", "starz electronik");
        let different = trigram_similarity("foxconn", "samsung");

        assert!(similar > 0.5);
        assert!(different < 0.2);
        assert!(similar > different);
    }
}
