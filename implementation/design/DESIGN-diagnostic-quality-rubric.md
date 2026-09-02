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
`grade_diag_quality` reads). Two independent checks per fault:

1. **No forbidden phrase** — the message text contains none of the globally-forbidden substrings (§1).
2. **Required tokens present** — for a fault whose code is in the required-token map (§2), the message
   contains that code's mandatory tokens.

The check applies to the user-facing **message text of an emitted coded diagnostic ONLY** — never to
Rust source comments, doc-comments, or the `Reject::unsupported(` / `Code::…` constructor names. (This
matters: "not yet" appears in ~18 source comments but in **zero** emitted messages — see Grounding.)

Roll-out is **opt-in per corpus scope first** (a case-level or file-level `(diagnostic-quality)` marker),
graded on the same nix diagnostics bar, then tightened to default once the corpus is clean — no flag-day red.

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

**§1-note (TODO carve-out):** a `(trap "TODO")` / `(trap "TODO: …")` string inside a **suggested fix
template** (the code we tell the user to fill in) is legitimate and must NOT trip — the forbidden check
is on the diagnostic **message**, not on `(fix …)` suggestion payloads. Scope the `TODO` match to the
message field only.

### 1b. Internal-implementation leak (GLOBAL — every coded diagnostic)

Rationale: a user must never see Rust-internal vocabulary; if one of these reaches a corpus case the fix
is to make the site unreachable / not a user diagnostic, not to reword.

| Match (word-boundaried) | Rationale |
|---|---|
| `internal error` | leaks implementation state |
| `\bICE\b` | "internal compiler error" jargon |
| `panicked` / `panic!` | Rust runtime-failure leak |
| `\bunwrap\b` | Rust `Option`/`Result` API leak |
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

### NOT forbidden (explicit carve-outs — do NOT add these)

- `unsupported` / `not supported` — **CDZ0900 `UnsupportedConstruct` legitimately declares a construct
  unsupported.** This is the honest permanent semantics of a coded decline (declines must be coded, per
  operator directive), not a quality defect. Forbidding it would red every CDZ0900 case.
- `trap` — `potentially reachable trap` (CDZ0309), `always traps but its value is never used` are precise
  golden messages, not leaks.
- `None` / `Some` — **Cadenza's own `Option` constructors**, named legitimately in golden diagnostics
  (did-you-mean candidates, `` "`Some` needs its payload argument" ``, `` "`None` is nullary" ``, `"wrap
  the value in `Some`"`). NOT a Rust leak. (See the resolved calibration note in §1b.)

## §2 — Per-code required-token map

For a coded fault, the tokens a golden message MUST contain. Start with these load-bearing codes; widen later.
Match case-insensitively; "one of" = at least one pair/token present.

| Code | Name | Required (message must contain) | Rationale |
|---|---|---|---|
| CDZ0203 | TypeMismatch | one of: (`expected` AND `found`) · (`should be` AND `but`) | expected/found is the core Rust-golden shape; grounded on "field `x` should be Int64, but this one is Bool" and "expected a tuple with 2 elements, but this one has 3" |
| CDZ0101 | Unbound | `unbound` OR `not found` | names the failure; SHOULD additionally carry `did you mean`/`closest matches` when a near name exists (advisory — conditional, see §2-note) |
| CDZ0210 | NonExhaustive | `not covered` OR `exhaustive` | names the uncovered case |
| CDZ0213 | RedundantArm | `unreachable` OR `never reached` | names why the arm is dead |
| CDZ0308 | UnreachableBranch | `unreachable` OR `never reached` | same shape as CDZ0213 |
| CDZ0306 | UnusedBinding | `unused` | names the unused binding |
| CDZ0307 | DiscardedValue | `never used` OR `discarded` | names the discarded value |
| CDZ0301 | NumericMismatch | one of: (`expected` AND `found`) · `different` | numeric-domain expected/found |
| CDZ0302 | IntOutOfRange | `range` | names the valid range (grounded on "the valid range is") |

**§2-note (advisory did-you-mean):** whether a message *should* carry a did-you-mean is **conditional**
on a near-name existing, which the lint cannot know from the message alone. Do NOT make `did you mean` a
hard requirement for CDZ0101; instead the corpus keeps its existing per-case `(fix …)`/`(message "did you
mean")` pins for the cases where a near-name exists. The general lint asserts only the unconditional
`unbound`/`not found` token.

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

- `v-corpus-harness`: encode §1 + §2 as the C1 general lint; opt-in per scope first. `(b)` calibration
  targets: the surface is largely clean, so the calibration set is small — primarily the `\bNone\b`/`\bSome\b`
  word-boundary validation and confirming the CDZ0900 carve-out holds.
- `v-diagnostics`: validate `\bNone\b`/`\bSome\b` against the full corpus once the scaffold lands; as the
  lint flags any real weak message, rewrite it to golden standard co-landing its now-passing assertion `(c)`.
- Widen §2 to the remaining CDZ codes incrementally as the load-bearing ones settle.
