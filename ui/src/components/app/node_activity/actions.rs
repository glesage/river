//! Room changes wait for the next UPDATE sent by `process_rooms`, then its
//! reply; scoped actions wait for a guard to drop. Both are capped so an
//! unsubscribed room or lost reply cannot leave the indicator on.

use super::ledger::SlotId;
use freenet_stdlib::prelude::ContractInstanceId;
use std::collections::{HashMap, HashSet};

pub(crate) type ActionId = u64;

/// Variant order sets indicator priority: the earliest pending kind wins.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ActionKind {
    CreatingRoom,
    /// Sending, replying, editing, deleting, reacting, and direct messages.
    Sending,
    /// Settings, nickname, moderation, invitations, and preferences.
    Saving,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wait {
    Unsent(ContractInstanceId),
    Slot(SlotId),
    Scoped,
}

#[derive(Clone, Copy, Debug)]
struct Action {
    kind: ActionKind,
    started_at: f64,
    wait: Wait,
}

#[derive(Default, Debug)]
pub(crate) struct Actions {
    pending: HashMap<ActionId, Action>,
}

impl Actions {
    pub(crate) fn await_update(
        &mut self,
        id: ActionId,
        contract: ContractInstanceId,
        kind: ActionKind,
        now: f64,
    ) {
        self.pending.insert(
            id,
            Action {
                kind,
                started_at: now,
                wait: Wait::Unsent(contract),
            },
        );
    }

    pub(crate) fn attach_unsent(&mut self, contract: ContractInstanceId, slot: SlotId) {
        for action in self.pending.values_mut() {
            if action.wait == Wait::Unsent(contract) {
                action.wait = Wait::Slot(slot);
            }
        }
    }

    pub(crate) fn slots_settled(&mut self, slots: &[SlotId]) {
        self.pending
            .retain(|_, a| !matches!(a.wait, Wait::Slot(s) if slots.contains(&s)));
    }

    pub(crate) fn begin_scoped(&mut self, id: ActionId, kind: ActionKind, now: f64) {
        self.pending.insert(
            id,
            Action {
                kind,
                started_at: now,
                wait: Wait::Scoped,
            },
        );
    }

    pub(crate) fn end(&mut self, id: ActionId) {
        self.pending.remove(&id);
    }

    pub(crate) fn expire(&mut self, now: f64, cap_ms: f64) {
        self.pending.retain(|_, a| a.started_at > now - cap_ms);
    }

    pub(crate) fn attached_slots(&self) -> HashSet<SlotId> {
        self.pending
            .values()
            .filter_map(|a| match a.wait {
                Wait::Slot(slot) => Some(slot),
                Wait::Unsent(_) | Wait::Scoped => None,
            })
            .collect()
    }

    pub(crate) fn reason(&self) -> Option<ActionKind> {
        self.pending.values().map(|a| a.kind).min()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(n: u8) -> ContractInstanceId {
        ContractInstanceId::new([n; 32])
    }

    #[test]
    fn an_unsent_change_waits_for_its_rooms_update_then_its_reply() {
        let mut a = Actions::default();
        a.await_update(1, c(1), ActionKind::Sending, 0.0);
        assert_eq!(a.reason(), Some(ActionKind::Sending));

        a.attach_unsent(c(2), 10);
        assert!(
            a.attached_slots().is_empty(),
            "another room's UPDATE does not carry it"
        );

        a.attach_unsent(c(1), 11);
        assert_eq!(a.attached_slots(), HashSet::from([11]));
        assert_eq!(
            a.reason(),
            Some(ActionKind::Sending),
            "still waiting on the reply"
        );

        a.slots_settled(&[10]);
        assert!(!a.is_empty(), "a different request's reply does not end it");
        a.slots_settled(&[11]);
        assert!(a.is_empty());
    }

    #[test]
    fn one_update_carries_every_change_waiting_for_its_room() {
        let mut a = Actions::default();
        a.await_update(1, c(1), ActionKind::Sending, 0.0);
        a.await_update(2, c(1), ActionKind::Saving, 0.0);
        a.attach_unsent(c(1), 5);
        a.slots_settled(&[5]);
        assert!(a.is_empty());
    }

    #[test]
    fn a_later_change_is_not_attached_to_an_earlier_update() {
        let mut a = Actions::default();
        a.await_update(1, c(1), ActionKind::Sending, 0.0);
        a.attach_unsent(c(1), 5);
        a.await_update(2, c(1), ActionKind::Sending, 1.0);
        a.slots_settled(&[5]);
        assert!(!a.is_empty(), "the second change has not been sent yet");
        a.attach_unsent(c(1), 6);
        a.slots_settled(&[6]);
        assert!(a.is_empty());
    }

    #[test]
    fn scoped_actions_end_with_their_guard() {
        let mut a = Actions::default();
        a.begin_scoped(7, ActionKind::Saving, 0.0);
        assert_eq!(a.reason(), Some(ActionKind::Saving));
        assert!(a.attached_slots().is_empty());
        a.end(7);
        assert!(a.is_empty());
    }

    #[test]
    fn every_action_is_capped() {
        let mut a = Actions::default();
        a.await_update(1, c(1), ActionKind::Sending, 0.0);
        a.begin_scoped(2, ActionKind::Saving, 0.0);
        a.begin_scoped(3, ActionKind::Saving, 15_000.0);
        a.expire(20_000.0, 20_000.0);
        assert_eq!(a.pending.len(), 1, "only the young action survives");
        assert!(a.pending.contains_key(&3));
    }

    #[test]
    fn reason_is_the_highest_priority_kind() {
        let mut a = Actions::default();
        a.begin_scoped(1, ActionKind::Saving, 0.0);
        a.begin_scoped(2, ActionKind::Sending, 0.0);
        assert_eq!(a.reason(), Some(ActionKind::Sending));
        a.begin_scoped(3, ActionKind::CreatingRoom, 0.0);
        assert_eq!(a.reason(), Some(ActionKind::CreatingRoom));
    }
}
