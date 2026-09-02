//! NIP-FI assertion validation at WebSocket upgrade.
//!
//! This module owns the exact NIP-FI HTTP denial contract for upgrade denials
//! and the header-parsing that feeds assertion validation.
//!
//! Per [NIP-FI.md](../../../docs/nips/NIP-FI.md) §Client-attached transport:
//! - Exactly one `Nostr-Federated-Identity: Bearer <compact-JWS>` field.
//! - Missing, repeated, comma-combined, empty, non-Bearer, and mixed-profile
//!   fields all deny. [FI-TRACE-TRANSPORT-CLOSED]
//! - Per §Rejection table, pre-101 denials are HTTP responses; the exact wire
//!   contract is fixed (status, body, headers). [FI-TRACE-DENIAL-ORACLE]

use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode};
use buzz_auth::{
    DenialClass, FederatedAssertionVerifier, IssuerKeySource, NipFiMode, VerifiedAssertion,
    CLIENT_ATTACHED_HEADER,
};

/// Outcome of NIP-FI assertion validation at upgrade time.
pub(crate) enum NipFiUpgradeOutcome {
    /// Assertion validated successfully. Carry the result into the connection.
    Admitted(VerifiedAssertion),
    /// Enforcement is off — no assertion required.
    NotRequired,
    /// Enforcement active but assertion absent/rejected — return the HTTP
    /// denial response.
    Denied(Response<Body>),
}

/// Validate the NIP-FI assertion on a WebSocket upgrade request.
///
/// Returns:
/// - `NotRequired` when the relay is in `Off` mode.
/// - `Admitted(assertion)` when the token is present, valid, and passes.
/// - `Denied(response)` with the exact NIP-FI HTTP denial contract otherwise.
///
/// The `DenyProtected` mode always returns `Denied(authorization_denied)`.
pub(crate) fn check_nip_fi_at_upgrade<S: IssuerKeySource>(
    headers: &HeaderMap,
    verifier: Option<&FederatedAssertionVerifier<S>>,
    mode: NipFiMode,
) -> NipFiUpgradeOutcome {
    if matches!(mode, NipFiMode::Off) {
        return NipFiUpgradeOutcome::NotRequired;
    }

    if matches!(mode, NipFiMode::DenyProtected) {
        return NipFiUpgradeOutcome::Denied(denial_response(DenialClass::AuthorizationUnavailable));
    }

    // Enforce mode: validate the assertion.
    let token = match extract_bearer_token(headers) {
        Ok(t) => t,
        Err(class) => return NipFiUpgradeOutcome::Denied(denial_response(class)),
    };

    let verifier = match verifier {
        Some(v) => v,
        None => {
            // Verifier not yet constructed (startup race); fail closed.
            return NipFiUpgradeOutcome::Denied(denial_response(
                DenialClass::AuthorizationUnavailable,
            ));
        }
    };

    match verifier.verify(token) {
        Ok(assertion) => NipFiUpgradeOutcome::Admitted(assertion),
        Err(err) => {
            tracing::debug!(code = err.code(), "nip-fi assertion denied at upgrade");
            NipFiUpgradeOutcome::Denied(denial_response(err.denial_class()))
        }
    }
}

/// Extract the single `Bearer <token>` value from the NIP-FI header.
///
/// Rejects all forms the spec prohibits:
/// - absent → `MissingEvidence`
/// - repeated (multiple header values) → `EvidenceRejected`
/// - comma-combined (`,` in a single value) → `EvidenceRejected`
/// - empty after `Bearer ` stripping → `EvidenceRejected`
/// - non-`Bearer ` prefix → `EvidenceRejected`
/// - value containing whitespace after the scheme → `EvidenceRejected`
///
/// [FI-TRACE-TRANSPORT-CLOSED]
fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, DenialClass> {
    let mut values = headers.get_all(CLIENT_ATTACHED_HEADER).iter();
    let first = match values.next() {
        Some(v) => v,
        None => return Err(DenialClass::MissingEvidence),
    };
    // Repeated header fields deny.
    if values.next().is_some() {
        return Err(DenialClass::EvidenceRejected);
    }
    let raw = first.to_str().map_err(|_| DenialClass::EvidenceRejected)?;
    // Comma-combined values deny.
    if raw.contains(',') {
        return Err(DenialClass::EvidenceRejected);
    }
    // Must be `Bearer <token>` — exactly that prefix.
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or(DenialClass::EvidenceRejected)?;
    // Empty value after stripping denies.
    if token.is_empty() {
        return Err(DenialClass::EvidenceRejected);
    }
    // Whitespace within the token denies (mixed-profile detection).
    if token.contains(char::is_whitespace) {
        return Err(DenialClass::EvidenceRejected);
    }
    Ok(token)
}

