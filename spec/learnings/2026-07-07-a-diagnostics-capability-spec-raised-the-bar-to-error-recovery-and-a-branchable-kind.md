# A diagnostics capability spec raised the bar to error recovery and a machine-branchable kind — and it names the distinction the loop has been improvising

*2026-07-07*

**What happened.** A new tracked `spec/capabilities/diagnostics.md` landed, formalizing the compiler's
diagnostics contract. Probing each requirement against the (refreshed, integrity-verified) stable seed split
them into met and spec-ahead:

- **Met** (the corpus already pins these): every diagnostic has a stable code (`rejected CDZ####` cases),
  severity, machine-readable form.
- **Spec-ahead of the seed**, confirmed by probe:
  - **Maximal independent set in one pass** — the compiler MUST recover from an error and report ALL independent
    problems, not just the first. Seed violates: `(do (+ 1 true) (< 2 false))` (two independent type errors)
    reports only the first, then stops. No error recovery exists.
  - **Primary vs derived** — mark each diagnostic as a root cause or a cascade. Seed has no such model.
  - **A machine-branchable KIND** distinguishing a *rejection* (ill-formed), a *decline* (not yet handled), and
    a *trap* (runtime halt). The seed conflates rejection and decline at the CLI (`declined: …` for both).
  - Structural fixes, verified/applicability markers, precise spans — largely absent.

The striking one is the machine-branchable kind. **The conformance loop has spent a dozen cycles improvising
exactly this distinction** — the byte gate needed a decline discriminator because a decline (bare `unreachable`)
and a real miscompile looked identical (ask-29); it needed a trap-cause discriminator because a decline landing
on a trap oracle looked like a semantic trap (ask-26); it needed a runtime-trap check because "entry is bare
unreachable" missed 77 hidden declines (ask-33). Every one of those was the loop reconstructing "is this a
rejection, a decline, or a trap?" from the emitted bytes, because the compiler didn't *say*. `diagnostics.md`
now makes the compiler saying it a normative requirement — a machine-branchable kind on the diagnostic — which,
if the seed implements it, retires the loop's whole discriminator apparatus: the gate would read the kind
instead of disassembling the entry func.

**Why.** This is the third capability spec (after value-interchange, build-tool-interface) the loop has
reconciled at a freeze, and the pattern is now clear enough to name as the loop's job at a spec landing:
**probe the seed against each requirement the new spec adds, and split met-from-spec-ahead — a capability spec
states the target, not the current behavior, and the gap it opens is the real work.** But this one carried an
extra lesson specific to a long-running measurement loop: **a distinction the loop has been reconstructing by
hand may be one the spec intends the artifact to expose directly.** The loop built decline/reject/trap
discriminators as tooling because the compiler was opaque about its own outcome kind; the spec's #A Diagnostic
Names Its Kind says the compiler MUST *not* be opaque about it. So the loop's improvised discriminators were
compensating for a missing capability, and the durable fix is not a better discriminator but the compiler
emitting the kind — at which point the loop consumes it instead of reconstructing it. A measurement loop that
finds itself repeatedly reconstructing the same distinction from indirect evidence should suspect the artifact
owes that distinction as a first-class output.

**The requirement it drove.** No corpus case — the diagnostics requirements are diagnostics-shape/behavior, not
`(output (: v T))` values (the corpus pins the single-rejection code, which the seed meets; recovery /
primary-derived / kind / fixes aren't corpus-expressible until the diagnostics-returning ABI lands, gated on
ask-40/46). The output is ask-48, scoping the seed gaps the new spec opens, in priority: (1) a machine-branchable
rejection/decline/trap KIND — smallest, highest-leverage, and it subsumes the loop's ad-hoc discriminators
(ask-26/29/33); (2) error recovery / maximal independent set; (3) primary/derived; (4) structural fixes. All
spec-ahead-of-seed, no gate breakage. General lesson: **at a capability-spec landing, probe the seed against
each new requirement and record the spec-ahead gaps — and when the spec formalizes a distinction the loop has
been improvising from indirect evidence, that improvisation was a workaround for a missing first-class output;
the fix is the artifact exposing it, after which the loop stops reconstructing and starts reading.**
