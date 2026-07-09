## 30. 🔴 The self-hosted compiler has NO type-checker — it compiles ill-typed programs native rejects (33 corpus cases, the newly-exposed frontier)

**Finding.** With the `component-check` decline discriminator landed (ask-29), the byte gate's honest frontier
is visible: **58 agree, 152 disagree, 344 decline, 204 skip.** Categorizing the 152 disagreements:

- **117** `native=ok / component=ok`, byte-different — the fold-vs-overflow-helper `soft` middle ground
  (SUCCESS cases; values verified correct — native emits overflow-checked arithmetic helpers, `compiler.cdz`
  const-folds). Expected, not a bug.
- **33** `native=rejected / component=ok` — **the real gap: `compiler.cdz` COMPILES ill-typed programs that
  native REJECTS.** Verified directly: `(if true 1 false)` → native `declined: conditional branches have
  different types`, `compiler.cdz` → `Ok (89 bytes)`; `(+ 1 true)` → native `declined: operation on mismatched
  types`, `compiler.cdz` → `Ok`. The self-hosted compiler reads → resolves → folds → lowers → emits but runs
  **no type-rejection pass** — it never diagnoses a type error, it just compiles.
- **2** `native=declined / component=ok`; **0** cases where `compiler.cdz` is MORE strict (no false rejections).

The 33 missing rejections span three diagnostic families:

| code | count | what native rejects |
|---|---|---|
| CDZ0201 | 19 | conditional/type errors AND arity/malformed-form errors — see the split below |
| CDZ0301 | 11 | no-implicit-promotion operand type errors (`(+ 1 true)`, int-vs-float on `+ - * / % & \| < > <= >=`) |
| CDZ0210 | 3 | non-exhaustive `match` (bool match missing an arm; runtime scrutinee matching no arm traps) |

**The exact 33 (enumerated 2026-07-07 from `component-check`), and a refinement: not all are TYPE errors.**
The CDZ0201 group mixes THREE distinct rejection kinds — worth separating because a type-inference pass catches
only the first:
- **Genuine type errors** (need type inference): *a conditional with an integer then and boolean else*; *a
  conditional type error caught even when the mismatched branch is taken*; *integer/float branches*; *an integer
  `if` condition*; *a boolean connective with a non-boolean operand*; *an operation on mismatched types*;
  *ordering int-vs-bool*; *ordering int-vs-string*; *splicing a non-list into a quasiquote*.
- **Arity / malformed-form errors** (need a WELL-FORMEDNESS check at read/resolve, NOT type inference): *a
  conditional with a missing branch*; *a bare conditional keyword*; *equality applied to one operand*; *a bare
  equality keyword*; *an arithmetic operator with a single operand*; *a bare arithmetic keyword*; *an ordering
  operator with a single operand*; *a conditional with too many operands*; *a bare binding form with no bindings
  and no body*; *a binding form with bindings but no body*.
- CDZ0301 (all 11): int-vs-float on each of `+ - * / % & | < > <= >=` (no silent promotion).
- CDZ0210 (all 3): bool match missing false arm / missing true arm / runtime scrutinee matching no arm.

So ask-30 is really TWO passes: a **well-formedness/arity check** (≈10 of the 33 — structural, cheap, belongs in
the reader/resolver) and a **type-inference/rejection pass** (≈20 — needs kinds across branches/operands/match).
Both produce a rejection instead of lowering.

**Root cause of the arity subset, isolated (2026-07-07) — `read-app` reads fixed arity without checking the
application HAS it, silently dropping missing operands.** Verified by disassembling what compiler.cdz emits:
- `(+ 1)` (malformed, arity 1) → `i64.const 1` — reads the one present operand, drops the `+` entirely.
- `(if true 1)` (missing else) → `i64.const 1` — reads the then-branch, drops the missing else.
- `(+ 1 2)` (well-formed control) → `i64.const 3` (folds correctly).
So `read-app` dispatches an operator by head name and reads its *expected* operand count from the CBOR array
positionally; when the array is SHORTER than expected it reads whatever's there (or a default) and produces a
truncated node, rather than checking `app-arity == expected` and declining. **The cheap fix: `read-app` compares
the application's actual operand count to the head's arity and routes a mismatch to `KError` (decline).** This
alone converts the ~10 arity cases from mis-accept → decline (moving them `disagree → decline` in the byte gate;
`→ agree` awaits the diagnostics ABI for the coded `malformed … form` message). It is a reader-side structural
check — no type inference — and can land independently of and before the type-inference pass.

**Why it touches the spec / self-hosting.** A self-hosted compiler that emits a valid component for an ill-typed
program is a **reject-don't-miscompile violation at the whole-program level** — the strongest form, since the
program is accepted rather than declined. The spec mandates ill-typed programs are rejected with a coded
diagnostic (constitution: static typing mandatory post-pivot); `compiler.cdz` cannot self-host faithfully until
it performs the same rejections. This is the natural frontier AFTER the reader: the reader decodes the surface,
the type-checker rejects the ill-typed subset before lowering.

**Two coupled sub-gaps (both needed):**
1. **A type-checking pass in `compiler.cdz`** — infer/verify Kinds across `if` branches, operator operands, and
   `match` exhaustiveness, and produce a rejection instead of lowering. Much of the machinery exists (the
   return-kind fixpoint `build-ktab`, `kind-of`) — it computes kinds but does not yet *reject* on mismatch.
2. **The diagnostics ABI** — `compiler.cdz`'s only failure channel today is a TRAP (`KError → unreachable`), no
   CDZ code. Faithful rejection needs the `compile` export to return `result<list<u8>, list<diagnostic>>` (the
   WIT world already declares it) and a way to construct a diagnostic with a code. Until then a rejection can
   at best become a decline (trap), not a coded diagnostic — so even a type-checking pass can't match native's
   `rejected CDZ0201` output byte-for-byte; it would move these 33 from `disagree` to `decline`, not to `agree`.

