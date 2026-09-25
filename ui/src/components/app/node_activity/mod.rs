//! The composer indicator tracks user actions; the connection pill tracks
//! other outstanding requests. [`NodeApi::send`] records requests and
//! [`on_reply`] settles them. Room-changing handlers register with
//! [`await_room_update`]; awaited node calls hold a [`busy`] guard.

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

// Exceeds the node's 60 s per-attempt budget to catch lost or unmatched replies.
const REQUEST_BACKSTOP_MS: f64 = 90_000.0;

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

    pub fn user_reason(&self) -> Option<ActionKind> {
        self.actions.reason()
    }

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

// Defer signal writes: callers may hold WEB_API or lack a Dioxus scope.
// Capture time before deferring. FIFO scheduling records a sent request
// before its later reply settles it.

pub(crate) fn record_request(kind: RequestKind) {
    let now = now_ms();
    crate::util::defer(move || {
        NODE_ACTIVITY.with_mut(|a| a.record(kind, now));
    });
    arm_sweep(REQUEST_BACKSTOP_MS);
}

/// Call from the WebApi result closure before any early return.
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

/// Call before `mark_needs_sync` so the action exists when its UPDATE is recorded.
/// Key derivation must match `process_rooms`.
pub fn await_room_update(room_owner: VerifyingKey, kind: ActionKind) {
    let contract = *crate::util::owner_vk_to_contract_key(&room_owner).id();
    let id = next_action_id();
    let now = now_ms();
    crate::util::defer(move || {
        NODE_ACTIVITY.with_mut(|a| a.actions.await_update(id, contract, kind, now));
    });
    arm_sweep(ACTION_CAP_MS);
}

/// Hold across the awaited node call; dropping it ends the user action.
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

pub fn busy(kind: ActionKind) -> BusyGuard {
    let id = next_action_id();
    let now = now_ms();
    crate::util::defer(move || {
        NODE_ACTIVITY.with_mut(|a| a.actions.begin_scoped(id, kind, now));
    });
    arm_sweep(ACTION_CAP_MS);
    BusyGuard { id }
}

/// Track only explicit user actions, not background node calls.
pub async fn track<F: std::future::Future>(kind: ActionKind, fut: F) -> F::Output {
    let _busy = busy(kind);
    fut.await
}

