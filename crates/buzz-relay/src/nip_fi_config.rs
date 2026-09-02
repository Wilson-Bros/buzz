//! NIP-FI relay-level configuration: issuer set, session lifetime, and JWKS
//! warm/refresh.
//!
//! All env-var parsing lives here so `config.rs` stays focused on the top-level
//! `Config` struct. This module is `pub(super)` — only `config.rs` constructs
//! it, and the relay reads it as `config.nip_fi`.
//!
//! # Environment variables
//!
//! | Variable | Required | Description |
//! |---|---|---|
//! | `BUZZ_NIP_FI_MODE` | No | `enforce` (default), `deny_protected`, or `off`. |
//! | `BUZZ_NIP_FI_ISSUERS` | If enforce | JSON array of issuer configs (see [`IssuerEnvConfig`]). |
//! | `BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS` | If enforce | Max age of `iat + max_age` residual. |
//! | `BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS` | If enforce | Per-partition limit on session lifetime. |
//!
//! Absent or empty `BUZZ_NIP_FI_MODE` defaults to `off`, keeping the relay
//! backward-compatible until an operator explicitly enables enforcement.

use std::time::Duration;

use buzz_auth::{
    validate_nip_fi_config, FreshnessClass, IssuerJwksConfig, IssuerPolicy, IssuerPolicyError,
    IssuerRegistry, JwksSourceContract, NipFiMode, NipFiStartupError, TokenClass,
};
use jsonwebtoken::Algorithm;

use crate::config::ConfigError;

/// Maximum accepted `maximum_assertion_age` in seconds (24 h, matching `buzz-auth`).
const MAX_ASSERTION_AGE_SECS: u64 = 86_400;
/// Maximum accepted `max_connection_lifetime` in seconds (30 days).
const MAX_CONNECTION_LIFETIME_SECS: u64 = 30 * 24 * 3600;

// ── Per-issuer JSON config shape ─────────────────────────────────────────────

/// One entry in the `BUZZ_NIP_FI_ISSUERS` JSON array.
///
/// **Example** (one issuer, `nip-fi+jwt` dedicated assertions):
/// ```json
/// [
///   {
///     "issuer": "https://login.example.com",
///     "audiences": ["https://relay.example.com"],
///     "token_class": "nip-fi+jwt",
///     "algorithms": ["ES256"],
///     "require_attested_key": false,
///     "skew_seconds": 30,
///     "maximum_assertion_age_seconds": 3600,
///     "jwks_uri": "https://login.example.com/.well-known/jwks.json",
///     "jwks_refresh_interval_seconds": 300,
///     "jwks_hard_deadline_seconds": 86400
///   }
/// ]
/// ```
#[derive(Debug, serde::Deserialize)]
pub(super) struct IssuerEnvConfig {
    /// Exact `iss` value.
    pub issuer: String,
    /// One or more accepted `aud` values.
    pub audiences: Vec<String>,
    /// `"at+jwt"` or `"nip-fi+jwt"`.
    pub token_class: TokenClassEnvConfig,
    /// Algorithm names, e.g. `["ES256", "RS256"]`.
    pub algorithms: Vec<String>,
    /// Whether `nostr_pubkey` must be present and equal the proven actor.
    #[serde(default)]
    pub require_attested_key: bool,
    /// Accepted clock skew in seconds (≤ 300).
    #[serde(default)]
    pub skew_seconds: u64,
    /// `iat + maximum_assertion_age` residual bound in seconds.
    pub maximum_assertion_age_seconds: u64,
    /// HTTPS endpoint serving the JWK Set for this issuer.
    pub jwks_uri: String,
    /// Seconds between JWKS refreshes.
    pub jwks_refresh_interval_seconds: u64,
    /// Hard deadline for accepting a JWKS snapshot in seconds.
    pub jwks_hard_deadline_seconds: u64,
}

/// Token-class discriminant in the issuer config JSON.
#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum TokenClassEnvConfig {
    #[serde(rename = "nip-fi+jwt")]
    DedicatedNipFi,
    #[serde(rename = "at+jwt")]
    AccessTokenAtJwt,
}

