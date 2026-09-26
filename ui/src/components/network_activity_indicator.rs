//! User-action waits appear in the chat or active modal; background work appears
//! in the connection pill. A single [`NetworkActivityIndicator`] owns both gates.

use crate::components::app::chat_delegate::{RoomsLoadState, ROOMS_LOAD_STATE};
use crate::components::app::freenet_api::freenet_synchronizer::SynchronizerStatus;
use crate::components::app::node_activity::{ActionKind, NODE_ACTIVITY};
use crate::components::app::sync_info::{now_ms, SYNC_INFO};
use crate::components::app::{PENDING_INVITES, ROOMS, SYNC_STATUS};
use crate::components::loading_dots::{small_dot_spans, wave_dot_spans};
use dioxus::prelude::*;

// Written synchronously by the owner's effect and inside `defer` by its timers.
static ACTIVITY_GATE: GlobalSignal<DisplayGate<LoadingReason>> =
    Global::new(|| DisplayGate::new(PRIMARY_TIMING));

// Same writers as ACTIVITY_GATE.
static BACKGROUND_GATE: GlobalSignal<DisplayGate<BackgroundReason>> =
    Global::new(|| DisplayGate::new(SECONDARY_TIMING));


/// `read()`, not `try_read()`: every write to the gate is a single `set` in a
/// clean context, so no borrow outlives a statement and a render cannot meet
/// one. A failed `try_read()` in a render, on the other hand, drops the
/// subscription with no nudge to restore it, leaving the dots stuck.
pub(crate) fn visible_activity() -> Option<LoadingReason> {
    ACTIVITY_GATE.read().visible_reason()
}

/// Same `read()` reasoning as [`visible_activity`].
pub(crate) fn background_activity() -> Option<BackgroundReason> {
    BACKGROUND_GATE.read().visible_reason()
}

/// Keep mounted once in `App` for the gates and screen-reader live region.
#[component]
pub fn NetworkActivityIndicator() -> Element {
    // Fall back to idle on contention: a brief blank is preferable to false activity.
    let reason = use_memo(move || {
        crate::util::signal_guard::anchor();
        let Ok(status) = SYNC_STATUS.try_read() else {
            crate::util::signal_guard::schedule_nudge();
            return None;
        };
        let status = status.clone();
        let joining = match PENDING_INVITES.try_read() {
            Ok(invites) => invites.map.values().any(|j| j.status.is_in_progress()),
            Err(_) => {
                crate::util::signal_guard::schedule_nudge();
                false
            }
        };
        let user_action = match NODE_ACTIVITY.try_read() {
            Ok(activity) => activity.user_reason(),
            Err(_) => {
                crate::util::signal_guard::schedule_nudge();
                None
            }
        };
        loading_reason(&ActivityInputs {
            sync_status: &status,
            joining,
            user_action,
        })
    });

    let background = use_memo(move || {
        crate::util::signal_guard::anchor();
        let Ok(status) = SYNC_STATUS.try_read() else {
            crate::util::signal_guard::schedule_nudge();
            return None;
        };
        let status = status.clone();
        let rooms_load_state = match ROOMS_LOAD_STATE.try_read() {
            Ok(state) => *state,
            Err(_) => {
                crate::util::signal_guard::schedule_nudge();
                RoomsLoadState::Loaded
            }
        };

        let rooms_syncing = 'syncing: {
            let Ok(rooms) = ROOMS.try_read() else {
                crate::util::signal_guard::schedule_nudge();
                break 'syncing false;
            };
            let Ok(sync_info) = SYNC_INFO.try_read() else {
                crate::util::signal_guard::schedule_nudge();
                break 'syncing false;
            };
            sync_info.has_syncing_rooms(|k| rooms.map.contains_key(k))
        };
        let requests = match NODE_ACTIVITY.try_read() {
            Ok(activity) => activity.background_requests(),
            Err(_) => {
                crate::util::signal_guard::schedule_nudge();
                false
            }
        };
        background_reason(&BackgroundInputs {
            sync_status: &status,
            sync_enabled: !cfg!(feature = "no-sync"),
            rooms_load_state,
            rooms_syncing,
            requests,
        })
    });


    use_effect(move || {
        crate::util::signal_guard::anchor();
        let Ok(r) = reason.try_read().map(|r| *r) else {
            crate::util::signal_guard::schedule_nudge();
            return;
        };
        drive_gate(&ACTIVITY_GATE, r);
    });
    use_effect(move || {
        crate::util::signal_guard::anchor();
        let Ok(b) = background.try_read().map(|b| *b) else {
            crate::util::signal_guard::schedule_nudge();
            return;
        };
        drive_gate(&BACKGROUND_GATE, b);
    });

    // Gate announcements with the dots, not the raw activity.
    let label = visible_activity().map(LoadingReason::label).unwrap_or("");

    rsx! {
        // Keep the live region mounted so screen readers announce content changes.
        span {
            class: "sr-only",
            role: "status",
            "aria-live": "polite",
            "data-testid": "network-activity-status",
            "{label}"
        }
    }
}

