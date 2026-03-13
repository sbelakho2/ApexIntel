//! Pagination — cursor-based and offset-based pagination logic.
//!
//! Pure helpers for computing page metadata, applying limits/offsets,
//! and building pagination response envelopes.

use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────
// Query params
// ────────────────────────────────────────────

/// Pagination query parameters (offset-based).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageParams {
    pub page: u32,
    pub per_page: u32,
}

impl Default for PageParams {
    fn default() -> Self {
        Self {
            page: 1,
            per_page: 25,
        }
    }
}

impl PageParams {
    pub fn new(page: u32, per_page: u32) -> Self {
        Self {
            page: page.max(1),
            per_page: per_page.clamp(1, 100),
        }
    }

    /// SQL OFFSET value.
    pub fn offset(&self) -> u32 {
        (self.page.max(1) - 1).saturating_mul(self.per_page)
    }

    /// SQL LIMIT value.
    pub fn limit(&self) -> u32 {
        self.per_page
    }

    /// Validate and sanitize page params.
    pub fn sanitize(&self) -> Self {
        Self::new(self.page, self.per_page)
    }
}

// ────────────────────────────────────────────
// Cursor-based pagination
// ────────────────────────────────────────────

/// Cursor-based pagination (for real-time feeds).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorParams {
    pub cursor: Option<String>,
    pub limit: u32,
    pub direction: CursorDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CursorDirection {
    Forward,
    Backward,
}

impl Default for CursorParams {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: 25,
            direction: CursorDirection::Forward,
        }
    }
}

impl CursorParams {
    pub fn sanitize(&self) -> Self {
        Self {
            cursor: self.cursor.clone(),
            limit: self.limit.clamp(1, 100),
            direction: self.direction.clone(),
        }
    }
}

// ────────────────────────────────────────────
// Page metadata
// ────────────────────────────────────────────

/// Computed pagination metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PageMeta {
    pub page: u32,
    pub per_page: u32,
    pub total_items: u64,
    pub total_pages: u32,
    pub has_next: bool,
    pub has_prev: bool,
}

/// Compute page metadata from params and total count.
pub fn compute_page_meta(params: &PageParams, total_items: u64) -> PageMeta {
    let per_page = params.per_page.max(1);
    // Integer ceiling division, capped at u32::MAX to avoid f64→u32 truncation on huge totals.
    let total_pages = if total_items == 0 {
        0u32
    } else {
        let tp = total_items.saturating_add(per_page as u64 - 1) / per_page.max(1) as u64;
        tp.min(u32::MAX as u64) as u32
    };
    let page = params.page.max(1).min(total_pages.max(1));

    PageMeta {
        page,
        per_page,
        total_items,
        total_pages,
        has_next: page < total_pages,
        has_prev: page > 1,
    }
}

/// Cursor metadata for cursor-based responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorMeta {
    pub next_cursor: Option<String>,
    pub prev_cursor: Option<String>,
    pub has_more: bool,
    pub count: u32,
}

/// Compute cursor metadata from items returned.
pub fn compute_cursor_meta(
    items_returned: u32,
    limit: u32,
    first_id: Option<&str>,
    last_id: Option<&str>,
) -> CursorMeta {
    CursorMeta {
        next_cursor: if items_returned >= limit {
            last_id.map(|s| s.to_string())
        } else {
            None
        },
        prev_cursor: first_id.map(|s| s.to_string()),
        has_more: items_returned >= limit,
        count: items_returned,
    }
}

// ────────────────────────────────────────────
// Paginated envelope
// ────────────────────────────────────────────

/// Generic paginated response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub meta: PageMeta,
}

