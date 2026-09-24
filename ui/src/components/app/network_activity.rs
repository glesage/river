//! In-flight GET/UPDATE tracker for the network activity indicator
//! (docs/plans/2026-09-24-network-activity-indicator.md, Phase 2).
//!
//! `RoomSynchronizer` fires GETs and UPDATEs whose responses arrive later,
//! asynchronously, through `ApiResponse`. A GET or UPDATE for an
//! already-`Subscribed` room (the wake refresh, an outbound message send)
//! changes no other tracked signal (`SYNC_STATUS`, `ROOMS_LOAD_STATE`,
//! `SYNC_INFO`), so without this module those requests are invisible to
//! `loading_reason()` (`network_activity_indicator.rs`). This is a small
//! counted set of outstanding requests, with a hard expiry so a lost
//! response (a dropped socket, a bug in a handler) can never leave the
//! indicator running forever.

use crate::components::app::sync_info::now_ms;
use dioxus::prelude::*;
use freenet_stdlib::prelude::ContractInstanceId;
use std::collections::HashMap;
use std::time::Duration;

/// The two kinds of outstanding request the indicator distinguishes: `Fetch`
/// for a GET, `Send` for an outbound UPDATE.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ActivityKind {
    Fetch,
    Send,
}

/// Outstanding tracked requests, keyed by `(kind, contract)`. Each entry is a
/// reference count (more than one outstanding request of the same kind for
/// the same room) plus the timestamp of the most recent `begin`, used by
/// [`InFlight::expire`] to drop an entry whose response was lost.
///
/// UI-local and never serialized, so a `HashMap` is fine here — the
/// determinism rule in `.claude/rules/contract-summary-determinism.md` is
/// about contract `Summary`/`Delta` types, not this signal.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct InFlight {
    entries: HashMap<(ActivityKind, ContractInstanceId), (u32, f64)>,
}

impl InFlight {
    /// Record a new outstanding request. `now_ms` is the caller's clock
    /// reading. A second concurrent `begin` for the same key bumps the count
    /// and moves `last` forward to `now_ms`, so an expiry timer armed by an
    /// earlier `begin` does not drop a request that is still legitimately in
    /// flight (see `rebegin_moves_last_forward_so_an_old_timer_does_not_drop_it`).
    pub fn begin(&mut self, kind: ActivityKind, id: ContractInstanceId, now_ms: f64) {
        let entry = self.entries.entry((kind, id)).or_insert((0, now_ms));
        entry.0 += 1;
        entry.1 = now_ms;
    }

    /// Settle one outstanding request. A no-op if none is tracked for this
    /// key — the untracked liveness GET's response can also land here, and
    /// must not panic or go negative.
    pub fn end(&mut self, kind: ActivityKind, id: ContractInstanceId) {
        let key = (kind, id);
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.0 <= 1 {
                self.entries.remove(&key);
            } else {
                entry.0 -= 1;
            }
        }
    }

    /// Drop entries whose most recent `begin` is at least `max_ms` old — a
    /// hard backstop for a response that never arrives.
    pub fn expire(&mut self, now_ms: f64, max_ms: f64) {
        self.entries.retain(|_, (_, last)| *last > now_ms - max_ms);
    }

    /// Drop every outstanding entry — used when the socket is known dead, so
    /// no response tracked against it will ever arrive.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of distinct `(kind, contract)` keys currently outstanding for
    /// `kind` — NOT the sum of their reference counts, matching "at least one
    /// tracked GET/UPDATE in flight" in the indicator's priority table.
    pub fn count(&self, kind: ActivityKind) -> usize {
        self.entries.keys().filter(|(k, _)| *k == kind).count()
    }
}

/// Matches the liveness watchdog's timeout (`connection_watchdog.rs`), so a
/// lost response is bounded by the same horizon the rest of the connection
/// machinery already uses rather than a separate constant to keep in sync.
pub const ACTIVITY_MAX_MS: f64 = 20_000.0;

pub static IN_FLIGHT: GlobalSignal<InFlight> = Global::new(InFlight::default);

