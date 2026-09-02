# DESIGN — Diagnostic-quality rubric (C1 general lint spec)

**Owners:** rubric = `v-diagnostics` (message quality). General lint / grader = `v-corpus-harness` (C1).
**Status:** rubric v1 — machine-readable spec for the corpus-wide C1 diagnostic-quality assertion.

## Purpose

The corpus already enforces message quality **per-case** — `(error CODE (message "phrase"))` presence,
`(not "phrase")` absence, `(no-diagnostic "phrase")` program-scoped absence, and the
`(fix …)`/`(no-fix)`/`(count N)` structured facets. **C1 adds a GENERAL rubric applied to EVERY emitted
coded diagnostic automatically** — a corpus-wide golden-standard guarantee (Rust golden standard: clear,
actionable, expected/found, did-you-mean, no jargon-only, no future-promise deferral wording), not just
the cases that hand-pin it.

This doc is the rubric the C1 lint asserts. It is grounded against the current emitted-message surface
(see "Grounding" below) so the lint has **zero false-positives** on today's corpus.

## Scope of the lint

C1 asserts over each **emitted coded diagnostic** the grader already parses (the structured faults
`grade_diag_quality` reads):

1. **No forbidden phrase** — the message text contains none of the globally-forbidden substrings (§1).
   This is the sound, universal check and the ONLY one C1 enforces.
2. ~~Required tokens present (per-code map)~~ — **WITHDRAWN, see §2**: the codes are umbrellas, so a
   per-code required token mass-false-reds golden messages. Message *shape* stays in per-case pins.

The check applies to the user-facing **message text of an emitted coded diagnostic ONLY** — never to
Rust source comments, doc-comments, or the `Reject::unsupported(` / `Code::…` constructor names. (This
matters: "not yet" appears in ~18 source comments but in **zero** emitted messages — see Grounding.)

