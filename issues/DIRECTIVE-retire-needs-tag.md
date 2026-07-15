# DIRECTIVE (operator, 2026-07-12): retire the `(needs …)` tag; rely on the decline mechanism

The operator has decided to **remove the `(needs <capability>)` tag from the corpus** and rely solely
on the existing **decline mechanism** to express "this generation doesn't do it yet".

## Why (the argument, with data)

`(needs …)` and decline are redundant, and `needs` is strictly weaker + actively hides bugs:

- **Mechanism** (xtask `grade_ran`): a `(needs …)` case returns **Todo _before the program is ever
  compiled or run_** — it is skipped. A case with no `needs` is **run**: it grades **Pass** (correct
  value — a live regression guard), **Fail** (wrong value / bad trap / an `(error)` case that ran to a
  value — a caught bug/regression), or **Todo** (the compiler declined — exactly what `needs` asserted,
  but decided by the compiler, not hand-annotated).
- So the decline mechanism **already** expresses "todo" correctly and automatically. `needs` just
  pre-empts the run, throwing away both the pass-guard and the regression-catch.
- **Measured impact of removing all `needs` corpus-wide** (scratch strip + gate, 2026-07-12 @365a4f2):
  **206 pass → 239 pass (+33 free live guards** the seed already computes but was skipping), and it
  **unmasks exactly 2 FAILs** (below). No other regressions.

## BLOCKER: 2 masked failures must be fixed BEFORE removing `needs`

Removing `needs` while these are open turns the gate RED (they become real FAILs). Fix these first, in
either order, THEN strip the tags:

1. **Runtime integer width not rejected** — `adv-runtime-width-not-rejected.sexp` (already filed).
   `(: 5 (UInt n))` with `n` a runtime value is silently accepted and runs to 5; must reject CDZ0302
   (a width MUST be a compile-time natural — numeric-model.md #An Integer Type Is Indexed By A
   Compile-Time Width). Same drop-instead-of-reject shape as the negative-width bug @c69f441:
   `const_width`/`width_in_env` return None for a non-const width, and None is treated as "no
   constraint" (annotation dropped) rather than "reject". Add the runtime/non-const branch to the
   CDZ0302 rejection.

2. **Effect with a duplicate operation name not rejected** — `(effect E (op f (-> Int64 Int64)) (op f
   (-> Int64 Int64)))` runs to 1 (the body) instead of rejecting CDZ0201. An effect's operations are a
   closed, statically-known SET; a duplicate operation name is the same ill-formedness already rejected
   for a duplicate record field `(record (a 1) (a 2))`, a duplicate module def, and a duplicate sum
   variant `(type T (A …) (A …))` (all → CDZ0201). The duplicate-member check reaches records / modules
   / sum variants but NOT effect operation sets — extend it to effect declarations. Corpus case: "an
   effect that declares an operation name twice is rejected" (14-effects-and-handlers.sexp:275),
   currently `(needs effects)`-gated (which is why it was hidden).

## How to remove `needs` (use the new refactoring tool!)

The operator wants you to **kick the tires on the new codemod / structural-rewrite tool** (`cdz-syntax`
codemod / matcher / `cdz-syntax diff` / lint — landed @df15c80, 3b2379d, 1e0d72a, 6fb04e1) for the bulk
edit. The transformation is mechanical: **delete every `(needs <cap>)` clause from every `(case …)`**
in `spec/semantics/*.sexp`. 482 tagged cases across ~28 capabilities (numeric-model 62, fallible-access
55, collections 55, binary-matching 49, sum-type-declaration 45, effects 43, … — full histogram in
[[rcdzc-adversarial-corpus-findings-2026-07-12]]). A structural rewrite that matches the `(needs …)`
node and removes it is the ideal test case for the tool; verify a sample by hand + round-trip
(`cargo xtask roundtrip`) after.

## Order of operations

