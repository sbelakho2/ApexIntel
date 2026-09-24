//! Deterministic render-readiness policy.
//!
//! A browser render is only "ready" when **all** of the following hold:
//!
//! 1. the DOM has finished loading (`document.readyState == "complete"`),
//! 2. the network has been quiet for [`RenderPolicy::quiet_window`]
//!    (the CDP implementation counts in-flight `Network.*` events, so this is
//!    the browser equivalent of Playwright's `networkidle`),
//! 3. the rendered text/content size is non-zero and identical across two
//!    consecutive samples,
//! 4. a bounded number of progressive scroll steps has been performed for
//!    lazy-loaded content, and
//! 5. the total render time stays under [`RenderPolicy::max_render_time`].
//!
//! The decision engine operates on [`PageSample`] observations supplied with
//! an explicit timeline (`at`), so it is fully deterministic and unit-tested
//! with injected samples — no sleeps or real browser required.

use std::time::Duration;

/// Default quiet window: no in-flight requests for one second.
pub const DEFAULT_NETWORK_QUIET_WINDOW: Duration = Duration::from_millis(1000);
/// Lower bound for the network-quiet window accepted from configuration.
pub const MIN_NETWORK_QUIET_WINDOW: Duration = Duration::from_millis(750);
/// Upper bound for the network-quiet window accepted from configuration.
pub const MAX_NETWORK_QUIET_WINDOW: Duration = Duration::from_millis(1500);
/// Default interval between readiness samples.
pub const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);
/// Default cap on total render time (readiness wait + DOM extraction).
pub const DEFAULT_MAX_RENDER_TIME: Duration = Duration::from_secs(30);
/// Default number of progressive scroll steps used to trigger lazy loading.
pub const DEFAULT_MAX_SCROLL_STEPS: u32 = 3;

/// One observation of the page while it renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageSample {
    /// `document.readyState == "complete"`.
    pub dom_loaded: bool,
    /// Number of network requests currently in flight.
    pub network_in_flight: usize,
    /// Length of the rendered document text (non-zero once content arrived).
    pub content_size: usize,
}

/// Why the renderer is not ready to extract yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    /// `document.readyState` has not reached `"complete"`.
    DomNotLoaded,
    /// Requests are currently in flight.
    NetworkBusy,
    /// No in-flight requests, but the quiet window has not elapsed yet.
    NetworkSettling,
    /// The document text size has not been stable across two samples.
    ContentUnstable,
    /// The document has no text content yet.
    NoContent,
}

/// What the renderer should do after an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessDecision {
    /// Keep sampling; the page is not settled yet.
    Waiting(WaitReason),
    /// The page settled once, but a bounded progressive scroll is still due to
    /// trigger lazy-loaded content. The caller scrolls by one viewport and
    /// keeps sampling.
    Scroll { step: u32 },
    /// All readiness conditions held: extract the final DOM now.
    Ready,
    /// [`RenderPolicy::max_render_time`] elapsed before readiness. The caller
    /// extracts the best-effort DOM (if the DOM loaded at all) and flags the
    /// page as timed out.
    TimedOut,
}

/// Tunables for the readiness wait. Defaults follow the P0 browser contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderPolicy {
    /// Total time the renderer may spend before extracting the DOM.
    pub max_render_time: Duration,
    /// How long the network must stay free of in-flight requests.
    pub quiet_window: Duration,
    /// Delay between readiness samples.
    pub sample_interval: Duration,
    /// Maximum progressive scroll steps for lazy-loaded content.
    pub max_scroll_steps: u32,
}

impl Default for RenderPolicy {
    fn default() -> Self {
        Self {
            max_render_time: DEFAULT_MAX_RENDER_TIME,
            quiet_window: DEFAULT_NETWORK_QUIET_WINDOW,
            sample_interval: DEFAULT_SAMPLE_INTERVAL,
            max_scroll_steps: DEFAULT_MAX_SCROLL_STEPS,
        }
    }
}

impl RenderPolicy {
    /// Clamp externally supplied values into the supported ranges.
    pub fn normalized(mut self) -> Self {
        self.quiet_window = self
            .quiet_window
            .clamp(MIN_NETWORK_QUIET_WINDOW, MAX_NETWORK_QUIET_WINDOW);
        if self.sample_interval.is_zero() {
            self.sample_interval = DEFAULT_SAMPLE_INTERVAL;
        }
        if self.max_render_time < self.quiet_window + self.sample_interval {
            self.max_render_time = self.quiet_window + self.sample_interval;
        }
        self
    }
}

