//! What River is waiting on from its node, split by who asked.
//!
//! Two indicators read this:
//!
//! - **Primary** (the dots above the composer): only while the USER is waiting
//!   on the node's reply to something they did. See [`user_reason`].
//! - **Secondary** (the dots in the connection pill): while any other request
//!   is outstanding. See [`background_requests`].
//!
//! Every request is recorded by [`NodeApi::send`] (the only way to reach the
//! node) and settled by [`on_reply`], called from the one closure every reply
//! arrives in. User actions ride on top: a handler that changes a room calls
//! [`await_room_update`], and a handler that awaits a node call holds a
//! [`busy`] guard.
//!
//! See docs/plans/loading-indicators.md.

mod actions;
mod ledger;
mod node_api;

pub use actions::ActionKind;
pub(crate) use ledger::RequestKind;
pub use node_api::NodeApi;

use crate::components::app::sync_info::now_ms;
use actions::{ActionId, Actions};
use dioxus::logger::tracing::warn;
use dioxus::prelude::*;
use ed25519_dalek::VerifyingKey;
use freenet_stdlib::client_api::{ClientError, HostResponse};
use ledger::{Ledger, Settle, SlotId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Longest a request may wait for its reply before it is dropped. Above the
/// node's own 60 s per-attempt budget, so it only fires for a reply that was
/// lost or that the ledger failed to match — and those are logged.
const REQUEST_BACKSTOP_MS: f64 = 90_000.0;

/// Longest a user action keeps the primary indicator on, whatever happens.
/// Matches the liveness watchdog's timeout.
const ACTION_CAP_MS: f64 =
    crate::components::app::freenet_api::constants::LIVENESS_PROBE_TIMEOUT_MS as f64;

/// A timer can fire a millisecond early against `Date.now()`; without this a
/// sweep would find its entry a hair too young and keep it.
const SWEEP_SLACK_MS: u64 = 50;

/// Outstanding requests and pending user actions, kept together so a request
/// and the actions it carries always change in one write.
#[derive(Default, Debug)]
pub struct NodeActivity {
    ledger: Ledger,
    actions: Actions,
}

impl NodeActivity {
    fn record(&mut self, kind: RequestKind, now: f64) {
        let update_for = match &kind {
            RequestKind::Update(contract) => Some(*contract),
            _ => None,
        };
        let slot = self.ledger.record(kind, now);
        if let Some(contract) = update_for {
            self.actions.attach_unsent(contract, slot);
        }
    }

    fn settle(&mut self, settle: &Settle) {
        if let Some(slot) = self.ledger.settle(settle) {
            self.actions.slots_settled(&[slot]);
        }
    }

    fn reset(&mut self) {
        let dropped = self.ledger.clear();
        self.actions.slots_settled(&dropped);
    }

    fn expire(&mut self, now: f64) {
        let stale = self.ledger.expire(now, REQUEST_BACKSTOP_MS);
        for (_, kind) in &stale {
            warn!("stale node request: no reply matched {kind:?} within the backstop");
        }
        let ids: Vec<SlotId> = stale.into_iter().map(|(id, _)| id).collect();
        self.actions.slots_settled(&ids);
        self.actions.expire(now, ACTION_CAP_MS);
    }

    /// What the user is waiting on, if anything.
    pub fn user_reason(&self) -> Option<ActionKind> {
        self.actions.reason()
    }

    /// Whether any outstanding request carries no user action.
    pub fn background_requests(&self) -> bool {
        if self.ledger.is_empty() {
            return false;
        }
        let attached = self.actions.attached_slots();
        self.ledger.ids().any(|id| !attached.contains(&id))
    }
}

pub static NODE_ACTIVITY: GlobalSignal<NodeActivity> = Global::new(NodeActivity::default);

/// Ids for actions, allocated outside the signal so a [`busy`] guard can be
/// created without touching it.
static NEXT_ACTION: AtomicU64 = AtomicU64::new(0);

fn next_action_id() -> ActionId {
    NEXT_ACTION.fetch_add(1, Ordering::Relaxed)
}

// Every wrapper below reads the clock at call time and writes the signal
// inside `crate::util::defer`: callers run in the synchronizer's polled future
// (holding `WEB_API`), in the WebApi reply callback, or in event handlers
// (`.claude/rules/dioxus-signal-safety.md`). `setTimeout(0)` is FIFO, so a
// request is always recorded before its reply settles it: the reply can only
// arrive after `send` returned.

/// A request went out. Called only by [`NodeApi::send`] (and test hooks).
pub(crate) fn record_request(kind: RequestKind) {
    let now = now_ms();
    crate::util::defer(move || {
        NODE_ACTIVITY.with_mut(|a| a.record(kind, now));
    });
    arm_sweep(REQUEST_BACKSTOP_MS);
}

/// A reply or error arrived from the node. Called from the WebApi result
/// closure in `connection_manager.rs` (wasm-only), before anything else looks
/// at it.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn on_reply(result: &Result<HostResponse, ClientError>) {
    let Some(settle) = ledger::settle_for(result) else {
        return;
    };
    crate::util::defer(move || {
        // Skip the write when nothing matches: every write re-runs the
        // indicators' memos, and unmatched replies are common.
        let matches = NODE_ACTIVITY.peek().ledger.would_settle(&settle);
        if matches {
            NODE_ACTIVITY.with_mut(|a| a.settle(&settle));
        }
    });
}