/// Build the exact NIP-FI HTTP denial response for a WebSocket upgrade request.
///
/// Per the NIP-FI rejection table: status + exact body + `Content-Type`.
/// `MissingEvidence` additionally carries `WWW-Authenticate: Nostr`.
/// No free text, request ID, or per-principal information. [FI-TRACE-DENIAL-ORACLE]
pub(crate) fn denial_response(class: DenialClass) -> Response<Body> {
    let status =
        StatusCode::from_u16(class.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", class.content_type());

    if let Some(www_auth) = class.www_authenticate() {
        builder = builder.header("WWW-Authenticate", www_auth);
    }

    builder
        .body(Body::from(class.http_body()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    // ── transport parsing ─────────────────────────────────────────────────────

    #[test]
    fn absent_header_gives_missing_evidence() {
        let h = HeaderMap::new();
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::MissingEvidence)),
            "absent NIP-FI header must be MissingEvidence"
        );
    }

    #[test]
    fn repeated_header_gives_evidence_rejected() {
        let mut h = HeaderMap::new();
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer aaa.bbb.ccc"),
        );
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer ddd.eee.fff"),
        );
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "repeated NIP-FI header must be EvidenceRejected"
        );
    }

    #[test]
    fn comma_combined_gives_evidence_rejected() {
        let h = headers_with("Bearer aaa.bbb.ccc, Bearer ddd.eee.fff");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "comma-combined NIP-FI header must be EvidenceRejected"
        );
    }

    #[test]
    fn empty_value_gives_evidence_rejected() {
        let h = headers_with("");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "empty NIP-FI header must be EvidenceRejected"
        );
    }

    #[test]
    fn non_bearer_prefix_gives_evidence_rejected() {
        let h = headers_with("Token aaa.bbb.ccc");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "non-Bearer scheme must be EvidenceRejected"
        );
    }

    #[test]
    fn bearer_with_empty_token_gives_evidence_rejected() {
        let h = headers_with("Bearer ");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "empty token after Bearer must be EvidenceRejected"
        );
    }

    #[test]
    fn whitespace_in_token_gives_evidence_rejected() {
        let h = headers_with("Bearer aa bb.ccc.ddd");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "whitespace in token must be EvidenceRejected (mixed-profile)"
        );
    }

    #[test]
    fn valid_bearer_token_is_extracted() {
        let h = headers_with("Bearer eyJhbGciOiJFUzI1NiJ9.e30.sig");
        let token = extract_bearer_token(&h).expect("valid Bearer header must succeed");
        assert_eq!(token, "eyJhbGciOiJFUzI1NiJ9.e30.sig");
    }

    // ── denial response contract ──────────────────────────────────────────────
    //
    // NIP-FI requires the EXACT bytes; tests assert on exact body + headers.
    // [FI-TRACE-DENIAL-ORACLE]

    fn body_bytes(resp: Response<Body>) -> Vec<u8> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .to_vec()
            })
    }

    #[test]
    fn missing_evidence_response_is_401_with_www_authenticate() {
        let resp = denial_response(DenialClass::MissingEvidence);
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get("WWW-Authenticate")
                .and_then(|v| v.to_str().ok()),
            Some("Nostr"),
            "MissingEvidence must carry WWW-Authenticate: Nostr"
        );
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(body_bytes(resp), b"authentication required\n");
    }

    #[test]
    fn evidence_rejected_response_is_403_exact_body() {
        let resp = denial_response(DenialClass::EvidenceRejected);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(
            resp.headers().get("WWW-Authenticate").is_none(),
            "EvidenceRejected must not carry WWW-Authenticate"
        );
        assert_eq!(body_bytes(resp), b"evidence rejected\n");
    }

    #[test]
    fn authorization_denied_response_is_403_exact_body() {
        let resp = denial_response(DenialClass::AuthorizationDenied);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(body_bytes(resp), b"authorization denied\n");
    }

    #[test]
    fn authorization_unavailable_response_is_503_exact_body() {
        let resp = denial_response(DenialClass::AuthorizationUnavailable);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_bytes(resp), b"authorization unavailable\n");
    }

    #[test]
    fn private_state_denials_are_byte_identical() {
        // All private-state rows in the rejection table map to
        // authorization_denied. Their responses must be byte-identical.
        // [FI-TRACE-DENIAL-ORACLE]
        let resp_denied = denial_response(DenialClass::AuthorizationDenied);
        let another_denied = denial_response(DenialClass::AuthorizationDenied);
        assert_eq!(resp_denied.status(), another_denied.status());
        assert_eq!(body_bytes(resp_denied), body_bytes(another_denied));
    }
}
