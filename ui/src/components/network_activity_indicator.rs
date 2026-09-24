//! Global "River is loading" indicator: ten accent-blue dots riding one
//! travelling sine wave, fixed at the bottom-centre of the page.
//!
//! Two pure pieces decide what it shows, so both are unit-testable natively:
//!
//! - [`loading_reason`] says whether River is busy **right now**, and why.
//! - [`DisplayGate`] decides whether the dots are **on screen**: a 200 ms
//!   debounce before they appear and a 1.5 s minimum once they have.
//!
//! See docs/plans/2026-09-24-network-activity-indicator.md.

use crate::components::app::chat_delegate::RoomsLoadState;
use crate::components::app::freenet_api::freenet_synchronizer::SynchronizerStatus;

/// Why the indicator is showing. Variants are in priority order: when several
/// apply, [`loading_reason`] picks the first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LoadingReason {
    Connecting,
    Reconnecting,
    JoiningRoom,
    LoadingRooms,
    SyncingRooms,
    Refreshing,
    Sending,
}

impl LoadingReason {
    /// Value of the indicator's `data-reason` attribute.
    pub(crate) fn as_attr(self) -> &'static str {
        match self {
            LoadingReason::Connecting => "connecting",
            LoadingReason::Reconnecting => "reconnecting",
            LoadingReason::JoiningRoom => "joining-room",
            LoadingReason::LoadingRooms => "loading-rooms",
            LoadingReason::SyncingRooms => "syncing-rooms",
            LoadingReason::Refreshing => "refreshing",
            LoadingReason::Sending => "sending",
        }
    }

    /// Screen-reader label for the live region.
    pub(crate) fn label(self) -> &'static str {
        match self {
            LoadingReason::Connecting => "Connecting to Freenet…",
            LoadingReason::Reconnecting => "Reconnecting to Freenet…",
            LoadingReason::JoiningRoom => "Joining room…",
            LoadingReason::LoadingRooms => "Loading your rooms…",
            LoadingReason::SyncingRooms => "Syncing rooms…",
            LoadingReason::Refreshing => "Fetching latest messages…",
            LoadingReason::Sending => "Sending…",
        }
    }
}

/// Everything [`loading_reason`] looks at, snapshotted from signals by the
/// component's memo.
pub(crate) struct ActivityInputs<'a> {
    pub sync_status: &'a SynchronizerStatus,
    /// False in a `no-sync` build, where `SYNC_STATUS` sits at `Disconnected`
    /// for good and must not read as "reconnecting".
    pub sync_enabled: bool,
    pub rooms_load_state: RoomsLoadState,
    /// Any pending invite still `PendingSubscription` or `Subscribing`.
    pub joining: bool,
    /// `SyncInfo::rooms_syncing_count` over the rooms in `ROOMS`.
    pub rooms_syncing: usize,
    pub fetches_in_flight: usize,
    pub sends_in_flight: usize,
}

/// Whether River is busy right now, and why. `None` means idle.
///
/// Every `SynchronizerStatus` variant is spelled out (no `_`), so a new variant
/// is a compile error here rather than a silent "idle".
pub(crate) fn loading_reason(i: &ActivityInputs) -> Option<LoadingReason> {
    match i.sync_status {
        SynchronizerStatus::Connecting => Some(LoadingReason::Connecting),
        // In a sync build the only writer of `Disconnected` is the
        // `ConnectionLost` handler, which always arms a reconnect first. A
        // `no-sync` build starts and stays `Disconnected`.
        SynchronizerStatus::Disconnected if i.sync_enabled => Some(LoadingReason::Reconnecting),
        SynchronizerStatus::Disconnected => None,
        // Sometimes a backoff reconnect is armed, sometimes it is terminal
        // ("Please refresh the page"), and the status cannot tell which. The
        // red pill explains it; the next attempt goes back to `Connecting`.
        SynchronizerStatus::Error(_) => None,
        // Everything below needs a live socket: nothing loads without one, and
        // the example build's `ROOMS_LOAD_STATE` sits at `Loading` forever.
        SynchronizerStatus::Connected => {
            if i.joining {
                Some(LoadingReason::JoiningRoom)
            } else if matches!(
                i.rooms_load_state,
                RoomsLoadState::Loading | RoomsLoadState::Migrating
            ) {
                Some(LoadingReason::LoadingRooms)
            } else if i.rooms_syncing > 0 {
                Some(LoadingReason::SyncingRooms)
            } else if i.fetches_in_flight > 0 {
                Some(LoadingReason::Refreshing)
            } else if i.sends_in_flight > 0 {
                Some(LoadingReason::Sending)
            } else {
                None
            }
        }
    }
}

