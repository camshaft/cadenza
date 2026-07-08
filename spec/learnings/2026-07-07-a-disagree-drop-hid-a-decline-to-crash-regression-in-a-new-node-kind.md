# A disagree-drop hid a decline→crash regression in a node kind the new check didn't model — a conservative check must be silent on kinds it doesn't recognize, not just operands it can't prove

*2026-07-07*

**What happened.** A compiler.cdz change landed the shape-fits-position type check (extending ask-53's `ck-of`
machinery from arith/comparison operands to member-access, application, and pattern positions), and the byte gate
looked like a big win: disagree 85 → 22, the ~63 int/type ask-30 under-rejects leaving the disagree bucket. But
reading the four-bucket FLOW (the Run-106 discipline) told a different story: agree stayed 105, decline rose 364 →
427 (+63) — so the 63 became DECLINE, not agree — AND the 22 disagreements that REMAINED had all become a single
new class: `component run error: error while executing at wasm backtrace`. The compiler component was **trapping**
on every program containing a float literal.

Isolation confirmed a genuine regression, not a re-classification: on the same stable seed, the previous
compiler.cdz (18:38) DECLINED a bare float `(def (main) 4.5)` (honest `component=ok` stub), and the new one
(18:53) TRAPS on it — while native compiles it to `4.5`. So a float moved decline → crash. Per the
reject-don't-miscompile ordering (wrong-value < crash < decline < correct), that is a regression UP the severity
ladder: a crash on valid input is strictly worse than the honest decline it replaced. The headline "disagree
85→22" was progress on the int/type frontier and a regression on floats, simultaneously, and the single number
hid the second half.

**Why.** Two lessons, both sharpenings of prior ones.

First, the conservative-check invariant has a second axis. ask-53 established: **emit a diagnostic only when you
can POSITIVELY prove a mismatch; an operand whose KIND is unprovable defaults to silence.** That covered unprovable
operand kinds (a Bool parameter → `CKUnk` → no emit). But the float crash reveals the other axis: a node KIND the
check doesn't MODEL. The coarse lattice knows `Ki64`/`KBool`/`KError`; a float literal is none of these, and the
shape check — visiting every node to compute its kind — apparently assumed every node it reached had a modeled
kind and trapped (an `unreachable`/unhandled arm) on the one it didn't. So the invariant must extend: **a
conservative check must be silent not only on operands whose kind it can't PROVE, but on node kinds it doesn't
RECOGNIZE — an unrecognized node kind is `CKUnk` → decline, never a trap.** "I don't have a rule for this node"
must degrade to a decline, exactly as "I can't prove this operand's kind" does; the failure of both is silence,
because the check's job is to catch provable errors, and everything it can't classify is out of its scope, not an
occasion to crash. A checker that traps on an unmodeled node kind has confused "outside my competence" with
"impossible."

Second, this is the strongest vindication yet of reading the four-bucket flow over the headline (Run 106/107). A
loop that recorded "disagree 85→22, great progress" would have SHIPPED a compiler that crashes on every float. The
regression was invisible in the aggregate (the count went the right way) and glaring in the flow (a new
trap-class appeared inside the residual disagree, and decline rose instead of agree). The discipline that catches
it: after any change that moves the disagree count, (a) read where the departed cases WENT (agree vs decline —
here decline, so "not the payoff"), and (b) characterize what REMAINS (here: a homogeneous new float-trap class,
the regression). A drop in the bad-bucket count is necessary but not sufficient for progress; you must confirm
nothing got WORSE in severity, and a crash appearing where a decline was is exactly the worse-in-severity a raw
count can't show.

**The requirement it drove.** No new corpus case — the float cases are already pinned (they are how the gate
caught the trap), and they must return to decline/agree, not stay traps. The output is ask-55 (the isolated
regression: float decline→crash at compiler.cdz 18:38→18:53, with the likely root — the shape check traps on the
unmodeled float node kind instead of treating it as `CKUnk`) and this learning. WRONG=0 for wrong-VALUES still
holds (no float compiled to a wrong value), but a crash-on-valid-input is the next-worst outcome and the loop
flags it as a ship-blocker. General lesson: **a conservative check must be silent on node kinds it doesn't
recognize, not just operands whose kind it can't prove — an unmodeled node kind degrades to a decline, never a
trap; and a drop in the disagree count is not progress until you read the four-bucket flow and confirm no class
moved UP the severity ladder (decline→crash is a regression a falling headline hides).**