**✅ UPDATE 2026-07-07 (spike) — the TYPE-INFERENCE subset ALSO LANDED compiler-side (the ~20 kind-mismatch cases).**
Added a `well-typed? : Core → Bool` rejection pass (+ `typecheck` wrapper) over the COARSE i64/Bool lattice
`kind-of` tracks, run PRE-FOLD (`typecheck-funcs` in `compile-program`, and in `compile-node`) so the fold can't
collapse a mismatch into a constant before the check — an ill-typed body → KError (→ trap). Rules: arithmetic/
bitwise/shift (`+ - * / % & | ^ << >>`) require BOTH operands i64; comparisons require MATCHING operand kinds
(i64=i64 or Bool=Bool); `not` requires Bool; `if` requires a Bool condition AND matching branch kinds; `let`
checks value+body; call checks args. Verified the mis-accept→reject flip: `(if true 1 false)`→was 1, now traps;
`(+ 1 true)`→was 2, now traps; `(< 1 true)`→was false, `(and 1 true)`→was true (desugared `if` cond not Bool),
`(^ 1 true)`, `(<< 1 true)`, `(not 5)` — all now reject. NO false positives: recursion, Bool-returning helpers
(used as `if` cond AND as return value), nested lets, multi-def+calls all still compile+run correctly; harness
unchanged (23 agree / 9 soft / 0 hard / 0 error) — no previously-working case regressed. ⚠SCOPE: only the
i64/Bool lattice is representable, so Float/String type errors (int-vs-float CDZ0301 on the operators) are NOT
caught here — the compiler emits neither type yet, so those inputs don't reach a bad emission anyway; when the
numeric model lands they'll need a real type. **So BOTH ask-30 compiler-side subsets are done** (arity + coarse
type-inference), moving the ~30 mis-accepts → decline. **REMAINING (seed-side, agent's domain):** the
DIAGNOSTICS ABI — `compile : … -> result<list<u8>, list<diagnostic>>` + a coded-diagnostic constructor — so a
rejection returns the matching `CDZ####` instead of a bare trap, moving these decline → agree. Until then the
compiler REJECTS ill-typed programs (honest) but cannot emit the code.

**✅ UPDATE 2026-07-07 (spike) — the ARITY/WELL-FORMEDNESS subset LANDED compiler-side (the ~10 cheap cases).**
Implemented the reader-side arity check ask-30 flagged as independently-landable: `read-app` now compares the
application's `app-arity` to each FIXED-arity form's expected count and routes a mismatch to a new `malformed`
node (`"?"` head → PUnknown → KError → `unreachable`) instead of reading past/dropping operands. Covers `if`
(=3), `not` (=1), and every binary operator (=2). Verified the mis-accept→decline flip on the exact cases the
finding names: `(+ 1)`→was `i64.const 1` (dropped `+`), now TRAPS; `(+ 1 2 3)`→was `3`, now traps; `(if true 1)`
→was `1`, now traps; `(< 5)`, `(= 7)`, `(not 1 2)`→all now trap. Well-formed forms unregressed (`(+ 1 2)`→3,
`(if true 10 20)`→10, `(< 3 5)`→true, `(^ 12 10)`→6, `(<< 1 7)`→128). Harness 0 hard/0 error; self-hosts
(33310 B). So these ~10 move **mis-accept (disagree) → decline** — the honest reject-don't-miscompile state; `→
agree` still awaits the diagnostics ABI (coded `malformed … form: arity mismatch`). **Still open (the bigger
half):** the TYPE-INFERENCE/rejection pass (~20 cases — `if`-branch kind mismatch, operand type mismatch
CDZ0201/0301, `match` exhaustiveness CDZ0210) and the diagnostics ABI (`result<_, list<diagnostic>>` + coded
constructor). `kind-of`/`build-ktab` compute kinds but do not yet REJECT on mismatch; that pass is the remaining
compiler.cdz work, plus the seed-side diagnostics channel for the coded output.

**Acceptance signal.** As the type-checker lands, the 33 `native=rejected / component=ok` disagreements move to
`decline` (compiler traps on ill-typed input — honest but not yet coded), then to `agree` once the diagnostics
ABI lets it return the matching `CDZ####` rejection. Corpus already pins every one of these (they are existing
rejection cases native realizes); no new corpus needed — the gate measures it directly. [Arity subset: done
compiler-side 2026-07-07, now `decline`; type-inference subset + diagnostics ABI remain.]
Learning: `spec/learnings/2026-07-07-the-byte-level-gate-decline-discriminator-exposes-the-missing-type-checker.md`.
Related: ask-29 (the discriminator that exposed this), ask-11/12/14 (type-rejection gaps already fixed in the
SEED; this is the same class in the Cadenza-authored compiler), the diagnostics-ABI note in `compiler.cdz`'s
`compile` entry comment.

**🟢 LOOP-VERIFIED 2026-07-07 (Run 75) — arity subset landing CONFIRMED; type-inference subset + a let-form tail remain.**
Re-probed the spike's arity-fix claim directly: `(+ 1)`, `(+ 1 2 3)`, `(if true 1)`, `(< 5)`, `(not 1 2)` all now
emit a component that TRAPS (mis-accept → decline), well-formed forms unregressed (`(+ 1 2)`→3, `(if true 10
20)`→10). Byte gate moved 59→**61 agree**, 148→**136 disagree**, +decline; standing WRONG sweep = **0**. Of the
original 33 native-rejected mis-accepts, **21 remain** in disagree (down ~12):
- **~19 TYPE-INFERENCE cases** (the bigger half, still open): int-vs-float no-promotion on ALL of `+ - * / % & |
  ^ << >> < > <= >=` (CDZ0301), mismatched-type ops, int-vs-float `if` branches, ordering int-vs-string,
  non-list quasiquote splice, `match` exhaustiveness (`a runtime scrutinee matching no arm traps`). These need
  the type-inference/rejection pass — `kind-of`/`build-ktab` compute kinds but don't REJECT on mismatch.
