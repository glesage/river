//! "River is loading" indicator: ten accent-blue dots riding one travelling
//! sine wave, shown inside the chat section. With a room open they dock at the
//! bottom of the message history, just above the composer, and the history
//! gains bottom padding so they never cover the last message; with no room
//! open they sit in the no-room screen. Never `position: fixed`: they belong
//! to the chat, not to the window.
//!
//! Two pure pieces decide what it shows, so both are unit-testable natively:
//!
//! - [`loading_reason`] says whether River is busy **right now**, and why.
//! - [`DisplayGate`] decides whether the dots are **on screen**: a 500 ms
//!   debounce before they appear and a 1 s minimum once they have.
//!
//! [`NetworkActivityIndicator`], mounted once in `App`, owns the gate and the
//! screen-reader live region. The places that draw the dots read the gate
//! through [`visible_activity`].
//!
//! See docs/plans/2026-09-24-network-activity-indicator.md.

use crate::components::app::chat_delegate::{RoomsLoadState, ROOMS_LOAD_STATE};
use crate::components::app::freenet_api::freenet_synchronizer::SynchronizerStatus;
use crate::components::app::network_activity::{ActivityKind, IN_FLIGHT};
use crate::components::app::sync_info::{now_ms, SYNC_INFO};
use crate::components::app::{PENDING_INVITES, ROOMS, SYNC_STATUS};
use dioxus::prelude::*;

const DOT_COUNT: usize = 10;

/// Whether the dots are on screen, and why. Written only by
/// [`NetworkActivityIndicator`]'s effect (synchronously, a single `set`) and by
/// its timers (inside `crate::util::defer`).
static ACTIVITY_GATE: GlobalSignal<DisplayGate> = Global::new(DisplayGate::default);

/// The debounced, held reason to draw the dots for, or `None`. Subscribes the
/// calling component, so it re-renders when the dots come and go.
///
/// `read()`, not `try_read()`: every write to the gate is a single `set` in a
/// clean context, so no borrow outlives a statement and a render cannot meet
/// one. A failed `try_read()` in a render, on the other hand, drops the
/// subscription with no nudge to restore it, leaving the dots stuck.
pub(crate) fn visible_activity() -> Option<LoadingReason> {
    ACTIVITY_GATE.read().visible_reason()
}

/// Owns the loading decision and the show/hide timing, and renders the
/// screen-reader live region. Mounted once in `App` and never unmounted. The
/// dots themselves are drawn by [`NetworkActivityDots`] inside the chat
/// section.
#[component]
pub fn NetworkActivityIndicator() -> Element {
    // Reads signals only, never a captured value: this component is always
    // mounted, so a memo over plain input would go stale for the session
    // (freenet/river#291).
    //
    // On a contended read each input falls back to its IDLE value. The
    // indicator is decorative: a blank lasting one macrotask (which the nudge
    // corrects) is better than a false "busy" that lingers.
    let reason = use_memo(move || {
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
        let joining = match PENDING_INVITES.try_read() {
            Ok(invites) => invites.map.values().any(|j| j.status.is_in_progress()),
            Err(_) => {
                crate::util::signal_guard::schedule_nudge();
                false
            }
        };
        // Both guards live only inside this block, so none is still held if a
        // later edit adds a write below.
        let rooms_syncing = 'syncing: {
            let Ok(rooms) = ROOMS.try_read() else {
                crate::util::signal_guard::schedule_nudge();
                break 'syncing 0;
            };
            let Ok(sync_info) = SYNC_INFO.try_read() else {
                crate::util::signal_guard::schedule_nudge();
                break 'syncing 0;
            };
            sync_info.rooms_syncing_count(|k| rooms.map.contains_key(k))
        };
        let (fetches_in_flight, sends_in_flight) = match IN_FLIGHT.try_read() {
            Ok(f) => (f.count(ActivityKind::Fetch), f.count(ActivityKind::Send)),
            Err(_) => {
                crate::util::signal_guard::schedule_nudge();
                (0, 0)
            }
        };
        loading_reason(&ActivityInputs {
            sync_status: &status,
            sync_enabled: !cfg!(feature = "no-sync"),
            rooms_load_state,
            joining,
            rooms_syncing,
            fetches_in_flight,
            sends_in_flight,
        })
    });

    // Debounce + minimum-visible timing, driven from the memo.
    use_effect(move || {
        crate::util::signal_guard::anchor();
        let Ok(r) = reason.try_read().map(|r| *r) else {
            crate::util::signal_guard::schedule_nudge();
            return;
        };
        // peek(): this effect must not subscribe to the signal it writes, or it
        // re-runs itself. Written synchronously for the same reason: the
        // "never defer in use_effect" rule is about signals the effect reads.
        let current = *ACTIVITY_GATE.peek();
        let (next, wake) = current.on_reason(r, now_ms());
        if next != current {
            *ACTIVITY_GATE.write() = next;
        }
        if let Some(delay_ms) = wake {
            schedule_gate_tick(delay_ms);
        }
    });

    // The GATED reason, not the raw memo: the label obeys the same 500 ms
    // debounce and 1 s minimum as the dots.
    let label = visible_activity().map(LoadingReason::label).unwrap_or("");

    rsx! {
        // Always mounted: a live region only announces changes to content that
        // was already in the DOM, so it must not come and go with the dots.
        span {
            class: "sr-only",
            role: "status",
            "aria-live": "polite",
            "data-testid": "network-activity-status",
            "{label}"
        }
    }
}

