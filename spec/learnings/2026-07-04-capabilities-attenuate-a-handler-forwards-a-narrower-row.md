# Capabilities attenuate: a handler forwards a narrower effect row, never a broader one

*2026-07-04*

**What happened.** The capability model gains **attenuation** (object-capability discipline): a
computation may hand a sub-computation *fewer* capabilities than it holds, never more. With the
effects-as-capabilities model ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]])
this is not new machinery — **a handler attenuates**: it discharges some effects and forwards a
*narrower* effect row to the code it runs. Authority only ever shrinks as it flows inward.

**Why this is required, not optional — the target makes it concrete.** Cadenza is the source language
and derivation tool for a system where **behavior is data**: units of behavior are published as source
plus a capability manifest, and run as sandboxed, content-addressed components (the target system's
"behavior is data" and "capability-scoped execution" invariants). In that system:
- **A component calls other components and invokes tools.** A module that invokes a tool, or runs
  another module's behavior, must be able to grant that callee *a subset* of its own authority — not
  leak its whole manifest. Without attenuation, every downstream call inherits the caller's full
  authority, which collapses the capability boundary the manifest exists to draw.
- **Cross-participant execution runs under the owner's identity with the owner's capabilities.** The
  target system requires a tool invoked on behalf of another participant to run under the *owner's*
  verified identity and only the capabilities the owner opted in — an attenuation of authority across a
  trust boundary. The language must be *able to express* granting-less-than-you-hold for this to be
  representable at all.
- **Authority comes only from the principal that holds it.** A sub-computation cannot acquire an
  authority its caller did not pass down; attenuation is the mechanism that makes "no ambient authority"
  (Constitution IV) hold *transitively*, not just at the outermost boundary.

**Why the effects model gives it for free.** An effectful computation is typed by its effect row
([[2026-07-04-records-are-rows-open-by-default]]); a handler *discharges* an effect (removes a label
from the row) or *forwards* it (keeps it). Attenuation is the guarantee that a handler's forwarded row
is a **subset** of the row it received — it can drop capabilities and it can *narrow* one (handle
`Write` to a specific blob by forwarding only a scoped `Write`), but it cannot introduce a label the
enclosing context did not grant. This is exactly the row-subset relationship the type system already
tracks, so attenuation is a *typing property of handlers*, not a separate access-control system:
- **Grant-less-than-you-hold** is forwarding a strict sub-row.
- **No amplification** is the invariant that the forwarded row never contains a label absent from the
  received row — the transitive form of "the compiler emits no import the manifest does not enumerate."
- **A capability handle may be affine** so a granted authority is used at most once where that is the
  intent ([[2026-07-04-linearity-is-surgical-not-core]]) — a spend-once delegated authority.

**Prior art.** The object-capability tradition — **E**, **Caja**, the ocap literature — where authority
is conveyed only by holding a reference and attenuated by wrapping it in a narrower forwarder. **Unison**
abilities and **Koka**/**OCaml 5** handlers are the effect-system realization: a handler *is* the
attenuating wrapper. The novelty here is unifying the two: the effect row is the capability set, and the
handler is the ocap attenuator.

**Consequences to hold.**
- **Attenuation is compile-time-checked, not runtime-enforced.** Because the forwarded row is a static
  subset relationship, a program that tries to forward a broader row than it holds is *rejected*
  (Constitution IV, transitively), not caught at runtime — consistent with reject-don't-miscompile
  ([[2026-07-03-decline-do-not-miscompile]]).
- **The manifest stays the outer boundary.** Attenuation governs authority flow *within* a program;
  the manifest still enumerates the effects that escape to the host
  ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]). A handler cannot widen the
  manifest; it can only route and narrow what the manifest already grants.

**The requirements it drives.** `spec/capabilities/capabilities-and-effects.md` gains a §"Capabilities
Attenuate" (a handler MAY forward a subset of the effect row it receives; a handler MUST NOT forward a
label absent from the row it received; the forwarded-row-is-a-subset check is compile-time). This
strengthens Constitution IV from an outer-boundary property to a transitive one, expressed as capability-
spec requirements that *realize* IV rather than amending it. Composes with
[[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]] (the row substrate) and
[[2026-07-04-linearity-is-surgical-not-core]] (affine delegated handles).
