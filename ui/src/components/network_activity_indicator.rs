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
}
