//! Bookkeeping for auth token refreshes after the server refuses a request
//! with `ErrUnauthenticated`.
//!
//! JS reference: `packages/sdk/src/client/client.ts` (`refreshAuthToken`,
//! `AuthRefreshBackoff`, `authRefreshDelay`, `loopRetryDelay`,
//! `AuthRetryMaxWaitMs`) and `test/unit/client/auth_refresh_test.ts`.
//!
//! Each refusal makes its own injector call. A call that fails starts a
//! backoff during which the background loops do not call the injector; a
//! one-shot call always does. Calls are numbered, so a call that settles
//! after a later one already set its token neither sets its own nor changes
//! the backoff, and a call still pending when the client activates again
//! does not override the new session's token. Failures that arrive together
//! count as one.

use crate::options::DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;
use tokio::time::Instant;

/// The first wait after a failed refresh (JS `AuthRefreshBackoff.initial`).
const AUTH_REFRESH_INITIAL: Duration = Duration::from_secs(1);
/// The longest wait between refreshes (JS `AuthRefreshBackoff.max`).
const AUTH_REFRESH_MAX: Duration = Duration::from_secs(30);
/// How far each wait is jittered either way (JS `AuthRefreshBackoff.jitter`).
const AUTH_REFRESH_JITTER: f64 = 0.2;
/// The longest a loop retry that got a refusal waits for the backoff (JS
/// `AuthRetryMaxWaitMs`), so channel sessions outlive the server's 15s TTL.
const AUTH_RETRY_MAX_WAIT: Duration = Duration::from_millis(DEFAULT_CHANNEL_HEARTBEAT_INTERVAL_MS);

/// Returns the wait before the next refresh after `failures` refreshes failed
/// in a row (JS `authRefreshDelay`). `random` is in `[0, 1)`; 0.5 means no
/// jitter.
pub(crate) fn auth_refresh_delay(failures: u32, random: f64) -> Duration {
    let base = AUTH_REFRESH_INITIAL
        .saturating_mul(2u32.saturating_pow(failures.saturating_sub(1)))
        .min(AUTH_REFRESH_MAX);
    let jitter = 1.0 + AUTH_REFRESH_JITTER * (random * 2.0 - 1.0);
    base.mul_f64(jitter).min(AUTH_REFRESH_MAX)
}

/// Returns a number in `[0, 1)` for the jitter (JS `Math.random`), from the
/// random keys std seeds each `RandomState` with.
pub(crate) fn random_unit() -> f64 {
    let bits = RandomState::new().build_hasher().finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// One injector call after a refusal.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RefreshCall {
    generation: u64,
    /// The failures in a row when the call started.
    failures: u32,
}

/// The state of token refreshes (JS `authRefreshFailures`,
/// `authRefreshRetryAt`, `authRefreshGeneration`, `authTokenGeneration`).
#[derive(Debug, Default)]
pub(crate) struct AuthRefresh {
    /// Refreshes that failed in a row.
    failures: u32,
    /// When the loops may call the injector again.
    retry_at: Option<Instant>,
    /// The number of the last injector call.
    generation: u64,
    /// The number of the call whose token is in use.
    token_generation: u64,
}

impl AuthRefresh {
    /// Records the token the injector gave at activation, which ends any
    /// backoff and outdates every call still pending.
    pub(crate) fn activated(&mut self) {
        self.failures = 0;
        self.retry_at = None;
        self.generation += 1;
        self.token_generation = self.generation;
    }

    /// Starts an injector call, or returns `None` when a loop must skip it
    /// because the backoff has not run out.
    pub(crate) fn begin(&mut self, in_retry_loop: bool, now: Instant) -> Option<RefreshCall> {
        if in_retry_loop && self.retry_at.is_some_and(|at| now < at) {
            return None;
        }
        self.generation += 1;
        Some(RefreshCall {
            generation: self.generation,
            failures: self.failures,
        })
    }

    /// Records a token the call returned and returns whether it is to be
    /// used: only when no later call has set one.
    pub(crate) fn succeeded(&mut self, call: RefreshCall) -> bool {
        if call.generation <= self.token_generation {
            return false;
        }
        self.token_generation = call.generation;
        self.failures = 0;
        self.retry_at = None;
        true
    }