/// The socket died or was replaced: nothing sent on it will be answered.
pub fn connection_reset() {
    crate::util::defer(|| {
        NODE_ACTIVITY.with_mut(|a| a.reset());
    });
}

/// The user changed the room owned by `room_owner`; the change reaches the
/// node in that room's next UPDATE. Call BEFORE `mark_needs_sync`, so the
/// action is known before the sync that sends it can record its UPDATE.
///
/// The contract is derived exactly as `process_rooms` derives the UPDATE's
/// key (`owner_vk_to_contract_key`), so the two always match.
pub fn await_room_update(room_owner: VerifyingKey, kind: ActionKind) {
    let contract = *crate::util::owner_vk_to_contract_key(&room_owner).id();
    let id = next_action_id();
    let now = now_ms();
    crate::util::defer(move || {
        NODE_ACTIVITY.with_mut(|a| a.actions.await_update(id, contract, kind, now));
    });
    arm_sweep(ACTION_CAP_MS);
}

/// Keeps a user action pending while it is alive. Hold it across an awaited
/// node call the user is waiting on.
#[must_use = "the action ends when the guard drops"]
pub struct BusyGuard {
    id: ActionId,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        let id = self.id;
        crate::util::defer(move || {
            NODE_ACTIVITY.with_mut(|a| a.actions.end(id));
        });
    }
}

/// Start a user action that lasts as long as the returned guard.
pub fn busy(kind: ActionKind) -> BusyGuard {
    let id = next_action_id();
    let now = now_ms();
    crate::util::defer(move || {
        NODE_ACTIVITY.with_mut(|a| a.actions.begin_scoped(id, kind, now));
    });
    arm_sweep(ACTION_CAP_MS);
    BusyGuard { id }
}

/// Run `fut` as a user action: for spawned node calls the user is waiting on
/// (a delegate save after an explicit preference change, say).
pub async fn track<F: std::future::Future>(kind: ActionKind, fut: F) -> F::Output {
    let _busy = busy(kind);
    fut.await
}