// Expiry is idempotent, so timers need no cancellation.
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


    use crate::util::source_scan::{fn_body, production_only, strip_line_comments};

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
            // Holds the shared configuration save, which room_name_field.rs
            // also uses; see `configuration_edits_share_one_signed_save`.
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
            checked, 13,
            "expected exactly the 13 known user room changes; update this \
             deliberately if you added or removed one"
        );
    }

    // The room-change pin counts the shared helper once; also check its callers.
    #[test]
    fn configuration_edits_share_one_signed_save() {
        let edit_room = strip_line_comments(production_only(include_str!(
            "../../room_list/edit_room_modal.rs"
        )));
        let room_name = strip_line_comments(production_only(include_str!(
            "../../room_list/room_name_field.rs"
        )));
        let helper = "sign_and_apply_configuration(";
        let definition = format!("fn {helper}");

        let save = fn_body(&edit_room, &definition);
        let busy = save
            .find("node_activity::busy(")
            .expect("the signed save must hold a busy guard");
        let sign = save
            .find("sign_config_with_fallback(")
            .expect("the signed save no longer signs; move the pin");
        assert!(busy < sign, "the guard must cover the signature");
        let apply = fn_body(save, "crate::util::defer(move ||");
        assert!(
            apply.contains("let _busy = busy;"),
            "the guard must move into the deferred apply, or it ends before \
             the UPDATE wait takes over"
        );
        let gate = apply
            .find("if applied")
            .expect("the deferred apply must branch on whether the delta applied");
        assert!(
            !apply[..gate].contains("mark_needs_sync("),
            "a failed apply must not reach the sync"
        );
        let applied = fn_body(&apply[gate..], "if applied");
        let awaited = applied
            .find("node_activity::await_room_update(")
            .expect("a successful apply must await its UPDATE");
        let synced = applied
            .find("mark_needs_sync(")
            .expect("a successful apply must hand the change to the sync");
        assert!(
            awaited < synced,
            "register the wait before the UPDATE is sent"
        );

        for (name, src, component) in [
            ("room_name_field.rs", &room_name, "fn RoomNameField("),
            ("edit_room_modal.rs", &edit_room, "fn RoomDescriptionField("),
            ("edit_room_modal.rs", &edit_room, "fn NumericConfigField("),
            ("edit_room_modal.rs", &edit_room, "fn MaxMembersField("),
        ] {
            let body = fn_body(src, component);
            assert_eq!(
                body.matches(helper).count(),
                1,
                "{name} {component}: must save through {helper}"
            );
            assert!(
                !body.contains("sign_config_with_fallback(") && !body.contains("mark_needs_sync("),
                "{name} {component}: signs or syncs on its own again"
            );
        }
        assert_eq!(
            edit_room.matches(helper).count() - edit_room.matches(&definition).count(),
            3,
            "edit_room_modal.rs: expected exactly the description, numeric and \
             member-cap callers"
        );
    }

    // Subscription and PUT can wait on the network; only local key storage
    // belongs inside the primary-indicator guard.
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
            // Four reorder sites share one save helper.
            ("room_list.rs", include_str!("../../room_list.rs"), 1),
            ("members.rs", include_str!("../../members.rs"), 1),
            // Leaving the room, and the shared configuration save (which
            // also serves room_name_field.rs).
            (
                "edit_room_modal.rs",
                include_str!("../../room_list/edit_room_modal.rs"),
                2,
            ),
            (
                "ban_button.rs",
                include_str!("../../members/member_info_modal/ban_button.rs"),
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

        let room_list = strip_line_comments(production_only(include_str!("../../room_list.rs")));
        assert!(
            fn_body(&room_list, "fn spawn_room_order_save(").contains("node_activity::track("),
            "spawn_room_order_save must hold the Saving guard"
        );
        assert_eq!(
            room_list.matches("spawn_room_order_save();").count(),
            4,
            "room_list.rs: expected the 4 reorder sites to save through \
             spawn_room_order_save"
        );

        // chat_delegate.rs has a mid-file test module, so production_only would
        // exclude these helpers.
        let src = strip_line_comments(include_str!("../chat_delegate.rs"));
        assert!(fn_body(&src, "pub fn hide_dm_thread(").contains("node_activity::track("));
        let unhide = fn_body(&src, "fn unhide_dm_thread_saving(");
        let guard = unhide
            .find("let _busy = by_user.then(")
            .expect("the guard must be a named binding conditional on by_user");
        assert!(unhide.contains("node_activity::busy("));
        let save = unhide
            .find("save_outbound_dms_to_delegate().await")
            .expect("unhide_dm_thread_saving must await the save");
        assert!(
            guard < save,
            "the guard must exist before the save is awaited"
        );
        assert_eq!(
            unhide.matches(".await").count(),
            1,
            "the save is awaited once, not once per flavour"
        );
        assert!(fn_body(&src, "pub fn unhide_dm_thread(")
            .contains("unhide_dm_thread_saving(room_owner_vk, peer, true)"));
        assert!(fn_body(&src, "fn unhide_dm_thread_in_background(")
            .contains("unhide_dm_thread_saving(room_owner_vk, peer, false)"));
    }

    #[test]
    fn web_api_holds_the_recording_wrapper() {
        let app = strip_line_comments(production_only(include_str!("../../app.rs")));
        assert!(
            app.contains("GlobalSignal<Option<NodeApi>>"),
            "WEB_API must hold node_activity::NodeApi, or requests go unrecorded"
        );
    }

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
