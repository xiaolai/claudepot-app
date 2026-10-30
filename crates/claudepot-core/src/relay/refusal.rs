//! Every response the relay itself generates, as one fixed enum.
//!
//! The body is Anthropic-shaped (`{"type":"error","error":{…}}`) so CC
//! surfaces it rather than choking, and the message is the enum's code —
//! never text from the upstream, the credential store or the request.
//!
//! Statuses follow what installed CC 2.1.312 does with them (P0): **401
//! and 503 are retried for 150 s or more** before CC gives up, and 404
//! makes CC blame the model name. So every refusal the user must act on
//! (bad token, ungranted upstream, owner conflict) is a **403**, which CC
//! reports at once. 503 is kept only for genuinely transient capacity,
//! where CC's retry is the right behaviour.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use super::upstream::{RequestRefusal, UpstreamError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The request line is not something this server serves.
    NotFound,
    /// No device token, or one that does not verify, or a revoked one.
    Unauthenticated,
    /// A well-formed upstream id this device holds no grant for.
    UpstreamNotGranted,
    /// The grant was accepted against an older generation of the target,
    /// or the device opted into future publications and has not yet
    /// accepted this one. `claudepot relay accept` clears it.
    AcceptanceRequired,
    /// The upstream does not accept this operation.
    OperationUnsupported,
    /// No pairing code is open: none offered, expired, or burned.
    PairingUnavailable,
    PairingCodeInvalid,
    /// `accept` named a fingerprint that is not the upstream's current one.
    FingerprintMismatch,
    /// A control request whose body is not the expected JSON.
    BadRequest,
    /// The device store could not be written; nothing was recorded.
    StoreUnavailable,
    BodyTooLarge,
    /// Slow upload: no body bytes for the idle window.
    BodyTimeout,
    /// A client-side read error mid-body.
    BodyUnreadable,
    /// Per-device or global admission, or the aggregate body budget.
    OverCapacity,
    Upstream(UpstreamError),
}

impl Refusal {
    pub fn status(self) -> StatusCode {
        match self {
            Refusal::NotFound => StatusCode::NOT_FOUND,
            Refusal::Unauthenticated
            | Refusal::UpstreamNotGranted
            | Refusal::AcceptanceRequired
            | Refusal::PairingUnavailable
            | Refusal::PairingCodeInvalid
            | Refusal::FingerprintMismatch
            | Refusal::Upstream(UpstreamError::CredentialUnavailable)
            | Refusal::Upstream(UpstreamError::OwnerConflict)
            | Refusal::Upstream(UpstreamError::IdentityChanged)
            | Refusal::Upstream(UpstreamError::CredentialRejected) => StatusCode::FORBIDDEN,
            Refusal::OperationUnsupported
            | Refusal::BadRequest
            | Refusal::BodyUnreadable
            | Refusal::Upstream(UpstreamError::Refused(_)) => StatusCode::BAD_REQUEST,
            Refusal::BodyTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Refusal::BodyTimeout => StatusCode::REQUEST_TIMEOUT,
            Refusal::OverCapacity => StatusCode::SERVICE_UNAVAILABLE,
            Refusal::StoreUnavailable => StatusCode::INTERNAL_SERVER_ERROR,
            Refusal::Upstream(UpstreamError::Transport)
            | Refusal::Upstream(UpstreamError::Redirect) => StatusCode::BAD_GATEWAY,
            Refusal::Upstream(UpstreamError::Timeout) => StatusCode::GATEWAY_TIMEOUT,
        }
    }

    /// The stable code: the JSON message, the status-event `last_error`,
    /// and the only thing a log line about the refusal may carry.
    pub fn code(self) -> &'static str {
        match self {
            Refusal::NotFound => "not_found",
            Refusal::Unauthenticated => "relay_unauthenticated",
            Refusal::UpstreamNotGranted => "upstream_not_granted",
            Refusal::AcceptanceRequired => "acceptance_required",
            Refusal::OperationUnsupported => "operation_unsupported",
            Refusal::PairingUnavailable => "pairing_unavailable",
            Refusal::PairingCodeInvalid => "pairing_code_invalid",
            Refusal::FingerprintMismatch => "fingerprint_mismatch",
            Refusal::BadRequest => "bad_request",
            Refusal::StoreUnavailable => "relay_store_unavailable",
            Refusal::BodyTooLarge => "body_too_large",
            Refusal::BodyTimeout => "body_timeout",
            Refusal::BodyUnreadable => "body_unreadable",
            Refusal::OverCapacity => "relay_over_capacity",
            Refusal::Upstream(e) => match e {
                UpstreamError::CredentialUnavailable => "credential_unavailable",
                UpstreamError::OwnerConflict => "owner_conflict",
                UpstreamError::IdentityChanged => "identity_changed",
                UpstreamError::Transport => "upstream_unreachable",
                UpstreamError::Timeout => "upstream_timeout",
                UpstreamError::Redirect => "upstream_redirect",
                UpstreamError::CredentialRejected => "upstream_credential_rejected",
                UpstreamError::Refused(r) => match r {
                    RequestRefusal::ContinuationLost => "continuation_state_lost",
                    RequestRefusal::CountTokensUnsupported => "count_tokens_unsupported",
                    RequestRefusal::ContentUnsupported => "content_unsupported",
                },
            },
        }
    }

    fn error_type(self) -> &'static str {
        match self.status() {
            StatusCode::NOT_FOUND => "not_found_error",
            StatusCode::FORBIDDEN => "permission_error",
            StatusCode::PAYLOAD_TOO_LARGE => "request_too_large",
            StatusCode::SERVICE_UNAVAILABLE => "overloaded_error",
            StatusCode::BAD_REQUEST | StatusCode::REQUEST_TIMEOUT => "invalid_request_error",
            _ => "api_error",
        }
    }
}

impl From<UpstreamError> for Refusal {
    fn from(e: UpstreamError) -> Self {
        Refusal::Upstream(e)
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "type": "error",
            "error": { "type": self.error_type(), "message": self.code() },
        })
        .to_string();
        let mut resp = (self.status(), body).into_response();
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_the_user_must_act_on_are_403_so_cc_fails_fast() {
        for r in [
            Refusal::Unauthenticated,
            Refusal::UpstreamNotGranted,
            Refusal::AcceptanceRequired,
            Refusal::Upstream(UpstreamError::OwnerConflict),
            Refusal::Upstream(UpstreamError::IdentityChanged),
            Refusal::Upstream(UpstreamError::CredentialRejected),
            Refusal::Upstream(UpstreamError::CredentialUnavailable),
        ] {
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{r:?}");
        }
        assert_eq!(
            Refusal::OverCapacity.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn the_body_is_anthropic_shaped_and_carries_only_the_code() {
        let resp = Refusal::UpstreamNotGranted.into_response();
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "permission_error");
        assert_eq!(v["error"]["message"], "upstream_not_granted");
    }

    #[test]
    fn content_refusals_are_400_and_count_tokens_is_never_501() {
        for (r, code) in [
            (RequestRefusal::ContinuationLost, "continuation_state_lost"),
            (
                RequestRefusal::CountTokensUnsupported,
                "count_tokens_unsupported",
            ),
            (RequestRefusal::ContentUnsupported, "content_unsupported"),
        ] {
            let refusal = Refusal::Upstream(UpstreamError::Refused(r));
            assert_eq!(refusal.status(), StatusCode::BAD_REQUEST, "{r:?}");
            assert_eq!(refusal.error_type(), "invalid_request_error");
            assert_eq!(refusal.code(), code);
        }
    }
}
