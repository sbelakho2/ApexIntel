//! Generic job scheduler — cron-like scheduling, job registry, execution tracking.
//!
//! Purely functional scheduling logic: no tokio spawns here.
//! The caller (main loop) drives ticks; this module decides *what* to run and *when*.

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ────────────────────────────────────────────
// Schedule spec
// ────────────────────────────────────────────

/// When a job should fire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Schedule {
    /// Run every N seconds.
    IntervalSecs(u64),
    /// Run once per day at the given hour (0-23) and minute (0-59), UTC.
    DailyAt { hour: u32, minute: u32 },
    /// Run once per week on the given weekday at the given time, UTC.
    WeeklyOn {
        day: IsoWeekday,
        hour: u32,
        minute: u32,
    },
}

/// ISO weekday (serialisable, unlike chrono::Weekday).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IsoWeekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl IsoWeekday {
    pub fn to_chrono(self) -> Weekday {
        match self {
            Self::Mon => Weekday::Mon,
            Self::Tue => Weekday::Tue,
            Self::Wed => Weekday::Wed,
            Self::Thu => Weekday::Thu,
            Self::Fri => Weekday::Fri,
            Self::Sat => Weekday::Sat,
            Self::Sun => Weekday::Sun,
        }
    }

    pub fn from_chrono(w: Weekday) -> Self {
        match w {
            Weekday::Mon => Self::Mon,
            Weekday::Tue => Self::Tue,
            Weekday::Wed => Self::Wed,
            Weekday::Thu => Self::Thu,
            Weekday::Fri => Self::Fri,
            Weekday::Sat => Self::Sat,
            Weekday::Sun => Self::Sun,
        }
    }
}

// ────────────────────────────────────────────
// Job identity & status
// ────────────────────────────────────────────

/// The kind of pipeline this job belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum JobKind {
    CrawlCycle,
    PatternMining,
    PoiRefresh,
    PromotionBoard,
    RecipeDeprecation,
    StrategyMemo,
    FeatureDriftCheck,
    Custom(String),
}

impl JobKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::CrawlCycle => "crawl_cycle",
            Self::PatternMining => "pattern_mining",
            Self::PoiRefresh => "poi_refresh",
            Self::PromotionBoard => "promotion_board",
            Self::RecipeDeprecation => "recipe_deprecation",
            Self::StrategyMemo => "strategy_memo",
            Self::FeatureDriftCheck => "feature_drift_check",
            Self::Custom(s) => s.as_str(),
        }
    }
}

/// Execution status of a single job run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded { duration_ms: u64 },
    Failed { error: String, duration_ms: u64 },
    Skipped { reason: String },
}

impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded { .. } | Self::Failed { .. } | Self::Skipped { .. })
    }
}

/// A single run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub run_id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub items_processed: u64,
    pub notes: String,
}

impl JobRun {
    pub fn new(kind: JobKind) -> Self {
        Self {
            run_id: Uuid::new_v4().to_string(),
            kind,
            status: JobStatus::Pending,
            started_at: Utc::now(),
            finished_at: None,
            items_processed: 0,
            notes: String::new(),
        }
    }

    pub fn start(&mut self) {
        self.status = JobStatus::Running;
        self.started_at = Utc::now();
    }

    pub fn succeed(&mut self, items: u64, notes: &str) {
        let elapsed = Utc::now()
            .signed_duration_since(self.started_at)
            .num_milliseconds()
            .max(0) as u64;
        self.status = JobStatus::Succeeded {
            duration_ms: elapsed,
        };
        self.finished_at = Some(Utc::now());
        self.items_processed = items;
        self.notes = notes.to_string();
    }

    pub fn fail(&mut self, error: &str) {
        let elapsed = Utc::now()
            .signed_duration_since(self.started_at)
            .num_milliseconds()
            .max(0) as u64;
        self.status = JobStatus::Failed {
            error: error.to_string(),
            duration_ms: elapsed,
        };
        self.finished_at = Some(Utc::now());
    }

