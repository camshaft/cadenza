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

use crate::event::ContentType;
use crate::hash::Hash;

/// A kernel-assigned identifier for a single dispatched effect, unique within a session. The reducer
/// stores its continuation keyed by this (§16c-S4) and the result/timeout event carries it back.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EffectId(pub u64);

/// The kind of effect — the coarse verb. Target/args live in [`EffectRequest`]. This is deliberately a
/// small, explicit enum for v0 (design §15b: a handful of local effects); it grows as executors land.
/// `Hash` so it can key a by-kind executor router ([`crate::executor::CompositeExecutor`]).
///
/// Migration note (extensible content-typed effects, operator seq-39): each kind has a canonical
/// lowercase FAMILY string ([`EffectKind::family`] / [`effect_ct`]) — the same string the codec
/// (event_ast) already writes and the Cedar authorizer's action-name uses. That family is the seam the
/// effect model migrates onto: routing/authz key on the family string (so a NEW effect type is added
/// without growing this enum + a kernel edit), and this enum stays the canonical family source until the
/// migration replaces it with a raw content-type tag.
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

/// The canonical lowercase FAMILY strings for the well-known effect kinds — the extensible-effects vocab
/// (operator seq-39). These are the SINGLE source of truth for the family names the codec writes
/// (event_ast `kind_atom`/`read_kind`), the executor router keys on, and the Cedar authorizer maps to a
/// policy ACTION — so they can't drift on a typo across those sites. A new effect type is a NEW family
/// string here (routed to a handler by string), never a new [`EffectKind`] variant + a kernel recompile.
pub mod effect_ct {
    pub const SHELL: &str = "shell";
    pub const HTTP: &str = "http";
    pub const MODEL: &str = "model";
    pub const NOW: &str = "now";
    pub const TIMER: &str = "timer";
    pub const EMIT: &str = "emit";

    /// The canonical, finite set of well-known effect families — the SAME set routing/authz/codec key on.
    /// Iterating it is what makes capability-manifest projection complete BY CONSTRUCTION (probe each known
    /// family; there is nothing to miss — see [`super::project_manifest`]). Keep in sync with the consts
    /// above (they're the single source; this just lists them for enumeration).
    pub const ALL: &[&str] = &[SHELL, HTTP, MODEL, NOW, TIMER, EMIT];
}

impl EffectKind {
    /// The canonical lowercase family string for this kind (see [`effect_ct`]) — the string the codec
    /// writes, the router keys on, and Cedar uses as the action. One source of truth for the wire name.
    pub fn family(&self) -> &'static str {
        match self {
            EffectKind::Shell => effect_ct::SHELL,
            EffectKind::Http => effect_ct::HTTP,
            EffectKind::Model => effect_ct::MODEL,
            EffectKind::Now => effect_ct::NOW,
            EffectKind::Timer => effect_ct::TIMER,
            EffectKind::Emit => effect_ct::EMIT,
        }
    }

    /// Parse a family string back to a well-known [`EffectKind`], or `None` for an unrecognized family
    /// (a future/extension effect type that this kernel version has no built-in variant for). The inverse
    /// of [`EffectKind::family`]; the codec's `read_kind` and any string-keyed router share this mapping.
    pub fn from_family(family: &str) -> Option<EffectKind> {
        match family {
            effect_ct::SHELL => Some(EffectKind::Shell),
            effect_ct::HTTP => Some(EffectKind::Http),
            effect_ct::MODEL => Some(EffectKind::Model),
            effect_ct::NOW => Some(EffectKind::Now),
            effect_ct::TIMER => Some(EffectKind::Timer),
            effect_ct::EMIT => Some(EffectKind::Emit),
            _ => None,
        }
    }
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
    /// How latency-sensitive this effect is (§ operator timeliness directive). A [`Timeliness::Batchable`]
    /// effect MAY be deferred/batched by the executor for cost (e.g. Bedrock batch inference is ~half the
    /// on-demand price at higher latency); [`Timeliness::Interactive`] must run now. First-class (not a
    /// payload convention) so it's on the durable log — replay-deterministic — and the executor reads it
    /// directly to pick the on-demand vs batch path. Meaningful for `Model` today; a first-class field so
    /// future batchable kinds (embeddings, bulk fetches) reuse it. Default `Interactive`.
    pub timeliness: Timeliness,
    /// The extensible content-type of this effect (seq-39): a `{family, version}` tag that routing and
    /// authz key on, so a NEW effect type is served by registering a handler for its family STRING rather
    /// than growing the [`EffectKind`] enum + recompiling the kernel. For the well-known kinds this is
    /// derived from `kind` ([`EffectKind::family`] + version 1) by [`EffectRequest::new`], so the two agree
    /// by construction; a future register-by-string slice lets an effect carry a family with no matching
    /// `EffectKind` variant at all. The `family` is the seam [`crate::authz::Authorizer`] and the executor
    /// router match on (via [`ContentType::matches_family`]).
    pub content_type: ContentType,
}

