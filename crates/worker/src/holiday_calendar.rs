//! Weekend/holiday-aware job scheduling.
//!
//! Skip certain worker jobs during weekends and public holidays
//! for Tunisia, Morocco, and Israel — configurable per region.

use chrono::{Datelike, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Configuration ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolidayCalendarConfig {
    /// Whether to skip jobs on weekends
    pub skip_weekends: bool,
    /// Weekend days per region (e.g., TN/MA: Fri+Sat, IL: Fri+Sat, EU/US: Sat+Sun)
    pub weekend_days: HashMap<String, Vec<Weekday>>,
}

impl Default for HolidayCalendarConfig {
    fn default() -> Self {
        let mut weekends = HashMap::new();
        // Tunisia & Morocco: Saturday + Sunday (recently shifted from Fri+Sat)
        weekends.insert("TN".into(), vec![Weekday::Sat, Weekday::Sun]);
        weekends.insert("MA".into(), vec![Weekday::Sat, Weekday::Sun]);
        // Israel: Friday + Saturday (Shabbat)
        weekends.insert("IL".into(), vec![Weekday::Fri, Weekday::Sat]);
        // EU/US standard
        weekends.insert("EU".into(), vec![Weekday::Sat, Weekday::Sun]);
        weekends.insert("US".into(), vec![Weekday::Sat, Weekday::Sun]);
        weekends.insert("CN".into(), vec![Weekday::Sat, Weekday::Sun]);

        Self {
            skip_weekends: true,
            weekend_days: weekends,
        }
    }
}

// ─── Holiday database ───────────────────────────────────────────────────

/// Static holiday database for 2026.
/// In production, this would be loaded from a YAML config or API.
pub fn holidays_2026() -> HashMap<String, Vec<(NaiveDate, String)>> {
    fn holiday_date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap_or_else(|| panic!("invalid holiday date: {year:04}-{month:02}-{day:02}"))
    }

    let mut holidays = HashMap::new();

    // Tunisia public holidays 2026
    holidays.insert(
        "TN".into(),
        vec![
            (holiday_date(2026, 1, 1), "New Year's Day".into()),
            (holiday_date(2026, 1, 14), "Revolution Day".into()),
            (holiday_date(2026, 3, 20), "Independence Day".into()),
            (holiday_date(2026, 4, 9), "Martyrs' Day".into()),
            (holiday_date(2026, 5, 1), "Labour Day".into()),
            (holiday_date(2026, 7, 25), "Republic Day".into()),
            (holiday_date(2026, 8, 13), "Women's Day".into()),
            (holiday_date(2026, 10, 15), "Evacuation Day".into()),
            // Estimated Eid al-Fitr 2026 (dates shift annually)
            (holiday_date(2026, 3, 30), "Eid al-Fitr".into()),
            (holiday_date(2026, 3, 31), "Eid al-Fitr".into()),
            // Estimated Eid al-Adha 2026
            (holiday_date(2026, 6, 7), "Eid al-Adha".into()),
            (holiday_date(2026, 6, 8), "Eid al-Adha".into()),
        ],
    );

    // Morocco public holidays 2026
    holidays.insert(
        "MA".into(),
        vec![
            (holiday_date(2026, 1, 1), "New Year's Day".into()),
            (holiday_date(2026, 1, 11), "Independence Manifesto".into()),
            (holiday_date(2026, 5, 1), "Labour Day".into()),
            (holiday_date(2026, 7, 30), "Throne Day".into()),
            (holiday_date(2026, 8, 14), "Oued Ed-Dahab".into()),
            (holiday_date(2026, 8, 20), "Revolution Day".into()),
            (holiday_date(2026, 8, 21), "Youth Day".into()),
            (holiday_date(2026, 11, 6), "Green March".into()),
            (holiday_date(2026, 11, 18), "Independence Day".into()),
            // Estimated Islamic holidays
            (holiday_date(2026, 3, 30), "Eid al-Fitr".into()),
            (holiday_date(2026, 6, 7), "Eid al-Adha".into()),
        ],
    );

    // Israel public holidays 2026
    holidays.insert(
        "IL".into(),
        vec![
            // Jewish holidays (approximate Gregorian dates for 2026)
            (holiday_date(2026, 4, 2), "Passover (start)".into()),
            (holiday_date(2026, 4, 8), "Passover (end)".into()),
            (holiday_date(2026, 4, 15), "Yom HaAtzmaut".into()),
            (holiday_date(2026, 5, 22), "Shavuot".into()),
            (holiday_date(2026, 9, 12), "Rosh Hashana".into()),
            (holiday_date(2026, 9, 13), "Rosh Hashana".into()),
            (holiday_date(2026, 9, 21), "Yom Kippur".into()),
            (holiday_date(2026, 9, 26), "Sukkot".into()),
            (holiday_date(2026, 10, 3), "Simchat Torah".into()),
        ],
    );

    holidays
}

