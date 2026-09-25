//! Handles pending room invitations and join requests
//!
//! This module manages the state of room invitations that are in the process
//! of being accepted or retrieved.

use ed25519_dalek::{SigningKey, VerifyingKey};
use river_core::room_state::member::AuthorizedMember;
use std::collections::HashMap;

/// Collection of pending room join requests
#[derive(Clone, Debug, Default)]
pub struct PendingInvites {
    /// Map of room owner keys to pending join information
    pub map: HashMap<VerifyingKey, PendingRoomJoin>, // TODO: Make this private and use methods to access
}

impl PendingInvites {
    /// Creates a new instance of `PendingInvites`
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
    /*
        /// Adds a new pending room join request
        pub fn add(&mut self, owner_key: VerifyingKey, join_info: PendingRoomJoin) {
            self.map.insert(owner_key, join_info);
        }

        /// Removes a pending room join request
        pub fn remove(&mut self, owner_key: &VerifyingKey) {
            self.map.remove(owner_key);
        }
    */
}

/// Information about a pending room join
#[derive(Clone)]
pub struct PendingRoomJoin {
    /// The authorized member data for the join
    pub authorized_member: AuthorizedMember,
    /// The signing key for the invited member
    pub invitee_signing_key: SigningKey,
    /// User's preferred nickname for this room
    pub preferred_nickname: String,
    /// Current status of the join request
    pub status: PendingRoomStatus,
    /// Timestamp (ms since epoch) when the status moved to `Subscribing`,
    /// used to detect stuck invitations that need retry.
    pub subscribing_since: Option<f64>,
    /// Number of times this invitation GET has been retried after timeout.
    /// On retry_count >= 1, falls back to requesting contract code from the
    /// network in case the local copy is stale.
    pub retry_count: u32,
    /// Room secrets carried in the invitation, copied here from
    /// `Invitation::room_secrets` at accept time so the pending-invite
    /// GET-response handler can seed them into `RoomData`. Empty for
    /// public rooms and pre-feature invitations.
    pub room_secrets: Vec<(u32, [u8; 32])>,
}

impl PendingRoomJoin {
    /// Must preserve the original bytes so URL and dismiss fingerprints match.
    pub fn invitation(&self, room: VerifyingKey) -> crate::components::members::Invitation {
        crate::components::members::Invitation {
            room,
            invitee_signing_key: self.invitee_signing_key.clone(),
            invitee: self.authorized_member.clone(),
            room_secrets: self.room_secrets.clone(),
        }
    }
}

/// Hand-written `Debug` that REDACTS `room_secrets` — the derived `Debug`
/// for `[u8; 32]` would print every room-secret byte if a `PendingRoomJoin`
/// were ever `{:?}`-logged. `SigningKey`'s own `Debug` is already
/// non-exhaustive, so it is safe to delegate to.
impl std::fmt::Debug for PendingRoomJoin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingRoomJoin")
            .field("authorized_member", &self.authorized_member)
            .field("invitee_signing_key", &self.invitee_signing_key)
            .field("preferred_nickname", &self.preferred_nickname)
            .field("status", &self.status)
            .field("subscribing_since", &self.subscribing_since)
            .field("retry_count", &self.retry_count)
            .field(
                "room_secrets",
                &format_args!("<{} room secret(s) redacted>", self.room_secrets.len()),
            )
            .finish()
    }
}

/// Status of a pending room join request
#[derive(Clone, Debug, PartialEq)]
pub enum PendingRoomStatus {
    /// Ready to subscribe to room data
    PendingSubscription,
    /// Subscription request sent, waiting for response
    Subscribing,
    /// Retained until retry to prevent automatic re-prompts; success removes the entry.
    Error(String),
}

impl PendingRoomStatus {
    pub fn is_in_progress(&self) -> bool {
        matches!(self, Self::PendingSubscription | Self::Subscribing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_join_rebuilds_the_invitation_it_came_from() {
        use river_core::room_state::member::Member;
        let inviter = SigningKey::from_bytes(&[1u8; 32]);
        let invitee_signing_key = SigningKey::from_bytes(&[2u8; 32]);
        let owner_vk = SigningKey::from_bytes(&[3u8; 32]).verifying_key();
        let member = Member {
            owner_member_id: owner_vk.into(),
            invited_by: inviter.verifying_key().into(),
            member_vk: invitee_signing_key.verifying_key(),
        };
        let accepted = crate::components::members::Invitation {
            room: owner_vk,
            invitee_signing_key,
            invitee: AuthorizedMember::new(member, &inviter),
            room_secrets: vec![(0, [0xA1; 32]), (3, [0xB2; 32])],
        };

        let join = PendingRoomJoin {
            authorized_member: accepted.invitee.clone(),
            invitee_signing_key: accepted.invitee_signing_key.clone(),
            preferred_nickname: "Alice".to_string(),
            status: PendingRoomStatus::PendingSubscription,
            subscribing_since: None,
            retry_count: 0,
            room_secrets: accepted.room_secrets.clone(),
        };
        assert_eq!(
            join.invitation(owner_vk).to_encoded_string(),
            accepted.to_encoded_string()
        );
    }

    #[test]
    fn is_in_progress_only_for_pending_and_subscribing() {
        assert!(PendingRoomStatus::PendingSubscription.is_in_progress());
        assert!(PendingRoomStatus::Subscribing.is_in_progress());
        assert!(!PendingRoomStatus::Error("x".to_string()).is_in_progress());
    }
}