/// Loading must still be in progress this long after it started before the dots appear.
pub(crate) const SHOW_DEBOUNCE_MS: f64 = 200.0;
/// Once shown, the dots stay at least this long. Equal to the CSS wave period
/// (`river-flow-wave 1.5s`), so the minimum is one full ripple.
pub(crate) const MIN_VISIBLE_MS: f64 = 1_500.0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum GatePhase {
    #[default]
    Idle,
    /// Busy, not shown yet. Becomes `Shown` at `since + SHOW_DEBOUNCE_MS`.
    Pending { since: f64 },
    /// Busy and on screen since `shown_at`.
    Shown { shown_at: f64 },
    /// No longer busy, held on screen until `shown_at + MIN_VISIBLE_MS`.
    Holding { shown_at: f64 },
}

/// Whether the dots are on screen. CSS can delay an element appearing but not
/// delay one being removed from the DOM, so the timing lives here. No signals
/// and no clock: every call takes `now` in ms.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct DisplayGate {
    phase: GatePhase,
    /// Latest non-None reason. Kept through `Holding` so `data-reason` and the
    /// screen-reader label don't blank out during the hold.
    reason: Option<LoadingReason>,
}

impl DisplayGate {
    /// The loading state changed. Returns the new gate, plus a delay in ms
    /// after which `on_tick` must run, if this transition armed a deadline.
    pub(crate) fn on_reason(self, reason: Option<LoadingReason>, now: f64) -> (Self, Option<f64>) {
        let kept = reason.or(self.reason);
        let with = |phase| Self {
            phase,
            reason: kept,
        };
        match (self.phase, reason) {
            (GatePhase::Idle, Some(_)) => (
                with(GatePhase::Pending { since: now }),
                Some(SHOW_DEBOUNCE_MS),
            ),
            (GatePhase::Idle, None) => (Self::default(), None),
            // The debounce timer armed on entry still covers it.
            (GatePhase::Pending { since }, Some(_)) => (with(GatePhase::Pending { since }), None),
            // Finished inside the debounce: never shown.
            (GatePhase::Pending { .. }, None) => (Self::default(), None),
            (GatePhase::Shown { shown_at }, Some(_)) => (with(GatePhase::Shown { shown_at }), None),
            (GatePhase::Shown { shown_at }, None) => {
                let until = shown_at + MIN_VISIBLE_MS;
                if now >= until {
                    (Self::default(), None)
                } else {
                    (with(GatePhase::Holding { shown_at }), Some(until - now))
                }
            }
            // Busy again mid-hold: back on, with the ORIGINAL clock. The
            // minimum is on total visible time, not per flicker.
            (GatePhase::Holding { shown_at }, Some(_)) => {
                (with(GatePhase::Shown { shown_at }), None)
            }
            (GatePhase::Holding { .. }, None) => (self, None),
        }
    }

    /// A timer armed by `on_reason` fired. Idempotent: a stale or early timer
    /// changes nothing, so timers never need cancelling.
    pub(crate) fn on_tick(self, now: f64) -> Self {
        match self.phase {
            // `now`, not `since + SHOW_DEBOUNCE_MS`: the minimum counts from
            // when the dots could first be seen, even if the timer ran late.
            GatePhase::Pending { since } if now >= since + SHOW_DEBOUNCE_MS => Self {
                phase: GatePhase::Shown { shown_at: now },
                ..self
            },
            GatePhase::Holding { shown_at } if now >= shown_at + MIN_VISIBLE_MS => Self::default(),
            GatePhase::Idle
            | GatePhase::Pending { .. }
            | GatePhase::Shown { .. }
            | GatePhase::Holding { .. } => self,
        }
    }

