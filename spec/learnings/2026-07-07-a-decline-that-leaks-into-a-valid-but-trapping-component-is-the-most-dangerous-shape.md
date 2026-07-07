# A decline that leaks into a VALID-but-trapping component is the most dangerous shape a decline can take — and the guard is a run-entry corpus case, not a discriminator

*2026-07-07*

**What happened.** The `Option.expect` field-projection fix (ask-52 — read a record field off an optional
unwrapped with `expect`) landed, and isolating it surfaced a sharper sibling defect on the run/`emit` entry. The
same projection, when the optional comes from a FUNCTION CALL rather than an inline literal or a `let` name, did
not honestly decline on the pre-fix seed — it emitted a **VALID component that TRAPPED at run** (the body compiled
to a bare `(func (result i64) unreachable)` stub the entry called). Seven probes localized it precisely: the
trap fired only for `(. (Option.expect <call> …) f)` where the scrutinee reached its `(Some (record …))` shape
through a user-function return; an inline `(Some (record …))` literal, a `let`-bound optional, or a `let`-bound
`expect` RESULT all worked. The root was `gen_runtime_member` emitting the `expect` operand inline and re-deriving
its Shape via `shape_of` on that same expression, where the `shape_of` expect-case didn't recover the record
payload through a call. On the fixed seed all forms return the correct value. I pinned the call-scrutinee form as
a run-entry corpus case ("a field is projected off a record unwrapped from an optional with expect", behavior
gate 571→572).

**Why.** Two things make this worth keeping.

First, it is the reject-don't-miscompile ordering (wrong-value < crash < decline < correct) caught in its most
insidious middle. A decline is supposed to be SAFE — the compiler says "I can't do this" and emits a stub that
traps immediately and visibly, or is counted as a decline by the gate. But here the decline leaked PAST the
emit-retry and produced a component that *passes validation* and only traps when RUN. That is strictly worse than
an honest decline: `component-check` sees a VALID component and a run-time trap, which is indistinguishable by
value from a program whose defined semantics IS a trap (the exact ask-26/ask-33 measurement gap — "a decline and
a semantic trap are indistinguishable by value alone"). So a shape-derivation gap didn't just fail to compile a
feature; it manufactured a value-gate ambiguity. **The dangerous declines are not the ones that emit
`unreachable` at the entry — those are honest and countable — but the ones that emit a valid-looking component
whose trap is deferred to run time.** A proxy for "is this a decline" that looks at the entry function's shape
(the ask-33 blind spot) cannot see this one; only running the artifact can.

Second, the fix for a leaked-decline-trap is not a better discriminator — it is a CORPUS CASE. The measurement
apparatus can be taught to classify the trap (ask-26/33 do this), but classification only tells you the current
count is honest; it does nothing to stop the defect from silently coming back. A run-entry corpus case with a
concrete expected value (`42`) converts "this dangerous shape is currently fixed" into "this dangerous shape
cannot regress without turning the gate red." The sibling's debug note said exactly this — "a run-entry corpus
case for the call-scrutinee form would guard against it regressing silently; the existing corpus pins only the
match-arm form." That is the loop's standing mandate (a spike's verified-correct behavior must become a corpus
case, not stay a probe) applied to its highest-value target: the behaviors most worth pinning are the ones whose
FAILURE mode is a silent valid-but-wrong artifact, because those are the ones a human reviewer and a coarse gate
both miss.

**The requirement it drove.** Corpus: "a field is projected off a record unwrapped from an optional with expect"
— the `expect`-unwrap companion of last cycle's match-arm binder case, deliberately using a call-produced
scrutinee (the demanding form that trapped pre-fix), with expected value 42 (behavior gate 571→572). It witnesses
member-access-projects-a-record-field composed with expect-unwrap, and its real job is to guard the
leaked-decline-into-runtime-trap boundary. General lesson: **the worst shape a decline can take is a VALID
component that traps only at run — it is strictly worse than an honest entry-stub decline and it re-creates the
decline-vs-semantic-trap value ambiguity; you cannot discriminate your way out of it (an entry-shape proxy can't
see it), so when a fix converts such a leak into a correct value, pin that value as a run-entry corpus case — the
behaviors most worth a regression guard are the ones whose failure mode is a silently valid wrong artifact.**