- **2 LET-FORM arity cases NOT covered** by the `read-app` fixed-arity fix: `a bare binding form with no bindings
  and no body` / `a binding form with bindings but no body`. `let` is variable-arity (bindings list + body), so
  the `read-app` fixed-count check (`if`=3/`not`=1/binop=2) doesn't reach it. Confirmed still mis-accepted (`(let
  () )` → Ok). Needs a small `read-let` well-formedness check (bindings present + body present) — the same
  reader-side structural pattern, one form over. Cheap follow-on to the arity subset.

So ask-30 status: **arity/well-formedness subset ~DONE** (read-app fixed-arity forms) with a **let-form tail** (2
cases, one small `read-let` check); **type-inference subset (~19) OPEN** (the real remaining work) + the
diagnostics ABI for `→ agree`.

---

**🚧 SEED PROGRESS 2026-07-07 (sub-gap 2, the diagnostics ABI — operator chose to build it).** The
seed-side enabler is under way: emit the compile entry as `result<list<u8>, list<diagnostic>>` (the WIT
world already declares it) so the type-checker's rejection can be a coded `Err` instead of a trap.
- ✅ **Envelope LANDED** (the hard wasm-encoder part): `xtask` `build_compile_result_reference` +
  generated `COMPILE_RESULT_HEAD`/`COMPILE_RESULT_TAIL` consts, validates clean. Gotcha solved: an
  exported func referencing a `record` needs that record EXPORTED as a named type AND the signature must
  use the index `c.export(...)` returns (else wasmparser "func not valid to be used as export"). The
  consts are additive/unconsumed — compiler.cdz's existing `list<u8>→list<u8>` envelope is untouched,
  gate stays 569/0.
- 🔭 **Retptr layout confirmed** from the real cargo-component `compile` wrapper: core sig stays
  `(i32 ptr,i32 len)→i32 retptr`; the return area is `[disc:i32 @0][ptr:i32 @4][len:i32 @8]` (a
  `cabi_post` cleanup companion also exists). Both arms are lists = `(ptr,len)`; the Err arm's element
  `diagnostic = record{string,string}` = 4 i32s (2 string (ptr,len) pairs), so the Err path needs a
  nested marshal loop (list→per-diagnostic→2 strings→bytes via `cabi_realloc`).
- ⏭ **REMAINING (next iteration):** `compile_result_wrapper_body` (marshal both arms to that layout,
  GUARDED loops per the hand-emitted-wasm rule), detect a `Result`-returning compile body in
  `compile_component_module` → select the result envelope, corpus + gates. Host `decode_compile_result`
  is ALREADY ready (handles Ok/Err + bare-list fallback). Once landed, compiler.cdz's `Core.KError`
  (its universal reject marker, today → `unreachable`/trap) can return `Err([{code:"CDZ0301",…}])` and
  the 33 `native=rejected/component=ok` disagreements move → `agree`.
See [[diagnostics-abi-result-envelope]].

**🟢 LOOP-VERIFIED 2026-07-07 (Run 76) — the TYPE-REJECTION pass landed too (as a decline); ask-30 compiler-side
is DONE, residue is diagnostics + let-tail.** The spike landed a `well-typed?` type-rejection pass run PRE-FOLD.
Verified by discriminating disassembly (both-operands-supported, mismatch-only, so an unsupported-operand decline
can't be the cause): `(if true 1 false)` → DECLINE (int/bool branch mismatch), `(if 1 2 3)` → DECLINE (non-Bool
cond), `(if true (+ 1 1) false)` → DECLINE (mismatch survives the fold that would make the then `2` — proves
PRE-fold placement), while well-typed `(if true 1 2)` → compiles. int-vs-float no-promotion (`(+ 1 2.0)` etc.)
and mismatched-type (`(+ 1 true)`) all decline. So both compiler-side subsets (arity + type-check) are DONE as
**declines** — the mis-accept miscompile is gone (reject-don't-miscompile satisfied). WRONG sweep=0.

**Why the byte gate stayed flat (61 agree / ~137 disagree):** the ~21 cases moved mis-accept → decline, but
`component-check` still scores them `disagree` because native gives a CODED rejection (CDZ0201/0301/0210) and
`compiler.cdz` gives a trap (no code) — decline ≠ coded-rejection. **The sole remaining blocker to `→ agree` is
the DIAGNOSTICS CHANNEL** (ask-40, spike-filed): `compile` must return `result<_, list<diagnostic>>` with a
constructed coded diagnostic instead of trapping. That is now the frontier for these ~21.

**Residue for ask-30 itself:** the 2 LET-FORM well-formedness cases (`(let () )` still Ok — the `read-app`
fixed-arity check doesn't reach variable-arity `let`; needs a small `read-let` check). Everything else is
diagnostics-ABI-blocked (ask-40), not a type-check gap. **ask-30 compiler-side type-rejection: COMPLETE (decline
level).** Consider moving to done once the let-tail lands, tracking `→ agree` under ask-40.

---

**🔎 LOOP FULL DISAGREE INVENTORY 2026-07-07 (component-check on the diagnostics-ACTIVE `compile`, stable seed
17:44). The diagnostics ABI is LIVE and CLASSIFYING — but the earlier "type-rejection COMPLETE (decline level)"
is TOO OPTIMISTIC: 89 rejection cases still emit a VALID COMPONENT (`component=ok`), NOT a decline or a
diagnostic. The check pass simply doesn't detect these error classes yet. This is the precise, prioritized
target list. CONFIDENCE: HIGH (every case from the gate's own output; the diagnostics wiring itself is verified
working — `(+ 1 true)` → `Diagnostics:[CDZ0201]`, `(+ 3 5)` → Ok component).**

Gate: **95 agree / 94 disagree / 25 soft / 364 decline / 204 skip.** Of the 94 disagree: **0 wrong-value**
(zero `native=ok/mine=ok` mismatches — no miscompiles), **5 mine-ahead-of-native** (`native=declined`), and
**89 = `native=rejected / component=ok`** — mine compiles an ill-typed program native rejects. **⚠️ These 89 are
NOT "decline, awaiting a code" (as the residue note implies) — they are `component=ok`, i.e. mine emits a real
component. So the check pass is not rejecting them at all; the diagnostics ABI can't surface a code for a
rejection that never fires.** The 89, grouped into DETECTOR FAMILIES (what the check must learn to detect):

| # | detector family | codes |
|---|---|---|
| 11 | **eq/compare type-mismatch** — `=`/`<>` across different shapes/types/nominal boundary (records w/ diff fields, tuples diff arity, sum disjoint variants, map-vs-record, nominal-vs-plain) | CDZ0201/0202/0203 |
| 10 | **numeric no-promotion** — int-vs-float on `+ - * / % & \| ^ << >>` and comparisons | CDZ0301 |
| 6 | **ordering across types** — `< > <= >=` on int-vs-bool / int-vs-string | CDZ0201 |
| 6 | **match exhaustiveness** — bool match missing an arm; sum match missing a variant | CDZ0210 |
| 6 | **pattern type/arity mismatch** — literal pattern vs scrutinee type; tuple pattern wrong arity / vs non-tuple | CDZ0201 |
| 6 | **malformed VARIABLE-arity forms** — `(let ())`, `(let ((x 1)))` (no body), `(quote)`, `(tuple.N)`, `(. r)` with no field, empty quote — the `read-app` FIXED-arity check (`if`=3/`not`=1/binop=2) doesn't reach these | CDZ0201 |
| 5 | **member access on non-record** — `(. x f)` where x is a tuple/string/map/bool | CDZ0201 |
| 4 | **effect/capability checks** — effect reached with no handler/delegation; delegation never reached; handler arm names an undeclared op | CDZ0401/0403/0404 |
| 4 | **list elements heterogeneous** — `(list 1 true)` etc. (differ in type or shape) | CDZ0201 |
| 4 | **map values heterogeneous** — map values differ in type/shape | CDZ0201 |
| 4 | **quasiquote/unquote well-formedness** — unquote outside quasiquote, >1 operand, splice a non-list | CDZ0201 |
| 3 | **annotation contradicts value type** — `(: <tuple> Int64)` etc. | CDZ0201 |
| 3 | **apply-a-non-function** — `(5 3)`, `(true 3)`, `(3.0 x)` | CDZ0201 |
| 2 | **integer literal out of Int64 range** — `9223372036854775808`, `0xFFFFFFFFFFFFFFFF` | CDZ0201 |
| 2 | **duplicate record field** — `(record (a 1) (a 2))` | CDZ0201 |
| 2 | **tuple access on non-tuple** — `tuple.N` on a record | CDZ0201 |
| 2 | **nullary variant carries non-unit payload** — `(None 5)`, `(Sign.Zero 5)` | CDZ0201 |
| 2 | **malformed record/map entry** — field/entry missing its value | CDZ0201/0202 |
| 2 | **constructor over-application** — `(Some 1 2)` (see ask-21 — seed splits reject/decline here; the compiler.cdz check needs the same) | CDZ0201 |
| 1 ea | if-branches-different-types; non-Bool if condition; duplicate map key; operation-on-mismatched-types; unbound name | CDZ0201/0101 |

**What this tells the fix (CONFIDENCE: HIGH on the grouping, MEDIUM on effort per family):**
- The current `well-typed?`/`check-node` pass over the coarse i64/Bool/compound kind lattice catches the SCALAR
  mismatches (arith/cmp/if-cond/branch on i64-vs-Bool) — those are already `agree`. **The 89 are almost all
  COMPOUND-SHAPE and STRUCTURAL checks the kind lattice can't express:** equality/ordering shape-agreement,
  match exhaustiveness, pattern-vs-scrutinee shape, member/tuple access target shape, record/map field rules,
  nullary-variant payload, annotation-vs-value. These need the check to consult `Shape` (the structural type),
  not just Kind — a richer analysis than the i64/Bool/compound trichotomy.
- **Cheapest independent wins (structural/well-formedness, no type inference):** the 6 malformed VARIABLE-arity
  cases (a `read-let`/`read-quote`/accessor arity check — the ask's own let-tail, generalized), the 2
  out-of-range int literals (a reader range check), duplicate record field / map key (a set check at
  construction), malformed record/map entry. ~12 cases, reader/resolver-side, land without the kind/shape
  machinery.
- **numeric no-promotion (10, CDZ0301)** is its own axis: it needs a Float kind (the compiler emits no floats
  yet, so int-vs-float operands currently both look like... whatever the float literal lowers to). Blocked on the
  numeric model, like the ask notes.
- **The rest (~60) need SHAPE-aware checking** in `check-node` — the equality/pattern/access/exhaustiveness
  families. That's the bulk of ask-30's remaining type-inference half, now enumerated concretely.

**Correction to record:** the residue line "type-rejection COMPLETE (decline level)" holds only for the i64/Bool
SCALAR mismatches. The 89 above are `component=ok` (not even declines) — so ask-30 is materially open: the check
pass detects the scalar subset but none of the 24 compound/structural/well-formedness families above. Suggest NOT
moving ask-30 to done. (Also: because these emit a valid component rather than trapping, they are the mildest
form — a produced-but-should-be-rejected program — but still the reject-don't-miscompile-at-whole-program gap the
ask names.)

---

**🔧 TURNKEY FIX for the `let` well-formedness sub-case (2 of the 89) — root-caused in source 2026-07-07.
CONFIDENCE: HIGH (source-located + native rule pinned).** The ask has repeatedly flagged "a small `read-let`
check" for `(let ())` / `(let ((x 1)))` (no body) without the exact edit. Here it is:

- **Root:** in `read-app` (compiler.cdz, the `let` dispatch — currently `(if (head-is b (read-head-index b i)
  b"let" 3) (read-let b (read-child-off b i 1) (read-child-off b i 2) env 0 (cbor-arg-len-bindings …) fenv) …)`),
  the `let` branch dispatches to `read-let` with **NO arity guard**. `(read-child-off b i 2)` reads the body as
  application element 2 — but `(let ((x 1)))` has no element 2, so it reads a missing/garbage offset and
  `read-let`'s final `(read-node b body …)` reads whatever's there → a valid component instead of a rejection.
  Note: `head-is`'s 3rd arg (`3`) is the head NAME LENGTH (`"let"`=3 chars), NOT an arity — so nothing checks
  operand count here. Contrast `do` (guarded: `(if (< (app-arity b i) 1) (malformed) …)`) and `if` (guarded:
  `(if (= (app-arity b i) 3) … (malformed))`).
- **Native rule, pinned:** a `let` needs **`app-arity >= 2`** (bindings + body). Verified on native: `(let ())`
  arity 0 → rejected; `(let ((x 1)))` arity 1 → rejected; `(let () 5)` arity 2 → VALID; `(let ((x 1)) x)` arity 2
  → VALID; `(let ((x 1)) x x)` arity 3 → VALID. So the threshold is `< 2 → malformed`.
- **Fix (one line, the `do` pattern one form over):** guard the `let` branch —
  `(if (< (app-arity b i) 2) (malformed) <the existing read-let call>)`. Reader-side, no type/kind/shape
  machinery, covers both missing-body cases. `malformed` → `(KError 1)` → the check emits CDZ0201 (matching
  native's `malformed \`let\` form: arity mismatch`), moving these 2 `disagree → agree` on the diagnostics-active
  gate.