// Use peek() so the calling effect does not subscribe to its own writes.
fn drive_gate<R: Copy + PartialEq + 'static>(
    gate: &'static GlobalSignal<DisplayGate<R>>,
    reason: Option<R>,
) {
    let current = *gate.peek();
    let (next, wake) = current.on_reason(reason, now_ms());
    if next != current {
        *gate.write() = next;
    }
    if let Some(delay_ms) = wake {
        schedule_gate_tick(gate, delay_ms);
    }
}

// Suppress duplicate chat dots behind a modal. Written only inside `defer`.
static DOTS_IN_MODAL: GlobalSignal<u32> = Global::new(|| 0);

/// Render at most one copy; `docked` requires a positioned message-history ancestor.
#[component]
pub fn NetworkActivityDots(docked: bool) -> Element {
    // Same `read()` reasoning as `visible_activity`.
    if *DOTS_IN_MODAL.read() > 0 {
        return rsx! {};
    }
    activity_dots(docked)
}


#[component]
pub fn ModalActivityDots() -> Element {
    use_hook(|| {
        crate::util::defer(|| *DOTS_IN_MODAL.write() += 1);
    });
    use_drop(|| {
        crate::util::defer(|| {
            let mut count = DOTS_IN_MODAL.write();
            *count = count.saturating_sub(1);
        });
    });
    activity_dots(false)
}

fn activity_dots(docked: bool) -> Element {
    // Same `read()` reasoning as `visible_activity`.
    let gate = *ACTIVITY_GATE.read();
    let Some(reason) = gate.visible_reason() else {
        return rsx! {};
    };
    // A stable seed prevents reason changes and holds from restarting the animation.
    let seed = gate.shown_at().map(f64::to_bits).unwrap_or(0);
    let placement = if docked {
        "river-flow river-flow--docked"
    } else {
        "river-flow"
    };
    rsx! {
        div {
            class: placement,
            "aria-hidden": "true",
            "data-testid": "network-activity-indicator",
            "data-reason": reason.as_attr(),
            {wave_dot_spans(seed, "network-activity-dot")}
        }
    }
}

/// Keep mounted for fade-out. Use spans: connection-status-indicator.spec.ts
/// identifies the pill's first div as its status dot.
#[component]
pub fn PillActivityDots() -> Element {
    let active = background_activity().is_some();
    rsx! {
        span {
            class: "pill-activity",
            "aria-hidden": "true",
            "data-testid": "connection-activity-dots",
            "data-active": if active { "true" } else { "false" },
            {small_dot_spans()}
        }
    }
}

// Stale ticks are harmless, so timers need no cancellation.
fn schedule_gate_tick<R: Copy + PartialEq + 'static>(
    gate: &'static GlobalSignal<DisplayGate<R>>,
    delay_ms: f64,
) {
    crate::util::safe_spawn_local(async move {
        crate::util::sleep(std::time::Duration::from_millis(
            delay_ms.max(0.0).ceil() as u64
        ))
        .await;
        crate::util::defer(move || {
            let current = *gate.peek();
            let now = now_ms();
            let next = current.on_tick(now);
            if next != current {
                *gate.write() = next;
            } else if let Some(deadline) = next.pending_deadline() {
                // Fired early against `Date.now()`. If this was the only timer
                // armed, doing nothing would strand the gate, so try again.
                if deadline > now {
                    schedule_gate_tick(gate, deadline - now);
                }
            }
        });
    });
}


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LoadingReason {
    JoiningRoom,
    CreatingRoom,
    Sending,
    Saving,
}

