# Plan: compiler-ml negative-sentinel / `0 - 1` cleanup (operator assign 2026-08-01, DE-PRIORITIZED — gated slices)

Operator (verbatim): *"The compiler-ml is using negative numbers everywhere as sentinel values and it's
really gross when we have sum types… It's also doing `0 - 1` to get those -1 values, which is another
massive head scratcher. All of this garbage needs to be cleaned up."*

## 🎯 THE STANDING MODELING RULE (operator 2026-08-01, supersedes all earlier framings — the master yardstick)
*"This compiler should be a pretty straightforward port of the rcdzc compiler… We want strong typing and
checked errors. If it's a compiler bug we should trap. If it's a checked get then we'd return an option or
result… Just treat it like an immutable rust program. Same exact conventions. But the way it is now we've got
potential silent corruption all over the place and it's not acceptable."*
- **TREAT compiler-ml LIKE AN IMMUTABLE RUST PROGRAM, SAME EXACT CONVENTIONS.** It's a PORT of rcdzc — mirror
  rcdzc's modeling, don't invent sentinel-integer encodings.
- **compiler BUG** (a should-never-happen, our own invariant violated) → **`trap`** (fail loud).
- **CHECKED GET / expected-can-fail** (an ill-formed INPUT program, a lookup that can legitimately miss) →
  **`Option`/`Result`** (a typed sum — absence/error is type-checked, not a magic value).
- **NO in-band sentinel integers AT ALL** — not even loud-failing ones (a magic 0/1 marker key is not
  idiomatic Rust; the idiom is an enum/Result). This is the bar for the marker-key follow-up below.
- **Yardstick for any ambiguous site:** *"what would rcdzc / idiomatic immutable Rust do here?"*

Two DISTINCT problems, very different risk. Measured on trunk 21c15bbe4.

## Measurement
- **139** negative-literal occurrences total; **127** are `0 - 1`; the rest are real negative VALUES
  (`0 - 5`/`0 - 128`/`0 - 32768`/`0 - 9223372036854775807` = test data + width/min-int constants, NOT sentinels).
- Per-file `0 - 1`: sread 39, int-width 14, infer-db 13, eval-db 10, lower-db 8, emit-db 8, parse-db 7, ty 3,
  resolve-db 2, sread-eval-ann 1.
