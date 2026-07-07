# The built-in Option loses its payload kind across a function boundary — the last blocker before the reader

*2026-07-07*

**What happened.** With runtime strings landed and the front end closed to component bytes
([[2026-07-07-the-nested-payload-binder-fix-closes-the-front-end]]), the only remaining piece before
self-hosting is the *reader* — the `bytes → AST` decode that walks the input with
`(match (Bytes.at input i) ((Some b) …) ((None _) …))` on every byte. Probing that idiom surfaced a
precise decline: a **built-in `Option` payload-binding `match` declines with "runtime sum match arms
differ in kind" once the `Option` has crossed a function boundary.** The boundary is sharp, and wider
than it first looks:

- `(match (Some 42) ((Some x) x) ((None _) 0))` **at the entrypoint directly** → compiles (→ 42).
- The *same* match moved into a helper — `(def (unwrap o d) (match o ((Some x) x) ((None _) d)))`,
  called `(unwrap (Some 42) 99)` → **declines** ("arms differ in kind"), even though both arms are
  plainly `Int64`.
- `(match (Bytes.at b i) ((Some x) x) ((None _) -1))` on a **runtime** `Bytes` param → **declines**
  (the reader's exact idiom).
- **But** the same shape over `List.at` → compiles (→ 20), and over a **user-declared** sum
  `Box (Full Int64 | Empty)` passed to a helper → compiles (→ 42). `Option.expect` on a runtime
  `Bytes.at` also compiles (→ the byte).

So the trigger is not payload-binding in general (user sums bind payloads across boundaries fine —
that is the just-landed Tier-2b fix), nor `Bytes.at` specifically (the spike's SEED-GAPS Tier 2c
framed it that way): it is the **built-in `Option` constructor losing its payload's kind once it flows
through a parameter**. Locally at the entrypoint the payload kind is known; across a boundary the
built-in `Option` carries no per-slot payload type (the `sum_payload_types` map a user `type`
declaration populates), so the `Some x` binder's kind can't be recovered and won't unify with the
other arm.

**Why.** This refines the spike's own diagnosis in a way that matters for the fix. SEED-GAPS Tier 2c
diagnosed the symptom as `Bytes.at`-specific ("make a runtime `Bytes.at` result match like any other
`Option<Int64>`"), because that is where the reader hits it. But the probe set shows a plain
`(Some 42)` literal *also* declines the moment it crosses a helper boundary, while `List.at`'s result
and every user sum compile in the identical shape. The common factor is not the *producer* (`Bytes.at`
vs a literal) — it is the *type*: the built-in `Option`/`Result` constructors do not register a payload
type the way a user `(type … (Ctor T …))` declaration and the collection accessors (`List.at`) do, so
their payload kind is recoverable only where local type context supplies it (the entrypoint), and is
lost across a boundary. The fix is therefore not to patch `Bytes.at` but to give the **built-in
polymorphic sums the same payload-type registration a user sum gets** — so a `Some`-bound payload
unifies with a scalar arm wherever the `Option` came from and wherever it is matched. Patching only
`Bytes.at` would leave `(unwrap (Some 42) 99)` and every other `Option`-through-a-helper still
declining, and the reader is not the only code that passes `Option`s between functions — it is simply
the first. This is the same lesson the type-directed emission work already taught in another guise
([[2026-07-06-result-valtype-is-type-directed-through-an-exhaustive-kind-sum]]): a value's kind must be
recoverable from its *type*, not reconstructed from the expression that happened to produce it.

**The requirement it drove.** A conformance case in `05-compound-types.sexp` — *"a built-in Option is
unwrapped by a helper that binds its payload"* (`(unwrap (Some 42) 99) → 42`) — pins the fundamental
facet: the built-in `Option` supports a payload-binding match across a function boundary, the same way
a user-declared sum does. It records the true `(output …)` oracle (42) and is tagged `fallible-access`
(which the seed realizes), so it currently scores **todo** — the seed declines it cleanly today
("arms differ in kind"), and it flips green when the built-in `Option`'s payload kind is made to
survive a boundary. It is deliberately the *literal-`(Some 42)`* form rather than the `Bytes.at` form,
because that isolates the type-registration gap from any `Bytes.at`-specific quirk and guards that the
fix addresses the built-in sum, not one accessor. This is the current gate on the reader, hence on
true `bytes → bytes` self-hosting; the corresponding `Bytes.at`-through-a-helper case in `10-bytes.sexp`
(the reader's literal idiom) is the companion to add once the fix lands and both can be pinned green.
Tracked as SEED-GAPS Tier 2c (seed fix) and a new SPEC-BACKLOG item (the broader built-in-sum framing).