impl LoadingReason {
    /// Value of the indicator's `data-reason` attribute.
    fn as_attr(self) -> &'static str {
        match self {
            LoadingReason::JoiningRoom => "joining-room",
            LoadingReason::CreatingRoom => "creating-room",
            LoadingReason::Sending => "sending",
            LoadingReason::Saving => "saving",
        }
    }

    /// Screen-reader label for the live region.
    fn label(self) -> &'static str {
        match self {
            LoadingReason::JoiningRoom => "Joining room…",
            LoadingReason::CreatingRoom => "Creating room…",
            LoadingReason::Sending => "Sending…",
            LoadingReason::Saving => "Saving…",
        }
    }
}


struct ActivityInputs<'a> {
    sync_status: &'a SynchronizerStatus,
    /// Any pending invite still `PendingSubscription` or `Subscribing`: the
    /// user accepted it and the room's data hasn't arrived yet.
    joining: bool,

    user_action: Option<ActionKind>,
}


fn loading_reason(i: &ActivityInputs) -> Option<LoadingReason> {
    // The pill handles lost connections; no-sync builds stay Disconnected.
    if !matches!(i.sync_status, SynchronizerStatus::Connected) {
        return None;
    }
    if i.joining {
        return Some(LoadingReason::JoiningRoom);
    }
    i.user_action.map(|kind| match kind {
        ActionKind::CreatingRoom => LoadingReason::CreatingRoom,
        ActionKind::Sending => LoadingReason::Sending,
        ActionKind::Saving => LoadingReason::Saving,
    })
}


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BackgroundReason {
    Connecting,
    Reconnecting,
    LoadingRooms,
    SyncingRooms,
    /// A request to the node that nobody is waiting on is outstanding.
    Requests,
}

impl BackgroundReason {
    /// Value of the pill's `data-busy-reason` attribute.
    pub(crate) fn as_attr(self) -> &'static str {
        match self {
            BackgroundReason::Connecting => "connecting",
            BackgroundReason::Reconnecting => "reconnecting",
            BackgroundReason::LoadingRooms => "loading-rooms",
            BackgroundReason::SyncingRooms => "syncing-rooms",
            BackgroundReason::Requests => "requests",
        }
    }

    // No ellipsis: the dots already convey ongoing activity.
    pub(crate) fn label(self) -> &'static str {
        match self {
            BackgroundReason::Connecting => "Connecting to Freenet",
            BackgroundReason::Reconnecting => "Reconnecting to Freenet",
            BackgroundReason::LoadingRooms => "Loading your rooms",
            BackgroundReason::SyncingRooms => "Syncing rooms with the network",
            BackgroundReason::Requests => "Waiting on the network",
        }
    }
}


struct BackgroundInputs<'a> {
    sync_status: &'a SynchronizerStatus,
    /// False in a `no-sync` build, where `SYNC_STATUS` sits at `Disconnected`
    /// for good and must not read as "reconnecting".
    sync_enabled: bool,
    rooms_load_state: RoomsLoadState,

    rooms_syncing: bool,

    requests: bool,
}

// Keep the match exhaustive so new statuses cannot silently become idle.
fn background_reason(i: &BackgroundInputs) -> Option<BackgroundReason> {
    match i.sync_status {
        SynchronizerStatus::Connecting => Some(BackgroundReason::Connecting),
        // In a sync build the only writer of `Disconnected` is the
        // `ConnectionLost` handler, which always arms a reconnect first.
        SynchronizerStatus::Disconnected if i.sync_enabled => Some(BackgroundReason::Reconnecting),
        SynchronizerStatus::Disconnected => None,
        // Errors may be terminal; the red pill explains them until a new attempt.
        SynchronizerStatus::Error(_) => None,
        SynchronizerStatus::Connected => {
            if matches!(
                i.rooms_load_state,
                RoomsLoadState::Loading | RoomsLoadState::Migrating
            ) {
                Some(BackgroundReason::LoadingRooms)
            } else if i.rooms_syncing {
                Some(BackgroundReason::SyncingRooms)
            } else if i.requests {
                Some(BackgroundReason::Requests)
            } else {
                None
            }
        }
    }
}


