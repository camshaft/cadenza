# gap: a comment/doc inside a collection literal (list `[]`, record `{}`) is DROPPED

**Owner:** v-syntax (self-filed 2026-07-21, after landing the sibling MODULE-body fix `9ceb28afd`).
**Severity:** round-trip fidelity (comment/doc LOSS) + `cdz fmt` REFUSES the whole file (comment-drop guard). Not a miscompile — the comment never reaches the compiler — but a real content-preservation gap in the v-syntax core invariant.

## Reproduction (on trunk `9ceb28afd`)
```
def l() -> List(Int64) = [1, 2
  // trailing
]
```
`cdz fmt` → `refusing to format: would drop 1 comment(s)`. The reader tree is `("list" 1 2)` — the `//` is gone.
Same for a `///` doc before `]`, a comment BETWEEN elements (`[1,\n // mid\n 2]`), and a trailing comment before a record's `}` (`{ a = 1, b = 2\n // t\n }`).

## Root cause (deeper than the module fix — NOT a quick mirror)
`list_literal`/`record_literal` parse each element via `self.expr(PREC_SEQ+1)`. Unlike `stmt`, `expr` does NOT drain the current token's leading-comment slot, so ANY comment before/between/after an element is stranded in a token's leading slot and dropped. TWO sub-gaps:
1. **trailing before the closer** (`]`/`}`) — analogous to the module-body fix (drain the closer's leading slot after the element loop).
2. **interior, between elements** — the harder half: each element position needs leading/trailing comment capture, like `stmt` does for module members.

**Printer half is ALSO missing:** even constructing `("list" 1 (comment-after "t" 2))` by hand does NOT render as `[1, 2 // t]` — the inline collection printer renders `comment-after(...)` as a CALL, not a same-line comment. So a real fix touches BOTH the reader (capture) AND the printer (render a comment inside an inline/broken collection literal).

## Why not fixed in-tick
This is a cross-cutting change in the HOT recursive value-parsing path (`expr` / `list_literal` / `record_literal`), which also feeds match-arm and block-body element lists. The v-syntax loop has a known frame-size trap in this exact neighborhood (a heavy inline arm in the recursive expr/list hub tipped a deep-flat-chain test over the stack). It needs its own careful tick with the full `cargo test -p cadenza-syntax --lib` frame check, not a rushed end-of-tick edit.

## Scoping (suggested slices, smallest-first)
- **S1 SAME-LINE half — DONE (MR `155d0adfb` sent 2026-07-21):** same-line trailing `//` on a list element (`[…, x // note]`) captured as `(comment-after …)` + printer renders same-line and forces `]` to its own line. Reader mirrors the `variant()` loop; new `doc.rs::hardbreak_with(offset)`. lib 654, gate 4507p/0f. REMAINING S1: an OWN-LINE trailing comment/doc before `]` (`[1, 2\n // note\n]`) is still dropped/refused (no corruption) — that's the interior-slot drain, part of S3 really. And the `///`-in-a-list design Q (no `(doc)` member concept) is still OPEN — degrade to `//`? reject? — decide before doing the doc case.
- **S2 TUPLE same-line — DONE (superseded by `e5d0ee6cf`, LANDED trunk `7f3ef1aa0`):** same-line trailing `//` on a tuple element, factored into a shared `bracketed_comment_aware` + `print_elem_maybe_commented` (list + tuple). ALSO carried the PR#758 fix (gate the capture on the closer being next — an unconditional capture swallowed a non-last comment's `, …`). 1-tuple-with-comment left to drop-guard.
- **S2 SET same-line — DONE (MR `a45e4608b` sent 2026-07-22):** set `#(…)` desugars to `Set.of([…])`, elements are list elements — reused `bracketed_comment_aware` + gated capture end-to-end (parser set_literal loop + `#(…)` printer path). Compiles to wasm.
- **S2 RECORD/MAP same-line — DONE (MR `87e08ad77` sent 2026-07-22):** field/entry is a `(name value)` PAIR, so the `(comment-after "text" (pair))` wrapper is made transparent to the shape machinery: `is_pairs`/`is_record_shape` unwrap via `strip_comment_after`; new printer `bracketed_pairs_comment_aware` unwraps + renders `name = value // text` + forces the brace break. Compiles to wasm. **⇒ S2 COMPLETE: same-line trailing comment now preserved across list + tuple + set + record + map.**
- **S3 TUPLE+SET own-line //-half — DONE (MR `bf871e6d0` sent 2026-07-22):** same reader-only pattern as list
  (`take_comments_here` + `wrap_comments` before each element; printer already renders leading `(comment …)`).
  Tuple `first` capture also covers a leading comment on transparent grouping `(\n //c\n e)`. NOT gated to last.
- **S3 RECORD/MAP own-line //-half — DONE (MR `c31243cec` sent 2026-07-22):** needed the extra printer work: new
  `strip_field_comments` peels BOTH leading `(comment …)` + trailing `(comment-after …)` around a field-pair;
  `is_pairs`/`is_record_shape` use it; `bracketed_pairs_comment_aware` renders a leading comment above the field +
  forces the break. **⇒ S3 own-line interior comments COMPLETE across list+tuple+set+record+map.**
- **S3 LIST own-line //-half — DONE (MR `1d38c0dee` sent 2026-07-22):** turned out READER-ONLY — the printer
  ALREADY renders a leading `(comment …)` as a `// …` line above the element; `list_literal` just wasn't draining
  the element's leading slot. Fix = `take_comments_here` + `wrap_comments` before each element (NOT gated to last —
  own-line has no swallow hazard). REMAINING S3: same reader-only capture for tuple/set/record/map element loops;
  + the `///`-doc case (operator-gated).
- **S3** (the deep half — original scoping, now largely de-risked by the list finding): interior between-element / own-line comments in all containers.
  SCOPED 2026-07-22: BOTH a comment before the first element (`[\n //leading\n 1, 2]`) AND between elements
  (`[1,\n //mid\n 2]`) are dropped today (fmt refuses, no corruption). ROOT: `list_literal`/tuple/set/record/map
  call `self.expr(…)` per element, which — unlike `stmt`/`body_expr` — does NOT drain the element's LEADING
  comment slot, so an own-line comment before an element is stranded+dropped. FIX PATTERN (analogue of
  `body_expr` line ~686): before parsing each element, drain its leading trivia and wrap `(comment "text" elem)`
  (LEADING, own-line — distinct from the trailing `comment-after` already handled). PRINTER is the deep part:
  an own-line interior comment must FORCE the collection to break and emit a `// comment` line ABOVE its element
  (the bracketed helpers currently only do trailing). Multi-container; do it as its own tick with the full
  `cargo test -p cadenza-syntax --lib` frame-check (hot expr/list path). `///`-DOC-in-collection: 🟢 concierge
  interim = HOLD (D) drop-guard; concierge RECs (B) reject-with-diagnostic to operator, ruling routed — do NOT
  implement B/C until OPERATOR confirms (never A). So S3's `//` half is unblocked NOW; the `///` half waits on operator.

Route back to v-syntax (self). No cross-vertical dependency.
