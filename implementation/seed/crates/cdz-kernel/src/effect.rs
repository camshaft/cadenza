//! Effects and capabilities — what a reducer can *ask the world to do*, and what it's *allowed* to.
//!
//! A reducer never touches the world directly (§3, §5): it emits **effect requests** and the kernel
//! executes them, folding results back as events. Two review findings shape these types and MUST NOT
//! be softened:
//!
//! - **SEC-F1 (resource-scoped capabilities):** the effect *kind* alone (`Http.get`) is NOT a
//!   sufficient permission — the security-relevant distinction lives in the *target argument*
//!   (`http://169.254.169.254/…` IMDS vs. an allowed host). So a `Capability` is `(kind, predicate)`
//!   and authorization checks the predicate against the *resolved* target of each request.
//! - **S4 (effect-id-keyed continuations):** every dispatched effect has a kernel-assigned `EffectId`;
//!   the reducer resumes when the *result event* carrying that id arrives. Correlation is by id, so
//!   concurrent / out-of-order results are unambiguous.

use crate::hash::Hash;

/// A kernel-assigned identifier for a single dispatched effect, unique within a session. The reducer
/// stores its continuation keyed by this (§16c-S4) and the result/timeout event carries it back.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EffectId(pub u64);

/// The kind of effect — the coarse verb. Target/args live in [`EffectRequest`]. This is deliberately a
/// small, explicit enum for v0 (design §15b: a handful of local effects); it grows as executors land.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EffectKind {
    /// Run a shell command. Target = the program + args (see `EffectRequest::target`).
    Shell,
    /// An HTTP request. Target = the URL (host is what the capability predicate gates — SEC-F1).
    Http,
    /// Invoke a model. Target = the model id.
    Model,
    /// Read the wall clock. Result is a recorded `time-result` (§9c) — the reducer never reads the
    /// clock directly.
    Now,
    /// Arm a timer. The kernel injects a `timer-fired` event at the (absolute) deadline (§9c/§16c-S5).
    Timer,
    /// Send a signal to a peer session's inbox (§5). Target = the session id.
    Emit,
}

/// A concrete effect the reducer wants performed: a kind plus its *resolved* target argument and
/// payload. The `target` is the SEC-F1 security-relevant string the capability predicate is checked
/// against (a URL, a repo, a command, a session id). `payload` is opaque content (large payloads are a
/// blob [`Hash`]; small ones inline — §4 blob boundary), interpreted by the executor, not the kernel.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EffectRequest {
    pub kind: EffectKind,
    /// The resolved target the capability predicate gates (SEC-F1). Never trust the kind alone.
    pub target: String,
    /// Opaque request body. `None` for argument-free effects (e.g. `Now`).
    pub payload: Option<Payload>,
}

/// An effect payload or result body: either inlined small bytes or a reference to a stored blob.
/// Keeps the log/KV thin (§4) — big transcripts/diffs live in the blob store by hash.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Payload {
    Inline(Vec<u8>),
    Blob(Hash),
}

/// A resource predicate: the SEC-F1 fix. A capability is not "may do `Http.get`" but "may do `Http.get`
/// to a target satisfying this predicate." Checked against the *resolved* [`EffectRequest::target`].
///
/// v0 keeps this a small, total, side-effect-free matcher (no regex-of-doom, no I/O) so authorization
/// is cheap and can never itself misbehave. It grows deliberately.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ResourcePredicate {
    /// Matches any target of this kind. Use sparingly — a broad `Http` `Any` is exactly the SSRF hole
    /// the review flagged. Prefer the scoped variants.
    Any,
    /// Target must equal this string exactly (a specific model id, a specific session).
    Exact(String),
    /// Target must be one of these exact strings.
    OneOf(Vec<String>),
    /// Target (parsed as a URL) must have a host in this allow-list. The SSRF/exfil guard for `Http`.
    HostIn(Vec<String>),
    /// Target must start with this prefix (e.g. a command allow-list, a path/repo scope).
    Prefix(String),
}

impl ResourcePredicate {
    /// Does `target` satisfy this predicate? Total, pure, cheap. This is the SEC-F1 enforcement point.
    pub fn admits(&self, target: &str) -> bool {
        match self {
            ResourcePredicate::Any => true,
            ResourcePredicate::Exact(s) => target == s,
            ResourcePredicate::OneOf(set) => set.iter().any(|s| s == target),
            ResourcePredicate::HostIn(hosts) => match host_of(target) {
                Some(h) => hosts.iter().any(|allowed| allowed == h),
                None => false, // unparseable target → deny (fail closed)
            },
            ResourcePredicate::Prefix(p) => target.starts_with(p),
        }
    }
}

/// A single grant: this effect *kind*, restricted to targets satisfying `predicate` (SEC-F1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Capability {
    pub kind: EffectKind,
    pub predicate: ResourcePredicate,
}

impl Capability {
    /// Does this grant permit the given request? Kind must match AND the predicate must admit the
    /// resolved target. Both conditions — the review's whole point (SEC-F1): kind alone is not enough.
    pub fn permits(&self, req: &EffectRequest) -> bool {
        self.kind == req.kind && self.predicate.admits(&req.target)
    }
}

/// Extract the host from a `scheme://host[:port]/…` target, for `HostIn`. Deliberately tiny and
/// conservative: no dependency, and anything it can't confidently parse yields `None` → deny.
fn host_of(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    // strip userinfo@ and :port
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = authority.split_once(':').map_or(authority, |(h, _)| h);
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(target: &str) -> EffectRequest {
        EffectRequest {
            kind: EffectKind::Http,
            target: target.to_string(),
            payload: None,
        }
    }

    #[test]
    fn host_allow_list_blocks_imds_and_exfil() {
        let cap = Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::HostIn(vec!["metrics.internal".into()]),
        };
        assert!(cap.permits(&http("https://metrics.internal/v1/query")));
        // The SEC-F1 attacks: same kind, hostile target — must be denied.
        assert!(!cap.permits(&http("http://169.254.169.254/latest/meta-data/")));
        assert!(!cap.permits(&http("https://attacker.example/exfil?d=secret")));
    }

    #[test]
    fn kind_mismatch_is_denied_even_if_predicate_matches() {
        let cap = Capability {
            kind: EffectKind::Shell,
            predicate: ResourcePredicate::Any,
        };
        assert!(!cap.permits(&http("https://anything")));
    }

    #[test]
    fn unparseable_url_fails_closed() {
        let pred = ResourcePredicate::HostIn(vec!["ok.host".into()]);
        assert!(!pred.admits("not a url"));
        assert!(!pred.admits("https://"));
    }

    #[test]
    fn host_parsing_strips_port_and_userinfo() {
        assert_eq!(
            host_of("https://user:pw@host.tld:8443/path"),
            Some("host.tld")
        );
        assert_eq!(host_of("http://h.tld"), Some("h.tld"));
    }

    #[test]
    fn prefix_scopes_commands() {
        let cap = Capability {
            kind: EffectKind::Shell,
            predicate: ResourcePredicate::Prefix("cargo ".into()),
        };
        let ok = EffectRequest {
            kind: EffectKind::Shell,
            target: "cargo test".into(),
            payload: None,
        };
        let bad = EffectRequest {
            kind: EffectKind::Shell,
            target: "rm -rf /".into(),
            payload: None,
        };
        assert!(cap.permits(&ok));
        assert!(!cap.permits(&bad));
    }
}