- ✅ **The ML surface accepts a bare `-1` literal** (verified: `def f() = -1` checks clean via `cdz check`; the
  reader's negative-literal arm builds `NLit(0 - mag)`). So `0 - 1` → `-1` is a SAFE mechanical rewrite.

## Problem A — `0 - 1` as an obfuscated way to WRITE the constant -1  (LOW risk, mechanical)
The operator's "massive head scratcher #2." Every `0 - 1` that just needs the value **-1** can become the
literal `-1`. Pure legibility, zero semantic change, and it does NOT touch the sentinel-vs-sum-type question.
- Scope: the 127 `0 - 1` sites (+ the handful of other `0 - N` that are constants, e.g. `0 - 128` → `-128`,
  `0 - 32768` → `-32768`, `0 - 9223372036854775807`) — anywhere the negative appears as a VALUE, not built by
  arithmetic on a runtime operand.
- ⚠ EXCLUDE genuine runtime subtractions `(0 - mag)` / `(0 - x)` where the operand is a variable (e.g. the
  reader's `NLit(0 - mag)`, eval's negate paths) — those are real arithmetic, not the constant -1. Grep-filter
  `0 - 1` (literal) vs `0 - <ident>`.
- Risk: LOW (constant-fold-equivalent). Gate: sread-eval + parse-db + infer-db + full self-host suite green;
  co-verify NOTHING emit-boundary changes (a `-1` literal and `0 - 1` lower to the same Core const).
- Slices (by file, smallest-first so each gates independently): S-A1 ty+resolve-db+sread-eval-ann (6) ✅ LANDED
  (batch #128, c6dd034ce), S-A2 parse-db+emit-db (15), S-A3 lower-db+eval-db (18), S-A4 infer-db+int-width (27),
  S-A5 sread (39).

### S-A2 site list (read-only categorized 2026-08-01 — ALL are the constant -1, ready to build on a clean base)
parse-db.cdz (7): :211 `argId == (0 - 1)` guard, :287 `argId == (0 - 1)` guard, :534 `op-code` no-op sentinel
`| _ => 0 - 1`, :957 `NApp(bodyId, 0 - 1)` (+ :958 `is-app(…, 0 - 1)` test assert), :1118 `NApp(900, 0 - 1)`
test, :1134 `NApp(900, 0 - 1)` test. All → `-1`.
emit-db.cdz (8): :55 `rest == (0 - 1)` guard, :205 `| [] => 0 - 1`, :276 defidx-miss `0 - 1`, :474 `byte-of`
None `0 - 1`, :527+:528 `sleb(0 - 1)` test (the -1 VALUE fed to sleb), :637 `0 - 1`, :643 `0 - 1`. All → `-1`.
⚠ EXCLUDE (genuine runtime negation, NOT the constant): emit-db :341 `0 - n` (int-to-decimal negate),
parse-db :542-544 (`0 - x` unary-minus desugar — doc + the NBin build, a runtime subtraction). Leave as-is.
Gate: parse-db + emit-db own suites + a self-host smoke (sread-eval) — no emit-boundary change (`-1`≡`0 - 1`).

### S-A3 site list (read-only categorized 2026-08-01 — lower-db + eval-db, 18 sites, ALL the constant -1)
lower-db.cdz (8): :236/:242/:263/:358 `argId == (0 - 1)` nullary-call guards, :628/:649/:768/:780
`NApp(N, 0 - 1)` nullary-app builds. All → `-1`.
eval-db.cdz (10): :74 wildcard-binder `b == (0 - 1)` sentinel, :217 div-by-neg-1 guard `b == (0 - 1)`,
:541 eval-bin test `0 - 1` arg (×1 — see ⚠), :542 `7 % -1` test arg `0 - 1`, :561/:575/:587/:601/:603
`NApp(N, 0 - 1)` builds, :803 `Core.CNum(0 - 1, true, 64)` Core constant. All the `0 - 1` → `-1`.
⚠ **eval-db:541 CARE** — `(match eval-bin(37, (0 - 9223372036854775807) - 1, 0 - 1) with`: the `(0 - MAX) - 1`
is the Int64.min construction (LEAVE the `) - 1` intact); only the FINAL `0 - 1` arg → `-1`. Do NOT blanket
substring-replace this line — edit it specifically. (No other `0 - <ident>` runtime negation in either file.)
Gate: lower-db + eval-db own suites + sread-eval smoke (both are emit-adjacent — lower→Core, eval runs Core).

### S-A4 site list (read-only categorized 2026-08-01 — infer-db + int-width, all NEGATIVE CONSTANTS)
⚠ **S-A4 is NOT a pure `0 - 1` slice** — int-width is negative-constant-dense (`0 - 128`/`0 - 129` test data).
🪤 A blanket substring `0 - 1`→`-1` would MANGLE `0 - 128`→`-128` by coincidence (correct value, but fragile /
confusing) and could corrupt `0 - 129`→`-129`. **Do per-occurrence edits, NOT a global substring replace.**
Treat S-A4 as "write every negative CONSTANT directly" (the operator's head-scratcher applies to all of them):
- infer-db.cdz (13): :164/:189/:220/:234/:261 `argId == (0 - 1)` guards, :743 `enc == (0 - 1)`, :769
  `bid == (0 - 1)`, :880 `| _ => 0 - 1`, :900 `not (nm == (0 - 1))`, :1246/:1255/:1342 `NApp(900, 0 - 1)`,
  :1528 `record-ctor-tag-typed(…, 0 - 1)`. All `0 - 1` → `-1`.
- int-width.cdz (~14): `0 - 1`→`-1` (:116/:121/:136/:199/:207/:248), `0 - 128`→`-128` (:147/:210/:213×2/:243/
  :248), `0 - 129`→`-129` (:150/:268). All test-data negative constants.
- ⛔ EXCLUDE int-width:47 `((0 - half) <= v)` — `0 - half` is RUNTIME negation of a variable, NOT a constant.
  (No other `0 - <ident>` in either file.)
Gate: infer-db + int-width own suites + sread-eval smoke (infer-db is on the pipeline). LARGEST slice — could
also split int-width (self-contained, own suite) from infer-db as S-A4a/S-A4b if a single MR is unwieldy.

### S-A5 site classification (read-only categorized 2026-08-01 — sread.cdz, 39 `0 - 1` + a few `0 - 128/129`)
🪤 sread is the MOST hazardous S-slice — it MIXES constant-sentinels, runtime negations, and `0-1NN` literals.
Do PER-OCCURRENCE edits, NEVER a global substring replace. Three categories:
- ✅ REWRITE `0 - 1`→`-1` (the constants, ~34): `op-code-of` miss (:79), `read-do-form` root-unset (:156/:1069),
  nullary NApp arg (:187), the read-def-body signature sentinels (:342/:358/:372/:388/:397/:411/:415/:425/:438/
  :441/:456/:459/:468 — bodyId/paramId/paramN = -1), `bad-type` bodyId (:481), export root-unset (:509/:514),
  `read-do-def` bodyId/paramId guards (:523/:529/:536/:539/:543), payload/binder -1 sentinels (:672/:675/:913),
  the NBin poison-op (:831), and the test asserts (:1512/:1523/:1602/:1625/:1788/:1817). All → `-1`.
- ⛔ EXCLUDE (runtime negation of a VARIABLE — NOT the constant): `NLit(0 - mag)` (:132), unary-minus `0 - x`
  desugar (:245 + doc :237), ann `0 - mag` (:305). These are real arithmetic — LEAVE.
- 🪤 `0 - 128` (:1589) / `0 - 129` (:1595) test constants CONTAIN `0 - 1` as a prefix → a blanket replace would
  MANGLE them. Rewrite to `-128`/`-129` as their own per-occurrence edits (same head-scratcher, still a constant).
Gate: sread + parse-db (imports name-id etc.) own suites + FULL sread-eval self-host smoke (sread is THE reader
— every pin routes through it). ~34 edits → LARGEST slice; fine to split by function-region if a single MR is
unwieldy. NOTE S-A5 overlaps Problem-B territory (many of these -1 sentinels are the SAME ones B2/B3 eliminate to
Option) — S-A5 only fixes how the -1 is WRITTEN; the sentinel-elimination is the separate B-pass on those sites.

## Problem B — ELIMINATE EVERY SENTINEL (operator HARDENED 2026-08-01) → Option or trap  (MANDATORY, not "nice")
🔴 **Operator (verbatim, hardened):** *"I don't want to use sentinel values at all in the compiler-ml. It's
better to return Option or trap. Otherwise we get silent corruption."* This RAISES Problem B from
"idiomatic-improvement" to a **REMOVE-ALL mandate**, and the objection is the SENTINEL PATTERN itself — so it
covers NON-negative in-band sentinels too (a magic `0`/other value standing for absence/error/not-found), not
just the negatives.

**The rule (operator's rationale = the design rule):** a sentinel causes SILENT CORRUPTION — a magic value
flows on as if valid and corrupts downstream instead of failing at the source. Replace EVERY sentinel with ONE
of exactly TWO sanctioned alternatives (NOT another in-band magic value):
- **`Option`** (typed absence) — where absence is a LEGITIMATE expected outcome the caller handles (None is a
  type-checked case, not a magic value).
- **`trap`** (fail loud) — where the sentinel was masking a SHOULD-NEVER-HAPPEN (the "silent corruption" the
  operator is killing: a -1 that flows on silently instead of failing at the source).
Pick per site by that rule; a genuinely-ambiguous site → RAISE to concierge (don't guess).

Negative-sentinel families (now MANDATORY-eliminate, ~counts):
1. **`argId = -1` = "nullary call, no argument"** (`NApp(callee, -1)`) — parse/resolve/infer/lower/eval (~30).
   → `NApp(callee, Option(NodeId))` (absence is legitimate — a nullary call). ⚠ HIGHEST ripple: a Node-arena
   field threaded through every pipeline stage AND an emit-boundary (lower→Core). Co-verify emit byte-identical.
2. **`paramId/binderId = -1` = "nullary def / wildcard `_` field"** (~16, sread) → `Option(NodeId)` (absence
   legit — no param / a `_` binds nothing).
3. **`bodyId = -1` = "unsupported signature → decline"** (~7) → this is a should-not-flow-silently case: it
   ALREADY routes to the `unsupported`/poison decline, so make the signature-reader return `Option` and let the
   caller decline explicitly (Option, since decline is an expected outcome — NOT trap; out-of-subset is legit).
4. **`argType enc = -1` = "unsupported payload type"** (~3) — a 3-way {≥2 int | 1 Bool | -1 unsup | 0 none} →
   `PayloadTy = TyInt(..) | TyBool | TyUnsupported | TyNone` sum. Emit-boundary — co-verify. SEQUENCE AFTER
   gap-B-C (both touch the encoding; gap-B-C is now built + queued 38e7cdca8).
5. **`op-code = -1` = "not an operator"** (~10) → `Option(Int64)` (a lookup miss is legitimate).
   ### B5 DESIGN (read-only surveyed 2026-08-01 — FIRST Problem-B slice, build-ready):
   Two distinct fns, both in-scope:
   - `sread.op-code-of(sym: String) -> Int64` (:76, `_ => 0 - 1`): sym→op-code, `-1` = "not an operator". Used
     purely inside sread.cdz (exported but NO external importer). Callers: `read-paren-form` :177
     `op-code-of(sym) == -1` (the OPERATOR-vs-plain-NAME discriminator) + `read-bin-form` :244
     `op-code-of(sym) == 45` (unary-minus check) + :250 `NBin(op-code-of(sym), …)` (the value producer, reached
     ONLY after the discriminator confirmed a real op, so -1 never flows into an NBin).
   - `parse-db.op-code(t: Tok) -> Int64` (:534, `_ => -1`): already `-1` (S-A2). Tok→op-code, same "not-an-op".
   FIX (VERIFIED restructure, read the fns 2026-08-01): `op-code-of` → `Option(Int64)` (None = not an operator).
   The discriminator is actually `read-app-or-bin` (:176), NOT :177 directly: rewrite to
   `match op-code-of(sym) with | Option.None(_) => (app path: nullary/param call) | Option.Some(op) =>
   read-bin-form(s, sym, op, a0, tree)` — THREAD the resolved `op` INTO read-bin-form as a new Int64 param so it
   does NOT re-call op-code-of. `read-bin-form` then uses `op` directly: unary check `op == 45`, NBin `NBin(op,
   …)`. Clean — ONE match, value threaded, zero re-calls. (:244/:250 the two re-calls disappear.)
   Old plan line for reference — :177
   re-call). `read-bin-form` restructures around ONE `op-code-of` match instead of 3 calls. `parse-db.op-code`
   likewise → `Option`. NOT emit-boundary (reader-only), so gate = sread + parse-db + sread-eval suites (no
   emit co-verify needed). LOW ripple (single module for op-code-of). This is the easiest Problem-B family →
   good FIRST elimination slice once Problem-A drains / a clean base is free.
   ⚠ **B5 SUBSUMES the S-A5 op-code-of edit** (sread:79 `op-code-of` miss + :177 `== (0 - 1)` discriminator):
   B5 REPLACES that `-1` with Option entirely, so do NOT also rewrite those `0 - 1`→`-1` in S-A5 (would be
   double-work / conflict). If S-A5 lands first, B5 just converts the `-1` it wrote; if B5 lands first, drop
   sread:79/:177 from S-A5's list. Sequence B5 and S-A5 mindful of this overlap (op-code-of is the only shared
   site — the other ~33 S-A5 sentinels are B2/B3 territory, not B5).

NON-negative sentinel sweep (NEW — the hardened bar):
6. **`ctor-argtype-of` returns `0` for a missing/nullary payload** (parse-db:451, and sread:911 folds a None to
   `0` arity) — a magic-`0` in-band absence. Audit whether `0` is distinguishable from a real Int64-placeholder;
   if it conflates "no payload" with "Int64 payload," that's exactly the silent-corruption pattern → `Option`.
7. Sweep the 58 "sentinel"-commented sites + any `=> 0`/`=> 0 - 1` lookup-miss fallthroughs for other in-band
   magic returns; each → Option (legit absence) or trap (should-never-happen).

- Risk: MEDIUM–HIGH. Emit-boundary families (1)(4) need byte-identical emit co-verify (assign constraint);
  (2)(3)(5)(6) internal, safer. Each family = its own gated slice; NO big-bang. A sum/Option that reaches
  lower/emit MUST co-verify emit unchanged on the self-host suite + width/payload corpus (not just lib tests).

### B-MARKER (RULED — follow-up slice): rework gap-B-C/illwidth marker-keys → a ReadResult sum
`illwidth-marker-key()`=0 / `unboundtype-marker-key()`=1 (sread) record a reserved DEF-TABLE entry as a
whole-program "this program is ill-formed" flag read by `run-src` via `def-body-of`. I RAISED this; **operator
RULED**: an ill-formed-program detection is a CHECKED ERROR → must be a `Result`/`ReadResult` sum, NOT in-band
marker integers (even loud ones — not idiomatic Rust). So this is a real cleanup, sequenced **NON-BLOCKING**:
- gap-B-C (38e7cdca8) LANDED AS-IS — closes a real reject-gap, marker fails LOUD (decline, not silent
  corruption), so it does NOT add the harm; DON'T rework a landed/queued MR retroactively for purity alone.
- **Follow-up slice B-MARKER:** change `read-source`'s return from `(root, tree)` to a
  `ReadResult = Ok(root, tree) | Declined(reason)` sum (reason enum: e.g. `IllWidth | UnboundType | …`),
  delete the two reserved marker keys + `has-illwidth-marker`/`has-unboundtype-marker`, and have
  `run-src`/`run-src-typed` match the ReadResult. ⚠ Touches read-source's signature + EVERY caller (the whole
  self-host + all sread tests). Emit-co-verify. The guest emit-ceiling is a design input for HOW: a COMPACT
  2-ish-variant sum (mirror how rcdzc models a read/parse outcome — we're PORTING rcdzc, so match its shape if
  it has a ReadResult/ParseResult). De-prio-respecting gated slice; sequence with the other Problem-B families.

## Sequencing — 🔴 RE-PRIORITIZED 2026-08-01 per OPERATOR STEER: LEAD WITH PROBLEM-B (sentinels are the point)
Operator: most `(0-1)` sites are sentinels needing Option, NOT domain values — don't let Problem-A cosmetic
churn crowd out the real elimination. AUDIT done → `vcml-sentinel-audit-domain-vs-option.md`: hypothesis
CONFIRMED (~100+ sentinel sites vs ~a dozen domain values).
1. **Problem A** (`0-1`→`-1`) — S-A1..S-A4 ✅ LANDED (#128/139/147/152). These were the DOMAIN-VALUE sites
   (int-width test data, div-guard, etc.) — legit de-obfuscation, done. **S-A5 DROPPED as a standalone cosmetic
   slice** — the audit shows most of its ~34 sites are SENTINELS (bodyId/paramId/argId/enc = B2/B3/B5) →
   Option-ize them in the Problem-B slices, do NOT re-spell them. (The one borderline domain `-1`, sread's NBin
   poison-op, is B5-adjacent.) **NO more Problem-A on sentinel sites.**
2. **Problem B — THE POINT, now leads** (eliminate sentinels → Option/Result/sum/trap), EASIEST→HARDEST:
   **B5 op-code→Option** (turnkey, designed) → **B7 emit-db lookups→Option** (index-of/func-index→Option,
   byte-of OOR→Option-or-trap) → **B3 bodyId→Option** → **B2 param/binder→Option** → **B4 argType-enc→sum**
   (gap-B-C landed, ready) → **B6 ctor-argtype-0→Option** (non-neg) → **B-MARKER read-source→ReadResult** →
   **B1 argId→Option** (biggest: Node arena field threaded parse→resolve→infer→lower→eval + emit-boundary, LAST).
   Each its own gated slice + emit co-verify where it reaches lower/emit. Yardstick: compiler-bug→trap,
   checked-get/ill-formed-input→Option/Result. REPORT progress as SITES ELIMINATED to Option, not re-spelled.
3. gap-B-C ✅ LANDED (#133). Breaker's differential reject-ledger EMPTY.

## Revised-scope note (2026-08-01 — operator standing rule, supersedes the hardening framing)
Operator gave the crisp modeling rule: TREAT compiler-ml like an IMMUTABLE RUST PROGRAM (it's a port of
rcdzc) — strong typing + checked errors; compiler-bug→trap, checked-get/ill-formed-input→Option/Result, ZERO
in-band sentinel integers (even loud ones). Problem B = eliminate EVERY sentinel to trap/Option/Result;
Problem A (0-1→-1) unchanged; + B-MARKER (my gap-B-C marker-key → ReadResult sum) as a ruled non-blocking
follow-up. Yardstick for any site: "what would rcdzc / idiomatic immutable Rust do?" Will note revised scope
to concierge.

## Status
PLAN ONLY (the assign asked for scope+plan → concierge, de-prioritized, don't jump the queue). Reporting this
to concierge. Will execute as gated slices when the prioritized lanes are quiet. Problem A can start any tick
(mechanical, low-risk); Problem B slices need their emit co-verify each.
