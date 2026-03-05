//! Website change diffing engine.
//!
//! When change_detection detects a page change (new_hash != old_hash),
//! computes a human-readable textual diff and stores the diff summary
//! in the observation value.

use serde::Serialize;

// ─── Diff types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DiffResult {
    /// Number of lines added
    pub lines_added: usize,
    /// Number of lines removed
    pub lines_removed: usize,
    /// Number of lines unchanged
    pub lines_unchanged: usize,
    /// Human-readable summary
    pub summary: String,
    /// Individual change hunks
    pub hunks: Vec<DiffHunk>,
    /// Change magnitude: "minor" | "moderate" | "major"
    pub magnitude: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffHunk {
    /// Where in the old text this hunk starts
    pub old_start: usize,
    /// Where in the new text this hunk starts
    pub new_start: usize,
    /// Lines in this hunk
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub content: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum DiffKind {
    Added,
    Removed,
    Context,
}

// ─── Diff algorithm (Myers-like LCS) ───────────────────────────────────

/// Compute a textual diff between old and new content.
pub fn compute_diff(old: &str, new: &str) -> DiffResult {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let lcs = longest_common_subsequence(&old_lines, &new_lines);
    let mut diff_lines = Vec::new();
    let mut old_idx = 0;
    let mut new_idx = 0;
    let mut lcs_idx = 0;

    while old_idx < old_lines.len() || new_idx < new_lines.len() {
        if lcs_idx < lcs.len() {
            // Emit removed lines (in old but not matching LCS)
            while old_idx < old_lines.len() && old_lines[old_idx] != lcs[lcs_idx] {
                diff_lines.push(DiffLine {
                    kind: DiffKind::Removed,
                    content: old_lines[old_idx].to_string(),
                });
                old_idx += 1;
            }
            // Emit added lines (in new but not matching LCS)
            while new_idx < new_lines.len() && new_lines[new_idx] != lcs[lcs_idx] {
                diff_lines.push(DiffLine {
                    kind: DiffKind::Added,
                    content: new_lines[new_idx].to_string(),
                });
                new_idx += 1;
            }
            // Emit context (matching line)
            if old_idx < old_lines.len() && new_idx < new_lines.len() {
                diff_lines.push(DiffLine {
                    kind: DiffKind::Context,
                    content: old_lines[old_idx].to_string(),
                });
                old_idx += 1;
                new_idx += 1;
                lcs_idx += 1;
            }
        } else {
            // Remaining old lines are removed
            while old_idx < old_lines.len() {
                diff_lines.push(DiffLine {
                    kind: DiffKind::Removed,
                    content: old_lines[old_idx].to_string(),
                });
                old_idx += 1;
            }
            // Remaining new lines are added
            while new_idx < new_lines.len() {
                diff_lines.push(DiffLine {
                    kind: DiffKind::Added,
                    content: new_lines[new_idx].to_string(),
                });
                new_idx += 1;
            }
        }
    }

    // Count stats
    let added = diff_lines.iter().filter(|l| l.kind == DiffKind::Added).count();
    let removed = diff_lines.iter().filter(|l| l.kind == DiffKind::Removed).count();
    let unchanged = diff_lines.iter().filter(|l| l.kind == DiffKind::Context).count();

    let total_change = added + removed;
    let total_lines = old_lines.len().max(new_lines.len()).max(1);
    let change_ratio = total_change as f64 / total_lines as f64;
    let magnitude = if change_ratio < 0.1 {
        "minor"
    } else if change_ratio < 0.4 {
        "moderate"
    } else {
        "major"
    };

    // Build hunks from consecutive changes
    let hunks = build_hunks(&diff_lines);

    let summary = format!(
        "{} lines added, {} lines removed ({} change)",
        added, removed, magnitude
    );

    DiffResult {
        lines_added: added,
        lines_removed: removed,
        lines_unchanged: unchanged,
        summary,
        hunks,
        magnitude: magnitude.into(),
    }
}

/// Build hunks from diff lines (group consecutive changes with context).
fn build_hunks(lines: &[DiffLine]) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;
    let mut context_gap = 0;
    let mut old_pos = 0;
    let mut new_pos = 0;

    for line in lines {
        match line.kind {
            DiffKind::Context => {
                context_gap += 1;
                if context_gap > 3 {
                    if let Some(hunk) = current_hunk.take() {
                        hunks.push(hunk);
                    }
                }
                old_pos += 1;
                new_pos += 1;
            }
            DiffKind::Added | DiffKind::Removed => {
                context_gap = 0;
                if current_hunk.is_none() {
                    current_hunk = Some(DiffHunk {
                        old_start: old_pos + 1,
                        new_start: new_pos + 1,
                        lines: Vec::new(),
                    });
                }
                if let Some(ref mut hunk) = current_hunk {
                    hunk.lines.push(line.clone());
                }
                match line.kind {
                    DiffKind::Removed => old_pos += 1,
                    DiffKind::Added => new_pos += 1,
                    _ => {}
                }
            }
        }
    }

    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    hunks
}