    /// The time at which the next `on_tick` would change something, if any.
    /// A timer can fire a millisecond early against `Date.now()`; if it was the
    /// only one armed, the caller re-arms from this instead of leaving the gate
    /// stuck in `Pending` or, worse, `Holding` forever.
    pub(crate) fn pending_deadline(&self) -> Option<f64> {
        match self.phase {
            GatePhase::Pending { since } => Some(since + SHOW_DEBOUNCE_MS),
            GatePhase::Holding { shown_at } => Some(shown_at + MIN_VISIBLE_MS),
            GatePhase::Idle | GatePhase::Shown { .. } => None,
        }
    }

    /// What to render: `Some` only in `Shown` / `Holding`.
    pub(crate) fn visible_reason(&self) -> Option<LoadingReason> {
        match self.phase {
            GatePhase::Shown { .. } | GatePhase::Holding { .. } => self.reason,
            GatePhase::Idle | GatePhase::Pending { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_REASONS: [LoadingReason; 7] = [
        LoadingReason::Connecting,
        LoadingReason::Reconnecting,
        LoadingReason::JoiningRoom,
        LoadingReason::LoadingRooms,
        LoadingReason::SyncingRooms,
        LoadingReason::Refreshing,
        LoadingReason::Sending,
    ];

    static CONNECTED: SynchronizerStatus = SynchronizerStatus::Connected;

    fn idle_connected() -> ActivityInputs<'static> {
        ActivityInputs {
            sync_status: &CONNECTED,
            sync_enabled: true,
            rooms_load_state: RoomsLoadState::Loaded,
            joining: false,
            rooms_syncing: 0,
            fetches_in_flight: 0,
            sends_in_flight: 0,
        }
    }

    /// Every input other than the connection status set to "busy".
    fn all_busy(sync_status: &SynchronizerStatus, sync_enabled: bool) -> ActivityInputs<'_> {
        ActivityInputs {
            sync_status,
            sync_enabled,
            rooms_load_state: RoomsLoadState::Loading,
            joining: true,
            rooms_syncing: 3,
            fetches_in_flight: 2,
            sends_in_flight: 1,
        }
    }

    #[test]
    fn idle_connected_is_none() {
        assert_eq!(loading_reason(&idle_connected()), None);
    }

    #[test]
    fn connecting_wins_over_everything() {
        let status = SynchronizerStatus::Connecting;
        assert_eq!(
            loading_reason(&all_busy(&status, true)),
            Some(LoadingReason::Connecting)
        );
    }

    /// A `no-sync` build is `Disconnected` for its whole life. Without the
    /// `sync_enabled` input the dots would run forever there, including in
    /// every Playwright run.
    #[test]
    fn disconnected_reconnects_only_when_sync_is_enabled() {
        let status = SynchronizerStatus::Disconnected;
        assert_eq!(
            loading_reason(&all_busy(&status, true)),
            Some(LoadingReason::Reconnecting)
        );
        assert_eq!(loading_reason(&all_busy(&status, false)), None);
    }

    /// `Error` can be terminal ("Please refresh the page"), and the red pill
    /// already explains it, so it never shows the dots.
    #[test]
    fn error_shows_nothing() {
        let status = SynchronizerStatus::Error("WebSocket connection failed".to_string());
        assert_eq!(loading_reason(&all_busy(&status, true)), None);
    }

    #[test]
    fn joining_beats_rooms_loading() {
        let i = ActivityInputs {
            joining: true,
            rooms_load_state: RoomsLoadState::Loading,
            ..idle_connected()
        };
        assert_eq!(loading_reason(&i), Some(LoadingReason::JoiningRoom));
    }

    #[test]
    fn rooms_loading_and_migrating_both_count() {
        for state in [RoomsLoadState::Loading, RoomsLoadState::Migrating] {
            let i = ActivityInputs {
                rooms_load_state: state,
                rooms_syncing: 1,
                ..idle_connected()
            };
            assert_eq!(
                loading_reason(&i),
                Some(LoadingReason::LoadingRooms),
                "{state:?}"
            );
        }
    }

    /// `LoadFailed` has its own Retry button, so it is not "loading".
    #[test]
    fn load_failed_and_loaded_do_not_count() {
        for state in [RoomsLoadState::LoadFailed, RoomsLoadState::Loaded] {
            let i = ActivityInputs {
                rooms_load_state: state,
                ..idle_connected()
            };
            assert_eq!(loading_reason(&i), None, "{state:?}");
        }
    }

    #[test]
    fn syncing_rooms_when_loaded() {
        let i = ActivityInputs {
            rooms_syncing: 1,
            fetches_in_flight: 1,
            sends_in_flight: 1,
            ..idle_connected()
        };
        assert_eq!(loading_reason(&i), Some(LoadingReason::SyncingRooms));
    }

    #[test]
    fn fetch_then_send_priority() {
        let both = ActivityInputs {
            fetches_in_flight: 1,
            sends_in_flight: 1,
            ..idle_connected()
        };
        assert_eq!(loading_reason(&both), Some(LoadingReason::Refreshing));
        let send_only = ActivityInputs {
            sends_in_flight: 1,
            ..idle_connected()
        };
        assert_eq!(loading_reason(&send_only), Some(LoadingReason::Sending));
    }

    /// Rows 4-8 are gated on `Connected`: the example build sits at
    /// `ROOMS_LOAD_STATE == Loading` forever, and nothing loads while the
    /// socket is down anyway.
    #[test]
    fn busy_inputs_are_ignored_unless_connected() {
        let status = SynchronizerStatus::Disconnected;
        let i = ActivityInputs {
            sync_enabled: false,
            ..all_busy(&status, false)
        };
        assert_eq!(loading_reason(&i), None);
    }

    #[test]
    fn as_attr_values_are_unique_and_kebab_case() {
        let mut seen = std::collections::HashSet::new();
        for r in ALL_REASONS {
            let a = r.as_attr();
            assert!(seen.insert(a), "duplicate data-reason {a:?}");
            assert!(
                !a.is_empty()
                    && !a.starts_with('-')
                    && !a.ends_with('-')
                    && a.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{a:?} is not kebab-case"
            );
            assert!(!r.label().is_empty(), "{r:?} has no label");
        }
    }

    // ---- DisplayGate ----------------------------------------------------

    const C: Option<LoadingReason> = Some(LoadingReason::Connecting);

    fn gate(phase: GatePhase) -> DisplayGate {
        DisplayGate { phase, reason: C }
    }

    #[test]
    fn a_load_shorter_than_the_debounce_never_shows() {
        let (g, wake) = DisplayGate::default().on_reason(C, 0.0);
        assert_eq!(g.phase, GatePhase::Pending { since: 0.0 });
        assert_eq!(wake, Some(200.0));
        assert_eq!(g.visible_reason(), None);

        let (g, wake) = g.on_reason(None, 150.0);
        assert_eq!(g.phase, GatePhase::Idle);
        assert_eq!(wake, None);
        assert_eq!(g.visible_reason(), None);

        let g = g.on_tick(200.0);
        assert_eq!(
            g.phase,
            GatePhase::Idle,
            "the stale debounce timer is a no-op"
        );
        assert_eq!(g.visible_reason(), None);
    }

    #[test]
    fn shows_once_the_debounce_elapses() {
        let (g, _) = DisplayGate::default().on_reason(C, 0.0);
        let g = g.on_tick(200.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 200.0 });
        assert_eq!(g.visible_reason(), C);
    }