/// The dots, drawn only while [`visible_activity`] says so. `docked` pins them
/// to the bottom-centre of the nearest positioned ancestor (the message
/// history, directly above the composer); otherwise they sit in the flow.
/// Render at most one at a time: tests and assistive tech address them by
/// `data-testid`.
#[component]
pub fn NetworkActivityDots(docked: bool) -> Element {
    // Same `read()` reasoning as `visible_activity`.
    let gate = *ACTIVITY_GATE.read();
    let Some(reason) = gate.visible_reason() else {
        return rsx! {};
    };
    // A fresh wave each time the dots appear, and the SAME wave for as long as
    // they stay: re-renders while shown (a reason change, the hold) must not
    // restart the animation, and `shown_at` survives both.
    let motions = dot_motions(gate.shown_at().map(f64::to_bits).unwrap_or(0));
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
            for (i, motion) in motions.iter().enumerate() {
                span {
                    key: "{i}",
                    class: "river-flow-dot",
                    "data-testid": "network-activity-dot",
                    style: motion.style(i),
                }
            }
        }
    }
}

/// Re-evaluate the gate after `delay_ms`. No cancellation is needed: `on_tick`
/// is idempotent, so a stale timer is a no-op.
fn schedule_gate_tick(delay_ms: f64) {
    crate::util::safe_spawn_local(async move {
        crate::util::sleep(std::time::Duration::from_millis(
            delay_ms.max(0.0).ceil() as u64
        ))
        .await;
        crate::util::defer(move || {
            let current = *ACTIVITY_GATE.peek();
            let now = now_ms();
            let next = current.on_tick(now);
            if next != current {
                *ACTIVITY_GATE.write() = next;
            } else if let Some(deadline) = next.pending_deadline() {
                // Fired early against `Date.now()`. If this was the only timer
                // armed, doing nothing would strand the gate, so try again.
                if deadline > now {
                    schedule_gate_tick(deadline - now);
                }
            }
        });
    });
}

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
    /// `InFlight::count(Fetch)`: tracked GETs awaiting their response.
    pub fetches_in_flight: usize,
    /// `InFlight::count(Send)`: tracked UPDATEs awaiting their response.
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
/// Long enough that the wake refresh's fast round trips on a tab switch stay
/// hidden.
pub(crate) const SHOW_DEBOUNCE_MS: f64 = 500.0;
/// Once shown, the dots stay at least this long. Shorter than the CSS wave
/// period (`river-flow-wave 1.5s`), so a short load shows part of one ripple.
pub(crate) const MIN_VISIBLE_MS: f64 = 1_000.0;

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

    /// When the dots came on screen, while they are on screen. Stable through
    /// `Holding` and back to `Shown`, since the hold never resets the clock.
    pub(crate) fn shown_at(&self) -> Option<f64> {
        match self.phase {
            GatePhase::Shown { shown_at } | GatePhase::Holding { shown_at } => Some(shown_at),
            GatePhase::Idle | GatePhase::Pending { .. } => None,
        }
    }
}

// ---- Wave motion ----------------------------------------------------------
//
// Every dot rides the same 1.5 s travelling wave (`river-flow-wave` in
// main.css), so the crest still reads as one ripple crossing the row. What
// varies per dot, so it moves like water rather than a metronome: how high it
// rises, a nudge to its place on the wave, which smooth curve it eases along,
// and a slower second swell on its own period. The periods do not divide each other, so the row never exactly
// repeats. Every curve is a monotone cubic-bezier: no steps, no linear, no
// overshoot.

