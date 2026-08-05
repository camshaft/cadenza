# PR #2180 review — cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN — security/leak [VERIFIED, MED] (residual on the fix for MY #2169)

https://github.com/camshaft/cadenza/pull/2180 (fold #2169 — redact guest-controlled target from tracing +
correct dep-floor doc; THE fix for MY #2169 tracing-leak finding). Copilot 1 inline — the redaction is
INCOMPLETE: it fixed `target` but still logs `family`, which is also guest-controllable.

## the fix redacts `target`→`target_len` but still logs `family = %req.content_type.family` verbatim; `family` is a `Cow<'static,str>` that is `Cow::Owned` (runtime/guest-controlled) for EXTENSION families → an extension family still leaks untrusted input off-box (Copilot, kernel.rs:769 & :912) — security/leak [VERIFIED, MED]
> `req.content_type.family` is a `Cow<'static, str>` and can be runtime/guest-controlled for extension
> families (`Cow::Owned`), so logging it verbatim in tracing can still leak untrusted input off-box. If
> the goal is "only non-sensitive metadata", consider redacting `family` the same way as `target` (e.g.,
> emit only length/hashes).

VERIFIED in the #2180 diff + source: the fix correctly redacts target — `target_len = req.target.len()`
(diff:41, replacing `target = %req.target`) — matching my #2169 finding. BUT the SAME `warn!`/`debug!`
still emit `family = %req.content_type.family` verbatim (diff:38). And `family` IS guest-controllable:
`ContentType.family` is a `Cow<'static, str>` (effect.rs:312), and for EXTENSION families the caller's Cow
is kept as-is — "Extension family: keep the caller's Cow as-is (Borrowed stays borrowed, Owned isn't
cloned)" (effect.rs:307), with `Cow::Owned(dynamic)` construction (effect.rs:733) and a test asserting
`matches!(owned_ext.content_type.family, Cow::Owned(_))` (effect.rs:740). So a reducer emitting an effect
with an extension family carrying guest bytes gets that string logged verbatim into the tracing stream —
which ships off-box (v-ah-host's subscriber). SAME leak vector as the `target` one my #2169 flagged, just
via `family`. MED. So the #2169 redaction is INCOMPLETE — it plugged `target`/`reason` but left `family`.
Fix per Copilot: redact `family` the same way — well-known families are `&'static` (safe to log), but an
EXTENSION family (`Cow::Owned`) should be emitted as length/hash only, OR log `family` only when it's a
known-static family (is_control_family / a wellknown match) and redact otherwise. v-agent-harness owns
cdz-kernel/src. PR OPEN → foldable. (Owning the chain: my #2169 flagged the target leak; the fix for it
left the sibling `family` leak — one-layer-deeper residual, same untrusted-in-telemetry class as
#2050/#2090.)