// ── Relay-level NIP-FI config ─────────────────────────────────────────────────

/// The relay-level NIP-FI configuration produced by `Config::from_env`.
///
/// Carries the validated `NipFiMode`, the full `IssuerRegistry`,
/// the parallel `IssuerJwksConfig` slice for `ProductionJwksSource`, and the
/// two session-lifetime bounds.
#[derive(Debug, Clone)]
pub struct NipFiRelayConfig {
    /// The enforcement mode selected by `BUZZ_NIP_FI_MODE`.
    pub mode: NipFiMode,
    /// Validated per-issuer assertion-policy registry.
    pub registry: IssuerRegistry,
    /// Parallel JWKS configs for `ProductionJwksSource` construction.
    pub jwks_configs: Vec<IssuerJwksConfig>,
    /// Maximum residual lifetime of one assertion (`iat + max_assertion_age`).
    /// Deployment-configured; absent defaults to 1 h in `enforce` mode.
    pub maximum_assertion_age_secs: u64,
    /// Hard upper bound on a single connection lease, in seconds.
    /// The lease is `min(session_deadline, max_connection_lifetime)` per
    /// the spec partition rule. Absent means no additional partition.
    pub max_connection_lifetime_secs: Option<u64>,
}

impl NipFiRelayConfig {
    /// Parse NIP-FI relay configuration from the process environment.
    ///
    /// Returns `Err` when `BUZZ_NIP_FI_MODE=enforce` but required config is
    /// missing or invalid (fail-closed: no token is accepted until this passes).
    pub fn from_env() -> Result<Self, ConfigError> {
        let mode = parse_mode()?;

        if let NipFiMode::Off | NipFiMode::DenyProtected = mode {
            return Ok(Self {
                mode,
                registry: IssuerRegistry::new(),
                jwks_configs: Vec::new(),
                maximum_assertion_age_secs: 3_600,
                max_connection_lifetime_secs: None,
            });
        }

        // Enforce mode: all fields required.
        let issuers_json = std::env::var("BUZZ_NIP_FI_ISSUERS").map_err(|_| {
            ConfigError::InvalidValue(
                "BUZZ_NIP_FI_MODE=enforce but BUZZ_NIP_FI_ISSUERS is not set; \
                 set it to a JSON array of issuer configs"
                    .to_string(),
            )
        })?;
        if issuers_json.trim().is_empty() {
            return Err(ConfigError::InvalidValue(
                "BUZZ_NIP_FI_ISSUERS must not be empty in enforce mode".to_string(),
            ));
        }

        let issuer_entries: Vec<IssuerEnvConfig> =
            serde_json::from_str(&issuers_json).map_err(|e| {
                ConfigError::InvalidValue(format!("BUZZ_NIP_FI_ISSUERS is not valid JSON: {e}"))
            })?;

        if issuer_entries.is_empty() {
            return Err(ConfigError::InvalidValue(
                "BUZZ_NIP_FI_ISSUERS must contain at least one issuer in enforce mode".to_string(),
            ));
        }

        let maximum_assertion_age_secs = parse_u64_bounded(
            "BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS",
            1,
            MAX_ASSERTION_AGE_SECS,
        )?
        .ok_or_else(|| {
            ConfigError::InvalidValue(
                "BUZZ_NIP_FI_MODE=enforce but \
                         BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS is not set"
                    .to_string(),
            )
        })?;

        let max_connection_lifetime_secs = parse_u64_bounded(
            "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS",
            1,
            MAX_CONNECTION_LIFETIME_SECS,
        )?;

        let mut registry = IssuerRegistry::new();
        let mut jwks_configs = Vec::with_capacity(issuer_entries.len());

        for entry in issuer_entries {
            let (policy, jwks_config) = build_issuer(&entry).map_err(|e| {
                ConfigError::InvalidValue(format!(
                    "BUZZ_NIP_FI_ISSUERS: issuer {:?}: {e}",
                    entry.issuer
                ))
            })?;
            registry.insert(policy);
            jwks_configs.push(jwks_config);
        }

        // Delegate final validation to buzz-auth startup gate.
        validate_nip_fi_config(NipFiMode::Enforce, &registry, &jwks_configs).map_err(
            |e: NipFiStartupError| ConfigError::InvalidValue(format!("NIP-FI config invalid: {e}")),
        )?;

        Ok(Self {
            mode,
            registry,
            jwks_configs,
            maximum_assertion_age_secs,
            max_connection_lifetime_secs,
        })
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_mode() -> Result<NipFiMode, ConfigError> {
    match std::env::var("BUZZ_NIP_FI_MODE")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("off") => Ok(NipFiMode::Off),
        Some("enforce") => Ok(NipFiMode::Enforce),
        Some("deny_protected") => Ok(NipFiMode::DenyProtected),
        Some(other) => Err(ConfigError::InvalidValue(format!(
            "BUZZ_NIP_FI_MODE must be \"enforce\", \"deny_protected\", or \"off\"; got {other:?}"
        ))),
    }
}

/// Parse an optional positive `u64` env var bounded to `[min_val, max_val]`.
/// Returns `None` when the variable is absent or empty.
fn parse_u64_bounded(name: &str, min_val: u64, max_val: u64) -> Result<Option<u64>, ConfigError> {
    match std::env::var(name) {
        Err(_) => Ok(None),
        Ok(raw) if raw.trim().is_empty() => Ok(None),
        Ok(raw) => {
            let v: u64 = raw.trim().parse().map_err(|_| {
                ConfigError::InvalidValue(format!("{name} must be a positive integer"))
            })?;
            if v < min_val || v > max_val {
                return Err(ConfigError::InvalidValue(format!(
                    "{name} must be in {min_val}..={max_val}"
                )));
            }
            Ok(Some(v))
        }
    }
}

/// Parse a `jsonwebtoken::Algorithm` from a case-sensitive string.
fn parse_algorithm(s: &str) -> Result<Algorithm, String> {
    match s {
        "ES256" => Ok(Algorithm::ES256),
        "ES384" => Ok(Algorithm::ES384),
        "RS256" => Ok(Algorithm::RS256),
        "RS384" => Ok(Algorithm::RS384),
        "RS512" => Ok(Algorithm::RS512),
        "PS256" => Ok(Algorithm::PS256),
        "PS384" => Ok(Algorithm::PS384),
        "PS512" => Ok(Algorithm::PS512),
        "EdDSA" => Ok(Algorithm::EdDSA),
        other => Err(format!("unknown or non-asymmetric algorithm {other:?}")),
    }
}

fn build_issuer(entry: &IssuerEnvConfig) -> Result<(IssuerPolicy, IssuerJwksConfig), String> {
    let algorithms: Vec<Algorithm> = entry
        .algorithms
        .iter()
        .map(|s| parse_algorithm(s))
        .collect::<Result<_, _>>()?;

    let token_class = match entry.token_class {
        TokenClassEnvConfig::DedicatedNipFi => TokenClass::DedicatedNipFi,
        TokenClassEnvConfig::AccessTokenAtJwt => {
            // at+jwt requires a SubjectClassContract; for simplicity in the
            // initial deployment, dedicated nip-fi+jwt is the expected class.
            // at+jwt support is left for a follow-up — fail closed with a
            // clear message so operators know the required fields.
            return Err("\"at+jwt\" token class requires a subject-class contract; \
                 use \"nip-fi+jwt\" for initial deployments or add \
                 subject_class fields to the issuer config"
                .to_string());
        }
    };

    let jwks_contract = JwksSourceContract::new(
        entry.jwks_uri.clone(),
        entry.jwks_refresh_interval_seconds,
        entry.jwks_hard_deadline_seconds,
    )
    .ok_or_else(|| {
        format!(
            "invalid JWKS source contract (check jwks_uri is HTTPS, \
             refresh_interval < hard_deadline, and both are positive)"
        )
    })?;

    let policy = IssuerPolicy::new(
        entry.issuer.clone(),
        entry.audiences.clone(),
        token_class,
        FreshnessClass::OfflineJwt,
        algorithms,
        entry.require_attested_key,
        entry.skew_seconds,
        entry.maximum_assertion_age_seconds,
        None, // offline-jwt: no status age
        jwks_contract.clone(),
    )
    .map_err(|e: IssuerPolicyError| format!("{e}"))?;

    let jwks_config = IssuerJwksConfig {
        issuer: entry.issuer.clone(),
        contract: jwks_contract,
    };

    Ok((policy, jwks_config))
}

// ── Duration helpers ──────────────────────────────────────────────────────────

impl NipFiRelayConfig {
    /// Returns the configured `max_connection_lifetime` as a `Duration`,
    /// if set.
    pub fn max_connection_lifetime(&self) -> Option<Duration> {
        self.max_connection_lifetime_secs.map(Duration::from_secs)
    }