/// Seconds between neighbouring dots on the travelling wave. Must match the
/// `0.15s` in main.css's `animation-delay` for `.river-flow-dot`.
const WAVE_STEP_S: f64 = 0.15;
/// Primary rise, px either side of rest.
const AMP_PX: (f64, f64) = (2.0, 3.0);
/// Largest nudge to a dot's place on the wave. Kept under half of
/// `WAVE_STEP_S` so the crest still travels strictly left to right.
const PHASE_JITTER_S: f64 = 0.045;
/// Secondary swell, px either side, and its period.
const SWELL_PX: (f64, f64) = (0.6, 1.4);
const SWELL_PERIOD_S: (f64, f64) = (2.2, 3.4);
/// Height of the `.river-flow` row in main.css. The motion must fit inside it,
/// because the 12 px clearances above and below are measured from its edges.
const ROW_HEIGHT_PX: f64 = 14.0;
/// Half a 5 px dot at the crest's `scale: 1`.
const DOT_RADIUS_PX: f64 = 2.5;

/// Smooth S-curves the dots ease along. Control points keep y at 0 and 1, so
/// each is monotone with no overshoot: every dot always moves on a curve.
const WAVE_EASES: [&str; 6] = [
    "cubic-bezier(0.37, 0, 0.63, 1)",
    "cubic-bezier(0.45, 0, 0.55, 1)",
    "cubic-bezier(0.65, 0, 0.35, 1)",
    "cubic-bezier(0.42, 0, 0.58, 1)",
    "cubic-bezier(0.3, 0, 0.5, 1)",
    "cubic-bezier(0.5, 0, 0.7, 1)",
];

/// One dot's share of the wave, emitted as custom properties that main.css's
/// keyframes read.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct DotMotion {
    amp_px: f64,
    phase_jitter_s: f64,
    ease: &'static str,
    swell_px: f64,
    swell_period_s: f64,
    /// Negative, so the swell is already under way on the first frame.
    swell_delay_s: f64,
}

impl DotMotion {
    fn style(&self, i: usize) -> String {
        format!(
            "--i: {i}; --amp: {:.2}px; --jitter: {:.3}s; --ease: {}; --swell: {:.2}px; \
             --swell-dur: {:.2}s; --swell-delay: {:.2}s;",
            self.amp_px,
            self.phase_jitter_s,
            self.ease,
            self.swell_px,
            self.swell_period_s,
            self.swell_delay_s,
        )
    }
}

/// The wave for one appearance of the dots. Deterministic in `seed`, so the
/// same appearance renders the same wave on every re-render.
pub(crate) fn dot_motions(seed: u64) -> [DotMotion; DOT_COUNT] {
    let mut rng = SplitMix64(seed);
    std::array::from_fn(|_| {
        let swell_period_s = rng.range(SWELL_PERIOD_S);
        DotMotion {
            amp_px: rng.range(AMP_PX),
            phase_jitter_s: rng.range((-PHASE_JITTER_S, PHASE_JITTER_S)),
            ease: WAVE_EASES[(rng.next_u64() % WAVE_EASES.len() as u64) as usize],
            swell_px: rng.range(SWELL_PX),
            swell_period_s,
            swell_delay_s: -rng.range((0.0, swell_period_s)),
        }
    })
}

