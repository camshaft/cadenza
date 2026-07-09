## 26. 🟠 The differential gate needs a trap-CAUSE discriminator — a decline and a semantic trap are indistinguishable by value alone (measurement gap, not a compiler bug)

**Update (2026-07-07) — interim harness side DONE.** `run_corpus.py` now disassembles the built component's
entry func (`is_bare_decline`) and splits the old `trap-ok` into **`trap-ok`** (RAN and trapped with real logic
before the trap — a verified semantic trap, e.g. `(/ 5 0)` → `i64.div_s`) vs **`trap-dc`** (a bare `unreachable`
decline that only coincidentally lands on a trap oracle). Verified: the four `Bytes.of`/missing-field cases moved
`trap-ok 4 → trap-dc 4`; a real `i64.div_s` semantic trap scores `is_bare_decline=False` (would be `trap-ok`).
So the interim harness no longer overstates conformance — a `trap-dc` reads as `decline` (frontier), and when a
construct gains real support its check moves to `trap-ok` (a wrong check surfaces as `hard`). **Still open:** the
real `component-check` differential (#22, unblocked seed-side) has the SAME blind spot and NO such discriminator
yet — it compares native-vs-mine values/traps in Rust and would count a decline-trap as agreeing with a
semantic-trap. The cheapest fix there is the in-range companion rule (below); the disassembly heuristic is
interim-harness-only. This item stays 🟠 until `component-check` gains a trap-cause check.


**Finding.** A value-first differential comparison cannot tell a **semantic trap** (the compiler executed the
trapping semantic — a byte-range check, a zero-divisor `i64.div_s`) from a **decline** (an unsupported construct
lowered to `KError → unreachable`): both produce the identical observable, a trap. On a *value*-expecting case
the two are distinct (a decline traps where a value is wanted → scored `decline`, the honest frontier); on a
*trap*-expecting case the distinction collapses and a decline scores as a correct trap. Verified 2026-07-07: all
four realized `trap-ok` cases in the interim harness (`Bytes.of` out-of-range/negative/runtime, missing field)
are bare-`unreachable` declines — `compiler.cdz` doesn't support `record`/`Bytes.of`, so it never examines the
byte value (a VALID in-range `(Bytes.of (list 65 66))` also traps). Right observable, wrong reason.

**Why it matters.** Not today's behavior (declining is correct now) but **masking**: when a construct gains real
support, a WRONG trapping check (off-by-one range, or no trap on a valid byte) would still score `trap-ok`/agree
for the out-of-range cases and regress silently — the comparison never distinguished the decline from the check.
A green trap-ok/trap-agree count reads as "these trapping semantics are conformant" when it can mean "these
constructs are unsupported and decline." This applies to BOTH the interim `run_corpus.py` (caveat added to its
README) AND the eventual `component-check` differential (SPEC-BACKLOG #22, now unblocked seed-side) — a
trap-vs-trap match agrees whether the trap is semantic or a decline.

**Fix (cheapest discriminator).** Pair each trap-expecting case with an **in-range companion that must NOT
trap** — e.g. alongside "byte out of range traps" `(Bytes.of (list 256))`, an in-range "byte in range yields a
sequence" `(Bytes.of (list 65 66))` that must produce a value. A decline traps on BOTH (fails the in-range
companion → visible); a correct implementation traps only on the out-of-range one. The in-range companion is the
discriminator a value-only trap oracle lacks. Most such in-range companions already exist as value cases; the
measurement fix is to REQUIRE the companion pass before a trap-expecting case's trap counts as conformance (a
gate rule / harness convention), not new corpus content. Until then, read every trap-agree as "traps, reason
unverified."
Learning: `spec/learnings/2026-07-07-a-decline-that-lands-on-a-trap-oracle-is-coincidental-agreement-not-a-semantic-trap.md`.

---

---

## ✅ DONE 2026-07-07 (conformance loop) — trap-cause discriminator in `component-check` (the seed side)

**Fixed** the residual blind spot ask-33 left in the run-the-artifact classifier. The "both trap" arm of the
byte-differing branch in `run_component_check` (main.rs) is reached ONLY after `is_decline_stub(comp_bytes)`
was already false — so a component that reaches it RAN REAL LOGIC and then trapped, not a bare-`unreachable`
decline. When native ALSO traps (the program's semantic is a trap), that is a genuine SEMANTIC-TRAP AGREEMENT:
changed `(Trap, Trap) => declined` → `(Trap, Trap) => agree`. The catch-all `_ => declined` now covers only
un-evaluable runs (no scalar `run()`, host error, Suspended).

**Why it closes the masking:** a WRONG trapping check (off-by-one range / no-trap-on-valid) can no longer hide
as a coincidental decline — the out-of-range case runs to a value / different outcome (⇒ a disagree/soft arm),
and its in-range value-companion (native=value, comp=trap) surfaces as `(Value, Trap) ⇒ decline`. The corpus's
~50 trap-expecting cases (mostly `06-numeric-model.sexp`) with existing value-companions give the discriminator
teeth — ask-26's "require the in-range companion" is satisfied by cases already present.

**Verified:** cdz-rustc (byte-identical to native) never enters the byte-differ branch ⇒ component-check
unchanged (577 agree/0/0/0, no regression). BEHAVIOR 572/0, IGNITION byte-identical, cargo test green. The
new arm's LIVE exercise (a byte-differing compiler running real trapping logic) awaits compiler.cdz being
parseable again (the sibling left it mid-edit unparseable ~17:20 — a compiler-side paren imbalance); the logic
is sound by construction. 📦 STABLE refreshed. Learning: `component-check-trap-cause-discriminator`. Both
harness sides now have the trap-cause split (interim `run_corpus.py` via `is_bare_decline`, DONE earlier; the
real `component-check` differential via this arm).
