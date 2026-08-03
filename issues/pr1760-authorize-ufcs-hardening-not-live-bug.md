# PR #1760 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1760 (_async trait-method bridge — drop the _async suffix on
trait methods). Security-adjacent (authz) but HARDENING, not a live bug.

## Call `Authorize::authorize` via UFCS to avoid a future inherent-method footgun (Copilot, kernel.rs:629, also :706) — robustness/defensive
> Call the `Authorize` trait method via UFCS to avoid accidental dispatch to an inherent `authorize`
> method on the concrete type (method-call resolution prefers inherent methods, which could bypass the
> kernel's intended authz implementation after this rename).

CONTEXT / accuracy: the call is `authz.authorize(&req).await` where `authz: &(impl Authorize + ?Sized)` —
a GENERIC trait bound, not a concrete type. Through the `impl Authorize` bound, `.authorize()` resolves to
the TRAIT method; there's no concrete inherent method visible, so this is NOT a live bypass today. Copilot's
point is DEFENSIVE: after the rename to the plain name `authorize`, IF a concrete `Authorize` implementor
ever also defined an inherent `authorize` AND a call site used the concrete type, Rust's
inherent-preferred method resolution could silently bypass the trait impl. On the authz path that footgun
is worth foreclosing. Recommend UFCS (`Authorize::authorize(authz, &req).await`, same at :706) — it's
zero-cost and makes the dispatch unambiguous regardless of future inherent methods. LOW/robustness (not a
correctness bug now; a cheap guard on a security-relevant call). v-agent-harness's call. Fix-forward.
