## 12. 🟢 The built-in Option/Result loses its payload kind across a function boundary (the reader gate) — RESOLVED 2026-07-07

**Update (2026-07-07) — 🟢 RESOLVED, all facets.** The seed rebuilt and closed every facet at once: the
bare `(Some 42)` through a helper (the general payload-kind-recovery facet, the deepest, untouched by the
earlier per-accessor fixes) → 42, and `String.from-bytes` through a helper (the reader's symbol-table
decode; real `gen_runtime_string_from_bytes`, a total fallible UTF-8 decode that validates with the
existing runtime — the in-flight `bytes-is-utf8` op was not needed on this path) → 2 (ill-formed → None
arm → -1). Both corpus cases withheld/todo in earlier cycles flipped **todo → PASS**: `05` *"a built-in
Option is unwrapped by a helper that binds its payload"* and `13-strings` *"a helper decodes bytes to a
string and consumes the fallible result"*. Confirms this item's thesis: per-accessor patching closed
symptoms; the general kind-recovery fix closed the class. Learning:
`spec/learnings/2026-07-07-the-reader-gate-closed-and-list-at-on-a-payload-list-is-the-next.md`.

**Finding.** A built-in `Option` payload-binding `match` declines "runtime sum match arms differ in
kind" once the `Option` crosses a function boundary. Sharp boundary: `(match (Some 42) ((Some x) x)
((None _) 0))` at the entrypoint compiles, but the same match in a helper (`(unwrap (Some 42) 99)`)
declines — as does the reader's idiom `(match (Bytes.at b i) ((Some x) x) (None …))` on a runtime
`Bytes`. Yet `List.at`'s `Option` and every **user-declared** sum compile in the identical shape (the
Tier-2b fix), and `Option.expect` works. So the gap is the **built-in `Option`/`Result` constructors
carrying no per-slot payload type** (the `sum_payload_types` a user `type` populates); their payload
kind is recoverable only where local type context supplies it, and is lost across a boundary.

**Why it touches the spec/seed.** This is the **current gate on the reader**, hence on true
`bytes → bytes` self-hosting — the reader passes `Option`s between helpers on every byte. The spike's
SEED-GAPS Tier 2c framed it as `Bytes.at`-specific; the probe set shows it is broader (a literal
`(Some 42)` through a helper also declines), so the fix must **register the built-in polymorphic sums'
payload types the way a user sum's are**, not patch `Bytes.at`. Not a spec *gap* (the behavior is
already what the corpus records); it is seed inference work. Recorded here so the operator sees it as
the reader gate.

**Status.** 🟢 **DONE (2026-07-07, seed side).** Both corpus cases now PASS: *"a built-in Option is
unwrapped by a helper that binds its payload"* (→ 42) and the new *"a generic unwrap helper consumes a
fallible Bytes.at result"* (→ 20), both `05-compound-types.sexp`. The fix was NOT registering the
built-in sums' payload types (they are genuinely polymorphic — `Some a`'s payload has no fixed kind);
it was RECOVERING the concrete kind at the match site by **unifying the arm result kinds**. A new
fallback `infer_sum_payload_override` (in `gen_match_runtime_sum`) — used when the scrutinee's static
shape can't pin the payload (an opaque `Heap` param) — seeds a shared `InferCtx` with the arm binders +
enclosing locals, infers/unifies/back-propagates the arm results, and reads back each binder's solved
concrete scalar kind, so `bind_sum_payload` unboxes it. This is the parameter-boundary twin of Tier 2c's
scrutinee-shape override; together the built-in `Option`'s payload survives a match anywhere a user
sum's does. Gate 521/0, ignition byte-identical, component-check 527/0.
See [[sum-match-payload-kind-recovered-by-arm-unification]].
Learning: `spec/learnings/2026-07-07-the-built-in-option-loses-its-payload-kind-across-a-boundary.md`.

**Update (2026-07-07, later) — being closed ACCESSOR-BY-ACCESSOR; the class is still open.** The seed
fixed the `Bytes.at` facet: `(match (Bytes.at b i) ((Some x) x) (None …))` through a helper now compiles
(the reader's per-byte idiom; sibling pinned the passing cases in `10-bytes.sexp`). But the fix is
accessor-specific, confirming this item's thesis. Current map of the gate:
- `List.at` through a helper → ✅ works; `Bytes.at` through a helper → ✅ works (fixed this cycle).
- `String.from-bytes` through a helper → ❌ declines *"unsupported dotted-application"* (a DIFFERENT
  message — it needs its own runtime lowering, not just payload-kind unify). The reader's **symbol-table
  decode** idiom. Now pinned: `13-strings.sexp` *"a helper decodes bytes to a string and consumes the
  fallible result"* (→ 2, **todo**).
- a bare literal `(Some 42)` through a helper → ❌ still declines *"arms differ in kind"* (the general
  built-in-`Option` facet, untouched).

The reader uses all of these at once, so it compiles only when the LAST accessor lands. The general fix
(payload-type registration for the built-in sums) closes all facets uniformly; accessor-by-accessor is a
sequence of symptom fixes to the same end. Learning:
`spec/learnings/2026-07-07-the-reader-gate-is-being-closed-accessor-by-accessor.md`.

**Update (2026-07-07, later) — the `String.from-bytes` facet's runtime support is IN-FLIGHT.** The
`from-bytes`-through-a-helper facet needs a runtime `String.from-bytes` (its own lowering, not just the
payload-kind unify the other accessors needed). The spike's in-flight fix (WIT append + codegen,
mid-landing — binary not yet rebuilt at this snapshot): a runtime `String` IS the same Bytes-backed UTF-8
leaf, so `from-bytes` is a validity CHECK (new runtime op `bytes-is-utf8`, WIT idx 54, the Unicode
validator — rejects overlong/surrogate/>U+10FFFF) plus a zero-cost retag, not a decode/copy. Design +
correctness requirements recorded in
`spec/learnings/2026-07-07-string-from-bytes-validates-in-the-runtime-a-string-is-a-utf8-bytes-leaf.md`;
strict-UTF-8 requirement pinned by two new `13-strings.sexp` cases (overlong `C0 80` + surrogate `ED A0
80` → None). Not yet confirmed by probe (binary not rebuilt); verify when built. The bare-`(Some 42)`
facet (the general payload-kind class) remains the deepest fix.

---