impl EffectRequest {
    /// Construct an effect request from its four fields. This is the canonical constructor — prefer it
    /// over a struct literal at every call site (kernel AND downstream crates).
    ///
    /// Why a constructor for a plain data struct: it lets the shared `EffectRequest` shape grow a field
    /// without editing every call site. Adding a field to a struct breaks every pre-existing struct literal
    /// at compile time (rustc E0063, and across a crate boundary too), so once construction goes through
    /// `new`, a field-add edits only THIS body — call sites are untouched. This is exactly how
    /// `content_type` was added: `new` DERIVES it from `kind` ([`EffectKind::family`] + version 1), so
    /// every caller that already builds via `new` got the new field for free. A caller passing a `kind`
    /// therefore always gets a matching `content_type` — the two can't drift.
    pub fn new(
        kind: EffectKind,
        target: impl Into<String>,
        payload: Option<Payload>,
        timeliness: Timeliness,
    ) -> Self {
        EffectRequest {
            content_type: ContentType {
                family: kind.family().to_string(),
                version: 1,
            },
            kind,
            target: target.into(),
            payload,
            timeliness,
        }
    }
}

/// How latency-sensitive an effect is — the operator's timeliness parameter (batchable-or-not). A sum,
/// not a bool (no-sentinels standing directive): the `Batchable` arm carries an optional caller latency
/// hint, and the type grows cleanly if a third timeliness class ever appears.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Timeliness {
    /// Latency-sensitive: the executor must run it NOW (on-demand). The default for every effect.
    #[default]
    Interactive,
    /// May be DEFERRED/batched for cost — the executor is free to route it to a cheaper, higher-latency
    /// batch path (e.g. Bedrock batch inference at ~half price). The deferred result folds back through
    /// the SAME durable dispatch→result→resume cycle whenever the batch completes (a Batchable call is
    /// just an effect whose `EffectResult` arrives much later — no new kernel machinery). `max_latency_ms`
    /// is an OPTIONAL caller hint for the longest latency it will tolerate (`None` = batch whenever); it
    /// also informs a longer auto-timeout deadline once §9d auto-timeout is wired, so a slow batch isn't
    /// prematurely cancelled.
    Batchable { max_latency_ms: Option<u64> },
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
    /// Does this grant permit the given request? The effect FAMILY must match AND the predicate must admit
    /// the resolved target. Both conditions — the review's whole point (SEC-F1): family alone is not enough.
    ///
    /// Family-keyed (seq-39): the match is `req.content_type.family == self.kind.family()`, via
    /// [`ContentType::matches_family`], NOT an `EffectKind` enum equality. So authz keys on the same family
    /// STRING the codec/router use — the seam that lets a future effect type (a family with no built-in
    /// `EffectKind`) be granted by family without a kernel enum edit. For the well-known kinds this is
    /// identical to the old `kind ==` check (a request built via [`EffectRequest::new`] has
    /// `content_type.family == kind.family()` by construction).
    pub fn permits(&self, req: &EffectRequest) -> bool {
        req.content_type.matches_family(self.kind.family()) && self.predicate.admits(&req.target)
    }
}

