# Proposal: a `range` is a first-class value; the slice family takes one

*Draft for sign-off — NOT yet normative. 2026-07-08. Supersedes the learning
`2026-07-08-string-slice-is-start-end-but-bytes-slice-is-start-length-spec-consistency.md` (which
recorded the divergence this proposal resolves). When accepted, its requirements move into
`collections-and-text.md` / `type-system.md` under stable headings and this file is retired to a
learning.*

## The problem this resolves

The sub-sequence slice operations carry an INVISIBLE, INCONSISTENT convention in two bare integer
arguments:

- `(String.slice "hello" 1 3)` → `"el"` — the third argument is an **END** index (scalars `[1, 3)`).
- `(Bytes.slice b 1 3)` → 3 bytes from index 1 — the third argument is a **LENGTH**.

Both conform to their own current spec, so the gate is green — but the two visually-identical call
shapes mean different things, and the mix-up is silent (both return a plausible `Some`). This is most
dangerous for a compiler authored in Cadenza, which manipulates BOTH `Bytes` (the wasm it emits) and
`String` (the text it renders) and is therefore the program most likely to call both and confuse them.

Aligning both to `(start, end)` would make them *consistent* but the meaning stays invisible:
`(Bytes.slice b 1 3)` still doesn't *say* what `1` and `3` are. The deeper fix is to make the
sub-range a **value with a name**, so every call site is self-documenting and no positional convention
can be transposed.

## The design

### A `range` is a first-class value

A `range` denotes a half-open interval of integer positions `[start, end)` — the positions from
`start` up to but not including `end`. It is an ordinary value: it can be bound, passed to and returned
from functions, compared, matched, and rendered, exactly like a tuple or a record.

- **Construction:** `(range start end)` where `start`/`end` are `Int64`. Both are ordinary
  expressions (a literal, a variable, a computed value).
- **Type:** `Range` — a structural type with two `Int64` fields (`start`, `end`). Structurally it is
  the record `{start: Int64, end: Int64}`; `Range` is the nominal name for that shape. (This reuses
  the existing tag-free-heap + `shape_of` + pattern-matching machinery — a `range` is a two-field
  compound at runtime, no new heap kind.)
- **Projection:** `(. r start)` / `(. r end)` read the fields (member access, already the sole
  accessor). A `range` also pattern-matches: `((range s e) …)` binds both endpoints.
- **Render:** `(range 1 3)` renders canonically as `(range 1 3)` (its constructor form), like any
  compound value.
- **Emptiness / validity:** a range with `start = end` is EMPTY (selects nothing) — well-formed, not
  an error, exactly as `String.slice`'s `start = end` already selects no scalars. A range with
  `start > end`, or a negative endpoint, is not rejected at construction (it is an ordinary value);
  it is the CONSUMING operation (the slice) that decides in-bounds-ness, keeping construction total.

### The slice family takes a `range`, uniformly

Every sub-sequence operation takes a value and a `range`, and the range's `[start, end)` semantics are
the SAME for all of them — one convention, named once, inherited by the whole family:

```
(String.slice s (range start end))   ; scalars [start, end)   → Option<String>
(Bytes.slice  b (range start end))   ; bytes   [start, end)   → Option<Bytes>
(List.slice   xs (range start end))  ; elements [start, end)  → Option<List>   (future, same shape)
```

