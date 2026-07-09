# Spike: the compiler in aspirational Cadenza — findings (2026-07-05)

**What this is.** `compiler.cdz` is the compiler's vertical slice authored as if every
documented capability were realized. `implementation/` is gitignored, so the durable
output of this spike is not the source — it is this analysis, plus the corpus cases and
learnings it should spawn. The slice compiles `(module m (def (main) (+ 20 22)))` end to
end: `parse → check → lower → serialize → frame`.

## What the flagship program demands, by frequency

Counting `DECLINE(...)` markers across the slice — a proxy for how hard each missing
capability presses on the compiler:

| Capability | Hits | Where it bites |
|---|---:|---|
| **effects** | 10 | Scope/Fresh/Diagnostics/Unify as effects; handler stacks; scope save/restore |
| **numeric-model** (M4) | 5 | LEB128: `& \| >>` on `UInt8`/`UInt32`; width-indexed `(UInt 32)` |
| **sum-type-declaration** | 3 | `(type Instr …)`, `(type Ty …)`, nested/`Cons` list patterns |
| **collections-and-text** | 2 | `List.concat` / `List.append` in the lowerer |
| **fallible-access** | 1 | list `Cons/Nil` destructuring in `serialize` |

**Reprioritization signal.** The operator chose "effects for everything," and the count
confirms the consequence: **effects is now the single largest blocker for a
Cadenza-authored compiler**, ahead of the numeric model. The roadmap has effects at M6,
after numeric (M4) and traits (M5). This spike argues effects should move earlier *if*
the compiler is to be authored in this style — or, the state model should fall back to
threaded context (the option the operator declined) to keep effects at M6. That tension is
a real decision, not a detail. Recorded for the operator.

## Findings (design signals — these touch the spec, not just the seed)

**FINDING #1 — the IR should be a typed sum, so `compiler-pipeline.md` §Representation
needs amending.** The spec mandates instructions be "AST sum type values… constructed via
quasiquote," to avoid "string-tagged pseudo-structures." But `(Ast.List (list (Ast.Name
"i64-const") …))` *is* a string tag in a `Name` payload, and it forfeits exhaustiveness:
`emit-instr` over a typed `Instr` sum is a compile error until every opcode is handled,
extending "reject, don't miscompile" to the backend. **Proposed amendment:** typed IR sum
for the backend; quasiquote reserved for the genuinely-`Ast`-valued frontend/macro layer
(where the values really are `Ast`). Quasiquote does not disappear — it moves to where it
is honest.

**FINDING #2 — RESOLVED: the env is a threaded map, NOT an effect.** The spike originally
modeled Scope as a State effect with manual `snapshot`/`restore`. Isolating that cost showed
it re-implements, by hand, the lexical nesting a threaded immutable map gives for free:
`(check (extend env b) body)` hands the callee the extended env while the caller keeps its
own, so a binding vanishes on return with zero bookkeeping. **Decision:** dynamic-extent
context (diagnostics, fresh supply, unify store — alive until a handler returns) → effect;
lexical-extent data (visible in a tree region) → parameter. Argument-passing *is* lexical
scoping. This refines "effects everywhere" into a sharper, still-uniform rule rather than
abandoning it — Diagnostics/Fresh/Unify remain effects.

**FINDING #3 — "record and continue" is genuinely elegant as an effect.** The
`Diagnostics` handler resumes with `unit` after appending, so a phase records a diagnostic
and continues over the well-formed remainder (compiler-pipeline.md §Phases Recover From
Errors) with **no** threaded error list and no early return. This is the strongest
argument *for* the effects choice — the one place it clearly beats threading.

**FINDING #4 — RESOLVED: the effect-declaration surface is now spec'd.** The corpus only ever
*handled* ad-hoc operations (`choose`, `get`); there was no way to *declare* an effect. Landed
a unified `(effect Name (op op-name (-> T… R))…)` form — one surface for both intra-program
effects and host imports, the latter marked `(host)`. Ops are qualified (`Effect.op`), the
host-bound declaration IS the manifest grant (the old `(import (host …))` and
`(use (capability …))` forms are removed — one way to declare, not several), and two new codes
land: `CDZ0402` (an effect performed with neither a handler nor a manifest entry) and `CDZ0403`
(a handler arm for an op the effect does not declare). Pinned in
`options/effects-model/algebraic-one-shot.md`, `spec/capabilities/capabilities-and-effects.md`,
`options/code-shape/`, and witnessed in `14-effects` (+5 new compiler-idiom cases). See
`effect-declaration-surface-decision-2026-07-05` (memory).

**FINDING #5 — framing is mechanical, not a language stress point.** The wasm-module and
component-envelope encoders are thousands of lines of `Bytes.concat`, but they add nothing
to the backlog beyond what `Bytes` + LEB128 already surface. Do not over-index on them when
prioritizing; `Bytes` is already realized.

## Corpus cases this spike should spawn (durable follow-up)

Each idiom below is a self-hosting-shaped case tagged with the capability it needs, so the
aspirational source turns into gate pressure:

- `(needs effects)` — declare an effect with typed ops and handle it: a `State`-style
  counter effect (models `Fresh`); a `Writer`-style accumulator resumed with `unit` (models
  `Diagnostics` record-and-continue). These are the compiler's real idioms, beyond
  14-effects' `choose`/`get`.
- `(needs numeric-model)` — `uleb128`/`sleb128` as recursive functions over `UInt32`/`Int64`
  with `& | >>`; a known-answer case (e.g. `uleb128 624485 = [0xE5 0x8E 0x26]`).
- `(needs sum-type-declaration)` — a typed IR sum with an EXHAUSTIVE serializer, and the
  compile error when an arm is missing (the exhaustiveness payoff, as a rejection case).
- `(needs collections-and-text)` — `List.concat`/`List.append` building an instruction list.

## Recommendation to the operator

1. **Settle the effect-declaration form** (FINDING #4) — it blocks authoring the compiler
   in *any* effectful style and is a pure spec addition.
2. **Decide the M-ordering tension** (reprioritization signal) — either pull effects earlier
   than M6, or accept threaded-context state and keep the roadmap. The spike makes the cost
   of each visible; the call is the operator's.
3. **Amend `compiler-pipeline.md` §Representation** for the typed IR (FINDING #1).
4. Land the four corpus-case families above so the next `/build` feels the pressure.
