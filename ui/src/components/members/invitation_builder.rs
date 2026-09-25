//! Creates a fresh, signed invitation for a room snapshot. Shared by the
//! invite-link modal and the invite-via-DM picker; callers own the key
//! preflight, the activity tracking, and the transport.
//!
//! Not for reconstructing an existing credential: `PendingRoomJoin::invitation`
//! rebuilds one byte-for-byte and must never mint a new identity.

use crate::components::members::{collect_invitation_secrets, Invitation};
use crate::room_data::RoomData;
use ed25519_dalek::{Signature, SigningKey};
use river_core::room_state::member::{AuthorizedMember, Member};

/// A new invitee identity and the `Member` record the inviter signs.
struct UnsignedInvitation {
    invitee_signing_key: SigningKey,
    member: Member,
    member_bytes: Vec<u8>,
}

/// `inviter` must be the local key from `room`. A serialization failure
/// returns the encoder's message; callers word the user-facing error.
pub(crate) async fn create_invitation(
    room: &RoomData,
    inviter: &SigningKey,
) -> Result<Invitation, String> {
    let UnsignedInvitation {
        invitee_signing_key,
        member,
        member_bytes,
    } = prepare(room, inviter)?;
    let signature =
        crate::signing::sign_member_with_fallback(room.room_key(), member_bytes, inviter).await;
    Ok(assemble(room, invitee_signing_key, member, signature))
}

fn prepare(room: &RoomData, inviter: &SigningKey) -> Result<UnsignedInvitation, String> {
    let invitee_signing_key = SigningKey::generate(&mut rand::thread_rng());
    let member = Member {
        owner_member_id: room.owner_vk.into(),
        invited_by: inviter.verifying_key().into(),
        member_vk: invitee_signing_key.verifying_key(),
    };
    let mut member_bytes = Vec::new();
    ciborium::ser::into_writer(&member, &mut member_bytes).map_err(|e| e.to_string())?;
    Ok(UnsignedInvitation {
        invitee_signing_key,
        member,
        member_bytes,
    })
}

fn assemble(
    room: &RoomData,
    invitee_signing_key: SigningKey,
    member: Member,
    signature: Signature,
) -> Invitation {
    // A private room carries the inviter's secrets so the invitee can read
    // on join, before the owner back-fills `encrypted_secrets`.
    let room_secrets = if room.is_private() {
        collect_invitation_secrets(&room.secrets)
    } else {
        Vec::new()
    };
    Invitation {
        room: room.owner_vk,
        invitee_signing_key,
        invitee: AuthorizedMember::with_signature(member, signature),
        room_secrets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_data::test_minimal_room_data;
    use ed25519_dalek::Signer;
    use river_core::room_state::member::MemberId;
    use river_core::room_state::privacy::PrivacyMode;

    fn room(owner_seed: u8, inviter: &SigningKey, private: bool) -> RoomData {
        let mut room =
            test_minimal_room_data(SigningKey::from_bytes(&[owner_seed; 32]).verifying_key());
        room.self_sk = Some(inviter.clone());
        if private {
            room.room_state.configuration.configuration.privacy_mode = PrivacyMode::Private;
        }
        room
    }

    // The same preparation and assembly `create_invitation` runs, signed the
    // way its fallback signs; `signing.rs` pins that a delegate signature is
    // accepted only when byte-identical to this.
    fn build(room: &RoomData, inviter: &SigningKey) -> Invitation {
        let unsigned = prepare(room, inviter).expect("a Member always serializes");
        let signature = inviter.sign(&unsigned.member_bytes);
        assemble(
            room,
            unsigned.invitee_signing_key,
            unsigned.member,
            signature,
        )
    }

    #[test]
    fn the_invitee_is_signed_by_the_inviter_for_the_candidate_room() {
        let inviter = SigningKey::from_bytes(&[7; 32]);
        let candidate = room(3, &inviter, false);
        let invitation = build(&candidate, &inviter);

        assert_eq!(invitation.room, candidate.owner_vk);
        let member = &invitation.invitee.member;
        assert_eq!(member.owner_member_id, MemberId::from(candidate.owner_vk));
        assert_eq!(member.invited_by, MemberId::from(inviter.verifying_key()));
        assert_eq!(
            member.member_vk,
            invitation.invitee_signing_key.verifying_key()
        );
        invitation
            .invitee
            .verify_signature(&inviter.verifying_key())
            .expect("the contract must accept the inviter's signature");
        assert_eq!(
            invitation.invitee,
            AuthorizedMember::new(member.clone(), &inviter),
            "the signed bytes must be the canonical Member preimage"
        );
    }

    #[test]
    fn every_invitation_gets_a_fresh_identity() {
        let inviter = SigningKey::from_bytes(&[7; 32]);
        let candidate = room(3, &inviter, false);
        let first = build(&candidate, &inviter);
        let second = build(&candidate, &inviter);
        assert_ne!(
            first.invitee_signing_key.to_bytes(),
            second.invitee_signing_key.to_bytes()
        );
        assert_ne!(first.invitee.member.member_vk, inviter.verifying_key());
    }

    #[test]
    fn a_private_room_carries_its_secrets_sorted_by_version() {
        let inviter = SigningKey::from_bytes(&[7; 32]);
        let mut candidate = room(3, &inviter, true);
        candidate
            .secrets
            .extend([(9, [9; 32]), (0, [1; 32]), (4, [4; 32])]);
        let invitation = build(&candidate, &inviter);
        assert_eq!(
            invitation.room_secrets,
            vec![(0, [1; 32]), (4, [4; 32]), (9, [9; 32])]
        );
    }

    #[test]
    fn a_public_room_carries_no_secrets() {
        let inviter = SigningKey::from_bytes(&[7; 32]);
        let mut candidate = room(3, &inviter, false);
        candidate.secrets.insert(0, [1; 32]);
        assert!(build(&candidate, &inviter).room_secrets.is_empty());
    }
}
