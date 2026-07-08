# String.slice is (start, end) but Bytes.slice is (start, length) — a spec-consistency footgun

*2026-07-08*

**What was observed (not a compiler break — each matches its spec).** The two sub-sequence slice
operations take DIFFERENT argument conventions:
- `String.slice` is **(start, end)**: `(String.slice "hello" 1 3)` → `"el"` (scalars at positions 1 and
  2, i.e. the half-open range `[1, 3)`). 13-strings.sexp pins this — §"an empty-range slice" says
  `(String.slice "hello" 2 2)` "has start = end, so it selects no scalar values", and `(String.slice
  "hello world" 0 5)` → `"hello"` (5 scalars, `[0, 5)`).
- `Bytes.slice` is **(start, length)**: `(Bytes.slice (Bytes.of (list 10 20 30 40)) 1 3)` → `[20 30 40]`
  (3 bytes from index 1). 10-bytes.sexp:168 pins this — "`(Bytes.slice b start length)` yields the
  `length` bytes of `b` beginning at `start`".

So the same third argument means END for a string and LENGTH for bytes. Both corpus families pass the
gate; the compiler faithfully implements each as its own spec file specifies. This is NOT a
reject-don't-miscompile violation — it is a spec-surface inconsistency between two sibling operations.

**Why it matters.** collections-and-text.md #Indexing And Lookup Are Fallible groups "indexing a list, a
string (by scalar or byte offset), or a `Bytes` value, or taking a sub-sequence slice" as one family
whose members are total/fallible in the same way — the spec presents slicing as a uniform operation
across sequence types. But the concrete argument convention diverges: a program that slices a string and
a byte sequence with "the same" call shape gets ranges computed two different ways. A self-hosted
compiler reading its own source (which manipulates both `Bytes` — the wasm it emits — and `String` — the
text it renders) is exactly the program most likely to call both and to confuse them: `(String.slice s 1
3)` takes scalars 1–2, but the visually identical `(Bytes.slice b 1 3)` takes 3 bytes from index 1. The
off-by-convention is silent — both return a `Some` of a plausible-looking sub-sequence — so a mix-up
miscompiles without a diagnostic.

**Recommendation (spec-side, not seed-side).** Make the two conventions uniform, or make the difference
impossible to confuse:
1. **Uniform (start, end)** for both — the more common convention (Python, Rust ranges), and the one
   `String.slice` already uses; `Bytes.slice`'s spec and corpus would change to (start, end).
2. **Uniform (start, length)** for both — `String.slice`'s spec/corpus change to (start, length).
3. If the divergence is deliberate, name them distinctly (e.g. `Bytes.slice-len` / `String.slice-range`)
   or document the divergence prominently at both definitions, so the reader is warned at the call site.
Either uniformity removes a silent-miscompile footgun for the self-hosted compiler. No corpus case is
added — both operations conform to their current specs; this is a spec-design decision for the author.

**Related:** the fallible-access family (collections-and-text.md #Indexing And Lookup Are Fallible), the
`Bytes.slice` cases in 10-bytes.sexp, the `String.slice` cases in 13-strings.sexp.