This is the single cheapest concrete win in the 89 (the "malformed VARIABLE-arity forms" family, 6 cases, is the
same pattern applied to `quote`/`tuple.N`/`(. r)`-with-no-field — each needs its own dispatch-site arity guard,
since the FIXED-arity `read-app` check (`if`=3/`not`=1/binop=2) never reaches these variable/special forms).

---

**🔎 CORRECTED SCOPE 2026-07-07 — the 89 are NOT all "mine emits a component that should be rejected." Most are
HONEST DECLINES (bare-`unreachable` stubs for constructs compiler.cdz doesn't support), MIS-SCORED `disagree` by a
gate measurement bug. The genuine type-checker gap is SMALLER than 89. CONFIDENCE: HIGH (disassembled the emitted
component per family).** Classifying each `native=rejected / component=ok` family by WHAT compiler.cdz actually
emits (entry-func disasm):

- **Honest DECLINE-STUB (bare `unreachable`, 0 real ops)** — mine doesn't support the construct, so it declines;
  native happens to REJECT it. Examples verified: `(record (a 1)(a 2))` dup-field, `(. (tuple 1 2) x)`
  member-on-tuple, `(5 3)` apply-non-fn, `9223372036854775808` int-out-of-range, `(match true ((true u) 1))`
  bool-match-missing-arm. **These are the record/map/tuple-access/apply/exhaustiveness families — compiler.cdz
  has no user-record/user-sum-match/apply-non-function support yet, so they lower to `unreachable`.** They are NOT
  a type-checker gap in compiler.cdz — they're an honest decline. The gate mis-scores them `disagree` ONLY because
  the ask-33 decline-discriminator is missing on the `native=rejected` branch (see the `📡` banner note: the
  `is_decline_stub` check in `component-check` runs only when `native=Ok`). Fixing that gate branch reclassifies
  them `decline` (correct), dropping the disagree count WITHOUT any compiler.cdz change.
- **DECLINE-that-TRAPS (real ops then `unreachable`)** — e.g. `(= (tuple 1 2) (tuple 1 2 3))` emits
  `unreachable; unreachable; i64.eq` (the tuple operands decline to `unreachable`, the `=` still emits `i64.eq`).
  Runs → trap. Also an honest decline (compound eq unsupported), also mis-scored disagree on the native-rejects
  branch. The run-the-artifact half of the gate fix catches these (component traps → decline).
- **GENUINE MIS-ACCEPT (runs to a wrong value)** — the real reject-don't-miscompile gap. Clearest: `(let ((x 1)))`
  (no body) → mine emits `i64.const 0`, a runnable component **returning 0** where native rejects. This is the
  `let`-arity bug (turnkey fix above). These are the cases that MUST be fixed in compiler.cdz (a detector), not
  just reclassified — and they are FEW relative to the 89.

**So ask-30's true remaining compiler.cdz work = the GENUINE MIS-ACCEPTS** (runs-to-a-value where native rejects:
the `let`/variable-arity malformed family, plus any scalar-representable mismatch not yet caught). The large
record/map/sum-match/apply families are DECLINES (compiler.cdz doesn't implement those constructs — tracked
elsewhere, e.g. ask-13 for list/sum match), and once the gate's native-rejects branch runs the decline
discriminator they stop counting as disagree. **Recommendation: (1) fix the gate branch (banner note) to get an
honest disagree count; (2) then the residual disagree = the genuine mis-accepts, which is the actual ask-30
detector list — far shorter than 89.** The scalar type-mismatch detectors (arith/cmp/if-cond/branch on i64/Bool)
already reject correctly (`(< 1 true)` → `Diagnostics[CDZ0201]` verified); the genuine remaining mis-accepts are
mostly the reader-side well-formedness guards (let/quote/accessor arity, out-of-range literal, dup field/key).