/// Sweep once `horizon_ms` has passed. Idempotent, so timers never need
/// cancelling.
fn arm_sweep(horizon_ms: f64) {
    crate::util::safe_spawn_local(async move {
        crate::util::sleep(Duration::from_millis(horizon_ms as u64 + SWEEP_SLACK_MS)).await;
        crate::util::defer(|| {
            NODE_ACTIVITY.with_mut(|a| a.expire(now_ms()));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use freenet_stdlib::prelude::ContractInstanceId;

    fn c(n: u8) -> ContractInstanceId {
        ContractInstanceId::new([n; 32])
    }

    #[test]
    fn an_update_carries_the_rooms_waiting_changes_and_its_reply_ends_them() {
        let mut a = NodeActivity::default();
        a.actions.await_update(1, c(1), ActionKind::Sending, 0.0);
        assert_eq!(a.user_reason(), Some(ActionKind::Sending));

        a.record(RequestKind::Update(c(1)), 1.0);
        assert!(
            !a.background_requests(),
            "the UPDATE carries a user action, so it is not background work"
        );

        a.settle(&Settle::Exact(RequestKind::Update(c(1))));
        assert_eq!(a.user_reason(), None);
        assert!(!a.background_requests());
    }

    #[test]
    fn requests_nobody_is_waiting_on_are_background() {
        let mut a = NodeActivity::default();
        a.record(RequestKind::Get(c(1)), 0.0);
        assert!(a.background_requests());
        assert_eq!(a.user_reason(), None);
        a.settle(&Settle::Exact(RequestKind::Get(c(1))));
        assert!(!a.background_requests());
    }

    /// Other rooms' requests stay background even while the user waits.
    #[test]
    fn user_and_background_work_are_counted_separately() {
        let mut a = NodeActivity::default();
        a.actions.await_update(1, c(1), ActionKind::Sending, 0.0);
        a.record(RequestKind::Update(c(1)), 0.0);
        a.record(RequestKind::Get(c(2)), 0.0);
        assert_eq!(a.user_reason(), Some(ActionKind::Sending));
        assert!(a.background_requests());
    }

    #[test]
    fn a_lost_connection_ends_what_was_in_flight() {
        let mut a = NodeActivity::default();
        a.actions.await_update(1, c(1), ActionKind::Sending, 0.0);
        a.record(RequestKind::Update(c(1)), 0.0);
        a.record(RequestKind::Get(c(2)), 0.0);
        a.reset();
        assert_eq!(a.user_reason(), None);
        assert!(!a.background_requests());
    }

    /// A change whose room hasn't sent yet survives a reset: it goes out on
    /// the new socket.
    #[test]
    fn an_unsent_change_survives_a_reset() {
        let mut a = NodeActivity::default();
        a.actions.await_update(1, c(1), ActionKind::Sending, 0.0);
        a.reset();
        assert_eq!(a.user_reason(), Some(ActionKind::Sending));
    }

    #[test]
    fn the_backstop_ends_stale_requests_and_their_actions() {
        let mut a = NodeActivity::default();
        a.actions
            .await_update(1, c(1), ActionKind::Sending, 80_000.0);
        a.record(RequestKind::Update(c(1)), 0.0);
        a.expire(REQUEST_BACKSTOP_MS);
        assert!(!a.background_requests());
        assert_eq!(
            a.user_reason(),
            None,
            "the action's request was dropped, so the action ends"
        );
    }

    #[test]
    fn the_cap_ends_actions_whose_room_never_sent() {
        let mut a = NodeActivity::default();
        a.actions.await_update(1, c(1), ActionKind::Sending, 0.0);
        a.expire(ACTION_CAP_MS);
        assert_eq!(a.user_reason(), None);
    }

    // --- Source-scrape pins ------------------------------------------------

    use crate::util::source_scan::{fn_body, production_only, strip_line_comments};

    /// Every reply goes through `on_reply` before any early return in the
    /// WebApi result closure; a skipped reply would leave its request, and
    /// any user action it carries, pending until the backstop.
    #[test]
    fn every_reply_is_matched_before_any_early_return() {
        let src = strip_line_comments(production_only(include_str!(
            "../freenet_api/connection_manager.rs"
        )));
        let closure = fn_body(&src, "move |result: Result<HostResponse, ClientError>|");
        let settle = closure
            .find("node_activity::on_reply(")
            .expect("the WebApi result closure must call node_activity::on_reply");
        if let Some(ret) = closure.find("return") {
            assert!(
                settle < ret,
                "on_reply must run before the first early return"
            );
        }
    }

    /// In the user-facing handlers, every `mark_needs_sync` (the hand-off of a
    /// user's room change to the sync) is preceded, since the previous one, by
    /// an `await_room_update` for it. Without it the user's change would
    /// travel to the node with no primary indicator. Registering first is what
    /// guarantees the action is known before its UPDATE is recorded.
    #[test]
    fn every_user_room_change_awaits_its_update() {
        let files = [
            ("conversation.rs", include_str!("../../conversation.rs")),
            (
                "nickname_field.rs",
                include_str!("../../members/member_info_modal/nickname_field.rs"),
            ),
            (
                "deputy_button.rs",
                include_str!("../../members/member_info_modal/deputy_button.rs"),
            ),
            (
                "ban_button.rs",
                include_str!("../../members/member_info_modal/ban_button.rs"),
            ),
            (
                "room_name_field.rs",
                include_str!("../../room_list/room_name_field.rs"),
            ),
            (
                "edit_room_modal.rs",
                include_str!("../../room_list/edit_room_modal.rs"),
            ),
            (
                "receive_invitation_modal.rs",
                include_str!("../../room_list/receive_invitation_modal.rs"),
            ),
            (
                "dm_thread_modal.rs",
                include_str!("../../direct_messages/dm_thread_modal.rs"),
            ),
            (
                "direct_messages.rs",
                include_str!("../../direct_messages.rs"),
            ),
        ];
        let mut checked = 0;
        for (name, src) in files {
            let src = strip_line_comments(production_only(src));
            let marks: Vec<usize> = src
                .match_indices("mark_needs_sync(")
                .map(|(i, _)| i)
                .collect();
            assert!(
                !marks.is_empty(),
                "{name}: no mark_needs_sync found; is the pin stale?"
            );
            let mut previous = 0;
            for mark in marks {
                let window = &src[previous..mark];
                assert!(
                    window.contains("node_activity::await_room_update("),
                    "{name}: a mark_needs_sync at byte {mark} has no \
                     node_activity::await_room_update before it"
                );
                previous = mark + 1;
                checked += 1;
            }
        }
        assert_eq!(
            checked, 16,
            "expected exactly the 16 known user room changes; update this \
             deliberately if you added or removed one"
        );
    }

    /// Creating a room keeps the primary dots only for the node-local step
    /// (storing the room's signing key). `EnsureRoomSubscription` can park on
    /// the network and the room's PUT travels through Freenet, so neither may
    /// sit inside the guard.
    #[test]
    fn creating_a_room_waits_only_on_the_node_local_step() {
        let src = strip_line_comments(production_only(include_str!(
            "../../room_list/create_room_modal.rs"
        )));
        let track = src
            .find("node_activity::track(")
            .expect("create_room must track its node-local step");
        let store = src
            .find("store_signing_key(room_key_bytes")
            .expect("create_room no longer stores the signing key; move the pin");
        let ensure = src
            .find("ensure_room_subscription_once(")
            .expect("create_room no longer ensures the subscription; move the pin");
        assert!(track < store && store < ensure);
        let tracked = &src[track..ensure];
        assert!(tracked.contains("ActionKind::CreatingRoom"));
        assert!(
            tracked.contains(".await"),
            "the tracked store must finish before EnsureRoomSubscription is sent"
        );
        assert_eq!(
            src.matches("node_activity::").count(),
            2,
            "only the key store is a user wait here (one track, one ActionKind)"
        );
    }

    /// Each explicit user action that awaits a node call holds a guard. A
    /// dropped guard would leave that action with no primary indicator.
    #[test]
    fn scoped_user_actions_keep_their_guard() {
        let files = [
            (
                "invite_member_modal.rs",
                include_str!("../../members/invite_member_modal.rs"),
                1,
            ),
            (
                "invite_via_dm_picker_modal.rs",
                include_str!("../../direct_messages/invite_via_dm_picker_modal.rs"),
                1,
            ),
            (
                "notification_modal.rs",
                include_str!("../../room_list/notification_modal.rs"),
                1,
            ),
            ("room_list.rs", include_str!("../../room_list.rs"), 4),
            ("members.rs", include_str!("../../members.rs"), 1),
            (
                "edit_room_modal.rs",
                include_str!("../../room_list/edit_room_modal.rs"),
                4,
            ),
            (
                "ban_button.rs",
                include_str!("../../members/member_info_modal/ban_button.rs"),
                1,
            ),
            (
                "room_name_field.rs",
                include_str!("../../room_list/room_name_field.rs"),
                1,
            ),
        ];
        for (name, src, expected) in files {
            let src = strip_line_comments(production_only(src));
            let guards = src.matches("node_activity::track(").count()
                + src.matches("node_activity::busy(").count();
            assert_eq!(
                guards, expected,
                "{name}: expected {expected} scoped user action(s); update this \
                 deliberately if you added or removed one"
            );
        }

        // chat_delegate.rs has a test module mid-file, so scope to the two
        // helpers instead: archiving, and the user flavour of un-archiving
        // (the inbound-sync flavour must stay background).
        let src = strip_line_comments(include_str!("../chat_delegate.rs"));
        assert!(fn_body(&src, "pub fn hide_dm_thread(").contains("node_activity::track("));
        let unhide = fn_body(&src, "fn unhide_dm_thread_saving(");
        assert!(unhide.contains("if by_user"));
        assert!(unhide.contains("node_activity::track("));
        assert!(fn_body(&src, "pub fn unhide_dm_thread(")
            .contains("unhide_dm_thread_saving(room_owner_vk, peer, true)"));
        assert!(fn_body(&src, "fn unhide_dm_thread_in_background(")
            .contains("unhide_dm_thread_saving(room_owner_vk, peer, false)"));
    }

    /// The only way to talk to the node records the request.
    #[test]
    fn web_api_holds_the_recording_wrapper() {
        let app = strip_line_comments(production_only(include_str!("../../app.rs")));
        assert!(
            app.contains("GlobalSignal<Option<NodeApi>>"),
            "WEB_API must hold node_activity::NodeApi, or requests go unrecorded"
        );
    }

    /// A reply to anything sent on a dead or replaced socket never comes.
    #[test]
    fn socket_changes_reset_node_activity() {
        let src = strip_line_comments(production_only(include_str!(
            "../freenet_api/freenet_synchronizer.rs"
        )));
        let lost = fn_body(&src, "SynchronizerMessage::ConnectionLost =>");
        assert!(
            lost.contains("node_activity::connection_reset()"),
            "the ConnectionLost arm must call node_activity::connection_reset()"
        );
        let connect = fn_body(&src, "SynchronizerMessage::Connect =>");
        let connected = fn_body(connect, "Ok(()) =>");
        assert!(
            connected.contains("node_activity::connection_reset()"),
            "the Connect -> Ok(()) arm must call node_activity::connection_reset()"
        );
    }
}