/// splitmix64: tiny, fast, and plenty for choosing how dots wobble.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[lo, hi)`.
    fn range(&mut self, (lo, hi): (f64, f64)) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * unit
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
        let (g, _) = DisplayGate::default().on_reason(C, 0.0);
        let g = g.on_tick(500.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 500.0 });
        assert_eq!(g.visible_reason(), C);
    }

    /// `shown_at` is when the tick actually fired, not `since + 500`, so the
    /// minimum is measured from when the user could first see the dots.
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

    /// An early tick is a no-op, so the caller needs to know when to try again.
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
        assert_eq!(DisplayGate::default().pending_deadline(), None);
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

    /// The 1 s is a minimum on total visible time, not a fresh 1 s after
    /// every flicker.
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
        let (g, _) = DisplayGate::default().on_reason(C, 0.0);
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
            .on_reason(Some(LoadingReason::SyncingRooms), 700.0);
        assert_eq!(g.phase, GatePhase::Shown { shown_at: 500.0 });
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
            g.on_tick(500.0).visible_reason(),
            Some(LoadingReason::LoadingRooms)
        );
    }

    #[test]
    fn ticks_in_idle_and_shown_are_noops() {
        let idle = DisplayGate::default();
        assert_eq!(idle.on_tick(10_000.0), idle);
        let shown = gate(GatePhase::Shown { shown_at: 500.0 });
        assert_eq!(shown.on_tick(10_000.0), shown);
    }

    #[test]
    fn idle_to_idle_arms_nothing() {
        let (g, wake) = DisplayGate::default().on_reason(None, 0.0);
        assert_eq!(g, DisplayGate::default());
        assert_eq!(wake, None);
    }

    #[test]
    fn shown_at_is_set_only_while_on_screen_and_survives_the_hold() {
        assert_eq!(DisplayGate::default().shown_at(), None);
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

    // ---- Wave motion ----------------------------------------------------

    /// Seeds as the component derives them, plus edge values.
    fn seeds() -> impl Iterator<Item = u64> {
        (0..400u64)
            .map(|n| (1_758_700_000_000.0 + n as f64 * 7_919.0).to_bits())
            .chain([0, 1, u64::MAX])
    }

    #[test]
    fn a_seed_always_gives_the_same_wave() {
        for seed in seeds() {
            assert_eq!(dot_motions(seed), dot_motions(seed));
        }
    }

    #[test]
    fn different_appearances_get_different_waves() {
        let a = dot_motions(1_758_700_000_000.0f64.to_bits());
        let b = dot_motions(1_758_700_000_250.0f64.to_bits());
        assert_ne!(a, b);
    }

    /// The randomness must not break the ripple: each dot's place on the wave
    /// stays strictly after its left neighbour's.
    #[test]
    fn the_crest_still_travels_left_to_right() {
        for seed in seeds() {
            let m = dot_motions(seed);
            let place = |i: usize| (i as f64 - 10.0) * WAVE_STEP_S + m[i].phase_jitter_s;
            for i in 1..DOT_COUNT {
                assert!(
                    place(i) > place(i - 1),
                    "seed {seed}: dot {i} starts before dot {}",
                    i - 1
                );
            }
        }
    }

    #[test]
    fn every_value_stays_in_range() {
        let within = |v: f64, (lo, hi): (f64, f64)| v >= lo && v < hi;
        for seed in seeds() {
            for d in dot_motions(seed) {
                assert!(within(d.amp_px, AMP_PX), "{d:?}");
                assert!(d.phase_jitter_s.abs() <= PHASE_JITTER_S, "{d:?}");
                assert!(within(d.swell_px, SWELL_PX), "{d:?}");
                assert!(within(d.swell_period_s, SWELL_PERIOD_S), "{d:?}");
                assert!(
                    d.swell_delay_s <= 0.0 && d.swell_delay_s > -d.swell_period_s,
                    "{d:?}"
                );
                assert!(WAVE_EASES.contains(&d.ease), "{d:?}");
            }
        }
    }

    /// The row's edges are what the 12 px clearances are measured from, so no
    /// dot may leave it.
    #[test]
    fn the_motion_fits_inside_the_row() {
        let reach = AMP_PX.1 + SWELL_PX.1 + DOT_RADIUS_PX;
        assert!(
            reach <= ROW_HEIGHT_PX / 2.0,
            "a dot can reach {reach}px from centre, outside a {ROW_HEIGHT_PX}px row"
        );
        let css = include_str!("../../assets/main.css");
        let row = &css[css.find(".river-flow {").expect(".river-flow rule")..];
        let row = &row[..row.find('}').unwrap()];
        assert!(
            row.contains(&format!("height: {ROW_HEIGHT_PX}px")),
            ".river-flow's height in main.css must equal ROW_HEIGHT_PX"
        );
    }

    #[test]
    fn wave_step_matches_the_css() {
        let css = include_str!("../../assets/main.css");
        assert!(css.contains(&format!("(var(--i) - 10) * {WAVE_STEP_S}s")));
    }

    /// "They should remain curves": monotone cubic-beziers only, no steps,
    /// no linear, no overshoot.
    #[test]
    fn every_ease_is_a_smooth_monotone_curve() {
        for ease in WAVE_EASES {
            let inner = ease
                .strip_prefix("cubic-bezier(")
                .and_then(|r| r.strip_suffix(')'))
                .unwrap_or_else(|| panic!("{ease} is not a cubic-bezier"));
            let p: Vec<f64> = inner
                .split(',')
                .map(|v| v.trim().parse().unwrap())
                .collect();
            assert_eq!(p.len(), 4, "{ease}");
            assert!(
                (0.0..=1.0).contains(&p[0]) && (0.0..=1.0).contains(&p[2]),
                "{ease}"
            );
            assert_eq!((p[1], p[3]), (0.0, 1.0), "{ease} can overshoot");
            assert!(p[0] > 0.0 && p[2] < 1.0, "{ease} is linear at an end");
        }
    }

    #[test]
    fn style_carries_every_custom_property() {
        let style = dot_motions(42)[3].style(3);
        for prop in [
            "--i: 3;",
            "--amp:",
            "--jitter:",
            "--ease: cubic-bezier(",
            "--swell:",
            "--swell-dur:",
            "--swell-delay: -",
        ] {
            assert!(style.contains(prop), "{prop} missing from {style}");
        }
    }

    /// Product requirements: changing either should be a conscious edit here too.
    #[test]
    fn timing_constants_are_as_specified() {
        assert_eq!(SHOW_DEBOUNCE_MS, 500.0);
        assert_eq!(MIN_VISIBLE_MS, 1000.0);
    }
}
