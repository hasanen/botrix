//! Shared query-string pagination parameters for list endpoints.

use serde::Deserialize;

/// Default page size for `GET /api/v1/notifications`.
pub const DEFAULT_NOTIFICATIONS_PER_PAGE: u32 = 25;
/// Default page size for `GET /api/v1/logs`.
pub const DEFAULT_LOGS_PER_PAGE: u32 = 50;
/// Upper bound on `per_page` to prevent unbounded result sets.
const MAX_PER_PAGE: u32 = 100;

/// `?page=&per_page=` query parameters, both optional.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Pagination {
    page: Option<u32>,
    per_page: Option<u32>,
}

impl Pagination {
    /// The requested page, floored to 1.
    pub fn page(&self) -> u32 {
        self.page.unwrap_or(1).max(1)
    }

    /// The requested page size, falling back to `default` and clamped to
    /// `[1, MAX_PER_PAGE]`.
    pub fn per_page(&self, default: u32) -> u32 {
        self.per_page.unwrap_or(default).clamp(1, MAX_PER_PAGE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_values_fall_back_to_defaults() {
        let p = Pagination {
            page: None,
            per_page: None,
        };
        assert_eq!(p.page(), 1);
        assert_eq!(p.per_page(25), 25);
    }

    #[test]
    fn zero_page_is_floored_to_one() {
        let p = Pagination {
            page: Some(0),
            per_page: None,
        };
        assert_eq!(p.page(), 1);
    }

    #[test]
    fn oversized_per_page_is_clamped() {
        let p = Pagination {
            page: None,
            per_page: Some(100_000),
        };
        assert_eq!(p.per_page(25), 100);
    }

    #[test]
    fn zero_per_page_is_floored_to_one() {
        let p = Pagination {
            page: None,
            per_page: Some(0),
        };
        assert_eq!(p.per_page(25), 1);
    }
}