/// Record a request as sent. Called from inside `RoomSynchronizer`'s polled
/// future while `WEB_API` is held, so the signal write is deferred
/// (`.claude/rules/dioxus-signal-safety.md`). `begin` takes no borrow itself.
///
/// `now_ms()` is read at call time, not inside the deferred closure: `begin`
/// runs immediately after `send(..)` returns `Ok`, so the call-time reading
/// is the more accurate timestamp for "when did this request go out". The
/// gap to the deferred write is a single `setTimeout(0)` tick, which does not
/// matter to a debounce measured in hundreds of milliseconds or a 20s expiry.
///
/// Also arms a hard expiry: if `end` for this key never arrives (a dropped
/// response, a bug in a handler), the whole set is swept `ACTIVITY_MAX_MS`
/// after this `begin`. A later `begin` of the same key moves `last` forward
/// in [`InFlight::begin`], so an earlier timer's sweep does not drop a
/// request that is still legitimately in flight.
pub fn begin(kind: ActivityKind, id: ContractInstanceId) {
    let started_at = now_ms();
    crate::util::defer(move || {
        IN_FLIGHT.with_mut(|f| f.begin(kind, id, started_at));
    });
    crate::util::safe_spawn_local(async move {
        crate::util::sleep(Duration::from_millis(ACTIVITY_MAX_MS as u64)).await;
        crate::util::defer(move || {
            IN_FLIGHT.with_mut(|f| f.expire(now_ms(), ACTIVITY_MAX_MS));
        });
    });
}

/// Settle a request on its response. Deferred for the same reason as
/// [`begin`].
pub fn end(kind: ActivityKind, id: ContractInstanceId) {
    crate::util::defer(move || {
        IN_FLIGHT.with_mut(|f| f.end(kind, id));
    });
}

/// Drop every outstanding entry — called when the socket is known dead
/// (`ConnectionLost`) or freshly re-established (`Connect` -> `Ok`), since no
/// response tracked against the old connection will ever arrive.
pub fn clear_all() {
    crate::util::defer(|| {
        IN_FLIGHT.with_mut(|f| f.clear());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ContractInstanceId {
        ContractInstanceId::new([n; 32])
    }

    #[test]
    fn begin_then_end_is_empty() {
        let mut f = InFlight::default();
        f.begin(ActivityKind::Fetch, id(1), 0.0);
        f.end(ActivityKind::Fetch, id(1));
        assert_eq!(f.count(ActivityKind::Fetch), 0);
    }

    #[test]
    fn two_begins_need_two_ends() {
        let mut f = InFlight::default();
        f.begin(ActivityKind::Send, id(1), 0.0);
        f.begin(ActivityKind::Send, id(1), 10.0);
        assert_eq!(f.count(ActivityKind::Send), 1, "one key, two outstanding");

        f.end(ActivityKind::Send, id(1));
        assert_eq!(
            f.count(ActivityKind::Send),
            1,
            "still one outstanding after a single end"
        );

        f.end(ActivityKind::Send, id(1));
        assert_eq!(f.count(ActivityKind::Send), 0);
    }

    #[test]
    fn end_without_begin_is_a_noop() {
        let mut f = InFlight::default();
        f.end(ActivityKind::Fetch, id(1)); // must not panic or underflow
        assert_eq!(f.count(ActivityKind::Fetch), 0);
    }

    #[test]
    fn expire_drops_only_entries_older_than_max() {
        let mut f = InFlight::default();
        f.begin(ActivityKind::Fetch, id(1), 0.0);
        f.begin(ActivityKind::Fetch, id(2), 15_000.0);

        f.expire(20_000.0, 20_000.0);

        assert_eq!(
            f.count(ActivityKind::Fetch),
            1,
            "only the entry at (or past) exactly the max age is dropped"
        );
    }

    #[test]
    fn rebegin_moves_last_forward_so_an_old_timer_does_not_drop_it() {
        let mut f = InFlight::default();
        f.begin(ActivityKind::Fetch, id(1), 0.0);
        f.begin(ActivityKind::Fetch, id(1), 15_000.0); // a second begin for the same key

        // An expiry timer armed by the FIRST begin fires at 20_000 (0 + max).
        f.expire(20_000.0, 20_000.0);

        assert_eq!(
            f.count(ActivityKind::Fetch),
            1,
            "the rebegin moved `last` to 15_000, so the entry is still fresh"
        );
    }

    #[test]
    fn count_is_per_kind() {
        let mut f = InFlight::default();
        f.begin(ActivityKind::Fetch, id(1), 0.0);
        f.begin(ActivityKind::Send, id(1), 0.0);
        f.begin(ActivityKind::Send, id(2), 0.0);

        assert_eq!(f.count(ActivityKind::Fetch), 1);
        assert_eq!(f.count(ActivityKind::Send), 2);
    }

    #[test]
    fn clear_empties() {
        let mut f = InFlight::default();
        f.begin(ActivityKind::Fetch, id(1), 0.0);
        f.begin(ActivityKind::Send, id(2), 0.0);

        f.clear();

        assert_eq!(f.count(ActivityKind::Fetch), 0);
        assert_eq!(f.count(ActivityKind::Send), 0);
    }

}