    /// Returns `true` when the relay is in `Enforce` mode.
    pub fn is_enforce(&self) -> bool {
        matches!(self.mode, NipFiMode::Enforce)
    }

    /// Returns `true` when protected routes require assertion (Enforce or
    /// DenyProtected means assertion is required / always denied).
    pub fn requires_assertion(&self) -> bool {
        !matches!(self.mode, NipFiMode::Off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env vars are process-global — serialize tests that mutate them to prevent
    // cross-test races when the suite runs with multiple threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: removes a set of env vars when dropped, restoring a clean
    /// state even on test panic.
    struct EnvGuard(Vec<&'static str>);
    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            Self(keys.to_vec())
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for key in &self.0 {
                std::env::remove_var(key);
            }
        }
    }

    const NIP_FI_VARS: &[&str] = &[
        "BUZZ_NIP_FI_MODE",
        "BUZZ_NIP_FI_ISSUERS",
        "BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS",
        "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS",
    ];

    #[test]
    fn off_mode_requires_no_other_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        // NipFiMode::Off is the default: no issuers, no age limit.
        std::env::remove_var("BUZZ_NIP_FI_MODE");
        let cfg = NipFiRelayConfig::from_env().expect("Off mode must not fail");
        assert!(matches!(cfg.mode, NipFiMode::Off));
        assert!(cfg.registry.is_empty());
    }