/// Whether a session may use a given effect family — one entry of a [`CapabilityManifest`]. Computed from
/// the TWO actual sources (host mechanism + policy), never a hand-maintained list, so it can't drift:
/// `Absent` when the host has no executor for the family; otherwise `Granted`/`Denied` by the authorizer's
/// decision. (The design leaves room for a future `Requestable` split of `Denied` — host has the executor
/// but policy denies AND the session may request it — once that policy model is ratified; today a
/// policy-denied family is a single `Denied`.)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GrantState {
    /// The host has an executor for this family AND policy admits this session's probe → usable now.
    Granted,
    /// The host has an executor for this family, but policy denied the probe.
    Denied,
    /// The host has NO executor for this family — unusable regardless of policy.
    Absent,
}

/// One family's entry in the capability manifest: the family string, its grant-state, and (when known) the
/// resource scope the grant is bound to. `scope` REUSES [`ResourcePredicate`] (never a new scope type); it
/// is `None` when there is no scope to report (an `Absent` family, or a `Denied` one).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CapabilityEntry {
    pub family: String,
    pub grant: GrantState,
    pub scope: Option<ResourcePredicate>,
}

/// A session's capability manifest (§host-capability-discovery I1): one [`CapabilityEntry`] per well-known
/// effect family. The reducer learns "what effect families may I use, and at what scope" from this — the
/// authorized projection of host mechanism ∩ policy. Content-typed as `capabilities-manifest` when it rides
/// the log (a later slice); this is the in-kernel value.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CapabilityManifest {
    pub entries: Vec<CapabilityEntry>,
}