    /// `shown_at` is when the tick actually fired, not `since + 200`, so the
    /// minimum is measured from when the user could first see the dots.
    #[test]
    fn a_late_tick_measures_the_minimum_from_when_it_fired() {
        let g = gate(GatePhase::Pending { since: 0.0 }).on_tick(900.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 900.0 });
    }

    #[test]
    fn an_early_tick_is_a_noop() {
        let g = gate(GatePhase::Pending { since: 0.0 });
        assert_eq!(g.on_tick(199.0), g);
    }

    /// An early tick is a no-op, so the caller needs to know when to try again.
    #[test]
    fn pending_deadline_tells_an_early_timer_when_to_retry() {
        assert_eq!(
            gate(GatePhase::Pending { since: 10.0 }).pending_deadline(),
            Some(210.0)
        );
        assert_eq!(
            gate(GatePhase::Holding { shown_at: 200.0 }).pending_deadline(),
            Some(1700.0)
        );
        assert_eq!(DisplayGate::default().pending_deadline(), None);
        assert_eq!(
            gate(GatePhase::Shown { shown_at: 200.0 }).pending_deadline(),
            None
        );
    }

    #[test]
    fn a_short_load_is_held_for_the_minimum() {
        let (g, wake) = gate(GatePhase::Shown { shown_at: 200.0 }).on_reason(None, 250.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 200.0 });
        assert_eq!(wake, Some(1450.0));

        let g = g.on_tick(1699.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 200.0 });
        let g = g.on_tick(1700.0);
        assert_eq!(g.phase, GatePhase::Idle);
        assert_eq!(g.visible_reason(), None);
    }

    #[test]
    fn the_hold_keeps_the_last_reason() {
        let (g, _) = gate(GatePhase::Shown { shown_at: 200.0 }).on_reason(None, 250.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 200.0 });
        assert_eq!(g.visible_reason(), C);
    }

    #[test]
    fn a_long_load_hides_as_soon_as_it_ends() {
        let (g, wake) = gate(GatePhase::Shown { shown_at: 200.0 }).on_reason(None, 3000.0);
        assert_eq!(g.phase, GatePhase::Idle);
        assert_eq!(wake, None);
        assert_eq!(g.visible_reason(), None);
    }

    /// The 1.5 s is a minimum on total visible time, not a fresh 1.5 s after
    /// every flicker.
    #[test]
    fn busy_again_during_the_hold_keeps_the_original_clock() {
        let g = gate(GatePhase::Holding { shown_at: 200.0 });
        let (g, wake) = g.on_reason(C, 900.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 200.0 });
        assert_eq!(wake, None);

        let (g, wake) = g.on_reason(None, 1000.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 200.0 });
        assert_eq!(wake, Some(700.0));

        assert_eq!(g.on_tick(1700.0).phase, GatePhase::Idle);
    }

    #[test]
    fn a_cancelled_pending_restarts_the_debounce() {
        let (g, _) = DisplayGate::default().on_reason(C, 0.0);
        let (g, _) = g.on_reason(None, 100.0);
        let (g, wake) = g.on_reason(C, 150.0);
        assert_eq!(g.phase, GatePhase::Pending { since: 150.0 });
        assert_eq!(wake, Some(200.0));

        let g = g.on_tick(200.0);
        assert_eq!(
            g.phase,
            GatePhase::Pending { since: 150.0 },
            "the first debounce timer is stale and must change nothing"
        );
        let g = g.on_tick(350.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 350.0 });
    }

    #[test]
    fn reason_updates_while_shown() {
        let (g, wake) = gate(GatePhase::Shown { shown_at: 200.0 })
            .on_reason(Some(LoadingReason::SyncingRooms), 400.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 200.0 });
        assert_eq!(wake, None);
        assert_eq!(g.visible_reason(), Some(LoadingReason::SyncingRooms));
    }

    #[test]
    fn reason_updates_while_pending_without_rearming() {
        let (g, _) = DisplayGate::default().on_reason(C, 0.0);
        let (g, wake) = g.on_reason(Some(LoadingReason::LoadingRooms), 50.0);
        assert_eq!(g.phase, GatePhase::Pending { since: 0.0 });
        assert_eq!(wake, None, "the original debounce timer still covers it");
        assert_eq!(
            g.on_tick(200.0).visible_reason(),
            Some(LoadingReason::LoadingRooms)
        );
    }

    #[test]
    fn ticks_in_idle_and_shown_are_noops() {
        let idle = DisplayGate::default();
        assert_eq!(idle.on_tick(10_000.0), idle);
        let shown = gate(GatePhase::Shown { shown_at: 200.0 });
        assert_eq!(shown.on_tick(10_000.0), shown);
    }

    #[test]
    fn idle_to_idle_arms_nothing() {
        let (g, wake) = DisplayGate::default().on_reason(None, 0.0);
        assert_eq!(g, DisplayGate::default());
        assert_eq!(wake, None);
    }

    /// Product requirements: changing either should be a conscious edit here too.
    #[test]
    fn timing_constants_are_as_specified() {
        assert_eq!(SHOW_DEBOUNCE_MS, 200.0);
        assert_eq!(MIN_VISIBLE_MS, 1500.0);
    }

    /// The minimum is "one full ripple". If either side changes, that
    /// rationale has to be revisited, not just one number.
    #[test]
    fn min_visible_matches_the_css_wave_period() {
        let css = include_str!("../../assets/main.css");
        assert!(
            css.contains("river-flow-wave 1.5s"),
            "main.css no longer runs river-flow-wave over 1.5s"
        );
        assert_eq!(MIN_VISIBLE_MS, 1500.0);
    }
}
