# DESIGN — binary-matching (`(bin …)` pattern) re-emit in the cadenza backend

Owner: v-cadenza-backend. Status: DESIGN (banked for incremental implementation, following the M4a /
list-pattern process — see `DESIGN-matchsum-nested-pattern-whole-slot.md`, `DESIGN-nested-list-pattern-reemit.md`).
Scope: the `--target cadenza` re-emit of a `match` over a runtime `Bytes` scrutinee whose arms are `(bin …)`
patterns (binary matching, chapter `spec/semantics/16-binary-matching.sexp`).

## 1. What already works vs the gap

- **CONSTRUCTION already re-emits** (mod.rs): `Core::BinBuild { segs }` → `(bin (uNN v) …)` (mod.rs ~2694),
  `Core::BinBitsBuild { fields }` → `(bin (bits v k) …)` (~2714). Runtime bin *building* round-trips today.
- **MATCH does NOT re-emit** — the frontier. A runtime `(match b ((bin (u16 n)) n) (_ 0))` declines:
  `CDZ0900 "does not support lowering this Core node back to Cadenza: BinIntRead"` (the generic fallback; the
  cadenza emit has no arm for the binary PATTERN-READ nodes). Confirmed: wasm computes 258, cadenza declines.
- 16-binary-matching has ~20+ cadenza-only VALUE gaps (all match-side): fixed-width int fields, bit-field runs,
  mixed-endian, signed, dependent-size `(bytes body n)`, final `(bytes rest)`, nested bin match, tag-dispatch,
  guarded bin arms. (Plus many error-assertion cases — a bin over a non-Bytes / kind-mismatch / ill-formed
  width — which correctly decline on cadenza as SHARED, since the program does not compile to wasm; NOT gaps.)

## 2. The desugared shape to reverse-engineer (`lower_match_bin`, match_tree.rs:188)

A runtime bin-match is NOT a single Core node — `lower_match_bin` lowers it to an **if-chain over per-arm
predicates**, built LAST-arm-backward: `acc` starts at the catch-all body; each `(bin …)` arm wraps
`(if <predicate> <arm-body> <acc>)`, where
- `<predicate>` = `bytes-len == total_width` ANDed with each LITERAL segment's read `== its literal`
  (for arms whose `(bin …)` is all fixed-width int segments — a bits/bytes/dependent segment is a later slice);
- `<arm-body>` reads each binder segment via a PATTERN-READ node off the scrutinee `Bytes`:
  - `Core::BinIntRead { bytes, byte_offset, off_plus, width, signed, little_endian }` — a fixed-width int
    field at a static (or `off_plus`-shifted) byte offset, sign/zero-extended to Int64;
  - `Core::BinRestRead { bytes, byte_offset, off_plus }` — a final `(bytes rest)` (tail from the offset);
  - `Core::BinSizedRead { …, len }` — a dependent-size `(bytes payload n)`, `n` a runtime earlier-segment read.
- A `(guard (bin …) cond)` arm carries an optional guard cond (reads the segment binders) — same fall-through
  as a scalar guarded arm.
- tail = the catch-all (`_` / bare binder) body.

So the re-emit must recognize this if-chain and reconstruct
`(match <b> ((bin <segs>) <body>) … (<catchall-binder/_> <body>))`, mapping each PATTERN-READ back to a segment
`(uW n)` / `(sW n)` (+ `le`), each literal predicate back to a literal segment `(uW <lit>)`, the length
predicate to the arm's implicit total width, and each read's value to the segment binder the body reads.

## 3. Fix points (all in `backend/cadenza/mod.rs`)

- The generic node fallback that currently declines `BinIntRead`/`BinRestRead`/`BinSizedRead` — these need
  dedicated re-emit as segment READS, but ONLY inside a recognized bin-match reconstruction (a bare
  `BinIntRead` outside a bin-match arm has no surface form — it IS the desugared read; the pattern is what
  re-lowers to it).
- A new recognizer at the `Core::If` re-emit (or a pre-pass): detect an if-chain whose predicate is a
  `bytes-len` probe (+ literal-field compares) over a Bytes scrutinee and whose body reads `BinIntRead`s off
  that scrutinee → reconstruct a `(match b ((bin …) body) … (_ acc))` instead of an `(if …)` chain.
- Reuse the CONSTRUCTION segment-emit (`BinBuild` at ~2694) for the surface segment spelling
  (`(uW v)`/`(sW v)`/`le`) so the pattern segments match the construction grammar exactly (idempotent-ish;
  value-only gate so value-equiv suffices).

## 4. Incremental sub-slices (simplest first)

1. **SINGLE fixed-width int segment + catch-all**: `(match b ((bin (u16 n)) body) (_ else))` — the minimal
   shape. Recognize `(if (= (bytes-len b) W) body[n=BinIntRead(b,0,W)] else)`; emit `(match b ((bin (uW' n))
   body') (_ else'))` where `n` binds at the `BinIntRead`'s key. Establishes the recognizer + read→segment
   mapping. First landable increment.
2. **MULTIPLE fixed-width int segments** (offsets 0,W0,W0+W1,…) — reconstruct segments in offset order.
3. **LITERAL segments** — a predicate `read == lit` → a literal segment `(uW lit)` in the pattern.
4. **`le` + `signed`** modifiers (from BinIntRead's `little_endian`/`signed`).
5. **final `(bytes rest)`** (`BinRestRead`), then **dependent-size `(bytes body n)`** (`BinSizedRead`, `off_plus`).
6. **bit-field runs** (the `(bits …)` match side), **guarded bin arms**, **nested bin match**, **tag-dispatch**.

Each sub-slice lands with the 16-binary-matching case(s) it flips + an A/B vs clean main (ADDITIVE only; the
value-only cadenza gate catches a wrong reconstruction as a value mismatch, not a silent miscompile).

## 5. Risk / why a design

The reconstruction is a REVERSE-engineering of an if-chain into a pattern — the failure mode of a careless
recognizer is either a mis-recognized non-bin if-chain (a corpus-cadenza RED / wrong value) or an
un-re-lowerable `(bin …)`. Gate the recognizer TIGHTLY (only an if whose predicate is exactly a `bytes-len`
probe over a Bytes scrutinee with `BinIntRead`s off it); anything not confidently a bin-match keeps the
existing `If` emit. Value-only gate (no byte-idempotence) means a value-equivalent reconstruction suffices,
and a wrong guess shows as a local A/B value mismatch — so implement per-sub-slice with a whole-16-chapter A/B
before each land. Repro: `/tmp` — `(match (bin (u16 (UInt16.wrap n))) ((bin (u16 x)) x) (_ 0))` folds (const);
a runtime witness is `(let ((b (bin (u16 (UInt16.wrap n))))) (match b ((bin (u16 x)) x) (_ 0)))` → 258 at n=258
on wasm.