/// Build a paginated response from a full dataset (for in-memory pagination).
pub fn paginate_in_memory<T: Clone>(items: &[T], params: &PageParams) -> PaginatedResponse<T> {
    let total = items.len() as u64;
    let meta = compute_page_meta(params, total);
    // Use the clamped page from meta (not raw params.offset()) so data matches metadata.
    let start = ((meta.page.max(1) - 1) as usize) * (meta.per_page as usize);
    let end = (start + meta.per_page as usize).min(items.len());
    let data = if start < items.len() {
        items[start..end].to_vec()
    } else {
        vec![]
    };
    PaginatedResponse { data, meta }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── PageParams ──

    #[test]
    fn test_page_params_default() {
        let p = PageParams::default();
        assert_eq!(p.page, 1);
        assert_eq!(p.per_page, 25);
    }

    #[test]
    fn test_page_params_offset() {
        let p = PageParams::new(3, 10);
        assert_eq!(p.offset(), 20); // (3-1)*10
        assert_eq!(p.limit(), 10);
    }

    #[test]
    fn test_page_params_page_zero_clamped() {
        let p = PageParams::new(0, 10);
        assert_eq!(p.page, 1);
        assert_eq!(p.offset(), 0);
    }

    #[test]
    fn test_page_params_per_page_clamped() {
        let p = PageParams::new(1, 500);
        assert_eq!(p.per_page, 100); // max 100

        let p2 = PageParams::new(1, 0);
        assert_eq!(p2.per_page, 1); // min 1
    }

    #[test]
    fn test_page_params_sanitize() {
        let p = PageParams {
            page: 0,
            per_page: 999,
        };
        let s = p.sanitize();
        assert_eq!(s.page, 1);
        assert_eq!(s.per_page, 100);
    }

    // ── CursorParams ──

    #[test]
    fn test_cursor_params_default() {
        let c = CursorParams::default();
        assert!(c.cursor.is_none());
        assert_eq!(c.limit, 25);
        assert_eq!(c.direction, CursorDirection::Forward);
    }

    #[test]
    fn test_cursor_params_sanitize() {
        let c = CursorParams {
            cursor: Some("abc".to_string()),
            limit: 500,
            direction: CursorDirection::Backward,
        };
        let s = c.sanitize();
        assert_eq!(s.limit, 100); // clamped
        assert_eq!(s.direction, CursorDirection::Backward);
    }

    #[test]
    fn test_page_params_deserialize_rejects_unknown_fields() {
        let json = r#"{"page":1,"per_page":25,"extra":"boom"}"#;
        let parsed = serde_json::from_str::<PageParams>(json);
        assert!(parsed.is_err(), "unknown fields must be rejected");
    }

    #[test]
    fn test_cursor_params_deserialize_rejects_unknown_fields() {
        let json = r#"{"cursor":null,"limit":25,"direction":"Forward","extra":1}"#;
        let parsed = serde_json::from_str::<CursorParams>(json);
        assert!(parsed.is_err(), "unknown fields must be rejected");
    }

    // ── PageMeta ──

    #[test]
    fn test_compute_page_meta_first_page() {
        let params = PageParams::new(1, 10);
        let meta = compute_page_meta(&params, 35);
        assert_eq!(meta.page, 1);
        assert_eq!(meta.per_page, 10);
        assert_eq!(meta.total_items, 35);
        assert_eq!(meta.total_pages, 4); // ceil(35/10)
        assert!(meta.has_next);
        assert!(!meta.has_prev);
    }

    #[test]
    fn test_compute_page_meta_middle_page() {
        let params = PageParams::new(2, 10);
        let meta = compute_page_meta(&params, 35);
        assert_eq!(meta.page, 2);
        assert!(meta.has_next);
        assert!(meta.has_prev);
    }

    #[test]
    fn test_compute_page_meta_last_page() {
        let params = PageParams::new(4, 10);
        let meta = compute_page_meta(&params, 35);
        assert_eq!(meta.page, 4);
        assert!(!meta.has_next);
        assert!(meta.has_prev);
    }

    #[test]
    fn test_compute_page_meta_beyond_last() {
        let params = PageParams::new(10, 10);
        let meta = compute_page_meta(&params, 35);
        assert_eq!(meta.page, 4); // clamped to last
        assert!(!meta.has_next);
    }

    #[test]
    fn test_compute_page_meta_empty() {
        let params = PageParams::new(1, 10);
        let meta = compute_page_meta(&params, 0);
        assert_eq!(meta.page, 1);
        assert_eq!(meta.total_pages, 0);
        assert!(!meta.has_next);
        assert!(!meta.has_prev);
    }

    #[test]
    fn test_compute_page_meta_exact_fit() {
        let params = PageParams::new(1, 10);
        let meta = compute_page_meta(&params, 10);
        assert_eq!(meta.total_pages, 1);
        assert!(!meta.has_next);
        assert!(!meta.has_prev);
    }

    // ── CursorMeta ──

    #[test]
    fn test_cursor_meta_has_more() {
        let meta = compute_cursor_meta(25, 25, Some("first"), Some("last"));
        assert!(meta.has_more);
        assert_eq!(meta.next_cursor, Some("last".to_string()));
        assert_eq!(meta.prev_cursor, Some("first".to_string()));
    }

    #[test]
    fn test_cursor_meta_no_more() {
        let meta = compute_cursor_meta(10, 25, Some("first"), Some("last"));
        assert!(!meta.has_more);
        assert_eq!(meta.next_cursor, None);
    }

    #[test]
    fn test_cursor_meta_empty() {
        let meta = compute_cursor_meta(0, 25, None, None);
        assert!(!meta.has_more);
        assert!(meta.next_cursor.is_none());
        assert!(meta.prev_cursor.is_none());
    }

    // ── In-memory pagination ──

    #[test]
    fn test_paginate_in_memory_first_page() {
        let items: Vec<i32> = (1..=35).collect();
        let params = PageParams::new(1, 10);
        let resp = paginate_in_memory(&items, &params);
        assert_eq!(resp.data.len(), 10);
        assert_eq!(resp.data[0], 1);
        assert_eq!(resp.data[9], 10);
        assert_eq!(resp.meta.total_items, 35);
    }

    #[test]
    fn test_paginate_in_memory_last_page() {
        let items: Vec<i32> = (1..=35).collect();
        let params = PageParams::new(4, 10);
        let resp = paginate_in_memory(&items, &params);
        assert_eq!(resp.data.len(), 5); // 31-35
        assert_eq!(resp.data[0], 31);
    }

    #[test]
    fn test_paginate_in_memory_empty() {
        let items: Vec<i32> = vec![];
        let params = PageParams::new(1, 10);
        let resp = paginate_in_memory(&items, &params);
        assert!(resp.data.is_empty());
        assert_eq!(resp.meta.total_items, 0);
    }

    #[test]
    fn test_paginate_in_memory_beyond_range() {
        // Requesting page 10 of 5 items (1 page total) → clamps to page 1, returns all data.
        // Meta and data are now consistent (previously meta said page 1 but data was empty).
        let items: Vec<i32> = (1..=5).collect();
        let params = PageParams::new(10, 10);
        let resp = paginate_in_memory(&items, &params);
        assert_eq!(resp.meta.page, 1);
        assert_eq!(resp.data.len(), 5);
        assert_eq!(resp.data[0], 1);
    }

    // ── Serialization ──

    #[test]
    fn test_page_meta_serialization() {
        let meta = compute_page_meta(&PageParams::new(2, 10), 50);
        let json = serde_json::to_string(&meta).unwrap();
        let back: PageMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back, meta);
    }

    #[test]
    fn test_paginated_response_serialization() {
        let resp = paginate_in_memory(&[1, 2, 3], &PageParams::new(1, 10));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"data\""));
        assert!(json.contains("\"meta\""));
    }
}