/// Project a session's capability manifest by PROBING (the LOCKED crux — not authorizer enumeration).
/// For each family in the canonical set (`families`, normally [`effect_ct::ALL`]): the mechanism dimension
/// is `handles(family)` (does the host have an executor?), and the policy dimension is ONE `authorize`
/// probe (the existing decide-only [`Authorize`] trait — no enumeration API). Complete BY CONSTRUCTION: the
/// family set is finite + canonical, so nothing is missed. Pure: deterministic given `(families, handles,
/// authorizer, probe_target)`; async only because the authorizer may `.await` a wasm policy eval.
///
/// `probe_target` is the resolved target used to probe each family's policy (e.g. a session-scoped default);
/// the concrete convention is coordinated with the host in I3. The probed request is built via
/// [`EffectRequest::new`] so its `content_type.family` matches the family being probed.
pub async fn project_manifest(
    families: &[&str],
    handles: impl Fn(&str) -> bool,
    authorizer: &dyn crate::authz::Authorize,
    probe_target: &str,
) -> CapabilityManifest {
    let mut entries = Vec::with_capacity(families.len());
    for &family in families {
        let grant = if !handles(family) {
            GrantState::Absent
        } else {
            // Mechanism present → the policy probe decides. Build the probe request via the family's
            // well-known kind when there is one (so kind + content_type.family agree); an extension family
            // with no `EffectKind` still probes by family once register-by-string lands.
            let kind = EffectKind::from_family(family).unwrap_or(EffectKind::Emit);
            let mut probe = EffectRequest::new(kind, probe_target, None, Timeliness::Interactive);
            probe.content_type.family = family.to_string();
            match authorizer.authorize_async(&probe).await {
                Ok(()) => GrantState::Granted,
                Err(_) => GrantState::Denied,
            }
        };
        entries.push(CapabilityEntry {
            family: family.to_string(),
            grant,
            scope: None,
        });
    }
    CapabilityManifest { entries }
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
        // Exercises the canonical constructor (the effect-schema-arc migration path).
        EffectRequest::new(EffectKind::Http, target, None, Timeliness::Interactive)
    }

    #[test]
    fn new_derives_content_type_from_kind_and_matches_a_full_literal() {
        // EffectRequest::new DERIVES content_type from kind (family = kind.family(), version 1) — this is
        // the field-add benefit realized: callers pass the same 4 args and get the extra field filled
        // consistently, so kind and content_type.family can't drift. Equivalent to a full literal that
        // spells out the derived content_type.
        let via_new = EffectRequest::new(
            EffectKind::Http,
            "https://ok.host/x",
            Some(Payload::Inline(b"body".to_vec().into())),
            Timeliness::Interactive,
        );
        // new() set content_type.family to the kind's canonical family string.
        assert_eq!(via_new.content_type.family, EffectKind::Http.family());
        assert_eq!(via_new.content_type.version, 1);
        let via_literal = EffectRequest {
            kind: EffectKind::Http,
            target: "https://ok.host/x".to_string(),
            payload: Some(Payload::Inline(b"body".to_vec().into())),
            timeliness: Timeliness::Interactive,
            content_type: ContentType {
                family: EffectKind::Http.family().to_string(),
                version: 1,
            },
        };
        assert_eq!(via_new, via_literal);
        // The `impl Into<String>` target arg accepts both &str and String uniformly.
        assert_eq!(
            EffectRequest::new(
                EffectKind::Now,
                String::new(),
                None,
                Timeliness::Interactive
            ),
            EffectRequest::new(EffectKind::Now, "", None, Timeliness::Interactive),
        );
    }

    #[test]
    fn effect_kind_family_round_trips_and_matches_the_canonical_consts() {
        // Every well-known kind maps to its canonical family string and back — the vocab the codec,
        // router, and Cedar action-map all share (extensible-effects seam, seq-39).
        for kind in [
            EffectKind::Shell,
            EffectKind::Http,
            EffectKind::Model,
            EffectKind::Now,
            EffectKind::Timer,
            EffectKind::Emit,
        ] {
            assert_eq!(EffectKind::from_family(kind.family()), Some(kind.clone()));
        }
        // The consts are the exact lowercase names (byte-for-byte — the codec/authz depend on these).
        assert_eq!(EffectKind::Http.family(), effect_ct::HTTP);
        assert_eq!(EffectKind::Http.family(), "http");
        // An unrecognized family (a future/extension effect type) → None, not a panic.
        assert_eq!(EffectKind::from_family("summary"), None);
        assert_eq!(EffectKind::from_family("not-a-kind"), None);
    }

    #[test]
    fn effect_family_strings_are_a_stable_lowercase_ascii_wire_contract() {
        // Slice 1 made the `effect_ct` family strings a CROSS-SITE WIRE CONTRACT: they are the exact
        // bytes the codec writes into the durable append-only log, the key the executor router matches
        // on, and the Cedar policy ACTION name. So their literal values are frozen — a rename (even one
        // that keeps family()/from_family consistent, e.g. "shell"→"Shell") would silently break on-disk
        // log compatibility + every deployed Cedar policy. The round-trip test only pins http's literal;
        // this pins the WHOLE vocab byte-for-byte, plus the two structural invariants a string-keyed
        // registry needs (no collisions, canonical lowercase-ascii) so a future extension can't drift.
        let vocab = [
            (EffectKind::Shell, effect_ct::SHELL, "shell"),
            (EffectKind::Http, effect_ct::HTTP, "http"),
            (EffectKind::Model, effect_ct::MODEL, "model"),
            (EffectKind::Now, effect_ct::NOW, "now"),
            (EffectKind::Timer, effect_ct::TIMER, "timer"),
            (EffectKind::Emit, effect_ct::EMIT, "emit"),
        ];
        // 1. Every const is its exact frozen literal, and family() returns that same const (one source).
        for (kind, konst, literal) in &vocab {
            assert_eq!(
                konst, literal,
                "the {literal:?} family const drifted from its wire byte"
            );
            assert_eq!(
                &kind.family(),
                konst,
                "family() must return the effect_ct const verbatim"
            );
        }
        // 2. The family strings are pairwise UNIQUE — a string-keyed router/authz would be ambiguous if
        //    two kinds shared a name (a copy-paste typo like MODEL:"http"), so pin no-collision directly.
        for i in 0..vocab.len() {
            for j in (i + 1)..vocab.len() {
                assert_ne!(
                    vocab[i].1, vocab[j].1,
                    "family strings must be unique: {} and {} collide",
                    vocab[i].1, vocab[j].1
                );
            }
        }
        // 3. Canonical form: lowercase ASCII, non-empty (the doc's "lowercase FAMILY string" promise —
        //    a new extension const that broke this would route/authorize inconsistently across sites).
        for (_, konst, _) in &vocab {
            assert!(!konst.is_empty(), "a family string must be non-empty");
            assert!(
                konst.chars().all(|c| c.is_ascii_lowercase()),
                "family string {konst:?} must be lowercase ascii (canonical wire form)"
            );
        }
    }

    #[test]
    fn timeliness_defaults_to_interactive_and_batchable_carries_its_hint() {
        // Default is Interactive (the operator's latency-sensitive default — every effect runs now
        // unless it opts into batching).
        assert_eq!(Timeliness::default(), Timeliness::Interactive);
        // A Batchable request carries its optional caller latency hint (a sum, not a bool — the hint
        // rides the variant). None = batch whenever; Some(ms) = the longest latency tolerated.
        let deferred = EffectRequest::new(
            EffectKind::Model,
            "anthropic.claude",
            None,
            Timeliness::Batchable {
                max_latency_ms: Some(3_600_000),
            },
        );
        match deferred.timeliness {
            Timeliness::Batchable { max_latency_ms } => assert_eq!(max_latency_ms, Some(3_600_000)),
            Timeliness::Interactive => panic!("expected Batchable"),
        }
        // Batchable-whenever (no hint) is distinct from Interactive.
        assert_ne!(
            Timeliness::Batchable {
                max_latency_ms: None
            },
            Timeliness::Interactive
        );
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
        let ok = EffectRequest::new(
            EffectKind::Shell,
            "cargo test",
            None,
            Timeliness::Interactive,
        );
        let bad = EffectRequest::new(EffectKind::Shell, "rm -rf /", None, Timeliness::Interactive);
        assert!(cap.permits(&ok));
        assert!(!cap.permits(&bad));
    }

    // ---- host-capability-discovery I1: manifest projection by probing --------------------------------

    #[tokio::test(flavor = "current_thread")]
    async fn project_manifest_computes_the_three_grant_states_from_mechanism_and_policy() {
        use crate::authz::Authorizer;

        // Policy: grant Http (any target) only. Mechanism (handles): the host serves Http + Model, but NOT
        // Shell. So over {http, model, shell} the projection must yield exactly one of each grant-state:
        //  - http  → GRANTED  (mechanism yes + policy allows)
        //  - model → DENIED   (mechanism yes + policy denies — the REQUESTABLE-precursor state)
        //  - shell → ABSENT   (mechanism no — policy irrelevant)
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let handles = |f: &str| f == effect_ct::HTTP || f == effect_ct::MODEL;
        let families = [effect_ct::HTTP, effect_ct::MODEL, effect_ct::SHELL];

        let manifest = project_manifest(&families, handles, &authz, "probe://scope").await;

        assert_eq!(manifest.entries.len(), 3);
        let state = |fam: &str| {
            manifest
                .entries
                .iter()
                .find(|e| e.family == fam)
                .map(|e| e.grant.clone())
                .unwrap()
        };
        assert_eq!(state(effect_ct::HTTP), GrantState::Granted);
        assert_eq!(state(effect_ct::MODEL), GrantState::Denied);
        assert_eq!(state(effect_ct::SHELL), GrantState::Absent);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn project_manifest_over_all_is_complete_by_construction() {
        use crate::authz::Authorizer;
        // Probing the canonical `effect_ct::ALL` set yields exactly one entry per known family — nothing
        // missed (the crux: the family set is finite + canonical). With deny_all + no mechanism, every
        // family is Absent (handles=false short-circuits before the policy probe).
        let manifest =
            project_manifest(effect_ct::ALL, |_| false, &Authorizer::deny_all(), "x").await;
        assert_eq!(manifest.entries.len(), effect_ct::ALL.len());
        assert!(manifest
            .entries
            .iter()
            .all(|e| e.grant == GrantState::Absent));
        // Every canonical family is represented.
        for &fam in effect_ct::ALL {
            assert!(manifest.entries.iter().any(|e| e.family == fam));
        }
    }
}