#[derive(Clone, Copy, PartialEq, Debug)]
struct GateTiming {
    show_after_ms: f64,
    min_visible_ms: f64,
}

// Debounce quick replies; hold visible dots to avoid flicker.
const PRIMARY_TIMING: GateTiming = GateTiming {
    show_after_ms: 500.0,
    min_visible_ms: 1_000.0,
};

// Even a quick background refresh should be visible.
const SECONDARY_TIMING: GateTiming = GateTiming {
    show_after_ms: 0.0,
    min_visible_ms: 1_000.0,
};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
enum GatePhase {
    #[default]
    Idle,

    Pending { since: f64 },

    Shown { shown_at: f64 },
    /// No longer busy, held on screen until `shown_at + min_visible_ms`.
    Holding { shown_at: f64 },
}

// Timing lives here because CSS cannot delay removal from the DOM.
#[derive(Clone, Copy, PartialEq, Debug)]
struct DisplayGate<R> {
    phase: GatePhase,
    // Retained during Holding so the label does not blank out.
    reason: Option<R>,
    timing: GateTiming,
}

impl<R: Copy + PartialEq> DisplayGate<R> {
    const fn new(timing: GateTiming) -> Self {
        Self {
            phase: GatePhase::Idle,
            reason: None,
            timing,
        }
    }

    fn idle(self) -> Self {
        Self::new(self.timing)
    }

    /// Returns the new gate and an optional delay in ms before `on_tick` must run.
    fn on_reason(self, reason: Option<R>, now: f64) -> (Self, Option<f64>) {
        let kept = reason.or(self.reason);
        let with = |phase| Self {
            phase,
            reason: kept,
            timing: self.timing,
        };
        match (self.phase, reason) {

            (GatePhase::Idle, Some(_)) if self.timing.show_after_ms <= 0.0 => {
                (with(GatePhase::Shown { shown_at: now }), None)
            }
            (GatePhase::Idle, Some(_)) => (
                with(GatePhase::Pending { since: now }),
                Some(self.timing.show_after_ms),
            ),
            (GatePhase::Idle, None) => (self.idle(), None),
            // The debounce timer armed on entry still covers it.
            (GatePhase::Pending { since }, Some(_)) => (with(GatePhase::Pending { since }), None),

            (GatePhase::Pending { .. }, None) => (self.idle(), None),
            (GatePhase::Shown { shown_at }, Some(_)) => (with(GatePhase::Shown { shown_at }), None),
            (GatePhase::Shown { shown_at }, None) => {
                let until = shown_at + self.timing.min_visible_ms;
                if now >= until {
                    (self.idle(), None)
                } else {
                    (with(GatePhase::Holding { shown_at }), Some(until - now))
                }
            }
            // The minimum applies to total visible time, not each burst of activity.
            (GatePhase::Holding { shown_at }, Some(_)) => {
                (with(GatePhase::Shown { shown_at }), None)
            }
            (GatePhase::Holding { .. }, None) => (self, None),
        }
    }


    fn on_tick(self, now: f64) -> Self {
        match self.phase {
            // `now`, not `since + show_after_ms`: the minimum counts from when
            // the dots could first be seen, even if the timer ran late.
            GatePhase::Pending { since } if now >= since + self.timing.show_after_ms => Self {
                phase: GatePhase::Shown { shown_at: now },
                ..self
            },
            GatePhase::Holding { shown_at } if now >= shown_at + self.timing.min_visible_ms => {
                self.idle()
            }
            GatePhase::Idle
            | GatePhase::Pending { .. }
            | GatePhase::Shown { .. }
            | GatePhase::Holding { .. } => self,
        }
    }


    fn pending_deadline(&self) -> Option<f64> {
        match self.phase {
            GatePhase::Pending { since } => Some(since + self.timing.show_after_ms),
            GatePhase::Holding { shown_at } => Some(shown_at + self.timing.min_visible_ms),
            GatePhase::Idle | GatePhase::Shown { .. } => None,
        }
    }


    fn visible_reason(&self) -> Option<R> {
        match self.phase {
            GatePhase::Shown { .. } | GatePhase::Holding { .. } => self.reason,
            GatePhase::Idle | GatePhase::Pending { .. } => None,
        }
    }


