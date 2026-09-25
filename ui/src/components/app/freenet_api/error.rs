#![allow(dead_code)]

use freenet_stdlib::client_api;
use thiserror::Error;

/// Error types for the Freenet synchronizer
#[derive(Error, Debug, Clone)]
pub enum SynchronizerError {
    #[error("WebSocket connection error: {0}")]
    WebSocketError(String),

    #[error("WebSocket operation not supported: {0}")]
    WebSocketNotSupported(String),

    #[error("Connection timeout after {0}ms")]
    ConnectionTimeout(u64),

    #[error("API not initialized")]
    ApiNotInitialized,

    #[error("Room data not found for key: {0}")]
    RoomNotFound(String),

    #[error("Contract info not found for key: {0}")]
    ContractInfoNotFound(String),

    #[error("Failed to send message: {0}")]
    MessageSendError(String),

    #[error("Failed to merge room state: {0}")]
    StateMergeError(String),

    #[error("Failed to apply delta to room state: {0}")]
    DeltaApplyError(String),

    #[error("Failed to put contract state: {0}")]
    PutContractError(String),

    #[error("Failed to subscribe to contract: {0}")]
    SubscribeError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Client API error: {0}")]
    ClientApiError(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl SynchronizerError {
    /// The text to show a user: the failure, said once.
    ///
    /// `Display` keeps its category label for logs and for the substring
    /// checks in `freenet_synchronizer.rs`; this drops the label wherever it
    /// only restates the payload. No `_` arm, so a new variant must choose.
    pub fn user_message(&self) -> String {
        match self {
            SynchronizerError::WebSocketError(msg)
            | SynchronizerError::WebSocketNotSupported(msg)
            | SynchronizerError::ClientApiError(msg)
            | SynchronizerError::Unknown(msg) => msg.clone(),
            SynchronizerError::ConnectionTimeout(_)
            | SynchronizerError::ApiNotInitialized
            | SynchronizerError::RoomNotFound(_)
            | SynchronizerError::ContractInfoNotFound(_)
            | SynchronizerError::MessageSendError(_)
            | SynchronizerError::StateMergeError(_)
            | SynchronizerError::DeltaApplyError(_)
            | SynchronizerError::PutContractError(_)
            | SynchronizerError::SubscribeError(_)
            | SynchronizerError::SerializationError(_)
            | SynchronizerError::DeserializationError(_) => self.to_string(),
        }
    }
}

impl From<String> for SynchronizerError {
    fn from(error: String) -> Self {
        SynchronizerError::Unknown(error)
    }
}

impl From<&str> for SynchronizerError {
    fn from(error: &str) -> Self {
        SynchronizerError::Unknown(error.to_string())
    }
}

impl From<client_api::Error> for SynchronizerError {
    fn from(error: client_api::Error) -> Self {
        SynchronizerError::ClientApiError(node_error_message(&error))
    }
}

/// A freenet-stdlib client error as text for a user.
///
/// The browser client's `ConnectionError` displays as `request error: ` plus
/// raw JSON that can carry the whole request's `Debug`; this keeps only the
/// JSON's `"error"` field.
pub fn node_error_message(error: &client_api::Error) -> String {
    #[cfg(target_family = "wasm")]
    {
        if let client_api::Error::ConnectionError(value) = error {
            if let Some(text) = connection_error_text(value) {
                return capitalise_first(text);
            }
        }
    }
    capitalise_first(&error.to_string())
}

/// The `"error"` field of the JSON freenet-stdlib's browser client puts in
/// `ConnectionError` (`browser.rs`: `{"error": …, "source"|"origin": …}`).
/// Pure, so it is testable natively, where that variant does not exist.
#[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
fn connection_error_text(value: &freenet_stdlib::prelude::serde_json::Value) -> Option<&str> {
    value.get("error")?.as_str().filter(|s| !s.is_empty())
}

fn capitalise_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One instance of every variant, so a check over "all errors" can't
    /// silently skip one.
    fn every_variant() -> Vec<SynchronizerError> {
        vec![
            SynchronizerError::WebSocketError("a".into()),
            SynchronizerError::WebSocketNotSupported("a".into()),
            SynchronizerError::ConnectionTimeout(5000),
            SynchronizerError::ApiNotInitialized,
            SynchronizerError::RoomNotFound("a".into()),
            SynchronizerError::ContractInfoNotFound("a".into()),
            SynchronizerError::MessageSendError("a".into()),
            SynchronizerError::StateMergeError("a".into()),
            SynchronizerError::DeltaApplyError("a".into()),
            SynchronizerError::PutContractError("a".into()),
            SynchronizerError::SubscribeError("a".into()),
            SynchronizerError::SerializationError("a".into()),
            SynchronizerError::DeserializationError("a".into()),
            SynchronizerError::ClientApiError("a".into()),
            SynchronizerError::Unknown("a".into()),
        ]
    }

    #[test]
    fn user_message_drops_a_label_that_restates_the_payload() {
        let msg = "WebSocket connection failed or timed out";
        for error in [
            SynchronizerError::WebSocketError(msg.into()),
            SynchronizerError::WebSocketNotSupported(msg.into()),
            SynchronizerError::ClientApiError(msg.into()),
            SynchronizerError::Unknown(msg.into()),
        ] {
            assert_eq!(error.user_message(), msg, "{error:?}");
        }
    }

    #[test]
    fn user_message_keeps_a_label_that_says_what_failed() {
        let cause = "WebSocket is not open (state: CLOSED)";
        assert_eq!(
            SynchronizerError::SubscribeError(cause.into()).user_message(),
            format!("Failed to subscribe to contract: {cause}")
        );
        assert_eq!(
            SynchronizerError::PutContractError(cause.into()).user_message(),
            format!("Failed to put contract state: {cause}")
        );
        assert_eq!(
            SynchronizerError::ConnectionTimeout(5000).user_message(),
            "Connection timeout after 5000ms"
        );
        assert_eq!(
            SynchronizerError::ApiNotInitialized.user_message(),
            "API not initialized"
        );
    }

    /// Logs and the substring checks in `freenet_synchronizer.rs` read
    /// `Display`, so it keeps its label.
    #[test]
    fn display_keeps_its_label_for_logs() {
        assert_eq!(
            SynchronizerError::WebSocketError("x".into()).to_string(),
            "WebSocket connection error: x"
        );
    }

    #[test]
    fn connection_error_text_takes_the_error_field() {
        use freenet_stdlib::prelude::serde_json::json;
        assert_eq!(
            connection_error_text(&json!({"error": "connection closed", "source": "close"})),
            Some("connection closed")
        );
        assert_eq!(
            connection_error_text(&json!({
                "error": "WebSocket is not open (state: CLOSED)",
                "origin": "send precondition check",
                "request": "ContractOp(..)"
            })),
            Some("WebSocket is not open (state: CLOSED)")
        );
        // Anything else falls back to `Display`.
        for value in [
            json!({"source": "close"}),
            json!({"error": ""}),
            json!({"error": 5}),
        ] {
            assert_eq!(connection_error_text(&value), None, "{value}");
        }
    }

    #[test]
    fn node_error_message_capitalises_and_drops_no_meaning() {
        assert_eq!(
            node_error_message(&client_api::Error::ConnectionClosed),
            "Connection closed"
        );
        assert_eq!(
            node_error_message(&client_api::Error::ChannelClosed),
            "Channel closed"
        );
    }

    #[test]
    fn capitalise_first_handles_empty_and_non_ascii() {
        assert_eq!(capitalise_first(""), "");
        assert_eq!(capitalise_first("é"), "É");
        assert_eq!(capitalise_first("WebSocket"), "WebSocket");
    }

    #[test]
    fn client_api_error_converts_without_a_double_label() {
        assert_eq!(
            SynchronizerError::from(client_api::Error::ConnectionClosed).user_message(),
            "Connection closed"
        );
    }

    #[test]
    fn no_user_message_starts_with_a_bare_error_label() {
        for error in every_variant() {
            let msg = error.user_message();
            for label in [
                "Error: ",
                "Unknown error: ",
                "Client API error: ",
                "WebSocket connection error: ",
            ] {
                assert!(!msg.starts_with(label), "{error:?} → {msg:?}");
            }
        }
    }
}
