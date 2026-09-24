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

/// Extra wait on the expiry timer beyond `ACTIVITY_MAX_MS`. A timer can fire a
/// millisecond early against `Date.now()`; without this, the sweep would find
/// the entry it was armed for a hair younger than the max, keep it, and — if
/// that was its only timer — leave the dots on for good.
const EXPIRY_TIMER_SLACK_MS: u64 = 50;

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
        crate::util::sleep(Duration::from_millis(
            ACTIVITY_MAX_MS as u64 + EXPIRY_TIMER_SLACK_MS,
        ))
        .await;
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

    // --- Task 14: source-scrape pins on the call sites in other files ------
    //
    // Same technique as `util/signal_guard.rs`'s `production_only` /
    // `strip_line_comments`: cut the file at its own test module so a needle
    // appearing only in a test can't satisfy the pin, and drop `//` comments
    // so the pin can't match its own explanatory prose.

    fn production_only(src: &str) -> &str {
        match src.find("\nmod tests") {
            Some(i) => &src[..i],
            None => src,
        }
    }

    fn strip_line_comments(src: &str) -> String {
        src.lines()
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Body of `fn <name>(...) { ... }`, delimited by balancing the brace the
    /// function opens. Anchored on `prefix` (e.g. `"pub async fn
    /// handle_get_response"`) rather than a bare function name, so a helper
    /// or a comment mentioning the name elsewhere in the file can't be
    /// mistaken for the definition.
    fn fn_body<'a>(src: &'a str, prefix: &str) -> &'a str {
        let start = src
            .find(prefix)
            .unwrap_or_else(|| panic!("{prefix:?} not found in source"));
        let open = src[start..]
            .find('{')
            .map(|i| start + i)
            .unwrap_or_else(|| panic!("no `{{` found after {prefix:?}"));
        let bytes = src.as_bytes();
        let mut depth = 0i32;
        for (i, byte) in bytes.iter().enumerate().skip(open) {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[open..=i];
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced braces after {prefix:?}");
    }

    /// `network_activity::end(ActivityKind::Fetch, ...)` must be the first
    /// statement of `handle_get_response`, before the backward-probe early
    /// `return` (freenet/river issue tracked by this plan) — otherwise a
    /// probe response leaves the Fetch entry stuck until the 20s expiry.
    #[test]
    fn get_response_ends_fetch_before_any_early_return() {
        let src = strip_line_comments(production_only(include_str!(
            "freenet_api/response_handler/get_response.rs"
        )));
        let body = fn_body(&src, "pub async fn handle_get_response");

        let end_pos = body
            .find("network_activity::end(ActivityKind::Fetch")
            .expect(
                "handle_get_response must call \
                 network_activity::end(ActivityKind::Fetch, ...) so the indicator's Fetch \
                 count is settled on response",
            );

        if let Some(return_pos) = body.find("return") {
            assert!(
                end_pos < return_pos,
                "network_activity::end(Fetch) must run before the first early `return` \
                 (e.g. the backward-probe routing) — otherwise that path leaves the Fetch \
                 entry stuck until the 20s expiry"
            );
        }
    }

    /// `network_activity::end(ActivityKind::Send, ...)` must be called from
    /// `handle_update_response`, so an outbound UPDATE's Send entry is
    /// settled when the node acknowledges it.
    #[test]
    fn update_response_ends_send() {
        let src = strip_line_comments(production_only(include_str!(
            "freenet_api/response_handler/update_response.rs"
        )));
        let body = fn_body(&src, "pub fn handle_update_response");

        assert!(
            body.contains("network_activity::end(ActivityKind::Send"),
            "handle_update_response must call \
             network_activity::end(ActivityKind::Send, ...) so the indicator's Send count \
             is settled on response"
        );
    }

    /// A GET for a contract the network does not have is answered with
    /// `ContractResponse::NotFound { instance_id }`, not a `GetResponse`, so
    /// `handle_get_response` never runs. That arm must settle the Fetch entry
    /// itself, or a missing room's refresh shows `refreshing` until the 20s
    /// expiry.
    #[test]
    fn not_found_ends_fetch() {
        let src = strip_line_comments(production_only(include_str!(
            "freenet_api/response_handler.rs"
        )));
        let start = src
            .find("ContractResponse::NotFound")
            .expect("response_handler.rs must match ContractResponse::NotFound explicitly");
        let rest = &src[start + 1..];
        // Delimit to the next arm so a call elsewhere can't satisfy the pin.
        let end = [rest.find("ContractResponse::"), rest.find("_ =>")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(rest.len());
        let arm = &rest[..end];

        assert!(
            arm.contains("network_activity::end(ActivityKind::Fetch"),
            "the ContractResponse::NotFound arm must call \
             network_activity::end(ActivityKind::Fetch, ...), or a GET for a missing \
             contract leaves the indicator on until the 20s expiry"
        );
    }

    /// The `ConnectionLost` arm must clear every in-flight entry — a response
    /// tracked against a socket that just died will never arrive.
    #[test]
    fn connection_lost_clears_in_flight() {
        let src = strip_line_comments(production_only(include_str!(
            "freenet_api/freenet_synchronizer.rs"
        )));
        let start = src
            .find("SynchronizerMessage::ConnectionLost =>")
            .expect("ConnectionLost arm not found");
        let rest = &src[start..];
        // Delimit to the next match arm so a `clear_all()` call anywhere else
        // in the file (e.g. the Connect arm) can't satisfy this pin.
        let end = rest[1..]
            .find("SynchronizerMessage::")
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        let arm = &rest[..end];

        assert!(
            arm.contains("network_activity::clear_all()"),
            "the ConnectionLost arm must call network_activity::clear_all() — a response \
             for a dead socket will never arrive, so its in-flight entries must not wait \
             out the 20s expiry"
        );
    }

    /// The `Connect` -> `Ok(())` arm must also clear every in-flight entry:
    /// any request tracked against the PREVIOUS (now-replaced) socket will
    /// never get a response on the new one.
    #[test]
    fn connect_ok_clears_in_flight() {
        let src = strip_line_comments(production_only(include_str!(
            "freenet_api/freenet_synchronizer.rs"
        )));
        let anchor = "Connection established successfully";
        let start = src
            .find(anchor)
            .expect("\"Connection established successfully\" not found");
        // A tight window right after the log line: the Connect arm's body is
        // long and itself mentions `SynchronizerMessage::` many times (nested
        // match arms for ConnectionStable/ProcessRooms/etc.), so delimiting
        // to "next SynchronizerMessage::" like the ConnectionLost pin above
        // would not isolate this call the same way.
        // By chars, not bytes: a byte offset can land inside a multi-byte
        // character and panic the slice.
        let window: String = src[start..].chars().take(300).collect();

        assert!(
            window.contains("network_activity::clear_all()"),
            "the Connect -> Ok(()) arm must call network_activity::clear_all() shortly \
             after establishing the connection — any in-flight entry tracked against the \
             previous (dead) socket will never get its response"
        );
    }
}
