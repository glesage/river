//! Replies carry no request ID, so they settle the oldest request of their
//! kind and key. Errors without a key fall back to kind, then oldest overall.
//! Timestamps are supplied by the caller in milliseconds.

use freenet_stdlib::client_api::{
    ClientError, ContractError, ContractResponse, DelegateError, ErrorKind, HostResponse,
    RequestError,
};
use freenet_stdlib::prelude::{ContractInstanceId, DelegateKey};

pub(crate) type SlotId = u64;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum RequestKind {
    Update(ContractInstanceId),
    Put(ContractInstanceId),
    Get(ContractInstanceId),
    Subscribe(ContractInstanceId),
    /// Any delegate request, including `RegisterDelegate`: the node answers
    /// every one with a `DelegateResponse` for the delegate's key.
    Delegate(DelegateKey),
}

/// A kind without its key, for errors that name the operation but not what
/// it was for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Family {
    Update,
    Put,
    Get,
    Subscribe,
    Delegate,
}

impl RequestKind {
    fn family(&self) -> Family {
        match self {
            RequestKind::Update(_) => Family::Update,
            RequestKind::Put(_) => Family::Put,
            RequestKind::Get(_) => Family::Get,
            RequestKind::Subscribe(_) => Family::Subscribe,
            RequestKind::Delegate(_) => Family::Delegate,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Settle {
    /// The oldest request of exactly this kind and key.
    Exact(RequestKind),
    OldestOf(Family),
    Oldest,
}

#[derive(Clone, Debug)]
struct Slot {
    id: SlotId,
    kind: RequestKind,
    sent_at: f64,
}

/// Outstanding requests in the order they were sent.
#[derive(Default, Debug)]
pub(crate) struct Ledger {
    next_id: SlotId,
    slots: Vec<Slot>,
}

impl Ledger {
    pub(crate) fn record(&mut self, kind: RequestKind, now: f64) -> SlotId {
        let id = self.next_id;
        self.next_id += 1;
        self.slots.push(Slot {
            id,
            kind,
            sent_at: now,
        });
        id
    }

    pub(crate) fn settle(&mut self, settle: &Settle) -> Option<SlotId> {
        let index = self.find(settle)?;
        Some(self.slots.remove(index).id)
    }

    pub(crate) fn would_settle(&self, settle: &Settle) -> bool {
        self.find(settle).is_some()
    }

    fn find(&self, settle: &Settle) -> Option<usize> {
        match settle {
            Settle::Exact(kind) => self.slots.iter().position(|s| &s.kind == kind),
            Settle::OldestOf(family) => self.slots.iter().position(|s| s.kind.family() == *family),
            Settle::Oldest => (!self.slots.is_empty()).then_some(0),
        }
    }

    pub(crate) fn clear(&mut self) -> Vec<SlotId> {
        self.slots.drain(..).map(|s| s.id).collect()
    }

    pub(crate) fn expire(&mut self, now: f64, max_ms: f64) -> Vec<(SlotId, RequestKind)> {
        self.slots
            .extract_if(.., |s| s.sent_at <= now - max_ms)
            .map(|s| (s.id, s.kind))
            .collect()
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = SlotId> + '_ {
        self.slots.iter().map(|s| s.id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// Pushes settle nothing; connection errors are handled by `Ledger::clear`.
pub(crate) fn settle_for(result: &Result<HostResponse, ClientError>) -> Option<Settle> {
    match result {
        Ok(HostResponse::ContractResponse(response)) => match response {
            ContractResponse::GetResponse { key, .. } => {
                Some(Settle::Exact(RequestKind::Get(*key.id())))
            }
            ContractResponse::PutResponse { key } => {
                Some(Settle::Exact(RequestKind::Put(*key.id())))
            }
            ContractResponse::UpdateResponse { key, .. } => {
                Some(Settle::Exact(RequestKind::Update(*key.id())))
            }
            ContractResponse::SubscribeResponse { key, .. } => {
                Some(Settle::Exact(RequestKind::Subscribe(*key.id())))
            }
            ContractResponse::NotFound { instance_id } => {
                Some(Settle::Exact(RequestKind::Get(*instance_id)))
            }
            _ => None,
        },
        Ok(HostResponse::DelegateResponse { key, .. }) => {
            Some(Settle::Exact(RequestKind::Delegate(key.clone())))
        }
        // `HostResponse::Ok` answers nothing River tracks: even
        // `RegisterDelegate` is answered with a `DelegateResponse`.
        Ok(_) => None,
        Err(err) => settle_for_error(err),
    }
}

fn settle_for_error(err: &ClientError) -> Option<Settle> {
    match err.kind() {
        ErrorKind::RequestError(RequestError::ContractError(error)) => match error {
            ContractError::Update { key, .. } => {
                Some(Settle::Exact(RequestKind::Update(*key.id())))
            }
            ContractError::Put { key, .. } => Some(Settle::Exact(RequestKind::Put(*key.id()))),
            ContractError::Get { key, .. } => Some(Settle::Exact(RequestKind::Get(*key.id()))),
            ContractError::Subscribe { key, .. } => {
                Some(Settle::Exact(RequestKind::Subscribe(*key.id())))
            }
            _ => Some(Settle::Oldest),
        },
        ErrorKind::RequestError(RequestError::DelegateError(error)) => match error {
            DelegateError::RegisterError(key)
            | DelegateError::Missing(key)
            | DelegateError::MissingSecret { key, .. } => {
                Some(Settle::Exact(RequestKind::Delegate(key.clone())))
            }
            _ => Some(Settle::OldestOf(Family::Delegate)),
        },
        ErrorKind::RequestError(RequestError::Timeout) => Some(Settle::Oldest),
        // freenet-core's op_ctx_task drivers prefix errors with the operation.
        // Before 0.2.136, delegate failures also arrive here as untyped executor
        // messages. Unrecognised wording falls back to the oldest request.
        ErrorKind::OperationError { cause } => {
            let cause = cause.to_ascii_uppercase();
            let family = [
                ("UPDATE", Family::Update),
                ("PUT", Family::Put),
                ("GET", Family::Get),
                ("SUBSCRIBE", Family::Subscribe),
            ]
            .into_iter()
            .find(|(op, _)| cause.starts_with(op))
            .map(|(_, family)| family)
            .or_else(|| cause.contains("DELEGATE").then_some(Family::Delegate));
            Some(family.map_or(Settle::Oldest, Settle::OldestOf))
        }
        ErrorKind::FailedOperation
        | ErrorKind::PeerNotJoined
        | ErrorKind::EmptyRing
        | ErrorKind::NodeUnavailable => Some(Settle::Oldest),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freenet_stdlib::prelude::{ContractKey, StateSummary, WrappedState};

    fn c(n: u8) -> ContractInstanceId {
        ContractInstanceId::new([n; 32])
    }

    fn key(n: u8) -> ContractKey {
        crate::util::owner_vk_to_contract_key(
            &ed25519_dalek::SigningKey::from_bytes(&[n; 32]).verifying_key(),
        )
    }

    fn d(n: u8) -> DelegateKey {
        DelegateKey::new([n; 32], freenet_stdlib::prelude::CodeHash::new([n; 32]))
    }

    fn contract(response: ContractResponse) -> Result<HostResponse, ClientError> {
        Ok(HostResponse::ContractResponse(response))
    }

    fn error(kind: ErrorKind) -> Result<HostResponse, ClientError> {
        Err(ClientError::from(kind))
    }

    fn op_error(cause: &'static str) -> Result<HostResponse, ClientError> {
        error(ErrorKind::OperationError {
            cause: cause.into(),
        })
    }

    #[test]
    fn a_reply_settles_the_oldest_request_of_its_kind_and_key() {
        let mut l = Ledger::default();
        let first = l.record(RequestKind::Update(c(1)), 0.0);
        let second = l.record(RequestKind::Update(c(1)), 1.0);
        l.record(RequestKind::Update(c(2)), 2.0);
        l.record(RequestKind::Get(c(1)), 3.0);

        let settle = Settle::Exact(RequestKind::Update(c(1)));
        assert_eq!(l.settle(&settle), Some(first));
        assert_eq!(l.settle(&settle), Some(second));
        assert_eq!(l.settle(&settle), None, "only two UPDATEs for c1 were sent");
        assert_eq!(l.ids().count(), 2, "c2's UPDATE and c1's GET are untouched");
    }

    #[test]
    fn oldest_of_ignores_the_key_and_oldest_ignores_the_kind() {
        let mut l = Ledger::default();
        let get = l.record(RequestKind::Get(c(1)), 0.0);
        let update = l.record(RequestKind::Update(c(2)), 1.0);
        assert_eq!(l.settle(&Settle::OldestOf(Family::Update)), Some(update));
        assert_eq!(l.settle(&Settle::Oldest), Some(get));
        assert!(l.is_empty());
        assert_eq!(l.settle(&Settle::Oldest), None);
    }

    #[test]
    fn would_settle_matches_settle() {
        let mut l = Ledger::default();
        l.record(RequestKind::Put(c(1)), 0.0);
        assert!(l.would_settle(&Settle::Exact(RequestKind::Put(c(1)))));
        assert!(!l.would_settle(&Settle::Exact(RequestKind::Put(c(2)))));
        assert!(!l.would_settle(&Settle::OldestOf(Family::Get)));
    }

    #[test]
    fn clear_drops_everything() {
        let mut l = Ledger::default();
        let a = l.record(RequestKind::Get(c(1)), 0.0);
        let b = l.record(RequestKind::Delegate(d(1)), 0.0);
        assert_eq!(l.clear(), vec![a, b]);
        assert!(l.is_empty());
    }

    #[test]
    fn expire_drops_only_requests_at_least_max_old() {
        let mut l = Ledger::default();
        let old = l.record(RequestKind::Get(c(1)), 0.0);
        let young = l.record(RequestKind::Get(c(2)), 50_000.0);
        let dropped = l.expire(90_000.0, 90_000.0);
        assert_eq!(dropped, vec![(old, RequestKind::Get(c(1)))]);
        assert_eq!(l.ids().collect::<Vec<_>>(), vec![young]);
    }

    #[test]
    fn expire_keeps_survivor_and_expiry_order_when_interleaved() {
        let mut l = Ledger::default();
        let stale_a = l.record(RequestKind::Get(c(1)), 0.0);
        let fresh_a = l.record(RequestKind::Get(c(2)), 50_000.0);
        let stale_b = l.record(RequestKind::Get(c(3)), 10_000.0);
        let fresh_b = l.record(RequestKind::Get(c(4)), 60_000.0);
        let dropped = l.expire(100_000.0, 90_000.0);
        assert_eq!(
            dropped,
            vec![
                (stale_a, RequestKind::Get(c(1))),
                (stale_b, RequestKind::Get(c(3)))
            ]
        );
        assert_eq!(l.ids().collect::<Vec<_>>(), vec![fresh_a, fresh_b]);
        assert_eq!(l.settle(&Settle::Oldest), Some(fresh_a));
        assert_eq!(l.settle(&Settle::Oldest), Some(fresh_b));
    }

    #[test]
    fn contract_replies_name_their_request() {
        let cases = [
            (
                contract(ContractResponse::GetResponse {
                    key: key(1),
                    contract: None,
                    state: WrappedState::new(vec![]),
                }),
                RequestKind::Get(*key(1).id()),
            ),
            (
                contract(ContractResponse::PutResponse { key: key(2) }),
                RequestKind::Put(*key(2).id()),
            ),
            (
                contract(ContractResponse::UpdateResponse {
                    key: key(3),
                    summary: StateSummary::from(vec![]),
                }),
                RequestKind::Update(*key(3).id()),
            ),
            (
                contract(ContractResponse::SubscribeResponse {
                    key: key(4),
                    subscribed: true,
                }),
                RequestKind::Subscribe(*key(4).id()),
            ),
            (
                contract(ContractResponse::NotFound { instance_id: c(5) }),
                RequestKind::Get(c(5)),
            ),
        ];
        for (reply, kind) in cases {
            assert_eq!(settle_for(&reply), Some(Settle::Exact(kind)));
        }
    }

    #[test]
    fn delegate_replies_name_their_delegate() {
        let reply = Ok(HostResponse::DelegateResponse {
            key: d(7),
            values: vec![],
        });
        assert_eq!(
            settle_for(&reply),
            Some(Settle::Exact(RequestKind::Delegate(d(7))))
        );
        assert_eq!(
            settle_for(&Ok(HostResponse::Ok)),
            None,
            "nothing River tracks is answered with a bare Ok"
        );
    }

    // Tracking registration as a separate kind used to settle the next delegate
    // request instead, leaving registration pending until the backstop.
    #[test]
    fn a_registration_and_the_requests_after_it_are_all_settled() {
        let mut l = Ledger::default();
        // `set_up_chat_delegate`: register, then list, then a GET.
        for _ in 0..3 {
            l.record(RequestKind::Delegate(d(1)), 0.0);
        }
        let reply = Ok(HostResponse::DelegateResponse {
            key: d(1),
            values: vec![],
        });
        for _ in 0..3 {
            let settle = settle_for(&reply).unwrap();
            assert!(l.settle(&settle).is_some());
        }
        assert!(l.is_empty(), "three requests, three replies, nothing left");
    }

    #[test]
    fn pushes_settle_nothing() {
        let reply = contract(ContractResponse::UpdateNotification {
            key: key(1),
            update: freenet_stdlib::prelude::UpdateData::Delta(vec![].into()),
        });
        assert_eq!(settle_for(&reply), None);
    }

    #[test]
    fn keyed_errors_settle_that_request() {
        let cases = [
            (
                ContractError::Update {
                    key: key(1),
                    cause: "no".into(),
                },
                RequestKind::Update(*key(1).id()),
            ),
            (
                ContractError::Put {
                    key: key(2),
                    cause: "no".into(),
                },
                RequestKind::Put(*key(2).id()),
            ),
            (
                ContractError::Get {
                    key: key(3),
                    cause: "no".into(),
                },
                RequestKind::Get(*key(3).id()),
            ),
            (
                ContractError::Subscribe {
                    key: key(4),
                    cause: "no".into(),
                },
                RequestKind::Subscribe(*key(4).id()),
            ),
        ];
        for (e, kind) in cases {
            let reply = error(ErrorKind::RequestError(RequestError::ContractError(e)));
            assert_eq!(settle_for(&reply), Some(Settle::Exact(kind)));
        }

        let missing = error(ErrorKind::RequestError(RequestError::DelegateError(
            DelegateError::Missing(d(8)),
        )));
        assert_eq!(
            settle_for(&missing),
            Some(Settle::Exact(RequestKind::Delegate(d(8))))
        );
        let register = error(ErrorKind::RequestError(RequestError::DelegateError(
            DelegateError::RegisterError(d(9)),
        )));
        assert_eq!(
            settle_for(&register),
            Some(Settle::Exact(RequestKind::Delegate(d(9))))
        );
        let execution = error(ErrorKind::RequestError(RequestError::DelegateError(
            DelegateError::ExecutionError("boom".into()),
        )));
        assert_eq!(
            settle_for(&execution),
            Some(Settle::OldestOf(Family::Delegate))
        );
    }

    #[test]
    fn operation_errors_settle_the_oldest_of_their_kind() {
        let cases = [
            ("UPDATE failed: rejected", Family::Update),
            ("PUT failed: timeout", Family::Put),
            ("GET failed: nope", Family::Get),
            ("subscribe failed: nope", Family::Subscribe),
        ];
        for (cause, family) in cases {
            assert_eq!(
                settle_for(&op_error(cause)),
                Some(Settle::OldestOf(family)),
                "{cause}"
            );
        }
        assert_eq!(
            settle_for(&op_error("something else broke")),
            Some(Settle::Oldest)
        );
        // freenet-core before 0.2.136: an untyped delegate failure.
        assert_eq!(
            settle_for(&op_error("executor error: delegate not found in store")),
            Some(Settle::OldestOf(Family::Delegate))
        );
    }

    #[test]
    fn node_wide_errors_settle_the_oldest_request() {
        for kind in [
            ErrorKind::FailedOperation,
            ErrorKind::RequestError(RequestError::Timeout),
            ErrorKind::PeerNotJoined,
            ErrorKind::EmptyRing,
            ErrorKind::NodeUnavailable,
        ] {
            assert_eq!(settle_for(&error(kind)), Some(Settle::Oldest));
        }
    }

    #[test]
    fn connection_errors_settle_nothing() {
        for kind in [ErrorKind::ChannelClosed, ErrorKind::Disconnect] {
            assert_eq!(settle_for(&error(kind)), None);
        }
    }
}