    pub fn skip(&mut self, reason: &str) {
        self.status = JobStatus::Skipped {
            reason: reason.to_string(),
        };
        self.finished_at = Some(Utc::now());
    }

    pub fn duration_ms(&self) -> u64 {
        match &self.status {
            JobStatus::Succeeded { duration_ms } => *duration_ms,
            JobStatus::Failed { duration_ms, .. } => *duration_ms,
            _ => Utc::now()
                .signed_duration_since(self.started_at)
                .num_milliseconds()
                .max(0) as u64,
        }
    }
}

// ────────────────────────────────────────────
// Registered job definition
// ────────────────────────────────────────────

/// A registered job: its schedule + bookkeeping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDef {
    pub kind: JobKind,
    pub schedule: Schedule,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub last_status: Option<JobStatus>,
    pub consecutive_failures: u32,
    pub max_consecutive_failures: u32,
}

impl JobDef {
    pub fn new(kind: JobKind, schedule: Schedule) -> Self {
        Self {
            kind,
            schedule,
            enabled: true,
            last_run: None,
            last_status: None,
            consecutive_failures: 0,
            max_consecutive_failures: 5,
        }
    }

    /// Record a completed run.
    pub fn record_run(&mut self, run: &JobRun) {
        self.last_run = run.finished_at.or(Some(Utc::now()));
        self.last_status = Some(run.status.clone());
        match &run.status {
            JobStatus::Failed { .. } => {
                self.consecutive_failures += 1;
                if self.consecutive_failures >= self.max_consecutive_failures {
                    self.enabled = false;
                    tracing::warn!(
                        "{} disabled after {} consecutive failures",
                        self.kind.as_str(),
                        self.consecutive_failures
                    );
                }
            }
            JobStatus::Succeeded { .. } => {
                self.consecutive_failures = 0;
            }
            _ => {}
        }
    }

    /// Has this job been auto-disabled due to repeated failures?
    pub fn is_circuit_broken(&self) -> bool {
        !self.enabled && self.consecutive_failures >= self.max_consecutive_failures
    }

    /// Reset the circuit breaker (operator override).
    pub fn reset_circuit_breaker(&mut self) {
        self.consecutive_failures = 0;
        self.enabled = true;
    }
}

// ────────────────────────────────────────────
// Scheduler (the brain)
// ────────────────────────────────────────────