1. Fix bug #1 (runtime width → CDZ0302) and bug #2 (effect dup-op → CDZ0201).
2. Confirm the ungated scratch gate is **0 fail** (currently 2 fail = exactly these two).
3. Use the codemod tool to strip all `(needs …)` clauses corpus-wide.
4. `cargo xtask gate` (expect ~+33 newly-passing cases, 0 fail) + `cargo xtask roundtrip` (0 failures).
5. Delete the `(needs …)` vocabulary from the corpus README and the gate's `CorpusRecord.needs`
   handling (grade_ran's early `if !rec.needs.is_empty()` return) once no case uses it.

## Note

If a capability genuinely CRASHES or HANGS the seed (not just declines), keep it out — but none of the
28 do today (they decline cleanly). The whole point is: decline already means todo; let the cases run.

---

## STATUS (2026-07-12, loop update)

- **Step 1 DONE.** Bug #1 (runtime width → CDZ0302) fixed `@e9294c5`. Bug #2 (effect dup-op) is now
  MOOT — `effect` is unmodeled and DECLINES cleanly (does not run to 1), so there is no masked FAIL; the
  dup-op check belongs with the eventual effects implementation, not here.
- **The gate MECHANISM is already retired** (`@d572403`): `grade_ran` no longer early-returns on
  `(needs …)`; every case is compiled and graded. So the two blockers are cleared and the gate is
  **0 fail** with `(needs)` clauses still present-but-inert.
- **Steps 3–5 (strip the ~529 `(needs …)` clauses + delete the vocabulary) are BLOCKED on the codemod
  tool** — see `implementation/asks/open/ask-88` and `ask-89`. Two limitations make the bulk edit
  unlandable today: (88) `rewrite` allows at most one `,@` splice, so a clause can't be deleted at an
  arbitrary child position (only via a fragile fixed-position pattern + `--fixpoint`); (89) `--write`
  re-serializes through the pretty-printer and **collapses each hand-formatted `.sexp` file onto one
  line** (1387 lines → 1), making the diff unreviewable and the corpus unmaintainable. The transform
  itself is correct (529 → 0 with the fixed-position + fixpoint pattern), but the output is unusable.
  RESUME the strip once a formatting-preserving (span-splicing) `--write` lands (ask-89), then do step 5
  (remove `CorpusRecord.needs` handling once no case uses it). The `(needs)` clauses are inert
  meanwhile — no urgency, purely text-cleanliness.

---

## STATUS (2026-07-13, loop update — STRIP LANDED @1ee9f7e9)

**Steps 3–4 DONE.** ask-88 (multi-`,@`-splice) and ask-89 (formatting-preserving span-splicing
`--write`) both landed (moved to `implementation/asks/pending-validation/`), which unblocked the bulk
edit. Stripped all `(needs …)` clauses corpus-wide with the codemod tool:

    cdz rewrite '(case ,@before (needs ,_) ,@after)' '(case ,@before ,@after)' \
        spec/semantics/*.sexp --write   (a 2nd pass + --fixpoint mops up nested/adjacent cases)

536 clauses removed across 20 files. Diff was MINIMAL and layout-preserving (0 added, 536 removed, every
removed line a `(needs)` clause, no reflow — the span-splicing edit works as ask-89 promised). Verified
`gate --check` 0 fail / 0 regressions / 0 newly-passing (behavior-neutral — the grade mechanism was
already retired @d572403) and `roundtrip` 1292/0. README "generation divergence" + "which cases a
generation runs" sections rewritten (they described `needs` as an active skip filter — now state decline
is the sole todo signal).

**Residual (step 5, OPTIONAL text-cleanliness — no urgency):**
- The per-file bullet `(needs X)` LABELS in `spec/semantics/README.md` (lines ~205–221: "…, `(needs
  numeric-model)` for rational arithmetic", etc.) still name the removed tag — descriptive-only, now
  slightly stale. Rephrase to "cases a later generation realizes" without the tag syntax.
- `CorpusRecord.needs` (xtask/src/main.rs:~1603) + the markdown converter (cdz-corpus markdown.rs:198,361)
  still PARSE a `(needs)` clause — inert (no case uses it), kept harmless. Remove once confirmed unused
  (⚠ the markdown round-trip test references `(needs collections)` — update those fixtures too).

The DIRECTIVE's core goal (retire the tag, rely on decline) is COMPLETE; only the above cleanup remains.

## STATUS (2026-07-13, loop update — README RESIDUAL DONE @f899fbfd)

The per-file README `(needs X)` labels are cleaned (rephrased to prose "a later generation realizes";
`(needs …)` removed from the inline-annotations list). The only surviving `(needs)` mention in the
README is the sentence stating the tag was retired.

⚠ WHACK-A-MOLE: concurrent corpus work keeps RE-INTRODUCING `(needs …)` clauses (stripped 4 in
18-units + 1 in 14-effects this fire alone) because the parser still ACCEPTS the clause. Each strip is
trivial (`cdz rewrite '(case ,@before (needs ,_) ,@after)' '(case ,@before ,@after)' <file> --write
--fixpoint`) but a `--check`/CI lint that FAILS on any `^\s*\(needs ` clause, or removing the parser
support (below), is what would durably stop it.

REMAINING (the last residual): remove the inert `CorpusRecord.needs` parse-support so `(needs)` no
longer parses at all — xtask/src/main.rs:~1603/1667/1701, cdz-corpus/src/lib.rs:~49/184/342/400, and
cdz-corpus/src/markdown.rs:~198/361 (⚠ the markdown round-trip TESTS at markdown.rs:~654/669 use
`(needs collections)`/`(needs effects)` fixtures — update or delete those). Once it no longer parses, a
re-introduced clause becomes a hard parse error (auto-enforcing needs-free), which also solves the
whack-a-mole. Deferred as a code change touching the record-format + markdown round-trip contract.

## STATUS (2026-07-13, loop update — RE-STRIP #2 @ee03f3bd; whack-a-mole confirmed recurring)

Stripped 10 more `(needs …)` clauses this fire (7 initial + 3 pulled in mid-rebase), all from
concurrent units-of-measure / compound-types work. This is now RECURRING every ~fire.

⚠ PARSER-FIELD REMOVAL (step 5) DOES NOT STOP IT — verified: the record parser's clause loop
(cdz-corpus/src/lib.rs `parse`) ends in a catch-all `_ => {}`, so removing the `Some("needs") =>` arm
would make a re-introduced `(needs …)` SILENTLY IGNORED, not a hard error. So that removal is pure
dead-code deletion with no anti-re-introduction value (and it touches the markdown round-trip contract +
its `(needs collections)`/`(needs effects)` test fixtures — net risk > payoff).

RECOMMENDATION (needs operator sign-off — it would turn concurrent worktrees RED): add a **corpus lint /
CI check that FAILS on any `^\s*\(needs ` clause** in `spec/semantics/*.sexp` (e.g. in `cargo xtask
check`, or a `cdz lint` rule). That makes re-introduction a hard, self-enforcing error and ends the
whack-a-mole — the only durable fix. Until then this loop re-strips each fire (trivial: `cdz rewrite
'(case ,@before (needs ,_) ,@after)' '(case ,@before ,@after)' <files> --write --fixpoint`).