    fn shown_at(&self) -> Option<f64> {
        match self.phase {
            GatePhase::Shown { shown_at } | GatePhase::Holding { shown_at } => Some(shown_at),
            GatePhase::Idle | GatePhase::Pending { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_REASONS: [LoadingReason; 4] = [
        LoadingReason::JoiningRoom,
        LoadingReason::CreatingRoom,
        LoadingReason::Sending,
        LoadingReason::Saving,
    ];

    static CONNECTED: SynchronizerStatus = SynchronizerStatus::Connected;

    fn idle_connected() -> ActivityInputs<'static> {
        ActivityInputs {
            sync_status: &CONNECTED,
            joining: false,
            user_action: None,
        }
    }

    #[test]
    fn idle_connected_is_none() {
        assert_eq!(loading_reason(&idle_connected()), None);
    }

    #[test]
    fn each_user_action_has_its_reason() {
        for (kind, reason) in [
            (ActionKind::CreatingRoom, LoadingReason::CreatingRoom),
            (ActionKind::Sending, LoadingReason::Sending),
            (ActionKind::Saving, LoadingReason::Saving),
        ] {
            let i = ActivityInputs {
                user_action: Some(kind),
                ..idle_connected()
            };
            assert_eq!(loading_reason(&i), Some(reason), "{kind:?}");
        }
    }

    #[test]
    fn joining_beats_other_actions() {
        let i = ActivityInputs {
            joining: true,
            user_action: Some(ActionKind::Sending),
            ..idle_connected()
        };
        assert_eq!(loading_reason(&i), Some(LoadingReason::JoiningRoom));
    }


    #[test]
    fn the_primary_needs_a_live_socket() {
        for status in [
            SynchronizerStatus::Connecting,
            SynchronizerStatus::Disconnected,
            SynchronizerStatus::Error("WebSocket connection failed".to_string()),
        ] {
            let i = ActivityInputs {
                sync_status: &status,
                joining: true,
                user_action: Some(ActionKind::Sending),
            };
            assert_eq!(loading_reason(&i), None, "{status:?}");
        }
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


    fn quiet(sync_status: &SynchronizerStatus, sync_enabled: bool) -> BackgroundInputs<'_> {
        BackgroundInputs {
            sync_status,
            sync_enabled,
            rooms_load_state: RoomsLoadState::Loaded,
            rooms_syncing: false,
            requests: false,
        }
    }

    #[test]
    fn a_quiet_connected_client_is_not_busy() {
        assert_eq!(background_reason(&quiet(&CONNECTED, true)), None);
    }

    #[test]
    fn connecting_and_reconnecting_are_background_work() {
        assert_eq!(
            background_reason(&quiet(&SynchronizerStatus::Connecting, true)),
            Some(BackgroundReason::Connecting)
        );
        assert_eq!(
            background_reason(&quiet(&SynchronizerStatus::Disconnected, true)),
            Some(BackgroundReason::Reconnecting)
        );
        assert_eq!(
            background_reason(&quiet(&SynchronizerStatus::Disconnected, false)),
            None,
            "a no-sync build is Disconnected for good"
        );
        let error = SynchronizerStatus::Error("x".to_string());
        assert_eq!(
            background_reason(&quiet(&error, true)),
            None,
            "the red pill explains it"
        );
    }

    #[test]
    fn each_background_input_counts_while_connected() {
        for state in [RoomsLoadState::Loading, RoomsLoadState::Migrating] {
            let i = BackgroundInputs {
                rooms_load_state: state,
                rooms_syncing: true,
                requests: true,
                ..quiet(&CONNECTED, true)
            };
            assert_eq!(
                background_reason(&i),
                Some(BackgroundReason::LoadingRooms),
                "{state:?}"
            );
        }
        for state in [RoomsLoadState::LoadFailed, RoomsLoadState::Loaded] {
            let i = BackgroundInputs {
                rooms_load_state: state,
                ..quiet(&CONNECTED, true)
            };
            assert_eq!(background_reason(&i), None, "{state:?}");
        }
        let syncing = BackgroundInputs {
            rooms_syncing: true,
            requests: true,
            ..quiet(&CONNECTED, true)
        };
        assert_eq!(
            background_reason(&syncing),
            Some(BackgroundReason::SyncingRooms)
        );
        let requests = BackgroundInputs {
            requests: true,
            ..quiet(&CONNECTED, true)
        };
        assert_eq!(
            background_reason(&requests),
            Some(BackgroundReason::Requests)
        );
    }

    #[test]
    fn background_reasons_have_distinct_kebab_case_attrs_and_labels() {
        let all = [
            BackgroundReason::Connecting,
            BackgroundReason::Reconnecting,
            BackgroundReason::LoadingRooms,
            BackgroundReason::SyncingRooms,
            BackgroundReason::Requests,
        ];
        let mut seen = std::collections::HashSet::new();
        for r in all {
            let a = r.as_attr();
            assert!(seen.insert(a), "duplicate data-busy-reason {a:?}");
            assert!(
                a.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{a:?}"
            );
            assert!(!r.label().is_empty(), "{r:?} has no label");
            assert!(
                !r.label().ends_with('…') && !r.label().ends_with("..."),
                "{r:?}: the pill's dots replace the ellipsis"
            );
        }
    }


    const C: Option<LoadingReason> = Some(LoadingReason::Sending);

    fn primary() -> DisplayGate<LoadingReason> {
        DisplayGate::new(PRIMARY_TIMING)
    }

    fn gate(phase: GatePhase) -> DisplayGate<LoadingReason> {
        DisplayGate {
            phase,
            reason: C,
            timing: PRIMARY_TIMING,
        }
    }

    #[test]
    fn a_load_shorter_than_the_debounce_never_shows() {
        let (g, wake) = primary().on_reason(C, 0.0);
        assert_eq!(g.phase, GatePhase::Pending { since: 0.0 });
        assert_eq!(wake, Some(500.0));
        assert_eq!(g.visible_reason(), None);

        let (g, wake) = g.on_reason(None, 400.0);
        assert_eq!(g.phase, GatePhase::Idle);
        assert_eq!(wake, None);
        assert_eq!(g.visible_reason(), None);

        let g = g.on_tick(500.0);
        assert_eq!(
            g.phase,
            GatePhase::Idle,
            "the stale debounce timer is a no-op"
        );
        assert_eq!(g.visible_reason(), None);
    }

    #[test]
    fn shows_once_the_debounce_elapses() {
        let (g, _) = primary().on_reason(C, 0.0);
        let g = g.on_tick(500.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 500.0 });
        assert_eq!(g.visible_reason(), C);
    }


    #[test]
    fn a_late_tick_measures_the_minimum_from_when_it_fired() {
        let g = gate(GatePhase::Pending { since: 0.0 }).on_tick(1200.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 1200.0 });
    }

