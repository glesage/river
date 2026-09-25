//! The one door every request to the node goes through.
//!
//! `WEB_API` holds a [`NodeApi`] rather than a bare `WebApi`, so each request
//! is recorded in the ledger the moment it is handed to the socket, with no
//! call site having to remember to do it. `send` has `WebApi::send`'s exact
//! signature, so callers are unchanged.

use super::ledger::RequestKind;
use freenet_stdlib::client_api::{
    ClientRequest, ContractRequest, DelegateRequest, Error as ClientApiError, WebApi,
};

pub struct NodeApi {
    inner: WebApi,
}

impl NodeApi {
    /// Only `connection_manager.rs` (wasm-only) opens connections.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn new(inner: WebApi) -> Self {
        Self { inner }
    }

    /// Send `request` to the node and, once it is on the socket, record it as
    /// awaiting a reply.
    pub async fn send(&mut self, request: ClientRequest<'static>) -> Result<(), ClientApiError> {
        let kind = request_kind(&request);
        self.inner.send(request).await?;
        if let Some(kind) = kind {
            super::record_request(kind);
        }
        Ok(())
    }
}

/// What a request awaits a reply for, or `None` for requests the node doesn't
/// answer one-for-one (disconnects, queries, stream chunks).
fn request_kind(request: &ClientRequest<'_>) -> Option<RequestKind> {
    match request {
        ClientRequest::ContractOp(op) => match op {
            ContractRequest::Put { contract, .. } => Some(RequestKind::Put(*contract.key().id())),
            ContractRequest::Update { key, .. } => Some(RequestKind::Update(*key.id())),
            ContractRequest::Get { key, .. } => Some(RequestKind::Get(*key)),
            ContractRequest::Subscribe { key, .. } => Some(RequestKind::Subscribe(*key)),
            _ => None,
        },
        ClientRequest::DelegateOp(op) => match op {
            DelegateRequest::ApplicationMessages { key, .. } => {
                Some(RequestKind::Delegate(key.clone()))
            }
            // Answered with a `DelegateResponse` for the delegate's key, like
            // any other delegate request, so it is tracked as one.
            DelegateRequest::RegisterDelegate { delegate, .. } => {
                Some(RequestKind::Delegate(delegate.key().clone()))
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freenet_stdlib::prelude::{
        ContractInstanceId, DelegateKey, Parameters, RelatedContracts, UpdateData, WrappedState,
    };

    fn room_key() -> freenet_stdlib::prelude::ContractKey {
        crate::util::owner_vk_to_contract_key(
            &ed25519_dalek::SigningKey::from_bytes(&[3; 32]).verifying_key(),
        )
    }

    #[test]
    fn contract_requests_are_classified_by_their_contract() {
        let key = room_key();
        let update = ClientRequest::ContractOp(ContractRequest::Update {
            key,
            data: UpdateData::Delta(vec![].into()),
        });
        assert_eq!(request_kind(&update), Some(RequestKind::Update(*key.id())));

        let get = ClientRequest::ContractOp(ContractRequest::Get {
            key: *key.id(),
            return_contract_code: false,
            subscribe: false,
            blocking_subscribe: false,
        });
        assert_eq!(request_kind(&get), Some(RequestKind::Get(*key.id())));

        let subscribe = ClientRequest::ContractOp(ContractRequest::Subscribe {
            key: ContractInstanceId::new([9; 32]),
            summary: None,
        });
        assert_eq!(
            request_kind(&subscribe),
            Some(RequestKind::Subscribe(ContractInstanceId::new([9; 32])))
        );
    }

    /// A PUT's reply names the contract the container derives, so the ledger
    /// must record that same key.
    #[test]
    fn a_put_is_classified_by_its_containers_key() {
        use crate::constants::ROOM_CONTRACT_WASM;
        use freenet_stdlib::prelude::{
            ContractCode, ContractContainer, ContractWasmAPIVersion, WrappedContract,
        };
        let vk = ed25519_dalek::SigningKey::from_bytes(&[3; 32]).verifying_key();
        let params = river_core::room_state::ChatRoomParametersV1 { owner: vk };
        let parameters = Parameters::from(crate::util::to_cbor_vec(&params));
        let code = ContractCode::from(ROOM_CONTRACT_WASM);
        let container = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
            std::sync::Arc::new(code),
            parameters,
        )));
        let put = ClientRequest::ContractOp(ContractRequest::Put {
            contract: container,
            state: WrappedState::new(vec![]),
            related_contracts: RelatedContracts::default(),
            subscribe: true,
            blocking_subscribe: false,
        });
        assert_eq!(
            request_kind(&put),
            Some(RequestKind::Put(*room_key().id())),
            "the PUT must be keyed like owner_vk_to_contract_key, which its reply uses"
        );
    }

    #[test]
    fn delegate_messages_are_classified_by_their_delegate() {
        let key = DelegateKey::new([4; 32], freenet_stdlib::prelude::CodeHash::new([4; 32]));
        let msg = ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages {
            key: key.clone(),
            params: Parameters::from(vec![]),
            inbound: vec![],
        });
        assert_eq!(request_kind(&msg), Some(RequestKind::Delegate(key)));
    }

    /// The node answers a registration with a `DelegateResponse` for the
    /// delegate's key, so it must be tracked exactly like a delegate message
    /// to that key, or the reply settles the wrong request and the
    /// registration lingers until the backstop.
    #[test]
    fn a_delegate_registration_is_tracked_as_a_delegate_request() {
        use freenet_stdlib::prelude::{
            Delegate, DelegateCode, DelegateContainer, DelegateWasmAPIVersion,
        };
        let code = DelegateCode::from(vec![0u8, 97, 115, 109]);
        let params = Parameters::from(Vec::<u8>::new());
        let delegate = Delegate::from((&code, &params));
        let key = delegate.key().clone();
        let register = ClientRequest::DelegateOp(DelegateRequest::RegisterDelegate {
            delegate: DelegateContainer::Wasm(DelegateWasmAPIVersion::V1(delegate)),
            cipher: [0; 32],
            nonce: [0; 24],
        });
        assert_eq!(request_kind(&register), Some(RequestKind::Delegate(key)));
    }

    #[test]
    fn unanswered_requests_are_not_tracked() {
        assert_eq!(
            request_kind(&ClientRequest::Disconnect { cause: None }),
            None
        );
    }
}