/// Summary of how readiness resolved, attached to every rendered page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReadinessReport {
    /// Number of samples the renderer took.
    pub samples: usize,
    /// Wall-clock time spent waiting for readiness.
    pub waited: Duration,
    /// The network satisfied the quiet window at least once.
    pub network_quiet_achieved: bool,
    /// Two consecutive samples reported the same non-zero content size.
    pub content_stable: bool,
    /// The DOM reported `"complete"`.
    pub dom_loaded: bool,
    /// Progressive scroll steps performed for lazy loading.
    pub scroll_steps: u32,
    /// The readiness wait hit [`RenderPolicy::max_render_time`].
    pub timed_out: bool,
}

/// Pure readiness state machine. Feed it [`PageSample`]s with a monotonically
/// increasing `at` (time since navigation start) and act on the returned
/// [`ReadinessDecision`].
#[derive(Debug, Clone)]
pub struct ReadinessTracker {
    policy: RenderPolicy,
    dom_loaded: bool,
    quiet_since: Option<Duration>,
    quiet_achieved: bool,
    last_content_size: Option<usize>,
    stable_confirmations: u32,
    content_stable: bool,
    scroll_steps_started: u32,
    size_before_scroll: Option<usize>,
    last_scroll_grew: bool,
    samples: usize,
}

impl ReadinessTracker {
    pub fn new(policy: RenderPolicy) -> Self {
        Self {
            policy: policy.normalized(),
            dom_loaded: false,
            quiet_since: None,
            quiet_achieved: false,
            last_content_size: None,
            stable_confirmations: 0,
            content_stable: false,
            scroll_steps_started: 0,
            size_before_scroll: None,
            last_scroll_grew: false,
            samples: 0,
        }
    }

    pub fn policy(&self) -> &RenderPolicy {
        &self.policy
    }

    /// Whether `document.readyState` reached `"complete"` in any sample. Used
    /// to decide whether a timed-out render still has a DOM worth extracting.
    pub fn dom_loaded(&self) -> bool {
        self.dom_loaded
    }

    /// Observe the page at `at` (time since navigation start) and decide.
    pub fn observe(&mut self, sample: PageSample, at: Duration) -> ReadinessDecision {
        self.samples += 1;

        // A scroll was requested last round: record whether content grew and
        // restart the quiet/stability windows, because scrolling may kick off
        // new lazy-load requests.
        if let Some(size_before) = self.size_before_scroll.take() {
            self.last_scroll_grew = sample.content_size > size_before;
            self.quiet_since = None;
            self.stable_confirmations = 0;
        }

        self.dom_loaded |= sample.dom_loaded;

        // Network quiet: the window starts at the first sample with zero
        // in-flight requests and resets whenever a request becomes in flight.
        if sample.network_in_flight == 0 {
            if self.quiet_since.is_none() {
                self.quiet_since = Some(at);
            }
        } else {
            self.quiet_since = None;
        }
        let network_quiet = self
            .quiet_since
            .is_some_and(|since| at.saturating_sub(since) >= self.policy.quiet_window);
        if network_quiet {
            self.quiet_achieved = true;
        }

        // Content stability across two consecutive samples.
        let stable_across_samples =
            self.last_content_size == Some(sample.content_size) && sample.content_size > 0;
        if stable_across_samples {
            self.stable_confirmations += 1;
            self.content_stable = true;
        } else {
            self.stable_confirmations = 0;
        }
        self.last_content_size = Some(sample.content_size);

        if at >= self.policy.max_render_time {
            return ReadinessDecision::TimedOut;
        }

        if !self.dom_loaded {
            return ReadinessDecision::Waiting(WaitReason::DomNotLoaded);
        }
        if sample.content_size == 0 {
            return ReadinessDecision::Waiting(WaitReason::NoContent);
        }
        if sample.network_in_flight > 0 {
            return ReadinessDecision::Waiting(WaitReason::NetworkBusy);
        }
        if self.stable_confirmations == 0 {
            return ReadinessDecision::Waiting(WaitReason::ContentUnstable);
        }
        if !network_quiet {
            return ReadinessDecision::Waiting(WaitReason::NetworkSettling);
        }

        // Settled. Scroll for lazy content while it is still growing, up to
        // the bounded step budget.
        let should_scroll = self.policy.max_scroll_steps > 0
            && self.scroll_steps_started < self.policy.max_scroll_steps
            && (self.scroll_steps_started == 0 || self.last_scroll_grew);
        if should_scroll {
            self.scroll_steps_started += 1;
            self.size_before_scroll = Some(sample.content_size);
            return ReadinessDecision::Scroll {
                step: self.scroll_steps_started,
            };
        }

        ReadinessDecision::Ready
    }