    /// Records that the call failed. It counts, and starts or extends the
    /// backoff, only if no later call has set a token and no call that
    /// started alongside it has counted already.
    pub(crate) fn failed(&mut self, call: RefreshCall, now: Instant, random: f64) {
        if call.generation > self.token_generation && self.failures == call.failures {
            self.failures = call.failures + 1;
            self.retry_at = Some(now + auth_refresh_delay(self.failures, random));
        }
    }

    /// Returns how long a loop retry that got a refusal waits: its own
    /// `delay`, or until the next refresh may run if that is later, but no
    /// longer than 5s (JS `loopRetryDelay` for an `ErrUnauthenticated`).
    pub(crate) fn loop_retry_delay(&self, delay: Duration, now: Instant) -> Duration {
        let backoff = self
            .retry_at
            .map_or(Duration::ZERO, |at| at.saturating_duration_since(now))
            .min(AUTH_RETRY_MAX_WAIT);
        delay.max(backoff)
    }
}

#[cfg(test)]
mod tests {
    use super::{auth_refresh_delay, random_unit, AuthRefresh};
    use std::time::Duration;
    use tokio::time::Instant;

    /// No jitter, as the JS tests mock `Math.random` to 0.5.
    const NO_JITTER: f64 = 0.5;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// A loop's refusal with an injector that fails: whether it called the
    /// injector.
    fn loop_refusal_fails(refresh: &mut AuthRefresh, now: Instant) -> bool {
        match refresh.begin(true, now) {
            Some(call) => {
                refresh.failed(call, now, NO_JITTER);
                true
            }
            None => false,
        }
    }