    #[test]
    fn an_early_tick_is_a_noop() {
        let g = gate(GatePhase::Pending { since: 0.0 });
        assert_eq!(g.on_tick(499.0), g);
    }


    #[test]
    fn pending_deadline_tells_an_early_timer_when_to_retry() {
        assert_eq!(
            gate(GatePhase::Pending { since: 10.0 }).pending_deadline(),
            Some(510.0)
        );
        assert_eq!(
            gate(GatePhase::Holding { shown_at: 500.0 }).pending_deadline(),
            Some(1500.0)
        );
        assert_eq!(primary().pending_deadline(), None);
        assert_eq!(
            gate(GatePhase::Shown { shown_at: 500.0 }).pending_deadline(),
            None
        );
    }

    #[test]
    fn a_short_load_is_held_for_the_minimum() {
        let (g, wake) = gate(GatePhase::Shown { shown_at: 500.0 }).on_reason(None, 550.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 500.0 });
        assert_eq!(wake, Some(950.0));

        let g = g.on_tick(1499.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 500.0 });
        let g = g.on_tick(1500.0);
        assert_eq!(g.phase, GatePhase::Idle);
        assert_eq!(g.visible_reason(), None);
    }

    #[test]
    fn the_hold_keeps_the_last_reason() {
        let (g, _) = gate(GatePhase::Shown { shown_at: 500.0 }).on_reason(None, 550.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 500.0 });
        assert_eq!(g.visible_reason(), C);
    }

    #[test]
    fn a_long_load_hides_as_soon_as_it_ends() {
        let (g, wake) = gate(GatePhase::Shown { shown_at: 500.0 }).on_reason(None, 3000.0);
        assert_eq!(g.phase, GatePhase::Idle);
        assert_eq!(wake, None);
        assert_eq!(g.visible_reason(), None);
    }


    #[test]
    fn busy_again_during_the_hold_keeps_the_original_clock() {
        let g = gate(GatePhase::Holding { shown_at: 500.0 });
        let (g, wake) = g.on_reason(C, 900.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 500.0 });
        assert_eq!(wake, None);

        let (g, wake) = g.on_reason(None, 1000.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 500.0 });
        assert_eq!(wake, Some(500.0));

        assert_eq!(g.on_tick(1500.0).phase, GatePhase::Idle);
    }

    #[test]
    fn a_cancelled_pending_restarts_the_debounce() {
        let (g, _) = primary().on_reason(C, 0.0);
        let (g, _) = g.on_reason(None, 100.0);
        let (g, wake) = g.on_reason(C, 150.0);
        assert_eq!(g.phase, GatePhase::Pending { since: 150.0 });
        assert_eq!(wake, Some(500.0));

        let g = g.on_tick(500.0);
        assert_eq!(
            g.phase,
            GatePhase::Pending { since: 150.0 },
            "the first debounce timer is stale and must change nothing"
        );
        let g = g.on_tick(650.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 650.0 });
    }

    #[test]
    fn reason_updates_while_shown() {
        let (g, wake) = gate(GatePhase::Shown { shown_at: 500.0 })
            .on_reason(Some(LoadingReason::Saving), 700.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 500.0 });
        assert_eq!(wake, None);
        assert_eq!(g.visible_reason(), Some(LoadingReason::Saving));
    }

    #[test]
    fn reason_updates_while_pending_without_rearming() {
        let (g, _) = primary().on_reason(C, 0.0);
        let (g, wake) = g.on_reason(Some(LoadingReason::JoiningRoom), 50.0);
        assert_eq!(g.phase, GatePhase::Pending { since: 0.0 });
        assert_eq!(wake, None, "the original debounce timer still covers it");
        assert_eq!(
            g.on_tick(500.0).visible_reason(),
            Some(LoadingReason::JoiningRoom)
        );
    }

    #[test]
    fn ticks_in_idle_and_shown_are_noops() {
        let idle = primary();
        assert_eq!(idle.on_tick(10_000.0), idle);
        let shown = gate(GatePhase::Shown { shown_at: 500.0 });
        assert_eq!(shown.on_tick(10_000.0), shown);
    }

    #[test]
    fn idle_to_idle_arms_nothing() {
        let (g, wake) = primary().on_reason(None, 0.0);
        assert_eq!(g, primary());
        assert_eq!(wake, None);
    }

    #[test]
    fn shown_at_is_set_only_while_on_screen_and_survives_the_hold() {
        assert_eq!(primary().shown_at(), None);
        assert_eq!(gate(GatePhase::Pending { since: 0.0 }).shown_at(), None);
        let shown = gate(GatePhase::Shown { shown_at: 500.0 });
        assert_eq!(shown.shown_at(), Some(500.0));
        let (held, _) = shown.on_reason(None, 600.0);
        assert_eq!(held.shown_at(), Some(500.0));
        let (back, _) = held.on_reason(C, 700.0);
        assert_eq!(
            back.shown_at(),
            Some(500.0),
            "the seed must not change mid-show"
        );
    }


    #[test]
    fn the_secondary_gate_shows_at_once_and_holds_a_second() {
        let busy = Some(BackgroundReason::Requests);
        let g = DisplayGate::<BackgroundReason>::new(SECONDARY_TIMING);
        let (g, wake) = g.on_reason(busy, 100.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 100.0 });
        assert_eq!(wake, None, "nothing to wait for");
        assert_eq!(g.visible_reason(), busy);

        let (g, wake) = g.on_reason(None, 150.0);
        assert_eq!(g.phase, GatePhase::Holding { shown_at: 100.0 });
        assert_eq!(wake, Some(950.0));
        assert_eq!(g.on_tick(1_099.0).visible_reason(), busy);
        assert_eq!(g.on_tick(1_100.0).visible_reason(), None);
    }
}
