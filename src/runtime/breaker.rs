//! Circuit breaker for the inference endpoint.
//!
//! Closed: calls go through. After `consecutive_failures` failures in a row,
//! or an error rate of at least `error_rate` over `window` (with enough
//! calls to judge), the breaker **opens**: callers skip the model and send
//! reports to manual triage instead of waiting on timeouts. After
//! `half_open_after`, one trial call is allowed; success closes the breaker,
//! failure opens it again.
//!
//! Time is passed in by the caller so the behaviour is deterministic in tests.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Minimum calls in the window before the error rate can open the breaker.
const MIN_CALLS_FOR_RATE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

impl BreakerState {
    /// 0 = closed, 1 = half-open, 2 = open (for a metrics gauge).
    pub fn as_gauge(&self) -> f64 {
        match self {
            BreakerState::Closed => 0.0,
            BreakerState::HalfOpen => 1.0,
            BreakerState::Open => 2.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    consecutive_limit: u32,
    error_rate: f64,
    window: Duration,
    half_open_after: Duration,
    state: BreakerState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    trial_in_flight: bool,
    /// (time, succeeded) for calls within the window.
    recent: VecDeque<(Instant, bool)>,
}

impl CircuitBreaker {
    pub fn new(consecutive_limit: u32, error_rate: f64, window: Duration, half_open_after: Duration) -> Self {
        Self {
            consecutive_limit,
            error_rate,
            window,
            half_open_after,
            state: BreakerState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            trial_in_flight: false,
            recent: VecDeque::new(),
        }
    }

    pub fn state(&self) -> BreakerState {
        self.state
    }

    /// Whether a call may go to the model now. When the open period has
    /// passed this admits exactly one trial call (half-open).
    pub fn allow(&mut self, now: Instant) -> bool {
        match self.state {
            BreakerState::Closed => true,
            BreakerState::Open => {
                let ready = self.opened_at.is_some_and(|t| now.duration_since(t) >= self.half_open_after);
                if ready {
                    self.state = BreakerState::HalfOpen;
                    self.trial_in_flight = true;
                }
                ready
            }
            BreakerState::HalfOpen => {
                if self.trial_in_flight {
                    false
                } else {
                    self.trial_in_flight = true;
                    true
                }
            }
        }
    }

    pub fn record_success(&mut self, now: Instant) {
        self.push(now, true);
        self.consecutive_failures = 0;
        if self.state == BreakerState::HalfOpen {
            self.state = BreakerState::Closed;
            self.opened_at = None;
            self.trial_in_flight = false;
            self.recent.clear();
        }
    }

    pub fn record_failure(&mut self, now: Instant) {
        self.push(now, false);
        self.consecutive_failures += 1;
        match self.state {
            BreakerState::HalfOpen => self.open(now),
            BreakerState::Closed if self.should_open() => self.open(now),
            _ => {}
        }
    }

    fn should_open(&self) -> bool {
        if self.consecutive_failures >= self.consecutive_limit {
            return true;
        }
        if self.recent.len() < MIN_CALLS_FOR_RATE {
            return false;
        }
        let failures = self.recent.iter().filter(|(_, ok)| !ok).count();
        failures as f64 / self.recent.len() as f64 >= self.error_rate
    }

    fn open(&mut self, now: Instant) {
        self.state = BreakerState::Open;
        self.opened_at = Some(now);
        self.trial_in_flight = false;
    }

    fn push(&mut self, now: Instant, ok: bool) {
        self.recent.push_back((now, ok));
        while let Some((t, _)) = self.recent.front() {
            if now.duration_since(*t) > self.window {
                self.recent.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breaker() -> CircuitBreaker {
        CircuitBreaker::new(5, 0.5, Duration::from_secs(30), Duration::from_secs(30))
    }

    #[test]
    fn opens_after_consecutive_failures() {
        let mut b = breaker();
        let t = Instant::now();
        for _ in 0..4 {
            assert!(b.allow(t));
            b.record_failure(t);
        }
        assert_eq!(b.state(), BreakerState::Closed);
        b.record_failure(t);
        assert_eq!(b.state(), BreakerState::Open);
        assert!(!b.allow(t + Duration::from_secs(1)));
    }

    #[test]
    fn success_resets_the_consecutive_count() {
        let mut b = breaker();
        let t = Instant::now();
        for _ in 0..4 {
            b.record_failure(t);
        }
        b.record_success(t);
        for _ in 0..4 {
            b.record_failure(t);
        }
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[test]
    fn opens_on_error_rate_with_enough_calls() {
        let mut b = breaker();
        let t = Instant::now();
        // Alternate so there are never 5 failures in a row, but 50% fail.
        for _ in 0..5 {
            b.record_success(t);
            b.record_failure(t);
        }
        assert_eq!(b.state(), BreakerState::Open);
    }

    #[test]
    fn old_calls_leave_the_window() {
        let mut b = breaker();
        let t = Instant::now();
        for _ in 0..4 {
            b.record_failure(t);
            b.record_success(t);
        }
        // 40 s later the earlier calls no longer count toward the rate.
        let later = t + Duration::from_secs(40);
        b.record_failure(later);
        b.record_success(later);
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[test]
    fn half_open_allows_one_trial_then_closes_on_success() {
        let mut b = breaker();
        let t = Instant::now();
        for _ in 0..5 {
            b.record_failure(t);
        }
        let later = t + Duration::from_secs(30);
        assert!(b.allow(later), "one trial after the open period");
        assert!(!b.allow(later), "only one trial at a time");
        b.record_success(later);
        assert_eq!(b.state(), BreakerState::Closed);
        assert!(b.allow(later));
    }

    #[test]
    fn failed_trial_reopens() {
        let mut b = breaker();
        let t = Instant::now();
        for _ in 0..5 {
            b.record_failure(t);
        }
        let later = t + Duration::from_secs(30);
        assert!(b.allow(later));
        b.record_failure(later);
        assert_eq!(b.state(), BreakerState::Open);
        assert!(!b.allow(later + Duration::from_secs(10)));
        assert!(b.allow(later + Duration::from_secs(30)));
    }
}
