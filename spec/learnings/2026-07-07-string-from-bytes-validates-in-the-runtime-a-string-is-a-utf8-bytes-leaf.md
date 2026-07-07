# `String.from-bytes` validates in the runtime — a String is a UTF-8 Bytes leaf, so decode is a check, not a copy

*2026-07-07*

**What happened.** Subset-growth toward self-hosting reached the reader's symbol-table decode — turning
a prelude symbol's byte slice into a `String` for name comparison — which needs runtime
`String.from-bytes` (backlog item 12, the last reader-relevant gap). The spike's in-flight fix (a WIT
append + codegen change, mid-landing at this snapshot — the runtime `.wasm` and seed binary are not yet
rebuilt) resolves *how* that decode works, and the design is worth recording even before it is built:

- **A runtime `String` IS the same Bytes-backed UTF-8 leaf.** There is no separate String
  representation — a String is a Bytes buffer the type system knows is well-formed UTF-8. So
  `String.from-bytes` needs **no decode or copy**: the resulting String value is the *same* buffer; only
  its *validity* must be checked.
- **The validity check is a runtime primitive, not compiler-emitted.** The WIT gains
  `bytes-is-utf8: func(buf) -> bool` (index 54); the compiler emits a call to it to decide the
  `Some`/`None` of `String.from-bytes`, rather than hand-emitting a byte-level UTF-8 state machine. The
  runtime validator is the Unicode definition (matching Rust's `str::from_utf8`): it rejects overlong
  encodings, surrogates, and code points > U+10FFFF, not just structurally-broken bytes.

**Why.** Two durable design points. First, **making a String the same Bytes leaf collapses a decode to a
predicate** — the "conversion" `Bytes → Option<String>` is not a transformation but a *classification*
(is this buffer valid UTF-8?) plus a zero-cost retag. This is the same shape as several earlier runtime
decisions (the tag-free heap where rendering is type-directed, the rope where a slice shares storage):
the representation is chosen so an operation that looks like it copies is actually a check over shared
bytes. It is why `String.from-bytes` can be cheap and why `String.to-bytes` is its exact inverse on
well-formed input — they are the same bytes viewed through two types. Second, **validation belongs in
the runtime, not the compiler.** A hand-emitted UTF-8 validator is exactly the kind of subtle,
security-relevant state machine that is easy to get *structurally* right and *semantically* wrong — a
validator that checks only lead/continuation byte shape accepts overlong encodings (`C0 80` for NUL) and
surrogate encodings (`ED A0 80` for U+D800), both of which strict UTF-8 forbids and both of which have
been used to smuggle bytes past naive validators. Delegating to the runtime's correct validator (the
same one the host language already ships) removes that risk from the compiler entirely and keeps the
emitted code a single `bytes-is-utf8` call. The general principle: **when a runtime value's
well-formedness is a hard, standardized predicate, emit a call to a runtime check rather than open-code
the predicate in the compiler** — correctness lives in one audited place, and the emitted program stays
small.

**The requirement it drove.** Two conformance cases in `13-strings.sexp` pin the strict-UTF-8
requirement the runtime validator must meet, beyond the existing easy `0xFF` rejection: *"decoding an
overlong UTF-8 encoding yields none"* (`C0 80`, the overlong NUL — a decoder checking only byte shape
would wrongly accept it) and *"decoding a surrogate code point encoded as UTF-8 yields none"*
(`ED A0 80`, U+D800 — structurally valid three-byte UTF-8 but a non-scalar). Both record `= None → true`
and are tagged `(needs binary-matching)` (matching the existing `from-bytes` cases' capability gate), so
they **skip** until that capability is realized, then pin that the validator enforces shortest-form and
excludes surrogates — the Unicode-scalar boundary the `Char.from-int` surface already enforces, now on
the byte-decode path the reader uses. They record the *requirement* on the validator independent of the
in-flight `bytes-is-utf8` op being built. **Backlog item 12 is updated** to note its resolution is
in-flight with this approach (runtime-validated `String.from-bytes`, String-as-UTF-8-Bytes-leaf) rather
than open, and item 13 (list patterns) remains the other subset-growth item. No claim that the fix is
landed — the binary is not yet rebuilt at this snapshot; the design and its correctness requirements are
what this learning captures, to be confirmed by probe once built (the standing rule: probe the running
seed, don't trust the in-flight edit).
