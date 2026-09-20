//! `sd_notify(3)` helper for sy daemons — SPEC §4.5 "Rust integration"
//! / arch-supervision Step 4.
//!
//! Every long-running daemon (knowledge, agentd, stack-bar; aiplane
//! once split out) calls these helpers from `main` to participate in
//! `Type=notify` lifecycle:
//!
//! - [`ready`] after the listener bind: emits `READY=1 STATUS=ready`
//!   so `systemctl --user status sy-<name>.service` flips from
//!   `activating` to `active (running)`. Without it, systemd assumes
//!   the daemon is still starting and the `WatchdogSec=30s` timer
//!   never arms.
//! - [`stopping`] in the SIGTERM handler: emits
//!   `STOPPING=1 STATUS=draining` so siblings depending on us via
//!   `BindsTo=` (knowledge → qdrant) see a clean shutdown rather
//!   than a `Result=signal` failure.
//! - [`spawn_watchdog`] starts a background OS thread that pings
//!   `WATCHDOG=1` at half the systemd-configured `WATCHDOG_USEC`
//!   interval (`sd_watchdog_enabled(3)` recommends `usec/2` to leave
//!   margin for scheduling jitter). Returns `None` when the unit
//!   isn't notify-supervised — safe to spawn unconditionally. We
//!   pick `std::thread` over `tokio::spawn` because the knowledge
//!   daemon's main loop is thread-based (no tokio runtime on its
//!   path); the agentd daemon does run on tokio but a single
//!   timer-ping thread is uniformly cheap on both.
//!
//! All three helpers are safe to call without systemd. The
//! underlying `sd_notify::notify` checks `NOTIFY_SOCKET` once and
//! quietly returns `Ok(())` when it's unset — the dev workflow
//! (`cargo run`) thus emits a debug-level no-op instead of a stderr
//! warning. Anything noisier would pollute test output for the
//! daemons' in-thread integration tests.

use std::thread::{self, JoinHandle};
use std::time::Duration;

use sd_notify::NotifyState;

/// Half-interval used by [`spawn_watchdog`] to derive the
/// `WATCHDOG=1` ping cadence from systemd's `WATCHDOG_USEC`. Pulled
/// out as a pure function so the unit test can pin the math without
/// involving the tokio runtime or env vars. `sd_watchdog_enabled(3)`
/// recommends `usec/2` to leave margin for scheduling jitter.
pub fn compute_ping_interval(watchdog_usec: Duration) -> Duration {
    watchdog_usec / 2
}

/// Notify systemd that the daemon has finished bind + initial setup
/// and is ready to serve. Wraps `sd_notify(READY=1, STATUS=ready)`.
/// On non-systemd hosts (`NOTIFY_SOCKET` unset) this is a quiet
/// no-op — `sd_notify::notify` exits early before any syscall, so
/// the call is cheap and side-effect-free in tests.
pub fn ready() {
    let states = &[NotifyState::Ready, NotifyState::Status("ready")];
    if let Err(e) = sd_notify::notify(states) {
        tracing::debug!(
            target: "sparkplane::notify",
            error = %e,
            "sd_notify(Ready) failed (likely no NOTIFY_SOCKET)"
        );
    }
}

/// Notify systemd that the daemon is shutting down. Wraps
/// `sd_notify(STOPPING=1, STATUS=draining)`. Called from the
/// SIGTERM / SIGINT handler before flushing in-flight work so the
/// service manager classifies the exit as `clean` rather than
/// `signal`.
pub fn stopping() {
    let states = &[NotifyState::Stopping, NotifyState::Status("draining")];
    if let Err(e) = sd_notify::notify(states) {
        tracing::debug!(
            target: "sparkplane::notify",
            error = %e,
            "sd_notify(Stopping) failed (likely no NOTIFY_SOCKET)"
        );
    }
}

/// Spawn the watchdog-ping background thread.
///
/// Returns `None` if `WATCHDOG_USEC` is unset (unit isn't
/// notify-supervised, or `WatchdogSec=` not declared) — caller can
/// ignore the value. When `Some`, the returned handle drives a
/// `std::thread::sleep` loop that fires `WATCHDOG=1` at
/// [`compute_ping_interval`] cadence; dropping the handle does not
/// cancel the thread (it lives until process exit, which is the
/// intended lifecycle).
pub fn spawn_watchdog() -> Option<JoinHandle<()>> {
    let interval = sd_notify::watchdog_enabled().map(compute_ping_interval)?;
    Some(thread::spawn(move || {
        loop {
            thread::sleep(interval);
            if let Err(e) = sd_notify::notify(&[NotifyState::Watchdog]) {
                tracing::debug!(
                    target: "sparkplane::notify",
                    error = %e,
                    "sd_notify(Watchdog) failed"
                );
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_without_systemd_is_safe_in_an_isolated_process() {
        const CHILD: &str = "SPARKPLANE_NOTIFY_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            ready();
            stopping();
            assert!(spawn_watchdog().is_none());
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "notify::tests::notify_without_systemd_is_safe_in_an_isolated_process",
            ])
            .env(CHILD, "1")
            .env_remove("NOTIFY_SOCKET")
            .env_remove("WATCHDOG_USEC")
            .env_remove("WATCHDOG_PID")
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn watchdog_half_interval_computed_correctly() {
        assert_eq!(
            compute_ping_interval(Duration::from_secs(2)),
            Duration::from_secs(1)
        );
        assert_eq!(
            compute_ping_interval(Duration::from_secs(30)),
            Duration::from_secs(15)
        );
    }
}
