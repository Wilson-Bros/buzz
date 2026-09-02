//! Shared NIP-FI post-upgrade session seams.
//!
//! This module owns:
//!
//! * [`NipFiWsRoute`] — route discriminant for frame construction and logging.
//! * [`enforce_nip_fi_key_pairing`] — the single production function that owns
//!   the full NIP-FI key-pairing verdict, denial frame delivery, metric,
//!   auth-state transition (Root), and cancellation for both ingresses.
//! * [`spawn_nip_fi_expiry_task`] — the shared session-lifetime enforcement
//!   constructor used by both root and audio routes.
//! * [`authorization_denied_frame`] — route-specific frame builder used by
//!   both the pairing seam and the expiry seam.
//!
//! **Invariant**: both production call sites call `enforce_nip_fi_key_pairing`
//! and `spawn_nip_fi_expiry_task` from this module; no caller may re-implement
//! these side effects.

use axum::extract::ws::Message as WsMessage;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use crate::connection::ConnectionState;

// ── Route discriminant ────────────────────────────────────────────────────────

/// Which ingress a session is on. Governs denial frame format and log labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NipFiWsRoute {
    Root,
    Audio,
}

// ── Pairing seam ──────────────────────────────────────────────────────────────

/// Outcome of [`enforce_nip_fi_key_pairing`].
///
/// Callers MUST return immediately on `Denied`; all denial side-effects have
/// already been performed inside the function.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PairingOutcome {
    Paired,
    Denied,
}

/// Route-specific resources needed to deliver the pairing denial.
pub(crate) enum PairingDenialTarget<'a> {
    Root(&'a ConnectionState),
    Audio {
        ws_send: &'a mut futures_util::stream::SplitSink<
            axum::extract::ws::WebSocket,
            axum::extract::ws::Message,
        >,
        cancel: &'a CancellationToken,
        channel_id: Uuid,
    },
}

/// Enforce the NIP-FI key-pairing invariant [FI-INV-05].
///
/// When an assertion was presented at upgrade, the proven NIP-42 key MUST equal
/// the assertion's `nostr_pubkey` claim; a claimless assertion is also a denial.
///
/// This function owns the **entire denial path**: verdict, route-specific denial
/// frame delivery, `buzz_auth_failures_total{reason="nip_fi_key_mismatch"}`,
/// a route-labelled warning (no `iss`/`sub`/raw-assertion fields), auth-state
/// transition (Root only), and cancellation. Callers must not repeat any of
/// those effects.
///
/// Returns [`PairingOutcome::Paired`] when:
/// * no assertion is present (off-mode), or
/// * the assertion's `nostr_pubkey` claim matches `proven_pubkey`.
///
/// Returns [`PairingOutcome::Denied`] after performing all denial side-effects.
pub(crate) async fn enforce_nip_fi_key_pairing(
    assertion: Option<&buzz_auth::VerifiedAssertion>,
    proven_pubkey: nostr::PublicKey,
    target: PairingDenialTarget<'_>,
) -> PairingOutcome {
    // No assertion → off-mode; pass unconditionally.
    let Some(assertion) = assertion else {
        return PairingOutcome::Paired;
    };

    // Matching key → pass.
    if matches!(assertion.asserted_key(), Some(k) if k == proven_pubkey) {
        return PairingOutcome::Paired;
    }

    // Mismatch or claimless assertion — single shared denial branch.
    metrics::counter!(
        "buzz_auth_failures_total",
        "reason" => "nip_fi_key_mismatch"
    )
    .increment(1);

    match target {
        PairingDenialTarget::Root(conn) => {
            warn!(
                conn_id = %conn.conn_id,
                route = "root",
                proven_pubkey = %proven_pubkey.to_hex(),
                "NIP-FI key pairing mismatch — closing connection"
            );
            *conn.auth_state.write().await = crate::connection::AuthState::Failed;
            let _ = conn
                .ctrl_tx
                .try_send(authorization_denied_frame(NipFiWsRoute::Root));
            conn.cancel.cancel();
        }
        PairingDenialTarget::Audio {
            ws_send,
            cancel,
            channel_id,
        } => {
            warn!(
                %channel_id,
                route = "audio",
                proven_pubkey = %proven_pubkey.to_hex(),
                "NIP-FI key pairing mismatch — closing connection"
            );
            use futures_util::SinkExt as _;
            let _ = ws_send
                .send(authorization_denied_frame(NipFiWsRoute::Audio))
                .await;
            cancel.cancel();
        }
    }

    PairingOutcome::Denied
}

// ── Shared frame constructor ───────────────────────────────────────────────────

/// Build the exact NIP-FI authorization-denied frame for the given route.
///
/// * Root: a Nostr NOTICE — `["NOTICE","restricted: authorization denied"]`.
/// * Audio: `{"type":"restricted","message":"restricted: authorization denied"}`.
pub(crate) fn authorization_denied_frame(route: NipFiWsRoute) -> WsMessage {
    use buzz_auth::DenialClass;
    let text = DenialClass::AuthorizationDenied.nostr_text();
    WsMessage::Text(match route {
        NipFiWsRoute::Root => crate::protocol::RelayMessage::notice(text).into(),
        NipFiWsRoute::Audio => serde_json::json!({"type": "restricted", "message": text})
            .to_string()
            .into(),
    })
}

// ── Shared expiry task constructor ────────────────────────────────────────────

/// Spawn the NIP-FI session-lifetime enforcement task for either route.
///
/// At `deadline`, the task (in this exact order):
/// 1. Enqueues [`authorization_denied_frame(route)`] on `ctrl_tx`.
/// 2. Increments `buzz_nip_fi_lease_expirations_total` and warns with route.
/// 3. Calls `cancel.cancel()` — **unconditional**, regardless of queue success.
///
/// The queue-then-cancel ordering is contractual: the send loop's cancellation
/// branch drains `ctrl_rx` before writing `Close`, so the observable wire order
/// is the route-specific denial frame followed by `Close`.
///
/// Equality at deadline is expired; already-expired deadlines fire immediately.
/// No in-band renewal is added. [FI-TRACE-LEASE-BOUND]
pub(crate) fn spawn_nip_fi_expiry_task(
    deadline: chrono::DateTime<chrono::Utc>,
    ctrl_tx: mpsc::Sender<WsMessage>,
    cancel: CancellationToken,
    route: NipFiWsRoute,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let now = chrono::Utc::now();
        // Equality at deadline is expired: strict less-than.
        let remaining = if now < deadline {
            (deadline - now)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO)
        } else {
            std::time::Duration::ZERO
        };
        tokio::select! {
            _ = tokio::time::sleep(remaining) => {
                // 1. Queue denial frame BEFORE cancel so the send loop drains
                //    it ahead of the Close it emits on cancellation.
                let _ = ctrl_tx.try_send(authorization_denied_frame(route));
                // 2. Metric + warning (no private assertion fields).
                metrics::counter!("buzz_nip_fi_lease_expirations_total").increment(1);
                warn!(
                    route = ?route,
                    "NIP-FI session lease expired — closing connection"
                );
                // 3. Cancel — unconditional.
                cancel.cancel();
            }
            _ = cancel.cancelled() => {}
        }
    })
}
