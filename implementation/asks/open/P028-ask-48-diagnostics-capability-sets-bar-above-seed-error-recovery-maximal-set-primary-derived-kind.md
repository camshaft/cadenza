## 48. 🟡 The new `diagnostics.md` capability sets the diagnostics bar well above the seed — error recovery (maximal independent set), primary/derived, and a machine-branchable rejection/decline/trap kind

**Finding.** A new tracked capability spec `spec/capabilities/diagnostics.md` landed, formalizing the diagnostics
contract. Several of its requirements are already met (stable codes — the corpus's `rejected CDZ####` cases pin
them; severity; machine-readable), but several set the bar **above what the seed does today** — probed against
the refreshed stable seed:

- **#Diagnosis Reports The Maximal Independent Set In One Pass** — "MUST recover from an error and report the
  maximal set of independent problems in one pass rather than only the first." Seed VIOLATES: `(do (+ 1 true)
  (< 2 false))` (two independent type errors) → the seed reports only the first (`operation on mismatched
  types`), then stops. No error recovery / multi-diagnostic pass exists.
- **#A Diagnostic Distinguishes Primary From Derived** — "MUST mark each diagnostic as primary or derived."
  Seed has no primary/derived marking (single diagnostic, no cascade model).
- **#A Diagnostic Names Its Kind** — "MUST expose a machine-branchable kind … distinguishing a rejection
  (ill-formed), a decline (not yet handled), and a trap (runtime halt)." This is exactly the decline-vs-reject-
  vs-trap distinction the conformance loop has needed all along (it's why the byte gate needed decline
  discriminators, ask-26/29/33). The seed conflates them at the CLI (`emit` prints `declined: …` for both a type
  rejection and an unsupported-construct decline; a trap is separate). A machine-branchable kind would let a
  consumer — including this loop — route rejection/decline/trap without disassembling the emitted component.
- **#A Rejection Carries A Structural Fix** / #A Confirmed Fix Is Marked Verified / #A Fix Is A Deterministic
  Function Of The Source — the seed emits no structural fixes at all.
- **#Every Diagnostic Has A Precise Span** — the seed's diagnostics carry a message but (per the CLI) not a
  visible span; unverified whether an internal span exists.

**Why it touches the spec / self-hosting.** These are net-new normative requirements, and they interact with the
in-flight diagnostics work (ask-40 the return channel, ask-46 the compile-entry handler): the effect-based
diagnostics pass being built collects codes, but the spec now also demands error RECOVERY (don't stop at the
first), a primary/derived model, and a machine-branchable kind. So the diagnostics endgame is larger than "emit
one coded diagnostic" — it's a recovering, kind-tagged, fix-carrying diagnostics pass.

**Note — likely spec-ahead-of-seed by design.** Like `value-interchange.md` and `build-tool-interface.md`, this
is a capability spec stating the target; the seed doesn't meet all of it yet, and the corpus rejection cases pin
only the codes (not recovery / primary-derived / kind / fixes). No gate breakage (these aren't corpus-expressible
as `(output (: v T))` — they're diagnostics-shape/behavior, and the corpus pins the single-rejection code, which
the seed meets).

**Acceptance signal / scope for the operator.** The concrete seed gaps, in rough priority: (1) a machine-branchable
rejection/decline/trap KIND on the diagnostic (unblocks the loop + agents routing around compiler limits — and
subsumes the ad-hoc decline discriminators ask-26/29/33); (2) error RECOVERY to report the maximal independent
set in one pass; (3) primary/derived marking; (4) structural fixes + verified/applicability markers. Each is a
separate, sizable piece; (1) is the smallest and highest-leverage. The corpus can pin recovery once the seed does
it (a case whose input has N independent errors and whose oracle lists N diagnostics — needs the diagnostics-
returning ABI, i.e. gated on ask-40/46).
Learning: `spec/learnings/2026-07-07-a-diagnostics-capability-spec-raised-the-bar-to-error-recovery-and-a-branchable-kind.md`.