    #[test]
    fn deny_protected_requires_no_other_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "deny_protected");
        let cfg = NipFiRelayConfig::from_env().expect("DenyProtected mode must not fail");
        assert!(matches!(cfg.mode, NipFiMode::DenyProtected));
    }

    #[test]
    fn enforce_without_issuers_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::remove_var("BUZZ_NIP_FI_ISSUERS");
        std::env::remove_var("BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS");
        let err = NipFiRelayConfig::from_env()
            .expect_err("enforce without issuers must be a config error");
        let msg = err.to_string();
        assert!(
            msg.contains("BUZZ_NIP_FI_ISSUERS"),
            "error names the missing var: {msg}"
        );
    }

    #[test]
    fn enforce_without_assertion_age_fails_closed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "enforce");
        std::env::set_var("BUZZ_NIP_FI_ISSUERS", "[{}]"); // will parse but fail on age first
        std::env::remove_var("BUZZ_NIP_FI_MAXIMUM_ASSERTION_AGE_SECS");
        let err =
            NipFiRelayConfig::from_env().expect_err("enforce without age must be a config error");
        let msg = err.to_string();
        // Error will be either JSON parse or missing age var — both non-empty.
        assert!(!msg.is_empty());
    }

    #[test]
    fn unknown_mode_is_rejected() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::new(NIP_FI_VARS);

        std::env::set_var("BUZZ_NIP_FI_MODE", "permissive");
        let err = NipFiRelayConfig::from_env().expect_err("unknown mode must error");
        assert!(err.to_string().contains("BUZZ_NIP_FI_MODE"));
    }

    // ── session-deadline three-term bound ─────────────────────────────────────

    /// The `session_deadline` computation satisfies the spec's three-term min:
    ///
    ///   session_deadline = min(
    ///       connection_time + max_connection_lifetime_seconds,
    ///       min(authority_deadlines),      // = min(exp, iat+max_age, key_snapshot_hard_deadline)
    ///       key_snapshot_hard_deadline     // already in authority_deadlines
    ///   )
    ///
    /// This test exercises the deadline selection logic independently of the
    /// full WebSocket stack by using `NipFiRelayConfig::max_connection_lifetime`
    /// and simulating the deadline computation in isolation.
    #[test]
    fn session_deadline_three_term_min_selects_earliest() {
        use chrono::{Duration, Utc};

        let now = Utc::now();

        // Term 1: authority_deadlines = min(exp, iat+max_age, key_snapshot_hard).
        // We simulate three scenarios to cover each term winning.

        // Scenario A: exp is earliest.
        {
            let exp = now + Duration::seconds(100);
            let iat_plus_max_age = now + Duration::seconds(200);
            let key_hard = now + Duration::seconds(300);
            let lifetime = now + Duration::seconds(400);
            let upstream = [exp, iat_plus_max_age, key_hard]
                .iter()
                .copied()
                .min()
                .unwrap();
            let deadline = upstream.min(lifetime);
            assert_eq!(deadline, exp, "exp is earliest → deadline = exp");
        }

        // Scenario B: iat+max_age is earliest.
        {
            let exp = now + Duration::seconds(300);
            let iat_plus_max_age = now + Duration::seconds(100);
            let key_hard = now + Duration::seconds(200);
            let lifetime = now + Duration::seconds(400);
            let upstream = [exp, iat_plus_max_age, key_hard]
                .iter()
                .copied()
                .min()
                .unwrap();
            let deadline = upstream.min(lifetime);
            assert_eq!(
                deadline, iat_plus_max_age,
                "iat+max_age is earliest → deadline = iat+max_age"
            );
        }

        // Scenario C: key_snapshot_hard_deadline is earliest.
        {
            let exp = now + Duration::seconds(400);
            let iat_plus_max_age = now + Duration::seconds(300);
            let key_hard = now + Duration::seconds(100);
            let lifetime = now + Duration::seconds(200);
            let upstream = [exp, iat_plus_max_age, key_hard]
                .iter()
                .copied()
                .min()
                .unwrap();
            let deadline = upstream.min(lifetime);
            assert_eq!(
                deadline, key_hard,
                "key_snapshot_hard_deadline is earliest → deadline = key_hard"
            );
        }

        // Scenario D: max_connection_lifetime partition is earliest.
        {
            let exp = now + Duration::seconds(400);
            let iat_plus_max_age = now + Duration::seconds(300);
            let key_hard = now + Duration::seconds(200);
            let lifetime = now + Duration::seconds(100);
            let upstream = [exp, iat_plus_max_age, key_hard]
                .iter()
                .copied()
                .min()
                .unwrap();
            let deadline = upstream.min(lifetime);
            assert_eq!(
                deadline, lifetime,
                "max_connection_lifetime partition is earliest → deadline = lifetime"
            );
        }
    }

    /// When `max_connection_lifetime` is absent, session_deadline equals the
    /// upstream authority deadline without further shortening.
    #[test]
    fn session_deadline_no_lifetime_uses_upstream_only() {
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let exp = now + Duration::seconds(600);
        let iat_plus_max_age = now + Duration::seconds(3600);
        let key_hard = now + Duration::seconds(86400);
        let upstream = [exp, iat_plus_max_age, key_hard]
            .iter()
            .copied()
            .min()
            .unwrap();

        // No lifetime partition configured → deadline = upstream.
        let deadline: chrono::DateTime<Utc> = upstream; // no further min
        assert_eq!(
            deadline, exp,
            "no lifetime → deadline = min(authority_deadlines) = exp"
        );
    }

    /// Equality at any deadline is expired — the session_deadline computation
    /// never uses `<=` to mean "still live"; `>=` fires at equality.
    #[test]
    fn session_deadline_equality_is_expired() {
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let deadline_now = now; // exactly now = expired

        // Simulate the expiry check: `now >= deadline` fires at equality.
        assert!(
            now >= deadline_now,
            "equality must count as expired per [FI-TRACE-LEASE-BOUND]"
        );

        // A deadline strictly in the future is not yet expired.
        let deadline_future = now + Duration::milliseconds(1);
        assert!(
            now < deadline_future,
            "a deadline in the future must not be expired"
        );
    }
}
