//! Retryability classification for executor errors — the near-term convention (ruling (1), 2026-08-02)
//! the kernel supervisor branches on until a first-class `EffectOutcome::Err` retryable field lands.
//!
//! The operator's supervision-tree direction: an executor error must be a structured, RECOVERABLE
//! outcome the supervisor can act on — retry a transient failure (with backoff), give up on a permanent
//! one. Until `EffectOutcome::Err` carries a typed retryable flag (a kernel shared-surface change
//! v-agent-harness folds into the supervisor slice), we encode retryability in the `Err(reason)` STRING
//! with a leading token, agreed with v-agent-harness:
//!
//! - [`RETRYABLE`]` : <detail>` — a transient failure worth a backoff-retry (Bedrock 429/5xx/timeout,
//!   connection reset, throttle). The supervisor re-emits the effect (same `idempotency_key`, so a paid
//!   invoke dedups — §16c-S1/D).
//! - [`PERMANENT`]` : <detail>` — do NOT retry (a 400/auth/malformed request, an unroutable kind, a bad
//!   payload). Retrying can't help; the supervisor escalates or fails the branch.
//!
//! **Fail-closed default:** a supervisor treats an UNPREFIXED reason as PERMANENT — never auto-retry an
//! unclassified error (safer than retrying a truly-permanent one forever). So a transport that forgets
//! the prefix degrades to "not retried," not "retried forever."
//!
//! The reason STARTS with exactly one token then `": "`, so a supervisor matches with
//! `reason.starts_with("RETRYABLE:")` — cheap and unambiguous (no fragile substring-in-the-middle match).
//! [`classify`] reads a reason back into a [`Retryability`] for callers/tests.

/// The leading token marking a transient, retry-worthy failure.
pub const RETRYABLE: &str = "RETRYABLE";
/// The leading token marking a permanent, do-not-retry failure.
pub const PERMANENT: &str = "PERMANENT";

/// How a supervisor should treat an executor error. Read from a reason string by [`classify`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Retryability {
    /// Transient — a backoff-retry may succeed.
    Retryable,
    /// Permanent — retrying cannot help; escalate/fail.
    Permanent,
}

/// Build a `RETRYABLE: <detail>` reason for a transient failure.
pub fn retryable(detail: impl AsRef<str>) -> String {
    format!("{RETRYABLE}: {}", detail.as_ref())
}

/// Build a `PERMANENT: <detail>` reason for a non-retryable failure.
pub fn permanent(detail: impl AsRef<str>) -> String {
    format!("{PERMANENT}: {}", detail.as_ref())
}

/// Classify a reason string. A leading `RETRYABLE:` → [`Retryability::Retryable`]; anything else
/// (including an unprefixed reason) → [`Retryability::Permanent`] — the fail-closed default a supervisor
/// uses so an unclassified error is never auto-retried forever.
pub fn classify(reason: &str) -> Retryability {
    if reason.starts_with(&format!("{RETRYABLE}:")) {
        Retryability::Retryable
    } else {
        Retryability::Permanent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_produce_the_agreed_prefixes() {
        assert_eq!(
            retryable("bedrock throttled (429)"),
            "RETRYABLE: bedrock throttled (429)"
        );
        assert_eq!(
            permanent("bad request (400)"),
            "PERMANENT: bad request (400)"
        );
    }

    #[test]
    fn classify_reads_the_prefix() {
        assert_eq!(classify(&retryable("timeout")), Retryability::Retryable);
        assert_eq!(classify(&permanent("400")), Retryability::Permanent);
    }

    #[test]
    fn unprefixed_reason_is_permanent_fail_closed() {
        // The safety default: an unclassified error is treated as permanent, so a forgotten prefix means
        // "not retried" rather than "retried forever."
        assert_eq!(
            classify("some raw error with no token"),
            Retryability::Permanent
        );
        // A token that isn't exactly the RETRYABLE prefix must not be mistaken for retryable.
        assert_eq!(classify("RETRYABLE_ISH: nope"), Retryability::Permanent);
        assert_eq!(
            classify("this mentions RETRYABLE: in the middle"),
            Retryability::Permanent
        );
    }
}
