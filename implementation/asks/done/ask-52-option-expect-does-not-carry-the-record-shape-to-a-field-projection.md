## 52. ⚪ `Option.expect` doesn't carry the unwrapped record's Shape to a field projection — the per-binding-form tail of the runtime-record-field-access landing

**Finding.** Runtime record field access landed (a `(. r f)` on a genuine runtime record emits `arr-get` at the
field's sorted-key slot, and a `match` arm binding a `Some` payload carries that payload's record Shape to the
bound name — so `(match (List.at inputs 0) ((Some a) (. a bytes)) …)` works). But the SAME field projection,
when the artifact is unwrapped with `Option.expect` instead of bound in a `match` arm, still declines:

```
(. (Option.expect (List.at inputs 0) "no input") bytes)
→ declined: runtime compound element of a kind the runtime cannot box yet
```

Verified on the refreshed stable seed (16:27). The match idiom is the idiomatic and working way to read the
input, so this is NOT a blocker — it's the narrow per-binding-form tail.

**Why it matters (the pattern).** Shape-carrying is per-binding-form: each construct that can bind a runtime
compound (a `match` arm, a `let`, an `Option.expect`/`Result.expect` runtime unwrap, a function parameter, a
tuple/record destructure) must separately thread the static Shape to what it binds, or a downstream `(. r f)` /
`tuple.N` on the bound value has no slot to index. The match-arm binder learned this in the field-access landing;
`Option.expect`'s runtime unwrap did not. So `Option.expect` on a runtime optional whose payload is a compound,
followed by a projection, declines.

**Acceptance signal.** `(. (Option.expect (List.at inputs 0) "x") bytes)` compiles VALID and projects the field
(matching the `match`-arm idiom's result). More generally, `Option.expect`/`Result.expect` on a runtime optional
carrying a compound payload should give the unwrapped value its payload's Shape, the same way the `match` binder
now does.

**Status.** ⚪ Seed — a narrow per-binding-form shape-carrying gap (the `Option.expect` unwrap path), the tail of
the runtime-record-field-access landing (which fixed the `match`-arm binder + runtime `(. r f)` + the `inputs`
param shape). Related: the field-access landing (done, seed-side), the runtime `tuple.N`/`(. r f)` twin,
[[runtime-option-expect-unwrap-or-trap]] (the earlier `Option.expect` scalar landing this extends to compound
payloads). Learning: `spec/learnings/2026-07-07-reading-a-field-off-a-runtime-record-completes-read-your-own-input.md`.

**✅ LOOP-VERIFIED 2026-07-07 (Run 98)** on stable 16:27 (SHA OK): the `Option.expect` field projection declines
as above; the `match`-arm form of the same projection compiles VALID (confirmed by echoing a fed input's bytes).
The behavior corpus pins the working (match-arm) form as a run-entry value case ("a field is projected off a
record bound through a match arm", gate 570→571).

**⏳ MOVED open→pending-validation 2026-07-07 (Run 100) — LANDED on LIVE, awaiting stable.** The compiler agent
reported ask-52 fixed (SEED-GAPS: `gen_member`'s `resolve` on the `(Option.expect …)` operand returned the node
unchanged, mistaken for a resolved non-record → `unreachable`/`Never` → the enclosing ctor's `box_scalar`
declined; fix routes an unchanged-resolve operand to the runtime-record path). Loop re-probe: `(. (Option.expect
(List.at inputs 0) "x") bytes)` → `Ok (0 bytes)` (NO decline) on LIVE seed 16:54, but STILL `declined: runtime
compound element…cannot box` on stable 16:46 (stable predates the fix — same per-fix stable-lag as ask-51/Run 99).
So: fix verified against the running fresh artifact; `done/` awaits the next stable refresh (≥ the ask-52 build)
+ SHA re-stamp. Both input-read idioms (match-arm and `Option.expect`) now work on live.

---

**🔎 2nd-probe DEBUG NOTE (loop, ~16:58) — a RUN-ENTRY sibling of this bug that was WORSE than a decline
(VALID-but-TRAPS), and the exact shape-derivation boundary. CONFIDENCE: HIGH (isolated by 7 probes).** The
compile-entry input-read is fixed; while confirming I found the same defect on the plain `run`/`emit` entry with
a different, more dangerous symptom, and narrowed exactly which operand shapes trigger it. The trigger is whether
`shape_of` on the `Option.expect` OPERAND resolves the payload record shape — and it hinges on how the Option
scrutinee is written:

| # | `main` body (run entry) | stable 16:46 | fresh 16:54 |
|---|---|---|---|
| P | `(. (Option.expect (Some (record (a 41)(b 42))) "x") b)` — scrutinee INLINE | ✅ `42` | ✅ `42` |
| Q | `(let ((o (Some (record …)))) (. (Option.expect o "x") b))` — scrutinee a LET name | ✅ `42` | ✅ `42` |
| R | `(def (mk n) (Some (record (a n)(b (+ n 1))))) … (. (Option.expect (mk 41) "x") b)` — scrutinee a CALL | 🔴 **`Trap`** (VALID component, traps at run) | ✅ `42` |
| Z | `(. (Option.expect (Some (mkr 41)) "x") b)`, `mkr` returns a bare record | 🔴 **`Trap`** | ✅ `42` |
| W | `(let ((r (Option.expect (mk 41) "x"))) (. r b))` — LET-bind the expect RESULT | ✅ `42` | ✅ `42` |
| T | `(Option.expect (mk 41) "x")` — expect ALONE, no projection | ✅ renders `(record (a 41)(b 42))` | ✅ |
| H | `(match (mk 41) ((Some r) (. r b)) …)` — match-arm binder (the corpus case) | ✅ `42` | ✅ `42` |

**What this localizes (all on the pre-fix stable):** the decline/trap fires ONLY when the field is projected
DIRECTLY off an inline `(Option.expect scrut …)` AND `scrut` reaches its `(Some (record …))` shape through a
user-function-call return (R, Z) — NOT when `scrut` is an inline literal (P) or a `let` name (Q), and NOT when
the expect RESULT is `let`-bound before projecting (W). So the failing path is precisely: `gen_runtime_member`
emits the `(Option.expect …)` operand inline, then re-derives its Shape via `shape_of` on that same expression,
and the `shape_of` expect-case (`codegen.rs:~860`, which does `shape_of_guarded(scrut)` expecting `Shape::Sum`)
does not recover the record payload when `scrut` is a call — even though (T) proves the value itself unwraps and
renders correctly. **`let`-binding the result (W) is the workaround** (it attaches the payload Shape to the bound
name via the `scalar_shaped` binder, bypassing the fragile inline `shape_of`-through-expect-through-call chain).

**⚠️ Symptom class — worth a note for the differential gate:** on the pre-fix seed R/Z were **VALID components
that TRAP at run** (the body compiled to a bare-`unreachable` stub, `(func (result i64) unreachable)`, called by
the entry), NOT honest declines. That is a decline that leaked past the retry into a run-time trap — exactly the
"decline vs semantic trap indistinguishable by value" gap ask-26/ask-33 track. A run-entry corpus case for the
call-scrutinee form (R) would guard against it regressing silently (the existing corpus pins only the match-arm
form H).

**Fresh build (16:54) status:** R and Z now return the correct `42`/`41` — so the compiler agent's fix ALSO
covers the run-entry call-scrutinee path, not just the compile-entry input read. Suggest confirming R/Z are in
the regression set before publishing; if they aren't, they're a cheap add that pins the boundary this note maps.
