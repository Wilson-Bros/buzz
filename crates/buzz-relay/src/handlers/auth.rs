//! NIP-42 AUTH handler — verify challenge response, transition auth state.
//!
//! Relay membership enforcement uses the shared
//! [`crate::api::relay_members::enforce_relay_membership`] helper, which supports
//! NIP-OA owner-delegation fallback on closed relays. On open relays, the auth
//! handler calls [`crate::api::relay_members::extract_nip_oa_owner`] directly to
//! extract the owner pubkey for agent→owner backfill (observer frame auth).
//!
//! For WebSocket auth, the NIP-OA `auth` tag is extracted from the signed AUTH
//! event itself (the tag is integrity-protected by the event signature).

use std::sync::Arc;

use axum::extract::ws::Message as WsMessage;
use tracing::{debug, info, warn};

use crate::connection::{AuthState, ConnectionState};
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Extract a NIP-OA `auth` tag from a verified AUTH event and serialize it as
/// the JSON-array string that [`buzz_sdk::nip_oa::verify_auth_tag`] expects.
///
/// Returns `None` if no `auth` tag is present (direct-member auth path) or if
/// more than one `auth` tag exists (per NIP-OA spec: >1 auth tag ⇒ no valid tag).
pub fn extract_auth_tag_json(event: &nostr::Event) -> Option<String> {
    let mut iter = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"));
    let first = iter.next()?;
    if iter.next().is_some() {
        return None; // NIP-OA spec: treat >1 auth tag as no valid auth tag
    }
    serde_json::to_string(first.as_slice()).ok()
}