    /// Snapshot of the readiness outcome for the rendered page.
    pub fn report(&self, at: Duration) -> ReadinessReport {
        ReadinessReport {
            samples: self.samples,
            waited: at,
            network_quiet_achieved: self.quiet_achieved,
            content_stable: self.content_stable,
            dom_loaded: self.dom_loaded,
            scroll_steps: self.scroll_steps_started,
            timed_out: false,
        }
    }

    /// Finalize the report after the caller acted on [`ReadinessDecision::TimedOut`].
    pub fn timed_out_report(&self, at: Duration) -> ReadinessReport {
        ReadinessReport {
            timed_out: true,
            ..self.report(at)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Non-scrolling policy: isolates the dom/network/stability conditions.
    fn policy() -> RenderPolicy {
        RenderPolicy {
            max_render_time: Duration::from_secs(30),
            quiet_window: Duration::from_millis(1000),
            sample_interval: Duration::from_millis(250),
            max_scroll_steps: 0,
        }
    }

    /// Policy with a two-step progressive scroll budget.
    fn scrolling_policy() -> RenderPolicy {
        RenderPolicy {
            max_scroll_steps: 2,
            ..policy()
        }
    }

    fn sample(dom_loaded: bool, network_in_flight: usize, content_size: usize) -> PageSample {
        PageSample {
            dom_loaded,
            network_in_flight,
            content_size,
        }
    }

    fn millis(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn default_policy_matches_contract() {
        let policy = RenderPolicy::default();
        assert!(
            (MIN_NETWORK_QUIET_WINDOW..=MAX_NETWORK_QUIET_WINDOW).contains(&policy.quiet_window),
            "quiet window must be inside the 750-1500ms band"
        );
        assert!(policy.max_render_time > policy.quiet_window);
        assert_eq!(policy.sample_interval, DEFAULT_SAMPLE_INTERVAL);
        assert!(policy.max_scroll_steps > 0);
    }

    #[test]
    fn policy_normalization_clamps_quiet_window() {
        let too_small = RenderPolicy {
            quiet_window: millis(50),
            ..RenderPolicy::default()
        }
        .normalized();
        assert_eq!(too_small.quiet_window, MIN_NETWORK_QUIET_WINDOW);

        let too_large = RenderPolicy {
            quiet_window: Duration::from_secs(10),
            ..RenderPolicy::default()
        }
        .normalized();
        assert_eq!(too_large.quiet_window, MAX_NETWORK_QUIET_WINDOW);
    }

    #[test]
    fn waits_for_dom_load() {
        let mut tracker = ReadinessTracker::new(policy());
        let decision = tracker.observe(sample(false, 0, 0), millis(0));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::DomNotLoaded)
        );
    }

    #[test]
    fn waits_until_the_network_goes_quiet() {
        let mut tracker = ReadinessTracker::new(policy());
        // DOM loaded, but requests are still in flight.
        let decision = tracker.observe(sample(true, 3, 500), millis(0));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::NetworkBusy)
        );
        // Requests finished: quiet window starts now.
        let decision = tracker.observe(sample(true, 0, 500), millis(500));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::NetworkSettling)
        );
        // 600ms into the window -> still settling.
        let decision = tracker.observe(sample(true, 0, 500), millis(1100));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::NetworkSettling)
        );
        // 1000ms of quiet with a stable content size -> ready to extract.
        let decision = tracker.observe(sample(true, 0, 500), millis(1500));
        assert_eq!(decision, ReadinessDecision::Ready);
    }

    #[test]
    fn network_activity_resets_the_quiet_window() {
        let mut tracker = ReadinessTracker::new(policy());
        tracker.observe(sample(true, 0, 400), millis(0));
        // A late XHR restarts the window.
        tracker.observe(sample(true, 2, 400), millis(900));
        let decision = tracker.observe(sample(true, 0, 400), millis(1000));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::NetworkSettling)
        );
        // Only 200ms have passed since the window restarted.
        let decision = tracker.observe(sample(true, 0, 400), millis(1200));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::NetworkSettling)
        );
        let decision = tracker.observe(sample(true, 0, 400), millis(2100));
        assert_eq!(decision, ReadinessDecision::Ready);
    }

    #[test]
    fn waits_for_two_stable_content_samples() {
        let mut tracker = ReadinessTracker::new(policy());
        // First observation: nothing to compare against yet.
        let decision = tracker.observe(sample(true, 0, 100), millis(0));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::ContentUnstable)
        );
        // Content grew -> not stable.
        let decision = tracker.observe(sample(true, 0, 900), millis(250));
        assert_eq!(
            decision,
            ReadinessDecision::Waiting(WaitReason::ContentUnstable)
        );
        // Two equal samples, quiet for a while -> ready.
        let decision = tracker.observe(sample(true, 0, 900), millis(1250));
        assert_eq!(decision, ReadinessDecision::Ready);
    }

    #[test]
    fn waits_for_non_zero_content() {
        let mut tracker = ReadinessTracker::new(policy());
        let decision = tracker.observe(sample(true, 0, 0), millis(100));
        assert_eq!(decision, ReadinessDecision::Waiting(WaitReason::NoContent));
    }

    #[test]
    fn progressive_scroll_runs_while_content_grows_and_is_bounded() {
        let mut tracker = ReadinessTracker::new(scrolling_policy());
        // Settled at 1000 chars -> first scroll step.
        tracker.observe(sample(true, 0, 1000), millis(0));
        let decision = tracker.observe(sample(true, 0, 1000), millis(1200));
        assert_eq!(decision, ReadinessDecision::Scroll { step: 1 });

        // Scroll revealed more content: settle again, then a second scroll.
        tracker.observe(sample(true, 0, 2000), millis(1300));
        let decision = tracker.observe(sample(true, 0, 2000), millis(2600));
        assert_eq!(decision, ReadinessDecision::Scroll { step: 2 });

        // Scroll budget exhausted: extract even though content grew again.
        tracker.observe(sample(true, 0, 3000), millis(2700));
        let decision = tracker.observe(sample(true, 0, 3000), millis(4000));
        assert_eq!(decision, ReadinessDecision::Ready);

        let report = tracker.report(millis(4000));
        assert_eq!(report.scroll_steps, 2);
        assert!(report.content_stable);
        assert!(report.network_quiet_achieved);
        assert!(report.dom_loaded);
        assert!(!report.timed_out);
    }

    #[test]
    fn scrolling_stops_when_content_stops_growing() {
        let mut tracker = ReadinessTracker::new(scrolling_policy());
        tracker.observe(sample(true, 0, 1000), millis(0));
        let decision = tracker.observe(sample(true, 0, 1000), millis(1200));
        assert_eq!(decision, ReadinessDecision::Scroll { step: 1 });

        // Content did not grow after the scroll -> do not scroll again.
        tracker.observe(sample(true, 0, 1000), millis(1300));
        let decision = tracker.observe(sample(true, 0, 1000), millis(2500));
        assert_eq!(decision, ReadinessDecision::Ready);
        assert_eq!(tracker.report(millis(2500)).scroll_steps, 1);
    }

    #[test]
    fn scrolling_disabled_when_step_budget_is_zero() {
        let mut tracker = ReadinessTracker::new(RenderPolicy {
            max_scroll_steps: 0,
            ..policy()
        });
        tracker.observe(sample(true, 0, 1000), millis(0));
        let decision = tracker.observe(sample(true, 0, 1000), millis(1200));
        assert_eq!(decision, ReadinessDecision::Ready);
        assert_eq!(tracker.report(millis(1200)).scroll_steps, 0);
    }

    #[test]
    fn times_out_when_readiness_never_holds() {
        let mut tracker = ReadinessTracker::new(RenderPolicy {
            max_render_time: Duration::from_secs(5),
            ..policy()
        });
        // DOM never loads, content keeps churning.
        for step in 0..20 {
            let at = millis(step * 250);
            let decision = tracker.observe(sample(false, 1, step as usize * 10), at);
            assert!(
                matches!(decision, ReadinessDecision::Waiting(_)),
                "step {step} should still be waiting, got {decision:?}"
            );
        }
        let decision = tracker.observe(sample(false, 1, 999), Duration::from_secs(5));
        assert_eq!(decision, ReadinessDecision::TimedOut);

        let report = tracker.timed_out_report(Duration::from_secs(5));
        assert!(report.timed_out);
        assert!(!report.dom_loaded);
    }

    #[test]
    fn ready_requires_every_condition() {
        // DOM loaded but no quiet window.
        let mut tracker = ReadinessTracker::new(policy());
        tracker.observe(sample(true, 0, 100), millis(0));
        assert_eq!(
            tracker.observe(sample(true, 0, 100), millis(100)),
            ReadinessDecision::Waiting(WaitReason::NetworkSettling)
        );

        // Quiet window satisfied but content never stable.
        let mut tracker = ReadinessTracker::new(policy());
        tracker.observe(sample(true, 0, 100), millis(0));
        tracker.observe(sample(true, 0, 200), millis(1250));
        assert_eq!(
            tracker.observe(sample(true, 0, 300), millis(2500)),
            ReadinessDecision::Waiting(WaitReason::ContentUnstable)
        );
    }
}