    /// JS `authRefreshDelay`: 1s doubling up to 30s, jittered by 20%.
    #[test]
    fn refresh_delay_doubles_up_to_thirty_seconds() {
        let delays = (1..=7)
            .map(|failures| auth_refresh_delay(failures, NO_JITTER))
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                ms(1000),
                ms(2000),
                ms(4000),
                ms(8000),
                ms(16000),
                ms(30000),
                ms(30000)
            ],
            delays
        );
        assert_eq!(ms(800), auth_refresh_delay(1, 0.0));
        assert_eq!(ms(1200), auth_refresh_delay(1, 1.0));
        assert_eq!(ms(19200), auth_refresh_delay(5, 1.0));
        assert_eq!(ms(30000), auth_refresh_delay(6, 1.0));
        assert_eq!(ms(30000), auth_refresh_delay(u32::MAX, NO_JITTER));
        for _ in 0..100 {
            let random = random_unit();
            assert!((0.0..1.0).contains(&random), "{random}");
        }
    }

    /// JS "calls a rejecting injector again only after a doubling backoff".
    #[test]
    fn loops_call_a_failing_injector_after_a_doubling_backoff() {
        let start = Instant::now();
        let mut refresh = AuthRefresh::default();
        // A refusal every 100ms, as from loops that retried at their own
        // delay.
        let calls_from = |refresh: &mut AuthRefresh, from: u64, to: u64| {
            (from..to)
                .step_by(100)
                .filter(|t| loop_refusal_fails(refresh, start + ms(*t)))
                .count()
        };
        assert_eq!(1, calls_from(&mut refresh, 0, 1000));
        assert_eq!(1, calls_from(&mut refresh, 1000, 3000));
        assert_eq!(1, calls_from(&mut refresh, 3000, 7000));
        assert_eq!(1, calls_from(&mut refresh, 7000, 7100));

        // A refresh that succeeds ends the backoff.
        let call = refresh.begin(true, start + ms(15000)).unwrap();
        assert!(refresh.succeeded(call));
        assert!(refresh.begin(true, start + ms(15000)).is_some());
    }

    /// JS "caps the backoff at 30s".
    #[test]
    fn backoff_is_capped_at_thirty_seconds() {
        let mut refresh = AuthRefresh::default();
        let mut now = Instant::now();
        for wait in [1000, 2000, 4000, 8000, 16000] {
            assert!(loop_refusal_fails(&mut refresh, now));
            now += ms(wait);
        }
        assert!(loop_refusal_fails(&mut refresh, now));
        assert!(!loop_refusal_fails(&mut refresh, now + ms(29999)));
        assert!(loop_refusal_fails(&mut refresh, now + ms(30000)));
    }

    /// JS "counts 401s that arrive together as one failure".
    #[test]
    fn failures_that_arrive_together_count_once() {
        let now = Instant::now();
        let mut refresh = AuthRefresh::default();
        let calls = (0..3)
            .map(|_| refresh.begin(true, now).unwrap())
            .collect::<Vec<_>>();
        for call in calls {
            refresh.failed(call, now + ms(300), NO_JITTER);
        }
        // The next refresh may run 1s later, not 4s.
        assert_eq!(ms(1000), refresh.loop_retry_delay(ms(0), now + ms(300)));
    }

    /// JS "lets an earlier call that settles later neither set its token nor
    /// count".
    #[test]
    fn an_earlier_call_settling_later_neither_sets_its_token_nor_counts() {
        let now = Instant::now();
        let mut refresh = AuthRefresh::default();
        let first = refresh.begin(true, now).unwrap();
        let second = refresh.begin(true, now).unwrap();
        assert!(refresh.succeeded(second));
        assert!(!refresh.succeeded(first));

        let third = refresh.begin(true, now).unwrap();
        let fourth = refresh.begin(true, now).unwrap();
        assert!(refresh.succeeded(fourth));
        refresh.failed(third, now, NO_JITTER);
        assert_eq!(ms(0), refresh.loop_retry_delay(ms(0), now));
    }

    /// JS "sets the token of a call that settles while a later one is
    /// pending": a call is not dropped because a later one is in flight.
    #[test]
    fn a_call_settling_first_sets_its_token_while_a_later_one_is_pending() {
        let now = Instant::now();
        let mut refresh = AuthRefresh::default();
        let sync_loop = refresh.begin(true, now).unwrap();
        let watch = refresh.begin(true, now + ms(2000)).unwrap();
        assert!(refresh.succeeded(sync_loop));
        assert!(refresh.succeeded(watch));
    }

    /// JS "drops an injector call still pending when the client activates
    /// again".
    #[test]
    fn activation_outdates_pending_calls() {
        let now = Instant::now();
        let mut refresh = AuthRefresh::default();
        let old_session = refresh.begin(true, now).unwrap();
        refresh.activated();
        assert!(!refresh.succeeded(old_session));
    }

    /// JS "holds only the retries that got a 401 until the next refresh may
    /// run" and "retries the sync loop at least every 5s".
    #[test]
    fn loop_retries_wait_for_the_backoff_but_at_most_five_seconds() {
        let now = Instant::now();
        let mut refresh = AuthRefresh::default();
        assert_eq!(ms(100), refresh.loop_retry_delay(ms(100), now));

        assert!(loop_refusal_fails(&mut refresh, now));
        assert_eq!(ms(1000), refresh.loop_retry_delay(ms(100), now));
        assert_eq!(ms(600), refresh.loop_retry_delay(ms(100), now + ms(400)));
        assert_eq!(ms(100), refresh.loop_retry_delay(ms(100), now + ms(1000)));

        // Backed off to 16s: a loop still waits at most 5s, and a retry that
        // waits longer on its own keeps its delay.
        let mut now = now + ms(1000);
        for wait in [2000, 4000, 8000] {
            assert!(loop_refusal_fails(&mut refresh, now));
            now += ms(wait);
        }
        assert!(loop_refusal_fails(&mut refresh, now));
        assert_eq!(ms(5000), refresh.loop_retry_delay(ms(1000), now));
        assert_eq!(ms(8000), refresh.loop_retry_delay(ms(8000), now));
    }

    /// JS "lets a one-shot sync call the injector during the backoff": a
    /// one-shot call always calls it, and its failure counts.
    #[test]
    fn one_shot_calls_ignore_the_backoff() {
        let now = Instant::now();
        let mut refresh = AuthRefresh::default();
        assert!(loop_refusal_fails(&mut refresh, now));
        assert!(!loop_refusal_fails(&mut refresh, now));

        let call = refresh.begin(false, now).unwrap();
        refresh.failed(call, now, NO_JITTER);
        assert_eq!(ms(2000), refresh.loop_retry_delay(ms(0), now));

        let call = refresh.begin(false, now).unwrap();
        assert!(refresh.succeeded(call));
        assert_eq!(ms(0), refresh.loop_retry_delay(ms(0), now));
    }
}