---

**🔎 RAN-THE-ARTIFACT SWEEP 2026-07-07 (snapshot compiler.cdz 18:27, seed stable 18:09). Ran each
native-rejected case's emitted component to split honest-decline (traps) from genuine-mis-accept (runs to a
value). RESULT: the genuine reject-don't-miscompile violations have narrowed to the `let` BODY handling — and it
has TWO bugs, one of them a WRONG-VALUE miscompile (not just a should-reject). CONFIDENCE: HIGH (ran both
compilers; root-caused in `read-app`/`read-let`).**

Landed since the last sweep (now correctly REJECT → `Diagnostics[CDZ0201]`): integer-literal-out-of-range,
`(if true 1 2 3)` over-arity, `(= 5)` eq-arity, `(do)` empty. The record/map/tuple-access/apply/exhaustiveness
families are honest DECLINES (Ok-stub that TRAPS at run — `(record (a 1)(a 2))`, `(. (tuple 1 2) x)`, `(5 3)`,
`(match true ((true u) 1))` all trap) — they need the gate's native-rejects-branch decline discriminator (banner
note), not a compiler.cdz detector.

**The only components that RUN TO A VALUE where native rejects/differs — the real bugs — are both `let`-body:**

1. **`(let ((x 1)))` / `(let ())` — no body → runs to `0`.** Native REJECTS (`malformed let form: arity
   mismatch`). Mine reads a missing element as the body → `i64.const 0` → a runnable component returning `0`. A
   produced-should-be-rejected miscompile. Fix = the `app-arity >= 2` guard (turnkey fix above).
2. **⭐ `(let () 5 6)` — multi-form body → runs to `5`, but native returns `6`. A WRONG-VALUE MISCOMPILE (a
   truncation), the most serious class under "same results" — and NOT a rejection case (native ACCEPTS it, both
   compile, values DIFFER).** Native treats a `let` body of >1 form as an implicit `do` (returns the LAST):
   `(let () 5 6)`→6, `(let () 1 2 3)`→3, `(let ((x 1)) x 2 3)`→3 (all verified on native). compiler.cdz's
   `read-let` reads the body as a SINGLE element (`(read-child-off b i 2)` — application element 2) and SILENTLY
   DROPS forms 3..N, returning the FIRST body form. Root, in `read-app`'s let dispatch:
   `(read-let b (read-child-off b i 1) (read-child-off b i 2) env 0 (cbor-arg-len-bindings …) fenv)` — the body
   arg is one node; `read-let`'s tail `(read-node b body …)` reads only it.
   **Fix:** read the body as the form SEQUENCE (elements 2..app-arity) wrapped in an implicit `NDo` (the reader
   already has `NDo`/`read-call-args` from the `do` path) — i.e. the let body is `(do <form2> … <formN>)`. A
   single-form body (`(let () 5)`) stays `NDo` of one = the form itself, so no regression. This is a WRONG-VALUE
   fix (not a rejection), so it moves a `disagree` that the gate scores as a TRUE miscompile (`native=ok/mine=ok`,
   different value) → agree; it is higher priority than any should-reject case because it silently computes the
   wrong answer.
   ⚠ Not corpus-pinned: every corpus `let` uses a single body form (sequencing goes through an explicit `(do …)`),
   so the multi-body path is untested — worth a corpus case (`(let () 1 2 3)` → 3) once fixed.

