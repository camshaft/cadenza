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
/// `Hash` so it can key a by-kind executor router ([`crate::executor::CompositeExecutor`]).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
/// blob [`struct@Hash`]; small ones inline — §4 blob boundary), interpreted by the executor, not the kernel.
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
///
/// `Inline` holds ref-counted [`bytes::Bytes`] (operator perf directive): a payload is CLONED as it
/// crosses fold→dispatch→execute→result (and a large model-completion body is exactly the hot path), so
/// a `Bytes` clone is an O(1) refcount bump, not a deep memcpy of the whole body. Build ergonomics are
/// preserved via `Bytes: From<Vec<u8>>` — code that assembles a `Vec<u8>` freezes it with `.into()`,
/// and a reader borrows `&[u8]` (Bytes derefs to a slice), so a producer/consumer needn't care which it
/// was built from. This is what lets the executor peer (`cdz-agent-host`) keep constructing
/// `Payload::Inline(v.into())` unchanged across the flip.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Payload {
    Inline(bytes::Bytes),
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
                // Host comparison is case- and trailing-dot-insensitive (RFC 3986 §3.2.2: host is
                // case-insensitive; `ok.host.` is the same host as `ok.host`). Exact-string `==` here
                // was a bug: it wrongly DENIED `OK.host`/`ok.host.` (fail-closed, but a real
                // correctness gap). Still fail-closed — normalization only ever makes the SAME host
                // match, never widens to a different one.
                Some(h) => hosts.iter().any(|allowed| host_eq(allowed, &h)),
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

/// Extract the host from a `scheme://host[:port]/…` target, for `HostIn`. Uses the battle-tested `url`
/// crate (RFC 3986 / WHATWG) rather than a hand-rolled authority parser — operator directive: a bespoke
/// parser guarding an ALLOW-LIST is the worst place for edge-case bugs, since every missed case is an
/// authz BYPASS (Copilot PR#1015/1018 were exactly IPv6-literal bypasses in the old hand-rolled code).
/// `url` handles IPv6 literals, userinfo, ports, and normalization correctly, so this just parses and
/// pulls `.host_str()`.
///
/// Fail-closed (SEC-F1): any parse error, or a URL with no host authority (`mailto:`, a relative/opaque
/// target, an empty host), yields `None` → deny. Returns an OWNED `String` because `host_str()` borrows
/// from the parsed `Url` (a local). For an IPv6 literal `url` gives the bracketed form (`[::1]`); we
/// strip the brackets so the returned host is the bare address (`::1`) — what a `HostIn` entry carries.
fn host_of(target: &str) -> Option<String> {
    let parsed = url::Url::parse(target).ok()?;
    let host = parsed.host_str()?;
    if host.is_empty() {
        return None;
    }
    // IPv6 literals come back bracketed (`[::1]`); a HostIn allow-list entry is the bare address. Strip
    // exactly one matching pair. (A reg-name / IPv4 never has brackets, so this is a no-op for them.)
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    Some(host.to_string())
}