/// Pure-function scheduler: given the current time, decides which jobs are due.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scheduler {
    pub jobs: HashMap<String, JobDef>,
    pub history: Vec<JobRun>,
    pub max_history: usize,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            jobs: HashMap::new(),
            history: Vec::new(),
            max_history: 1000,
        }
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a job. Returns false if one with the same key already exists.
    pub fn register(&mut self, def: JobDef) -> bool {
        let key = def.kind.as_str().to_string();
        if self.jobs.contains_key(&key) {
            return false;
        }
        self.jobs.insert(key, def);
        true
    }

    /// Unregister a job by kind.
    pub fn unregister(&mut self, kind: &JobKind) -> bool {
        self.jobs.remove(kind.as_str()).is_some()
    }

    /// Return the list of job kinds that are due to run at `now`.
    pub fn due_jobs(&self, now: DateTime<Utc>) -> Vec<JobKind> {
        self.jobs
            .values()
            .filter(|def| def.enabled && is_due(&def.schedule, def.last_run, now))
            .map(|def| def.kind.clone())
            .collect()
    }

    /// Record a completed run in history and update the job definition.
    pub fn record_run(&mut self, run: JobRun) {
        let key = run.kind.as_str().to_string();
        if let Some(def) = self.jobs.get_mut(&key) {
            def.record_run(&run);
        }
        self.history.push(run);
        // Trim history
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(0..excess);
        }
    }

    /// Get all runs for a given job kind.
    pub fn runs_for(&self, kind: &JobKind) -> Vec<&JobRun> {
        self.history
            .iter()
            .filter(|r| &r.kind == kind)
            .collect()
    }

    /// Success rate for a given job kind over the last N runs.
    pub fn success_rate(&self, kind: &JobKind, last_n: usize) -> f64 {
        let runs: Vec<&JobRun> = self
            .history
            .iter()
            .rev()
            .filter(|r| &r.kind == kind)
            .take(last_n)
            .collect();
        if runs.is_empty() {
            return 0.0;
        }
        let ok = runs
            .iter()
            .filter(|r| matches!(r.status, JobStatus::Succeeded { .. }))
            .count();
        ok as f64 / runs.len() as f64
    }

    /// Average duration (ms) for a given job kind over the last N runs.
    pub fn avg_duration_ms(&self, kind: &JobKind, last_n: usize) -> f64 {
        let durations: Vec<u64> = self
            .history
            .iter()
            .rev()
            .filter(|r| &r.kind == kind && r.status.is_terminal())
            .take(last_n)
            .map(|r| r.duration_ms())
            .collect();
        if durations.is_empty() {
            return 0.0;
        }
        durations.iter().sum::<u64>() as f64 / durations.len() as f64
    }

    /// Summary of all registered jobs.
    pub fn status_summary(&self) -> Vec<JobSummary> {
        self.jobs
            .values()
            .map(|def| JobSummary {
                kind: def.kind.clone(),
                enabled: def.enabled,
                last_run: def.last_run,
                last_status: def.last_status.clone(),
                consecutive_failures: def.consecutive_failures,
                success_rate_last10: self.success_rate(&def.kind, 10),
                avg_duration_ms_last10: self.avg_duration_ms(&def.kind, 10),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSummary {
    pub kind: JobKind,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub last_status: Option<JobStatus>,
    pub consecutive_failures: u32,
    pub success_rate_last10: f64,
    pub avg_duration_ms_last10: f64,
}

// ────────────────────────────────────────────
// Schedule evaluation (pure)
// ────────────────────────────────────────────

/// Is a job due to run at `now`, given its schedule and when it last ran?
pub fn is_due(schedule: &Schedule, last_run: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match schedule {
        Schedule::IntervalSecs(secs) => {
            let Some(last) = last_run else {
                return true; // never ran → due immediately
            };
            let elapsed = now.signed_duration_since(last).num_seconds();
            elapsed >= *secs as i64
        }
        Schedule::DailyAt { hour, minute } => {
            let target = NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default();
            let now_time = now.time();
            let now_date = now.date_naive();

            match last_run {
                None => {
                    // Never ran — due if we're at or past the target time today
                    now_time >= target
                }
                Some(last) => {
                    let last_date = last.date_naive();
                    // Due if: it's a new day (or same day but we haven't hit the window yet)
                    // and current time >= target
                    if now_date > last_date && now_time >= target {
                        return true;
                    }
                    // Also due if last ran on a previous day and we're past today's target
                    if now_date > last_date && now_time >= target {
                        return true;
                    }
                    false
                }
            }
        }
        Schedule::WeeklyOn { day, hour, minute } => {
            let target_day = day.to_chrono();
            let target_time = NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default();
            let now_weekday = now.weekday();
            let now_time = now.time();

            if now_weekday != target_day {
                return false;
            }
            if now_time < target_time {
                return false;
            }
            match last_run {
                None => true,
                Some(last) => {
                    // Due if last run was before this week's scheduled time
                    let last_date = last.date_naive();
                    let now_date = now.date_naive();
                    if now_date > last_date {
                        return true;
                    }
                    if now_date == last_date && last.time() < target_time {
                        return true;
                    }
                    false
                }
            }
        }
    }
}

/// Compute the next fire time for a schedule relative to `now`.
pub fn next_fire_time(schedule: &Schedule, now: DateTime<Utc>) -> DateTime<Utc> {
    match schedule {
        Schedule::IntervalSecs(secs) => now + chrono::Duration::seconds(*secs as i64),
        Schedule::DailyAt { hour, minute } => {
            let today_target = now
                .date_naive()
                .and_time(NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default());
            let today_utc = today_target.and_utc();
            if now < today_utc {
                today_utc
            } else {
                // tomorrow
                let tomorrow = now.date_naive().succ_opt().unwrap_or(now.date_naive());
                tomorrow
                    .and_time(NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default())
                    .and_utc()
            }
        }
        Schedule::WeeklyOn { day, hour, minute } => {
            let target_day = day.to_chrono();
            let target_time = NaiveTime::from_hms_opt(*hour, *minute, 0).unwrap_or_default();
            let now_weekday = now.weekday();
            let now_num = now_weekday.num_days_from_monday();
            let target_num = target_day.num_days_from_monday();

            let days_ahead = if target_num > now_num {
                target_num - now_num
            } else if target_num < now_num {
                7 - (now_num - target_num)
            } else {
                // same day
                let today_target = now.date_naive().and_time(target_time).and_utc();
                if now < today_target {
                    return today_target;
                }
                7 // next week
            };
            let target_date = now.date_naive() + chrono::Duration::days(days_ahead as i64);
            target_date.and_time(target_time).and_utc()
        }
    }
}

// ────────────────────────────────────────────
// Default schedule presets
// ────────────────────────────────────────────

/// Build the default nightly + weekly scheduler.
pub fn default_scheduler() -> Scheduler {
    let mut s = Scheduler::new();

    // Hourly crawl
    s.register(JobDef::new(
        JobKind::CrawlCycle,
        Schedule::IntervalSecs(3600),
    ));

    // Nightly at 02:00 UTC
    s.register(JobDef::new(
        JobKind::PatternMining,
        Schedule::DailyAt {
            hour: 2,
            minute: 0,
        },
    ));

    s.register(JobDef::new(
        JobKind::PoiRefresh,
        Schedule::DailyAt {
            hour: 3,
            minute: 0,
        },
    ));

    s.register(JobDef::new(
        JobKind::FeatureDriftCheck,
        Schedule::DailyAt {
            hour: 4,
            minute: 0,
        },
    ));

    // Weekly on Monday at 06:00 UTC
    s.register(JobDef::new(
        JobKind::PromotionBoard,
        Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        },
    ));

    s.register(JobDef::new(
        JobKind::StrategyMemo,
        Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 7,
            minute: 0,
        },
    ));

    s.register(JobDef::new(
        JobKind::RecipeDeprecation,
        Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 8,
            minute: 0,
        },
    ));

    s
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, s).unwrap()
    }

    // ── Schedule evaluation ──

    #[test]
    fn test_interval_never_ran() {
        let sched = Schedule::IntervalSecs(3600);
        assert!(is_due(&sched, None, Utc::now()));
    }

    #[test]
    fn test_interval_not_yet_due() {
        let sched = Schedule::IntervalSecs(3600);
        let now = utc(2026, 2, 23, 10, 0, 0);
        let last = utc(2026, 2, 23, 9, 30, 0); // only 30 min ago
        assert!(!is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_interval_due() {
        let sched = Schedule::IntervalSecs(3600);
        let now = utc(2026, 2, 23, 11, 0, 0);
        let last = utc(2026, 2, 23, 9, 30, 0); // 90 min ago
        assert!(is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_daily_never_ran_before_time() {
        let sched = Schedule::DailyAt {
            hour: 14,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // before 14:00
        assert!(!is_due(&sched, None, now));
    }

    #[test]
    fn test_daily_never_ran_after_time() {
        let sched = Schedule::DailyAt {
            hour: 14,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 15, 0, 0); // after 14:00
        assert!(is_due(&sched, None, now));
    }

    #[test]
    fn test_daily_ran_today_already() {
        let sched = Schedule::DailyAt {
            hour: 2,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 15, 0, 0);
        let last = utc(2026, 2, 23, 2, 5, 0); // ran today at 02:05
        assert!(!is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_daily_new_day() {
        let sched = Schedule::DailyAt {
            hour: 2,
            minute: 0,
        };
        let now = utc(2026, 2, 24, 2, 30, 0); // next day, past 02:00
        let last = utc(2026, 2, 23, 2, 5, 0);
        assert!(is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_weekly_wrong_day() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        // 2026-02-23 is Monday, but let's pick a Tuesday
        let now = utc(2026, 2, 24, 7, 0, 0); // Tuesday
        assert!(!is_due(&sched, None, now));
    }

    #[test]
    fn test_weekly_right_day_before_time() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 5, 0, 0); // Monday 05:00
        assert!(!is_due(&sched, None, now));
    }

    #[test]
    fn test_weekly_right_day_after_time_never_ran() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 7, 0, 0); // Monday 07:00
        assert!(is_due(&sched, None, now));
    }

    #[test]
    fn test_weekly_already_ran_this_week() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday 10:00
        let last = utc(2026, 2, 23, 6, 5, 0); // ran today at 06:05
        assert!(!is_due(&sched, Some(last), now));
    }

    #[test]
    fn test_weekly_new_week() {
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 3, 2, 7, 0, 0); // next Monday
        let last = utc(2026, 2, 23, 6, 5, 0); // last Monday
        assert!(is_due(&sched, Some(last), now));
    }

    // ── next_fire_time ──

    #[test]
    fn test_next_fire_interval() {
        let sched = Schedule::IntervalSecs(7200);
        let now = utc(2026, 2, 23, 10, 0, 0);
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 23, 12, 0, 0));
    }

    #[test]
    fn test_next_fire_daily_before_target() {
        let sched = Schedule::DailyAt {
            hour: 14,
            minute: 30,
        };
        let now = utc(2026, 2, 23, 10, 0, 0);
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 23, 14, 30, 0));
    }

    #[test]
    fn test_next_fire_daily_after_target() {
        let sched = Schedule::DailyAt {
            hour: 2,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0);
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 24, 2, 0, 0)); // tomorrow
    }

    #[test]
    fn test_next_fire_weekly_same_day_before() {
        // Monday at 14:00, currently Monday 10:00
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 14,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 23, 14, 0, 0)); // today
    }

    #[test]
    fn test_next_fire_weekly_same_day_after() {
        // Monday at 06:00, currently Monday 10:00
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Mon,
            hour: 6,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday 10:00
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 3, 2, 6, 0, 0)); // next Monday
    }

    #[test]
    fn test_next_fire_weekly_different_day() {
        // Friday at 08:00, currently Monday
        let sched = Schedule::WeeklyOn {
            day: IsoWeekday::Fri,
            hour: 8,
            minute: 0,
        };
        let now = utc(2026, 2, 23, 10, 0, 0); // Monday
        let next = next_fire_time(&sched, now);
        assert_eq!(next, utc(2026, 2, 27, 8, 0, 0)); // Friday
    }

    // ── IsoWeekday conversions ──

    #[test]
    fn test_iso_weekday_roundtrip() {
        for w in &[
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ] {
            let iso = IsoWeekday::from_chrono(*w);
            assert_eq!(iso.to_chrono(), *w);
        }
    }

    // ── JobRun ──

    #[test]
    fn test_job_run_lifecycle() {
        let mut run = JobRun::new(JobKind::CrawlCycle);
        assert!(matches!(run.status, JobStatus::Pending));
        run.start();
        assert!(matches!(run.status, JobStatus::Running));
        run.succeed(42, "crawled 42 sources");
        assert!(matches!(run.status, JobStatus::Succeeded { .. }));
        assert_eq!(run.items_processed, 42);
        assert!(run.finished_at.is_some());
    }

    #[test]
    fn test_job_run_fail() {
        let mut run = JobRun::new(JobKind::PatternMining);
        run.start();
        run.fail("database timeout");
        match &run.status {
            JobStatus::Failed { error, .. } => assert_eq!(error, "database timeout"),
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn test_job_run_skip() {
        let mut run = JobRun::new(JobKind::PoiRefresh);
        run.skip("no new data");
        match &run.status {
            JobStatus::Skipped { reason } => assert_eq!(reason, "no new data"),
            _ => panic!("expected Skipped"),
        }
    }

    #[test]
    fn test_job_status_is_terminal() {
        assert!(!JobStatus::Pending.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
        assert!(JobStatus::Succeeded { duration_ms: 100 }.is_terminal());
        assert!(JobStatus::Failed {
            error: "e".to_string(),
            duration_ms: 50
        }
        .is_terminal());
        assert!(JobStatus::Skipped {
            reason: "r".to_string()
        }
        .is_terminal());
    }

    // ── JobDef circuit breaker ──

    #[test]
    fn test_circuit_breaker_triggers() {
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.max_consecutive_failures = 3;

        for i in 0..3 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.fail(&format!("error {}", i));
            def.record_run(&run);
        }

        assert!(def.is_circuit_broken());
        assert!(!def.enabled);
    }

    #[test]
    fn test_circuit_breaker_resets_on_success() {
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.max_consecutive_failures = 5;

        // 3 failures
        for _ in 0..3 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.fail("err");
            def.record_run(&run);
        }
        assert_eq!(def.consecutive_failures, 3);

        // 1 success resets
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.succeed(10, "ok");
        def.record_run(&run);
        assert_eq!(def.consecutive_failures, 0);
    }

    #[test]
    fn test_circuit_breaker_manual_reset() {
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.max_consecutive_failures = 2;
        for _ in 0..2 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.fail("err");
            def.record_run(&run);
        }
        assert!(def.is_circuit_broken());
        def.reset_circuit_breaker();
        assert!(!def.is_circuit_broken());
        assert!(def.enabled);
    }

    // ── Scheduler ──

    #[test]
    fn test_register_and_unregister() {
        let mut sched = Scheduler::new();
        let def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        assert!(sched.register(def.clone()));
        assert!(!sched.register(def)); // duplicate
        assert_eq!(sched.jobs.len(), 1);
        assert!(sched.unregister(&JobKind::CrawlCycle));
        assert!(sched.jobs.is_empty());
    }

    #[test]
    fn test_due_jobs_interval() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(3600));
        def.last_run = Some(utc(2026, 2, 23, 8, 0, 0));
        sched.register(def);

        // 30 min later — not due
        let due = sched.due_jobs(utc(2026, 2, 23, 8, 30, 0));
        assert!(due.is_empty());

        // 2 hours later — due
        let due = sched.due_jobs(utc(2026, 2, 23, 10, 0, 0));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0], JobKind::CrawlCycle);
    }

    #[test]
    fn test_due_jobs_respects_disabled() {
        let mut sched = Scheduler::new();
        let mut def = JobDef::new(JobKind::CrawlCycle, Schedule::IntervalSecs(60));
        def.enabled = false;
        sched.register(def);

        let due = sched.due_jobs(Utc::now());
        assert!(due.is_empty());
    }

    #[test]
    fn test_record_run_and_history() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(
            JobKind::CrawlCycle,
            Schedule::IntervalSecs(3600),
        ));

        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.succeed(50, "ok");
        sched.record_run(run);

        assert_eq!(sched.history.len(), 1);
        let def = sched.jobs.get("crawl_cycle").unwrap();
        assert!(def.last_run.is_some());
        assert!(matches!(
            def.last_status,
            Some(JobStatus::Succeeded { .. })
        ));
    }

    #[test]
    fn test_history_trimming() {
        let mut sched = Scheduler::new();
        sched.max_history = 5;
        sched.register(JobDef::new(
            JobKind::CrawlCycle,
            Schedule::IntervalSecs(60),
        ));

        for _ in 0..10 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            run.succeed(1, "ok");
            sched.record_run(run);
        }
        assert_eq!(sched.history.len(), 5);
    }

    #[test]
    fn test_success_rate() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(
            JobKind::CrawlCycle,
            Schedule::IntervalSecs(60),
        ));

        // 3 successes, 2 failures
        for i in 0..5 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.start();
            if i < 3 {
                run.succeed(1, "ok");
            } else {
                run.fail("err");
            }
            sched.record_run(run);
        }

        let rate = sched.success_rate(&JobKind::CrawlCycle, 10);
        assert!((rate - 0.6).abs() < 0.01);
    }

    #[test]
    fn test_success_rate_empty() {
        let sched = Scheduler::new();
        assert_eq!(sched.success_rate(&JobKind::CrawlCycle, 10), 0.0);
    }

    #[test]
    fn test_status_summary() {
        let mut sched = default_scheduler();

        // Record one crawl run
        let mut run = JobRun::new(JobKind::CrawlCycle);
        run.start();
        run.succeed(100, "all good");
        sched.record_run(run);

        let summary = sched.status_summary();
        assert!(summary.len() >= 7); // 7 default jobs
        let crawl_summary = summary.iter().find(|s| s.kind == JobKind::CrawlCycle).unwrap();
        assert!(crawl_summary.enabled);
        assert!(crawl_summary.last_run.is_some());
    }

    #[test]
    fn test_default_scheduler_job_count() {
        let s = default_scheduler();
        assert_eq!(s.jobs.len(), 7);
        assert!(s.jobs.contains_key("crawl_cycle"));
        assert!(s.jobs.contains_key("pattern_mining"));
        assert!(s.jobs.contains_key("poi_refresh"));
        assert!(s.jobs.contains_key("promotion_board"));
        assert!(s.jobs.contains_key("strategy_memo"));
        assert!(s.jobs.contains_key("recipe_deprecation"));
        assert!(s.jobs.contains_key("feature_drift_check"));
    }

    #[test]
    fn test_runs_for_filters_by_kind() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(
            JobKind::CrawlCycle,
            Schedule::IntervalSecs(60),
        ));
        sched.register(JobDef::new(
            JobKind::PoiRefresh,
            Schedule::DailyAt { hour: 3, minute: 0 },
        ));

        let mut r1 = JobRun::new(JobKind::CrawlCycle);
        r1.start();
        r1.succeed(10, "");
        sched.record_run(r1);

        let mut r2 = JobRun::new(JobKind::PoiRefresh);
        r2.start();
        r2.succeed(5, "");
        sched.record_run(r2);

        let mut r3 = JobRun::new(JobKind::CrawlCycle);
        r3.start();
        r3.succeed(20, "");
        sched.record_run(r3);

        assert_eq!(sched.runs_for(&JobKind::CrawlCycle).len(), 2);
        assert_eq!(sched.runs_for(&JobKind::PoiRefresh).len(), 1);
    }

    #[test]
    fn test_job_kind_custom() {
        let kind = JobKind::Custom("my_custom_job".to_string());
        assert_eq!(kind.as_str(), "my_custom_job");
    }

    #[test]
    fn test_job_kind_serialization() {
        let kind = JobKind::CrawlCycle;
        let json = serde_json::to_string(&kind).unwrap();
        let back: JobKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn test_schedule_serialization() {
        let s = Schedule::WeeklyOn {
            day: IsoWeekday::Fri,
            hour: 8,
            minute: 30,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Schedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn test_avg_duration_ms() {
        let mut sched = Scheduler::new();
        sched.register(JobDef::new(
            JobKind::CrawlCycle,
            Schedule::IntervalSecs(60),
        ));

        // Record runs with known durations manually
        for i in 0u32..3 {
            let mut run = JobRun::new(JobKind::CrawlCycle);
            run.started_at = utc(2026, 2, 23, 10, 0, 0);
            run.status = JobStatus::Succeeded {
                duration_ms: (i as u64 + 1) * 100,
            };
            run.finished_at = Some(utc(2026, 2, 23, 10, 0, i + 1));
            sched.record_run(run);
        }

        let avg = sched.avg_duration_ms(&JobKind::CrawlCycle, 10);
        assert!((avg - 200.0).abs() < 0.01); // (100+200+300)/3
    }

    #[test]
    fn test_avg_duration_empty() {
        let sched = Scheduler::new();
        assert_eq!(sched.avg_duration_ms(&JobKind::CrawlCycle, 10), 0.0);
    }
}