**Net: ask-30's genuine compiler.cdz correctness work is now essentially the `let`-body dispatch (both bugs, one
site) plus whatever scalar mismatches remain; the bulk of the "89" is either landed, honest-decline (gate-measure
fix), or the record/sum-match feature gaps tracked under ask-13. The multi-body `let` truncation (#2) is the one
active WRONG-VALUE miscompile I can find and should be fixed first.**

---

**🔴 REGRESSION 2026-07-07 (compiler.cdz 18:38) — the `let`-arity fix LANDED but OVER-REJECTS valid multi-body
`let`, AND my prior "wrap the body in an implicit `do`" advice was WRONG. Two corrections. CONFIDENCE: HIGH
(source-located guard + native semantic probed by running).** The fix caught the missing-body case (good) but
guards `(if (= (app-arity b i) 2) …(malformed))` — **arity EXACTLY 2**. Native accepts `app-arity >= 2`. So valid
programs now FALSE-REJECT:

| case | native | mine (18:38) | |
|---|---|---|---|
| `(let () 5 6)` | VALID → **6** | `Diagnostics` | 🔴 over-reject |
| `(let () 1 2 3)` | VALID → **3** | `Diagnostics` | 🔴 over-reject |
| `(let ((x 1)) x 2)` | VALID | `Diagnostics` | 🔴 over-reject |
| `(let ((x 1)))` no body | rejected | `Diagnostics` | ✅ |
| `(let ())` no body | rejected | `Diagnostics` | ✅ |
| `(let ((x 1)) x)` single | VALID | Ok | ✅ |

Per the operator's own learning (`df0be03`: over-rejecting valid code is worse than under-rejecting), this
regression is worse than the original truncation — it now rejects working programs. **Guard fix: `< 2 →
malformed` (reject only arity 0/1), NOT `== 2`.** The dispatch is `read-app`'s let branch (comment claims "arity
EXACTLY 2" — that premise is the bug).

**⚠️ CORRECTION to my prior fix note — the multi-form body is NOT an implicit `do`; it returns the LAST form and
DROPS the rest (does NOT evaluate/sequence them).** Verified by running native:
- `(let () (/ 1 0) 5)` → **`5`** (native does NOT trap — the non-last `(/ 1 0)` is dropped; entry folds to
  `i64.const 5`). Contrast `(do (/ 1 0) 5)` → **Traps** (a real `do` evaluates every form). So `let`-body ≠ `do`.
- `(let () 5 6)` → 6, `(let () 1 2 3)` → 3, `(let ((x 1)) x 2)` → last form.
So the correct lowering for a multi-form `let` body is **read the LAST body operand as the body** (elements
2..arity, take the last) — NOT wrap in `NDo` (that would make `(let () (/ 1 0) 5)` trap, disagreeing with
native's 5). Simplest: `body = (read-child-off b i <app-arity>)` (the last operand), bindings read as before,
guard `app-arity >= 2`. (This also matches the reader already reading a SINGLE body node — just point it at the
LAST operand instead of element 2, and relax the arity guard.)

**Combined fix (both bugs, one site):** (a) guard `(< (app-arity b i) 2) → malformed` — rejects only 0/1;
(b) pass the LAST operand as the body — `(read-child-off b i (app-arity b i))` instead of `(read-child-off b i
2)`. Then: `(let ())`/`(let ((x 1)))` reject ✅; `(let () 5)`/`(let ((x 1)) x)` → single body ✅; `(let () 5 6)`
→ 6 ✅; `(let () (/ 1 0) 5)` → 5 ✅ (matches native's drop-non-last). ⚠ Still not corpus-pinned — add
`(let () 1 2 3)` → 3 once fixed.

---

**🗺️ FRONTIER MAP (loop, Run 111, 2026-07-07) — the remaining 80 `native=rejected / mine=ok` under-rejects,
by code and sub-cluster, to guide port priority.** Measured on the byte gate (stable 18:09 / compiler.cdz 18:38,
after agree reached 105). ask-30 has fallen sub-family by sub-family (Bool over-reject fix → bool-exhaustiveness
CDZ0210 → out-of-range CDZ0201 → malformed-`let` + duplicate-field/key CDZ0201); this is what's left:

| code | count | biggest sub-clusters (count) |
|---|---|---|
| **CDZ0201** | 50 | shape/type mismatch in comparison (6+2+2), member-access-on-non-record (5), applying-a-non-function (3), tuple pattern arity/shape mismatch (2+2), tuple-access-on-non-tuple (2), list/map homogeneity "elements/values do not share one type/shape" (2+2+2+2), over-applying a constructor (2), literal-pattern-type-mismatch (2), unquote-splicing-non-list (2), ordering-different-types (2) |
| **CDZ0301** | 14 | numeric types do not silently promote (10), ordering between different types (4) |
| **CDZ0210** | 4 | user-SUM non-exhaustive (ask-13 — needs the declared variant-count table) |
| **CDZ0203** | 3 | (arity/application) |
| **CDZ0202** | 3 | ordering |
| **CDZ0401/0403/0404** | 5 | capability/effect routing (undeclared effect, host routing) |
| **CDZ0101** | 1 | the one genuine unbound name (`y`) — the discriminator companion of out-of-range→CDZ0201 |

**Priority read for the compiler agent:** CDZ0201 is half the frontier, and its biggest single win is a
**shape/type-mismatch check** — "comparison/ordering/member-access/apply/pattern between values of incompatible
shapes" (roughly 25 of the 50 are one underlying "operand shape doesn't fit the operation" check, the same
`ck-of`/provable-mismatch machinery ask-53 built, extended from arith/cmp operands to member-access, application,
and pattern positions). **CDZ0301** (14) is the next cluster — a "no silent numeric promotion" check (mixed
int/float operands), largely the same provable-mismatch shape. **CDZ0210** (4) is gated on ask-13 (variant-count).
The capability codes (CDZ04xx, 5) are a separate routing concern. WRONG=0 throughout — every one of these is an
honest under-reject (mine compiles what native rejects), never a wrong value; all corpus-pinned already.

---

**🎯 LOOP RE-MEASURE 2026-07-07 (fresh build, stable seed 18:44 with the gate-fix; compiler.cdz 18:55). HUGE
progress + the remaining disagree bucket is now ONE trivial fix. CONFIDENCE: HIGH (component-check + per-case
codes).** The gate measurement bug I flagged last cycle is FIXED (the native=Err branch now runs the decline
discriminator symmetrically — verified in `main.rs` ~L452, comment echoes the diagnosis). Result: **disagree
94 → 14, agree → 105, decline → 427, WRONG-value = 0.** The record/apply/exhaustiveness/member families correctly
reclassified `disagree → decline` (honest declines, not miscompiles).

**⚠️ CORRECTION to the CDZ0301 framing above: the 14 are NOT "under-rejects (mine compiles what native rejects)."
Mine DOES reject all 14 — it just emits the WRONG CODE.** Every one of the 14 remaining disagrees is identical in
shape: **int-vs-float in an operator → native rejects `CDZ0301`, mine rejects `component=diagnostics["CDZ0201"]`.**
So the detector already LANDED (compiler.cdz's `CKFloat` machinery correctly flags all 14: `+ - * / % & | ^ << >>`
int-vs-float, and `< > <= >=` int-vs-float ordering). It is purely a code-SELECTION bug: `check-arith` and
`check-cmp` call **`(emit-diag 201)` hard-coded** (compiler.cdz, the `check-arith`/`check-cmp` defs, ~L2081/2087),
regardless of whether the mismatch is int-vs-Bool (→ correctly CDZ0201) or int-vs-Float (→ should be CDZ0301).

**Fix (CONFIDENCE: HIGH — one code-selection change, no new machinery):** in `check-arith`/`check-cmp`, when the
provable mismatch is caused by a **float** operand, emit **301** instead of **201**. compiler.cdz already
distinguishes the kinds — `ck-of` returns `CKFloat` for a float operand (used by `ck-provably-not-i64`/
`ck-provably-mismatch`). So: if `(ck-of a)` or `(ck-of b)` is `CKFloat` (i.e. a float operand is present in the
mismatch), `(emit-diag 301)`, else `(emit-diag 201)`. Verified this is float-SPECIFIC and won't regress Bool:
**0 int-vs-Bool operator cases are in the disagree list** (`(+ 1 true)`, `(< 1 true)` already `agree` with
mine's 201 — corpus pins those as CDZ0201, `07-type-system.sexp:50`). And the corpus pins the int-vs-float
ordering as CDZ0301 (`07-type-system.sexp:72`), the arith int-vs-float as CDZ0301 (`06-numeric-model`).

**Net: this single code-selection fix moves ALL 14 remaining disagrees → agree (14/14 are this one bug), taking
the gate to 0 disagree / 0 WRONG.** After it lands, the only non-agree buckets are `decline` (honest
unsupported-construct declines — record/map/sum-match, ask-13 territory) and `soft` (byte-fidelity, deprioritized
ask-43). That is result-parity with native on everything compiler.cdz supports. (The `let`-body regression noted
above — over-rejecting valid multi-body `let` via the `== 2` guard — is separate and does NOT appear in these 14
because no corpus case uses a multi-form `let` body; still worth the `< 2` + last-operand fix to avoid rejecting
valid programs, but it is not blocking the gate today.)

---

**🎯 SHARPENED 2026-07-07 — the CDZ0301 fix's remaining gap has MOVED DOWNSTREAM to `code-string`. The check pass
NOW emits code 301 correctly; the code→string TABLE drops it to CDZ0201. One-clause fix. CONFIDENCE: HIGH
(source-traced the full emit→string pipeline).** Re-measured (fresh build, stable seed 18:44, compiler.cdz 19:03):
still 14 disagree, ALL identical — `native=rejected CDZ0301 / component=diagnostics["CDZ0201"]`, WRONG=0, 0 crashes.
Traced end-to-end this time:
- `check-arith` NOW calls `(emit-diag (numeric-mismatch-code a b ktab))`, and `check-cmp` too (both wired since my
  last note); `numeric-mismatch-code` = `(if (or (ck-is-float a) (ck-is-float b)) 301 201)` — **correct, emits 301
  for a float operand.** So the check-pass half of my prior recommendation LANDED.
- **BUT `code-string` (the `Int64 code → "CDZ####"` map) is a TWO-ENTRY table: `(if (= code 210) "CDZ0210"
  "CDZ0201")` — no `301` case, so 301 falls through to `"CDZ0201"`.** That is why the surfaced diagnostic is still
  CDZ0201 despite the check emitting 301. `code-message` has the same shape (only 210 special-cased).
- **Fix (one clause each, no new machinery):** in `code-string`, add `(= code 301) → "CDZ0301"`; in `code-message`,
  add a 301 message (native's is "numeric types do not silently promote" for arith/shift, "ordering between values
  of different types" for `< > <= >=` — either is acceptable since the corpus matches on the CODE, not the message).
  This is the LAST hop for all 14: the number is already correct at the emit site, only the string table lacks the
  entry. Verified 210 already round-trips (`code-string 210 → "CDZ0210"`), proving the emit→collect→record→string
  pipeline carries distinct codes; 301 just needs its table row.
- ⚠️ **The message stays generic per-code** (`code-message` maps code→one fixed string), so mine's CDZ0301 message
  won't match native's wording — but the gate's `outcomes_match` compares the CODE only (`diags.first().code ==
  ncode`), so a correct "CDZ0301" string is sufficient to flip all 14 → agree regardless of message text.

**After this one-clause `code-string`/`code-message` addition, the gate goes 14 → 0 disagree / 0 WRONG** —
result-parity with native on everything compiler.cdz supports (remaining non-agree = honest `decline` for
unsupported constructs + `soft` byte-fidelity). Supersedes my prior "fix check-arith/check-cmp" note: that half is
done; the residue is the `code-string` table.

**⚠️ Transient observed (not the finding, flagging for awareness):** compiler.cdz was churning during this probe
(commit `147f3f8`: "a disagree-drop hid a decline→crash regression in a node kind the new check didn't model" —
the agent already caught+fixed a float-node crash). Mid-edit I saw the exhaustiveness case `(match true ((true u)
1))` momentarily emit an `Ok` decline-stub (88 B) instead of its CDZ0210 — likely transient churn, worth a
re-probe once settled to confirm 210 still fires (it round-trips through `code-string` fine, so if it regressed
it's in `check-node`'s match arm, not the code table).

---

**🟢 MILESTONE 2026-07-07 — the `code-string` 301 clause LANDED (commit `418f5ec`) and the byte-level
self-hosting gate is now GREEN: `component-check` PASS, 0 disagree, 0 WRONG. ask-30's rejection frontier is CLOSED
at the gate level.** Re-measured (fresh build, stable seed 18:44, compiler.cdz 19:10):
- **COMPONENT-CHECK: PASS** — 120 agree / 25 soft / 434 honest declines / **0 disagree** / 0 crash / 0 wrong-value.
  `(+ 1 2.0)` now → `Diagnostics[("CDZ0301",…)]` (was CDZ0201) — all 14 int-vs-float cases flipped to agree.
- **BEHAVIOR-GATE: PASS** — 574 passed, 0 failed.

So the ask-30 arc is done at the differential-gate level: every `native=rejected` case is now either a matching
coded rejection from compiler.cdz (agree) or an honest decline (unsupported construct — the 434, dominated by
record/map/user-sum-match/apply-non-function, which are ask-13 / feature-implementation territory, NOT a
type-checker gap). No component runs an ill-typed program to a value (0 WRONG). **Recommend: ask-30 can move
toward done** — the type-rejection pass + coded diagnostics it called for are landed and gate-verified; what
remains under the "89" is honest declines tracked by their own feature asks.

**⚠️ ONE latent bug NOT visible to the gate — still worth fixing (CONFIDENCE: HIGH):** the `let`-body handling
regression persists. `(let () 5 6)` → mine `Diagnostics` (over-rejects) where native is `VALID → 6`; `(let ((x 1))
x 2)` same. It does NOT appear in `component-check` because NO corpus case uses a multi-form `let` body (all use a
single body or explicit `(do …)`). So the gate is green despite this. The fix (documented above): guard
`(< (app-arity b i) 2) → malformed` (not `== 2`), and read the LAST operand as the body (native drops non-last
forms; a multi-form body is NOT an implicit `do` — `(let () (/ 1 0) 5)` → native `5`, does not trap). Add a
corpus case `(let () 1 2 3)` → 3 so it can't regress unseen. This is the one place compiler.cdz still diverges
from native on a program native accepts — a false-reject, the failure mode the operator flagged as worse than
under-rejecting.

---

**🔬 BEYOND-CORPUS DIVERGENCE SWEEP 2026-07-07 (compiler.cdz 19:14, stable seed 18:44) — now that the gate is
0-disagree, I hand-probed ~30 programs OUTSIDE the corpus (native-vs-mine, running the emitted component) to find
latent divergences the corpus doesn't exercise. RESULT: the multi-form `let` body is the SOLE divergence on valid
programs; everything else agrees. CONFIDENCE: HIGH (ran both compilers on each).** Probed and AGREE (native value
== mine value): helper calls, recursion (`fac 5`→120), 2-helper chains, deeply-nested arith, `(if …)` in arith,
`and`/`or`, all comparisons incl. Bool ordering (`(< true false)`→false, `(< false true)`→true), `>>`/`<<`/`^`,
`(do 1 2 3)`→3, `(do …)` inside a let body, negative results. Probed ill-typed and AGREE (mine rejects/declines
where native rejects, 0 run-to-wrong-value): compound arith operand, `(if 5 …)`, `(not 5)`, `(= 1 true)`,
`(+ 1 2 3)` over-arity, nested `(+ (< 1 2) 3)`, `(if (+ 1 2) …)`. **The ONLY divergences found: the multi-form
`let` body false-reject — `(let () 5 6)`, `(let () 1 2)` (in a helper), `(let ((x 5)) x (+ x 1))` all mine=REJECTS
vs native=VALID.** Notably `(do 1 2 3)` and `(do x 2 3)` inside a let body BOTH work — so `do` sequencing is
correct; the bug is specifically `read-let` reading a single body element. This sweep raises confidence that the
gate's 0-disagree is genuinely representative and the `let` multi-body fix is the last known correctness item on
programs native accepts (the 434 declines remain feature gaps, ask-13 et al.).

---

**🔎 NATIVE/SPEC CONSISTENCY GAP (NOT a compiler.cdz bug — mine matches native): `if` type-checks a const-DEAD
branch, but `match` does NOT type-check a const-DEAD arm — even though bool-`match` desugars to `if`. CONFIDENCE:
HIGH (probed native + mine on both).** Prompted by commit `9caaf7c`'s principle ("a fold that eliminates a branch
must not eliminate its type-check" — landed for `if`, corpus-pinned). I checked whether the SAME principle holds
for `match`. It does NOT, in native OR mine:

| program | native | mine | note |
|---|---|---|---|
| `(if true 1 (+ 1 true))` — dead ELSE ill-typed | 🔴 rejected CDZ0201 | Diag(CDZ0201) | ✅ dead branch checked (9caaf7c) |
| `(match 5 (5 100) (_ (+ 1 true)))` — dead `_` arm ill-typed | ✅ **runs → 100** | ok→100 | ⚠ dead arm NOT checked |
| `(match true (true 1) (false (+ 1 true)))` — bool match, dead arm ill-typed | ✅ **runs → 1** | ok→1 | ⚠ dead arm NOT checked — yet bool-match DESUGARS to `if`, which WOULD check it |
| `(match 5 (5 (+ 1 true)) (_ 200))` — LIVE arm ill-typed | 🔴 rejected | Diag | ✅ live arm checked |

Confirmed the dead-arm body is unchecked for ALL mismatch kinds (arith `(+ 1 true)`, non-bool `if` cond `(if 1 2
3)`, disagreeing `if`-branches `(if false 1 false)`) — native compiles them all when the arm is const-eliminated.
So native's `match`-with-a-const-scrutinee folds to the selected arm and DROPS the dead arms' type-check — exactly
the `9caaf7c` unsoundness, but on the `match` path, in the REFERENCE compiler. The bool-`match` case is the
sharpest: it desugars to `if`, and a directly-written `(if true 1 (+ 1 true))` IS rejected, but routed through
`match` it is accepted — a desugaring/fold-order inconsistency.

**Why it's not a compiler.cdz ask (yet):** mine mirrors native exactly (if-dead→reject, match-dead→accept,
match-live→reject), so the differential gate stays green — there is NO compiler-vs-compiler divergence to fix.
This is a **spec/reference-consistency question for the operator:** should a const-dead `match` arm's body be
type-checked (matching the `if` rule + the `9caaf7c` principle "an unevaluated branch cannot carry a deferred
error"), or is `match` deliberately lazier than `if`? If the former, it is a latent wrong-acceptance in BOTH
compilers (an ill-typed dead match arm slips through) and the fix belongs in the seed first, then forward-ported —
a corpus case `(match 5 (5 100) (_ (+ 1 true))) → CDZ0201` would pin it. If `match` is intentionally lazier, the
`9caaf7c` learning should note the `if`/`match` asymmetry so it isn't mistaken for a bug later. Flagging for the
decision; no gate movement either way. Low priority (const-dead ill-typed arms are rare), but it's a real
soundness seam the just-articulated principle exposes.