/// LCS (Longest Common Subsequence) of two line arrays.
fn longest_common_subsequence<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<&'a str> {
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for i in 1..=m {
        for j in 1..=n {
            if a[i - 1] == b[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    // Backtrack to find LCS
    let mut result = Vec::new();
    let mut i = m;
    let mut j = n;
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            result.push(a[i - 1]);
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] > dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    result.reverse();
    result
}

/// Generate a compact human-readable diff summary suitable for storage.
pub fn summarize_changes(old: &str, new: &str, max_lines: usize) -> String {
    let diff = compute_diff(old, new);
    let mut summary = vec![diff.summary.clone()];

    for hunk in &diff.hunks {
        for line in hunk.lines.iter().take(max_lines) {
            match line.kind {
                DiffKind::Added => summary.push(format!("+ {}", line.content)),
                DiffKind::Removed => summary.push(format!("- {}", line.content)),
                DiffKind::Context => {}
            }
        }
        if hunk.lines.len() > max_lines {
            summary.push(format!("  ... ({} more changes)", hunk.lines.len() - max_lines));
        }
    }

    summary.join("\n")
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_content() {
        let text = "line 1\nline 2\nline 3";
        let diff = compute_diff(text, text);
        assert_eq!(diff.lines_added, 0);
        assert_eq!(diff.lines_removed, 0);
        assert_eq!(diff.lines_unchanged, 3);
    }

    #[test]
    fn test_added_line() {
        let old = "line 1\nline 2";
        let new = "line 1\nline 2\nline 3";
        let diff = compute_diff(old, new);
        assert_eq!(diff.lines_added, 1);
        assert_eq!(diff.lines_removed, 0);
    }

    #[test]
    fn test_removed_line() {
        let old = "line 1\nline 2\nline 3";
        let new = "line 1\nline 3";
        let diff = compute_diff(old, new);
        assert_eq!(diff.lines_removed, 1);
        assert_eq!(diff.lines_added, 0);
    }

    #[test]
    fn test_modification() {
        let old = "Company: ACME Corp\nProducts: PCB, PCBA\nLocation: Tunisia";
        let new = "Company: ACME Corp\nProducts: PCB, PCBA, Connectors\nLocation: Tunisia";
        let diff = compute_diff(old, new);
        assert_eq!(diff.lines_added, 1);
        assert_eq!(diff.lines_removed, 1);
        assert_eq!(diff.magnitude, "moderate");
    }

    #[test]
    fn test_major_change() {
        let old = "A\nB\nC\nD\nE";
        let new = "X\nY\nZ\nW\nV";
        let diff = compute_diff(old, new);
        assert_eq!(diff.magnitude, "major");
    }

    #[test]
    fn test_minor_change() {
        let old = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8\nLine 9\nLine 10";
        let new = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8\nLine 9\nLine 10 updated";
        let diff = compute_diff(old, new);
        assert_eq!(diff.magnitude, "minor");
    }

    #[test]
    fn test_summarize_changes() {
        let old = "Name: ACME\nRevenue: $10M";
        let new = "Name: ACME\nRevenue: $15M";
        let summary = summarize_changes(old, new, 5);
        assert!(summary.contains("added"));
        assert!(summary.contains("removed"));
    }

    #[test]
    fn test_empty_to_content() {
        let diff = compute_diff("", "new content\nline 2");
        assert_eq!(diff.lines_added, 2);
        assert_eq!(diff.lines_removed, 0);
    }

    #[test]
    fn test_content_to_empty() {
        let diff = compute_diff("old content\nline 2", "");
        assert_eq!(diff.lines_removed, 2);
        assert_eq!(diff.lines_added, 0);
    }
}
