//! Entry points for explicit user actions, so each one shows in the composer
//! indicator. Background sync and persistence call the untracked primitives
//! (`mark_needs_sync`, `save_rooms_to_delegate`) directly.

use crate::components::app::chat_delegate::save_rooms_to_delegate;
use crate::components::app::mark_needs_sync;
use crate::components::app::node_activity::{self, ActionKind};
use dioxus::logger::tracing::error;
use dioxus::prelude::*;
use ed25519_dalek::VerifyingKey;

/// Hand a local room change to the sync and wait for its UPDATE. The wait is
/// registered first so it exists when the UPDATE is recorded.
pub fn mark_user_change(room: VerifyingKey, kind: ActionKind) {
    node_activity::await_room_update(room, kind);
    mark_needs_sync(room);
}

/// Persist the rooms after a user's room mutation. Call after the mutation;
/// `context` names what failed to save.
pub fn spawn_user_rooms_save(context: &'static str) {
    spawn(async move {
        let saved = node_activity::track(ActionKind::Saving, save_rooms_to_delegate()).await;
        if let Err(e) = saved {
            error!("Failed to save {context}: {e}");
        }
    });
}