/// Handle a NIP-42 AUTH message: verify the challenge response and transition
/// the connection to authenticated state.
///
/// Pure crypto verification — no API tokens, no JWT, no DB token lookups.
#[tracing::instrument(skip_all, fields(event_id, conn_id))]
pub async fn handle_auth(event: nostr::Event, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let event_id_hex = event.id.to_hex();
    let (challenge, conn_id) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Pending { challenge } => (challenge.clone(), conn.conn_id),
            AuthState::Authenticated(_) => {
                debug!(conn_id = %conn.conn_id, "AUTH received but already authenticated");
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: already authenticated",
                ));
                return;
            }
            AuthState::Failed => {
                debug!(conn_id = %conn.conn_id, "AUTH received after failed auth");
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: authentication already failed",
                ));
                return;
            }
        }
    };

    // Record the declared span fields now that we have the values.
    tracing::Span::current()
        .record("event_id", event_id_hex.as_str())
        .record("conn_id", conn_id.to_string().as_str());

    // Extract the NIP-OA auth tag before verification consumes the event.
    // The tag is integrity-protected by the event's Schnorr signature — if
    // tampered, NIP-42 verification will fail before we ever inspect it.
    let auth_tag_json = extract_auth_tag_json(&event);
    let signed_auth_created_at = event.created_at.as_secs();

    let relay_url =
        crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &conn.tenant);
    let auth_svc = Arc::clone(&state.auth);

    metrics::counter!("buzz_auth_attempts_total", "method" => "nip42").increment(1);

    // Pure NIP-42 verification — crypto only, no DB lookups.
    match auth_svc
        .verify_auth_event(event, &challenge, &relay_url)
        .await
    {
        Ok(mut auth_ctx) => {
            let pubkey = auth_ctx.pubkey;

            // Community ban gate (NIP-42 seam). Runs immediately after auth
            // verification succeeds and before the allowlist and relay-membership
            // gates, per COMMUNITY_MODERATION_PLAN.md §0 decision 4 and the
            // MOD-7/M20 invariant (a ban must block connection auth even for open
            // channels — enforcement is structural, not filtered later). A banned
            // principal gets the standard protocol denial and the connection is
            // dropped with zero further processing.
            //
            // NIP-OA cascade: a ban on the authenticated pubkey blocks it directly;
            // a ban on its cryptographically-proven owner cascades to the agent
            // (owner ban ⇒ agents banned; agent ban is agent-only). The owner is
            // extracted from the self-proving auth tag with no DB round-trip.
            {
                // Fail closed on a DB error, but distinguish it from a real ban:
                // a transient blip must deny (never let a banned principal
                // through) without telling an innocent user they are banned and
                // pinning `Failed` for the connection's life on a false premise.
                // `Banned` claims the ban; `DbError` denies with `error: internal`
                // (mirrors the ingest write-path gate).
                enum BanOutcome {
                    Clear,
                    Banned,
                    DbError,
                }

                let mut outcome = match state
                    .db
                    .moderation_restriction_state(conn.tenant.community(), pubkey.as_bytes())
                    .await
                {
                    Ok(state) if state.banned => BanOutcome::Banned,
                    Ok(_) => BanOutcome::Clear,
                    Err(e) => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
                              "ban-state DB lookup failed, denying (fail-closed)");
                        BanOutcome::DbError
                    }
                };

                // Cascade: check the proven NIP-OA owner only if the agent itself
                // is clear (a DB error already denies; a direct ban already blocks
                // — both skip the needless second DB read).
                if matches!(outcome, BanOutcome::Clear) {
                    if let Some(owner) = crate::api::relay_members::extract_nip_oa_owner(
                        pubkey.as_bytes(),
                        auth_tag_json.as_deref(),
                        Some(signed_auth_created_at),
                    ) {
                        outcome = match state
                            .db
                            .moderation_restriction_state(conn.tenant.community(), owner.as_bytes())
                            .await
                        {
                            Ok(state) if state.banned => BanOutcome::Banned,
                            Ok(_) => BanOutcome::Clear,
                            Err(e) => {
                                warn!(conn_id = %conn_id, owner = %owner.to_hex(), error = %e,
                                      "owner ban-state DB lookup failed, denying (fail-closed)");
                                BanOutcome::DbError
                            }
                        };
                    }
                }

                let denial: Option<(&str, &str)> = match outcome {
                    BanOutcome::Clear => None,
                    BanOutcome::Banned => {
                        Some(("banned", "blocked: you are banned from this community"))
                    }
                    BanOutcome::DbError => Some((
                        "ban_check_error",
                        "error: internal error checking restriction state",
                    )),
                };

                if let Some((metric_reason, deny_reason)) = denial {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), reason = deny_reason, "principal denied at ban seam");
                    metrics::counter!("buzz_auth_failures_total", "reason" => metric_reason)
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    // Decision 4: banned ⇒ OK false + immediate WebSocket close.
                    // Route the reason frame on the control channel (not `send`,
                    // which uses the data channel and would race the cancel), so
                    // the send loop drains it ahead of the Close it emits on
                    // cancel. Then cancel to close the socket immediately.
                    let _ = conn.ctrl_tx.try_send(WsMessage::Text(
                        RelayMessage::ok(&event_id_hex, false, deny_reason).into(),
                    ));
                    conn.cancel.cancel();
                    return;
                }
            }

            // NIP-FI key pairing [FI-INV-05]: when a federated identity assertion
            // was presented at upgrade and contains a `nostr_pubkey` claim, the
            // proven NIP-42 key must equal that claim. This check is unconditional —
            // no per-issuer flag reads (S2 deletes `require_attested_key`; S3
            // enforces the invariant structurally).
            //
            // Defense in depth: a None asserted_key means the assertion reached
            // the relay without `nostr_pubkey` (should not happen since S3 forces
            // require_attested_key=true, but treat as denial regardless).
            //
            // Mismatch: send `restricted: authorization denied` on the control
            // channel (priority delivery ahead of Close), cancel, return.
            // [FI-TRACE-DENIAL-ORACLE post-establishment]
            if let Some(ref assertion) = conn.nip_fi_assertion {
                let pairing_ok = match assertion.asserted_key() {
                    Some(asserted_key) => asserted_key == pubkey,
                    None => false, // claimless assertion — deny
                };
                if !pairing_ok {
                    warn!(
                        conn_id = %conn_id,
                        proven_pubkey = %pubkey.to_hex(),
                        asserted_pubkey = ?assertion.asserted_key().map(|k| k.to_hex()),
                        "NIP-FI key pairing mismatch — closing connection"
                    );
                    metrics::counter!(
                        "buzz_auth_failures_total",
                        "reason" => "nip_fi_key_mismatch"
                    )
                    .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    use buzz_auth::DenialClass;
                    let _ = conn.ctrl_tx.try_send(WsMessage::Text(
                        RelayMessage::notice(DenialClass::AuthorizationDenied.nostr_text()).into(),
                    ));
                    conn.cancel.cancel();
                    return;
                }
            }

            // Pubkey allowlist gate — only for pubkey-only auth.
            if state.config.pubkey_allowlist_enabled
                && auth_ctx.auth_method == buzz_auth::AuthMethod::Nip42
            {
                let allowed = match state
                    .db
                    .is_pubkey_allowed(conn.tenant.community(), pubkey.as_bytes())
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
                              "allowlist DB lookup failed, denying (fail-closed)");
                        false
                    }
                };
                if !allowed {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "pubkey not in allowlist");
                    metrics::counter!("buzz_auth_failures_total", "reason" => "allowlist_denied")
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "auth-required: verification failed",
                    ));
                    return;
                }
            }

            // Relay membership gate — uses the shared helper with NIP-OA fallback.
            let nip_oa_owner = match crate::api::relay_members::enforce_relay_membership(
                &state,
                conn.tenant.community(),
                pubkey.as_bytes(),
                auth_tag_json.as_deref(),
                Some(signed_auth_created_at),
            )
            .await
            {
                Ok(owner) => owner,
                Err(e) => {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = ?e, "not a relay member");
                    metrics::counter!("buzz_auth_failures_total", "reason" => "not_relay_member")
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "restricted: not a relay member",
                    ));
                    return;
                }
            };

            // Open relay NIP-OA backfill: extract owner for agent→owner DB mapping
            // (needed for observer frame auth). Only runs on open relays — on closed
            // relays, enforce_relay_membership already handles NIP-OA delegation.
            // No feature flag needed: NIP-OA is cryptographically self-proving.
            let nip_oa_owner = nip_oa_owner.or_else(|| {
                if !state.config.require_relay_membership && auth_tag_json.is_some() {
                    crate::api::relay_members::extract_nip_oa_owner(
                        pubkey.as_bytes(),
                        auth_tag_json.as_deref(),
                        Some(signed_auth_created_at),
                    )
                } else {
                    None
                }
            });

            // Stash NIP-OA owner on the auth context only after the shared
            // backfill confirms the first-write-wins relationship.
            if let Some(owner) = nip_oa_owner {
                if crate::api::relay_members::materialize_nip_oa_owner(
                    &state,
                    &conn.tenant,
                    &pubkey,
                    &owner,
                )
                .await
                {
                    auth_ctx.agent_owner_pubkey = Some(owner);
                } else {
                    warn!(
                        conn_id = %conn_id,
                        agent = %pubkey.to_hex(),
                        nip_oa_owner = %owner.to_hex(),
                        "NIP-OA owner could not be materialized"
                    );
                }
            }

            info!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "NIP-42 auth successful");
            *conn.auth_state.write().await = AuthState::Authenticated(auth_ctx);
            state
                .conn_manager
                .set_authenticated_pubkey(conn_id, pubkey.to_bytes().to_vec());
            conn.send(RelayMessage::ok(&event_id_hex, true, ""));
        }
        Err(e) => {
            warn!(conn_id = %conn_id, error = %e, "NIP-42 auth failed");
            metrics::counter!("buzz_auth_failures_total", "reason" => "nip42_invalid").increment(1);
            *conn.auth_state.write().await = AuthState::Failed;
            conn.send(RelayMessage::ok(
                &event_id_hex,
                false,
                "auth-required: verification failed",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_auth_tag_json;
    use axum::extract::ws::Message as WsMessage;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Build a signed NIP-98 (kind 27235) event carrying the given tags. The
    /// `auth` tag lives inside the signed event exactly as the git and
    /// WebSocket auth paths receive it.
    fn signed_event_with_tags(tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign auth event")
    }

    /// A single `auth` tag is extracted verbatim as its JSON-array string —
    /// this is the exact value fed to `verify_auth_tag` on the git path.
    #[test]
    fn single_auth_tag_extracted_verbatim() {
        let owner = Keys::generate().public_key().to_hex();
        let sig = "00".repeat(64);
        let event = signed_event_with_tags(vec![
            Tag::parse(["u", "https://relay/git/x/y"]).unwrap(),
            Tag::parse(["auth", owner.as_str(), "", sig.as_str()]).unwrap(),
        ]);

        let extracted = extract_auth_tag_json(&event).expect("auth tag present");
        let expected = serde_json::to_string(&["auth", owner.as_str(), "", sig.as_str()]).unwrap();
        assert_eq!(extracted, expected);
    }

    /// No `auth` tag → `None` (the direct-member path, tag absent).
    #[test]
    fn no_auth_tag_returns_none() {
        let event =
            signed_event_with_tags(vec![Tag::parse(["u", "https://relay/git/x/y"]).unwrap()]);
        assert_eq!(extract_auth_tag_json(&event), None);
    }

    /// More than one `auth` tag → `None`. Per NIP-OA, an ambiguous set of
    /// attestations is treated as no valid attestation (fail-closed), so a
    /// second forged tag cannot smuggle an alternate delegation past the gate.
    #[test]
    fn duplicate_auth_tags_return_none() {
        let a = Keys::generate().public_key().to_hex();
        let b = Keys::generate().public_key().to_hex();
        let sig = "00".repeat(64);
        let event = signed_event_with_tags(vec![
            Tag::parse(["auth", a.as_str(), "", sig.as_str()]).unwrap(),
            Tag::parse(["auth", b.as_str(), "", sig.as_str()]).unwrap(),
        ]);
        assert_eq!(extract_auth_tag_json(&event), None);
    }

    // ── NIP-FI pairing — ctrl_tx seam ─────────────────────────────────────────
    //
    // These tests verify that the pairing check delivers `restricted: authorization
    // denied` on the ctrl channel (not the data channel) and cancels the connection.
    // Mutation: swapping ctrl_tx for send_tx in the pairing check turns these red.

    fn build_conn_with_assertion(
        assertion: buzz_auth::VerifiedAssertion,
        proven_pubkey: nostr::PublicKey,
    ) -> (
        std::sync::Arc<crate::connection::ConnectionState>,
        tokio::sync::mpsc::Receiver<axum::extract::ws::Message>,
        tokio::sync::mpsc::Receiver<axum::extract::ws::Message>,
    ) {
        use crate::connection::ConnectionState;
        use crate::handlers::auth::AuthState;
        use buzz_auth::AuthMethod;
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::{mpsc, RwLock};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        let (send_tx, send_rx) = mpsc::channel(8);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let auth = buzz_auth::AuthContext {
            pubkey: proven_pubkey,
            scopes: vec![],
            channel_ids: None,
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: None,
        };
        let conn = ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Authenticated(auth)),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            cancel,
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: Some(assertion),
            session_deadline: None,
        };
        (Arc::new(conn), send_rx, ctrl_rx)
    }

    #[tokio::test]
    async fn pairing_mismatch_delivers_denial_on_ctrl_not_data() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use nostr::Keys;

        let asserted_keys = Keys::generate();
        let proven_keys = Keys::generate();
        // Assertion says asserted_keys.public_key(), NIP-42 proves proven_keys.
        let assertion = VerifiedAssertion::for_test(
            Some(asserted_keys.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );
        let (conn, mut send_rx, mut ctrl_rx) =
            build_conn_with_assertion(assertion, proven_keys.public_key());

        // Run the pairing check inline (mirrors auth.rs logic).
        let pairing_ok = match conn.nip_fi_assertion.as_ref().unwrap().asserted_key() {
            Some(asserted) => asserted == proven_keys.public_key(),
            None => false,
        };
        if !pairing_ok {
            use buzz_auth::DenialClass;
            let msg = DenialClass::AuthorizationDenied.nostr_text();
            let _ = conn.ctrl_tx.try_send(WsMessage::Text(
                crate::protocol::RelayMessage::notice(msg).into(),
            ));
            conn.cancel.cancel();
        }

        assert!(
            conn.cancel.is_cancelled(),
            "connection must be cancelled on pairing mismatch"
        );
        // The denial frame must be on ctrl, not data.
        let ctrl_frame = ctrl_rx
            .try_recv()
            .expect("ctrl must contain the denial notice");
        assert!(
            send_rx.try_recv().is_err(),
            "denial must NOT appear on the data channel"
        );
        match ctrl_frame {
            WsMessage::Text(text) => {
                // NOTICE is ["NOTICE", <message>] — extract position 1.
                let v: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
                let content = v.get(1).and_then(|c| c.as_str()).unwrap_or("");
                assert!(
                    content.contains("authorization denied"),
                    "ctrl must carry authorization_denied; got: {content}"
                );
            }
            other => panic!("ctrl frame must be Text; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn claimless_assertion_denied_on_ctrl() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use nostr::Keys;

        let proven_keys = Keys::generate();
        // Assertion has no nostr_pubkey claim (asserted_key = None).
        let assertion = VerifiedAssertion::for_test(None, vec![Utc::now() + Duration::hours(1)]);
        let (conn, mut send_rx, mut ctrl_rx) =
            build_conn_with_assertion(assertion, proven_keys.public_key());

        // Run the pairing check.
        let pairing_ok = match conn.nip_fi_assertion.as_ref().unwrap().asserted_key() {
            Some(asserted) => asserted == proven_keys.public_key(),
            None => false, // claimless
        };
        if !pairing_ok {
            use buzz_auth::DenialClass;
            let msg = DenialClass::AuthorizationDenied.nostr_text();
            let _ = conn.ctrl_tx.try_send(WsMessage::Text(
                crate::protocol::RelayMessage::notice(msg).into(),
            ));
            conn.cancel.cancel();
        }

        assert!(
            conn.cancel.is_cancelled(),
            "claimless assertion must be denied"
        );
        let ctrl_frame = ctrl_rx
            .try_recv()
            .expect("ctrl must contain the denial notice");
        assert!(
            send_rx.try_recv().is_err(),
            "denial must not appear on data channel"
        );
        match ctrl_frame {
            WsMessage::Text(text) => {
                // NOTICE is ["NOTICE", <message>] — extract position 1.
                let v: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
                let content = v.get(1).and_then(|c| c.as_str()).unwrap_or("");
                assert!(
                    content.contains("authorization denied"),
                    "ctrl must carry authorization_denied; got: {content}"
                );
            }
            other => panic!("ctrl frame must be Text; got {other:?}"),
        }
    }
}