Roll-out is **opt-in per corpus scope first**, graded on the same nix diagnostics bar, then tightened to
default once the corpus is clean — no flag-day red. Two opt-in forms (landed #7851 + file-level #7872):

- **Case-level:** a bare `(diagnostic-quality)` facet inside a `(case …)` — enrolls that one case.
- **File-level:** a bare `(diagnostic-quality)` top-level form (a sibling of the `(case …)` forms, placed
  at the top of the file after the header comment) — enrolls **every** case in the file. This is the
  ergonomic form for a clean chapter (one line vs. a marker per case). The two OR (never override): a file
  can be file-level-enrolled AND carry a per-case marker on an outlier; both fire.

**Verification command:** `cargo xtask gate <chapter.sexp>` — it forwards to the nix per-case corpus
pipeline, which captures the `KIND_DIAGNOSTICS` wire and therefore *exercises* C1. The in-process gate
(and a bare `cargo test`) is **sidecar-blind** (`faults` is `None` → C1 inert), so it is NOT a valid C1
check. Exit 0 = GREEN (the gate fails non-zero only on a real FAIL); a §1 violation FAILs that case.

**Rollout method (per chapter):** add one top-of-file `(diagnostic-quality)`, `cargo xtask gate` it on
the nix bar; if a case is flagged, that message contains a §1 phrase → rewrite it to golden standard
(a real target), else land the enrollment. Enrolling non-diagnostic cases is harmless (no emitted fault
→ C1 no-ops).

## §1 — Forbidden-phrase set

Each entry: the phrase (as a **word-boundaried / anchored** match to avoid substring false-trips) + rationale.
A message containing any of these fails the lint.

### 1a. Future-promise / deferral framing (GLOBAL — every coded diagnostic)

Rationale for the class: these imply a **temporary** state — they mislead the user into believing the
*same code* will compile in a later version. A decline must state a **permanent** fact about what the
compiler represents ("this backend does not represent X"), never promise a future. (This is settled
policy — agents have already scrubbed "(not yet built)" suffixes from real messages; see Grounding.)

| Match (case-insensitive, word-boundaried) | Rationale |
|---|---|
| `not yet` | future promise ("not yet built/reducible/supported") — implies temporary |
| `unimplemented` | future promise; also reads as an internal-status leak |
| `\bWIP\b` | work-in-progress status leak, not a user statement |
| `\bTODO\b` | developer-status leak (exception: inside a suggested-fix code template, see §1-note) |
| `for now` | implies temporary |
| `coming soon` / `will be supported` / `will support` | explicit future promise |
| `later increment` / `not yet built` / `not yet reducible` | project-internal roadmap leak |

**§1-note (fix-stub carve-out) — REFINED 2026-09-02 after the first real flag:** a `(trap "…")` string is
a **suggested fix STUB** — runnable user-code the diagnostic tells the user to fill in (the Cadenza
analogue of Rust's `todo!()`), NOT diagnostic prose. Its contents (`(trap "TODO: collect")`,
`(trap "TODO")`, etc.) must NOT trip §1. This holds whether the stub is a separate `(fix …)` facet OR
**inline in the message text** — and CDZ0405 (HandlerNotExhaustive) emits it inline: `…add (collect () s
(resume (trap "TODO: collect") s))`. That message is golden (it hands the user a concrete completable
stub); flagging its `TODO` is a false positive. **Precise lint rule:** before the §1 scan, ignore the
contents of any `(trap "…")` s-expression in the message (strip them, or skip a forbidden-token match
that falls inside a `(trap "…")`). §1 governs the compiler's diagnostic PROSE — never the user-code it
suggests. (Surfaced by 14b `a handler that does not discharge every operation …`; routed to v-corpus-harness
for the lint tuning, 2026-09-02.)

### 1b. Internal-implementation leak (GLOBAL — every coded diagnostic)

Rationale: a user must never see Rust-internal vocabulary; if one of these reaches a corpus case the fix
is to make the site unreachable / not a user diagnostic, not to reword.

| Match (word-boundaried) | Rationale |
|---|---|
| `internal error` | leaks implementation state |
| `\bICE\b` | "internal compiler error" jargon |
| `panicked` / `panic!` | Rust runtime-failure leak |
| `unwrap(` / `.unwrap` | Rust `Option`/`Result` API leak — **CALL-SYNTAX only** (see the unwrap calibration below); the bare word `unwrap` is NOT forbidden |
| `compiler bug` | should-never-fire invariant text; if a corpus case hits it, the guard is misplaced |
| `unreachable!` | Rust macro leak |

**Calibration RESOLVED (2026-09-02): `None` / `Some` are NOT forbidden — dropped.** The original
concern was that these leak Rust's `Option` API. Corpus-wide validation (against the C1 scaffold in
#7851) found the opposite: **`None` and `Some` are Cadenza's OWN `Option` constructors**, and they
appear in many *golden-standard* emitted diagnostics — `` "`Some` needs its payload argument" `` (CDZ0201),
`` "wrap the value in `Some`" `` (actionable fix), `` "`None` is nullary" ``, `` "construct it as `None`" ``,
`"Some carries one payload"`, and as did-you-mean candidates (`closest matches: `t`, `None`, `Some``).
Forbidding them — even word-boundaried — would wrongly red these. They are not code-class-scopable either
(valid in did-you-mean/closest-matches across many codes). See the NOT-forbidden carve-out below. A genuine
Rust-`Option` leak would be Rust *syntax* (`Option::None`, a `Some(` with Rust call semantics), not the
bare constructor names — do not attempt to lint those two words.

**Calibration RESOLVED (2026-09-02): bare `unwrap` is NOT forbidden — scope to CALL syntax.** The rollout
flagged 05's CDZ0202 (NominalMismatch) newtype-boundary messages: `"Age and Int64 are not comparable
across the nominal boundary (unwrap the nominal to compare the underlying value)"` (fix: `"unwrap the
nominal with (match … ((variant n) n))"`). Same class as None/Some: **`unwrap` is a Cadenza SURFACE
operation** — `(unwrap …)` appears 81× in the corpus — so guiding the user to "unwrap the nominal" is
golden, not a leak. The genuine Rust leak is the **call form** `.unwrap()` / `unwrap()` (a panic-y
`Option::unwrap()` in an internal-error message), never the bare word. So §1 matches `unwrap(` / `.unwrap`
(call syntax) only; the bare word `unwrap` is exempt. (Routed to v-corpus-harness for the lint tuning,
2026-09-02; 05 enrollment deferred until it lands — the messages are golden and need NO change.)

### NOT forbidden (explicit carve-outs — do NOT add these)

- `unsupported` / `not supported` — **CDZ0900 `UnsupportedConstruct` legitimately declares a construct
  unsupported.** This is the honest permanent semantics of a coded decline (declines must be coded, per
  operator directive), not a quality defect. Forbidding it would red every CDZ0900 case.
- `trap` — `potentially reachable trap` (CDZ0309), `always traps but its value is never used` are precise
  golden messages, not leaks.
- `None` / `Some` — **Cadenza's own `Option` constructors**, named legitimately in golden diagnostics
  (did-you-mean candidates, `` "`Some` needs its payload argument" ``, `` "`None` is nullary" ``, `"wrap
  the value in `Some`"`). NOT a Rust leak. (See the resolved calibration note in §1b.)

## §2 — Per-code required tokens — WITHDRAWN (2026-09-02): unsound for umbrella codes; use per-case pins

**Status: the per-code required-token map is WITHDRAWN. §1 is the sound general lint; §2 must NOT be
enforced.** The original §2 assumed each `CDZ####` code carries a **uniform** message template
(so "every CDZ0203 says expected/found"). Corpus-wide survey (2026-09-02) disproves this: the rcdzc codes
are **BANDS / umbrellas**, and a single code legitimately covers many distinct situations with different
golden messages. A blanket per-code required token therefore **mass-false-reds golden messages**:

- **CDZ0203 (TypeMismatch)** is a general type-error umbrella. Golden messages include `` "`Box` takes 1
  type argument" ``, `` "`helper` is a value, not a type" ``, `"not fully determined"`, `"function of arity
  1"`, `"guard condition must be Bool"`, `"an Int64 and a String are different types"` — **none** contain
  `expected`/`found`/`should be`. Requiring them would red the majority of CDZ0203 cases. (Even the
  tuple-arity golden form `"expected a tuple with 2 elements, but this one has 3"` has `expected` + `but`
  but no `found` and no `should be` → fails the shipped predicate.)
- **CDZ0101 (Unbound)**: golden messages include `"names no definition"`, `` "unknown type `Nonesuch`" ``,
  `"COMPILE-TIME-VISIBLE AST"`, `"not a type variable"` — none contain `unbound`/`not found`.
- **CDZ0210 (NonExhaustive)**: `"map binding pattern is refutable"`, `"a set match must end in a catch-all"`
  — none contain `not covered`/`exhaustive`.
- **CDZ0301 (NumericMismatch)**: golden `"floating-point or rational quantity"` — no `expected`/`found`/`different`.

**Message SHAPE belongs in per-case pins, not a per-code blanket.** The corpus already asserts shape
precisely and per-situation via `(error CODE (message "expected") (message "found"))` etc. — that is the
right layer for "this specific case's message must say expected/found", because it is scoped to a case
whose situation IS the expected/found shape. A per-code rule cannot distinguish CDZ0203-the-arity-error
from CDZ0203-the-field-mismatch, so it cannot demand a shared token soundly.

**What C1 enforces, then: §1 only.** The forbidden-phrase set is sound *universally* — no golden message
ever contains deferral/future-promise or internal-leak vocabulary, regardless of which situation the code
covers. That is the defensible corpus-wide golden-standard guarantee. `c1_missing_required_tokens` should
be removed (or made an unconditional `None`) from the grader; keep `C1_FORBIDDEN_PHRASES`.

(If a genuinely uniform code is later identified — one whose EVERY emitted golden message provably shares a
token — a §2 entry could be re-added for that single code, proven against its full emitted-message breadth,
not a representative sample. None of the surveyed codes qualify.)

## Grounding (why this set is false-positive-free on today's corpus)

Measured on `origin/main` (rcdzc src, excluding tests):
- **"not yet"** appears in ~18 places, ALL in `//`/`///`/`//!` comments or one non-diagnostic type-render
  string (`ty.rs`: "a deferred float agrees with Float32 (its width is not yet fixed)") — **zero emitted
  diagnostic messages.** The comments at `diag.rs:1088`, `diag.rs:1156`, `lower.rs:2903` explicitly record
  that the "(not yet built)" suffix was *deliberately removed* from the emitted text.
- **"for now" / "will be supported" / "coming soon" / "WIP" / "internal error" / "panicked" / "compiler
  bug"** — zero hits inside emitted message strings.
- **"unsupported" (204) / "not supported" (134)** — dominated by the `Reject::unsupported(` constructor
  name and CDZ0900's legitimate declares → correctly carved OUT of the forbidden set.
- Corpus `(message …)` assertion survey shows the asserted surface is already largely golden: did-you-mean
  ("closest matches", "did you mean"), expected/found ("field … should be …, but this one is …"), actionable
  fix templates. C1's value is therefore **preventive** (block future regressions) more than remedial.

## Hand-off

- **C1 status (2026-09-02):** §1 forbidden-phrase lint = shipped (#7851) and sound — KEEP. Two calibrations
  landed since: (1) drop `None`/`Some` (Cadenza Option constructors, not leaks); (2) **withdraw §2 entirely**
  (per-code required tokens are unsound for the umbrella codes — see §2). `v-corpus-harness`: please remove
  `None`/`Some` from `C1_FORBIDDEN_PHRASES` AND make `c1_missing_required_tokens` a no-op (or delete it +
  its call at grade_run), so C1 enforces §1 only.
- `v-diagnostics` (c) rollout: with §1-only C1, opt cases/chapters into `(diagnostic-quality)` and rewrite
  any message the forbidden-phrase check flags, co-landing the marker. NOTE: `(diagnostic-quality)` grades
  only on the **nix diagnostics bar** (the in-process gate is sidecar-blind), so opt-ins verify there.
- Message *shape* assertions (expected/found, "not covered", …) stay as **per-case** `(message …)` pins,
  which the corpus already supports and which are scoped to the situation — not a per-code blanket.