/// Host equality for `HostIn` (RFC 3986 §3.2.2): ASCII-case-insensitive and insensitive to a single
/// trailing dot (the FQDN root, `ok.host.` ≡ `ok.host`). Pure + total; only ever matches the SAME
/// host, so it can't widen a capability to a different target.
fn host_eq(allowed: &str, actual: &str) -> bool {
    let norm = |h: &str| h.strip_suffix('.').unwrap_or(h).to_ascii_lowercase();
    norm(allowed) == norm(actual)
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
            host_of("https://user:pw@host.tld:8443/path").as_deref(),
            Some("host.tld")
        );
        assert_eq!(host_of("http://h.tld").as_deref(), Some("h.tld"));
    }

    #[test]
    fn host_parsing_handles_ipv6_literals() {
        // Bracketed IPv6 host must not be split on its internal colons (the old code returned "[").
        assert_eq!(host_of("http://[::1]/latest").as_deref(), Some("::1"));
        assert_eq!(
            host_of("https://[2001:db8::1]:8443/x").as_deref(),
            Some("2001:db8::1")
        );
        // And an IPv6 allow-list entry now actually matches its target.
        let pred = ResourcePredicate::HostIn(vec!["::1".into()]);
        assert!(pred.admits("http://[::1]/latest"));
        assert!(!pred.admits("http://[2001:db8::2]/x"));
    }

    #[test]
    fn ipv6_bracket_with_trailing_junk_is_denied_not_bypassed() {
        // Copilot PR#1015 (SEC-F1 allow-list BYPASS): after the closing `]` the only valid tail is empty
        // or `:port`. `[::1]evil.com` must NOT parse as host `::1` (which a HostIn(["::1"]) grant would
        // then AUTHORIZE, reaching evil.com). Fail closed: unparseable → None → deny.
        assert_eq!(host_of("http://[::1]evil.com/"), None);
        assert_eq!(host_of("http://[::1]x/"), None);
        let pred = ResourcePredicate::HostIn(vec!["::1".into()]);
        assert!(
            !pred.admits("http://[::1]evil.com/"),
            "an ::1 grant must NOT authorize [::1]evil.com — that's the bypass"
        );
        // The legitimate forms still parse (guard against over-rejecting).
        assert_eq!(host_of("http://[::1]:8080/").as_deref(), Some("::1"));
        assert_eq!(host_of("http://[::1]/").as_deref(), Some("::1"));
    }

    #[test]
    fn ipv6_bracket_tail_must_be_empty_or_a_real_numeric_port() {
        // Copilot PR#1018 REGRESSION repro (the bypass that mattered): a HOSTILE colon-tail after `]`
        // must NOT parse as host `::1` — else a HostIn(["::1"]) grant would authorize the hostile
        // target. The `url` crate correctly REJECTS all of these (malformed authority → parse error →
        // None → deny), so the bypass stays closed under the new parser:
        let pred = ResourcePredicate::HostIn(vec!["::1".into()]);
        assert_eq!(host_of("http://[::1]:80evil.com/"), None, ":80evil.com");
        assert_eq!(host_of("http://[::1]:0x50/"), None, "non-decimal port");
        assert_eq!(host_of("http://[::1]:80.evil/"), None, "port then junk");
        assert!(
            !pred.admits("http://[::1]:80evil.com/"),
            "an ::1 grant must NOT authorize [::1]:80evil.com — the regression bypass"
        );
        // NOTE a real difference from the old hand-rolled parser: `[::1]:` (bracket + a bare, EMPTY port)
        // is a VALID authority per WHATWG/RFC-3986 — the host genuinely IS `::1` (trailing empty port =
        // default port), NOT a hostile tail. The old parser wrongly REJECTED it (over-strict, fail-closed
        // but a false denial); `url` accepts it as host `::1`, which is correct + not a bypass. So this
        // assertion is UPDATED (the old `None` expectation was the hand-rolled parser's bug, not a
        // security property):
        assert_eq!(
            host_of("http://[::1]:/").as_deref(),
            Some("::1"),
            "empty port is valid, host is ::1"
        );
        // Real numeric ports still parse to the bare host.
        assert_eq!(host_of("http://[::1]:80/").as_deref(), Some("::1"));
        assert_eq!(
            host_of("http://[2001:db8::1]:443/x").as_deref(),
            Some("2001:db8::1")
        );
    }

    #[test]
    fn host_match_is_case_and_trailing_dot_insensitive() {
        // RFC 3986 §3.2.2: host is case-insensitive, and a trailing-dot FQDN is the same host. The old
        // exact `==` wrongly DENIED these (fail-closed, but a real correctness bug that breaks a
        // legitimately-granted request).
        let pred = ResourcePredicate::HostIn(vec!["ok.host".into()]);
        assert!(pred.admits("https://OK.Host/x"), "case-insensitive");
        assert!(pred.admits("https://ok.host./x"), "trailing-dot FQDN");
        assert!(pred.admits("https://ok.HOST.:443/x"), "both");
        // Normalization must NOT widen to a DIFFERENT host (still fail-closed on real mismatches).
        assert!(!pred.admits("https://ok.host.evil.com/x"));
        assert!(!pred.admits("https://notok.host/x"));
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