// ─── Schedule checker ───────────────────────────────────────────────────

pub struct HolidayScheduler {
    config: HolidayCalendarConfig,
    holidays: HashMap<String, Vec<(NaiveDate, String)>>,
}

impl HolidayScheduler {
    pub fn new(config: HolidayCalendarConfig) -> Self {
        Self {
            config,
            holidays: holidays_2026(),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(HolidayCalendarConfig::default())
    }

    /// Check if a given date is a non-working day for a region.
    pub fn is_non_working_day(&self, date: NaiveDate, region: &str) -> bool {
        // Check weekend
        if self.config.skip_weekends {
            if let Some(weekend_days) = self.config.weekend_days.get(region) {
                if weekend_days.contains(&date.weekday()) {
                    return true;
                }
            }
        }

        // Check holidays
        if let Some(region_holidays) = self.holidays.get(region) {
            if region_holidays.iter().any(|(d, _)| *d == date) {
                return true;
            }
        }

        false
    }

    /// Check if a given date is a holiday (not just weekend).
    pub fn is_holiday(&self, date: NaiveDate, region: &str) -> Option<String> {
        self.holidays
            .get(region)?
            .iter()
            .find(|(d, _)| *d == date)
            .map(|(_, name)| name.clone())
    }

    /// Get the next working day from a given date for a region.
    pub fn next_working_day(&self, date: NaiveDate, region: &str) -> NaiveDate {
        let mut candidate = date;
        let max_skip = 14; // Safety: never skip more than 14 days
        for _ in 0..max_skip {
            if !self.is_non_working_day(candidate, region) {
                return candidate;
            }
            candidate = candidate.succ_opt().unwrap_or(candidate);
        }
        candidate
    }

    /// Check if a job should run on this date for its configured regions.
    pub fn should_run(&self, date: NaiveDate, job_regions: &[&str]) -> bool {
        // Job should run if ANY of its regions is a working day
        job_regions
            .iter()
            .any(|r| !self.is_non_working_day(date, r))
    }

    /// Get upcoming holidays for a region within the next N days.
    pub fn upcoming_holidays(
        &self,
        region: &str,
        from: NaiveDate,
        days: i64,
    ) -> Vec<(NaiveDate, String)> {
        let until = from + chrono::Duration::days(days);
        self.holidays
            .get(region)
            .map(|holidays| {
                holidays
                    .iter()
                    .filter(|(d, _)| *d >= from && *d <= until)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn test_tunisia_weekend() {
        let scheduler = HolidayScheduler::with_defaults();
        // Saturday
        let sat = NaiveDate::from_ymd_opt(2026, 3, 7).unwrap();
        assert!(scheduler.is_non_working_day(sat, "TN"));
        // Monday
        let mon = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        assert!(!scheduler.is_non_working_day(mon, "TN"));
    }

    #[test]
    fn test_israel_weekend() {
        let scheduler = HolidayScheduler::with_defaults();
        // Friday in Israel
        let fri = NaiveDate::from_ymd_opt(2026, 3, 6).unwrap();
        assert!(scheduler.is_non_working_day(fri, "IL"));
        // Sunday is a regular working day in Israel
        let sun = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert!(!scheduler.is_non_working_day(sun, "IL"));
    }

    #[test]
    fn test_tunisia_holiday() {
        let scheduler = HolidayScheduler::with_defaults();
        let independence = NaiveDate::from_ymd_opt(2026, 3, 20).unwrap();
        assert!(scheduler.is_non_working_day(independence, "TN"));
        assert_eq!(
            scheduler.is_holiday(independence, "TN"),
            Some("Independence Day".into())
        );
    }

    #[test]
    fn test_israel_holiday() {
        let scheduler = HolidayScheduler::with_defaults();
        let yom_kippur = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        assert!(scheduler.is_non_working_day(yom_kippur, "IL"));
    }

    #[test]
    fn test_next_working_day() {
        let scheduler = HolidayScheduler::with_defaults();
        // If today is Saturday, next working day is Monday
        let sat = NaiveDate::from_ymd_opt(2026, 3, 7).unwrap();
        let next = scheduler.next_working_day(sat, "TN");
        assert_eq!(next.weekday(), Weekday::Mon);
    }

    #[test]
    fn test_should_run_multi_region() {
        let scheduler = HolidayScheduler::with_defaults();
        // Friday: off in Israel, working in Tunisia
        let fri = NaiveDate::from_ymd_opt(2026, 3, 6).unwrap();
        assert!(scheduler.should_run(fri, &["TN", "IL"])); // TN is working
    }

    #[test]
    fn test_upcoming_holidays() {
        let scheduler = HolidayScheduler::with_defaults();
        let from = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let upcoming = scheduler.upcoming_holidays("TN", from, 30);
        assert!(!upcoming.is_empty());
        assert!(upcoming.iter().any(|(_, name)| name == "Independence Day"));
    }
}