Slicing stays **fallible** (collections-and-text.md #Indexing And Lookup Are Fallible): a range with
`0 ≤ start ≤ end ≤ len` yields `Some` of the sub-sequence; any range outside that (negative start,
`end` past the length, `start > end`) yields `None`, never reading beyond the sequence. This is
unchanged from today — only the ARGUMENT SHAPE changes (one `range` value instead of two bare ints),
and `Bytes.slice`'s third-argument meaning changes from LENGTH to the range's END.

`at`-style single-position access (`List.at`, `Bytes.at`, `String.at`) is UNCHANGED — it takes a
single index, not a range. Only the sub-sequence (slice) family takes a `range`.

### Reader sugar (optional, deferred)

The `(range start end)` constructor is the canonical form. A later reader-sugar amendment MAY add
`start..end` surface that reads to the same `(range start end)` node (as `b"…"` sugars to
`(Bytes.of …)`), and MAY add open-ended forms `..end` / `start..` / `..` (Rust's
`RangeTo`/`RangeFrom`/`RangeFull`) as distinct range shapes. NOT in this proposal — the constructor
form is sufficient and keeps the first landing small.

## Why this is the right shape

- **One convention, enforced by the type.** The `[start, end)` semantics live in `Range`, so every
  slice-family op inherits it — the uniformity the divergence lacked, guaranteed structurally rather
  than by remembering per-op prose.
- **A value composes.** A range is bound/passed/returned/matched/range-checked as ONE thing, so a call
  site cannot transpose two endpoints, and a helper can take/return a `Range` (e.g. a tokenizer
  returning the span it consumed).
- **It generalizes.** `Range` is the natural home for future open-ended forms and for any position-set
  API, without adding more positional conventions.
- **Low runtime cost.** A `range` is a two-`Int64`-field compound the tag-free heap, `shape_of`, and
  pattern matching already handle; no new heap node kind or runtime op — the slice ops still lower to
  the existing runtime `bytes-slice(buf, start, len)` / string slice with `len = end - start` computed
  at the call boundary.

## Migration plan (sequenced; seed-side first, then compiler.cdz)

1. **Spec (normative):** add `### A Range Is A Half-Open Interval Value` to `collections-and-text.md`
   (and a `Range` entry to `type-system.md`'s declarable universe); reword `### Indexing And Lookup
   Are Fallible` so the slice family names a `range`; retire the divergence learning.
2. **Corpus:** reframe every `String.slice`/`Bytes.slice` case to `(… (range start end))`. `Bytes`
   cases convert LENGTH→END (`(Bytes.slice b 1 2)` [1 byte at idx 1] → `(Bytes.slice b (range 1 2))`
   only if it meant bytes `[1,2)`; the existing `1 2` = "1 byte" becomes `(range 1 2)` = bytes `[1,2)`
   = 1 byte — SAME result, so length `n` at start `s` → `(range s (+ s n))`). `String` cases wrap the
   existing `(start, end)` ints in `(range …)` with NO value change.
3. **Seed:** parse/fold/emit the `range` value (a two-field compound); rework `String.slice`/
   `Bytes.slice` lowering + const-fold to take one `range` arg and compute `len = end - start` for the
   runtime `bytes-slice` op (runtime op UNCHANGED — still `(buf, start, len)`, a frozen ABI). Keep all
   four gates green (behavior/ignition/component-check/cargo).
4. **compiler.cdz (sibling, coordinated):** its `Bytes.slice`/`String.slice` call sites + fold logic
   move to the `range` surface. Because the corpus is the shared oracle for component-check, the seed
   + corpus land together and the sibling updates compiler.cdz to match in the same coordinated step
   (reported via SEED-GAPS), so the differential gate never sees a mixed state.

## Open questions for sign-off

- **Nominal vs structural `Range`.** Is `Range` a nominal type (distinct from a bare `{start, end}`
  record, comparable only within the boundary) or just the structural record? Nominal is cleaner (a
  `range` won't accidentally unify with an unrelated 2-field record) but adds a nominal tag; structural
  is simpler. Recommendation: **nominal** (`Range`), reusing the existing nominal-over-structural
  machinery, so `(= (range 1 3) (some-record …))` is a type error, not a structural coincidence.
- **`at` with a range?** Keep `at` single-index only (this proposal), or later allow `(Bytes.at b
  range)` to mean the slice? Recommendation: keep them distinct — `at` is one element, `slice` is a
  sub-sequence; overloading `at` would blur the fallible-single vs fallible-range distinction.
- **Scope of the first landing:** constructor form only (no `..` sugar, no open-ended ranges, no
  `List.slice`)? Recommendation: **yes** — land `(range …)` + the two existing slice ops, defer sugar
  and `List.slice` to follow-on amendments.
