# Record and tuple reshaping is explicit row operations, completing the `project` the rows learning promised

*2026-07-05*

**What happened.** The record surface gained the shape-changing operations an author reaches for —
"pop a field off and get the remaining record", "add a field and get a new record", update a field,
merge two records, split one — and the positional tuple analogues (concatenate, split at a position,
pop). These were requested directly, but they were also **already owed**: the rows learning
([[2026-07-04-records-are-rows-open-by-default]]) named an explicit `project`/narrowing operation as
"the only thing that changes the shape" so that subset comparison stays projection-then-`=` and never
an overloaded `=` — yet no such operation was ever pinned as a form or witnessed in the corpus
(`15-rows-and-open-sums.sexp` faked subset comparison with a plain `.` field access). This work is the
delivery of that promise, not a new direction.

The design settled on **three record primitives** — `Record.project` (restrict to named fields),
`Record.without` (drop named fields), `Record.merge` (disjoint union) — the minimal complete row
algebra, from which the ergonomic operations reduce by a meaning-preserving rewrite: `Record.extend`
= merge-with-a-singleton (add an **absent** field), `Record.with` = without-then-merge (update a
**present** field, possibly retyping it), `Record.pop` = `(tuple (. r z) (Record.without r (z)))`.
Tuples mirror this positionally with `Tuple.concat` / `Tuple.split-at` / `Tuple.remove`. The operations are
reached through the `Record`/`Tuple` prelude records (like `Set.insert`/`List.at`) but remain
**special forms**, because a field name and a position are static operands the compiler resolves, not
runtime values a function receives.

Three design decisions carried the weight:
- **`merge` is strict and unbiased.** Two records that share a field name cannot be merged — there is
  no non-arbitrary value the shared field could take, exactly as `(record (a 1) (a 2))` cannot name
  `a` twice. So an overlap is a rejection (`CDZ0211`), never a silent left- or right-biased pick. This
  keeps the fixed-field-set invariant and "no implicit anything" intact.
- **`extend` and `with` are distinct, not one add-or-replace form.** `extend` requires the field
  **absent** (an accidental clobber is `CDZ0211`); `with` requires it **present** (an accidental
  introduction is `CDZ0212`). A single JS-spread-style add-or-replace was rejected because it silently
  overwrites — the very thing the strict `merge` exists to forbid — and because splitting the intent
  makes "grow the shape" vs "change a value" legible at the call and statically checked.
- **`pop` is row-typed, not `Option`-returning.** Whether a field is present is a static property of
  the record's row, so a missing field is a compile-time `CDZ0212`, not a runtime `None`. This is the
  deliberate counterpoint to `List.at`'s fallible `Option`: a list index is runtime data, a record
  field name is a static label ([[string-op-on-match-selected-string-declines]] is unrelated; the
  contrast here is fallible-access vs static-label). Values stay immutable — every operation yields a
  **new** closed record with structural sharing on the acyclic heap, never a mutation
  ([[2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete]]).

Because these are row operations, a definition over them is row-polymorphic and inference assigns a
principal type — `(def (stamp r) (Record.extend r #v 0))` is `∀ρ. (ρ ∌ v) ⇒ {ρ} → {v: Int64 | ρ}`,
monomorphized to a concrete closed shape per call site — so `CDZ0211`/`CDZ0212` are the ground cases
of the lacks/contains row constraints, dual to a failed generic constraint being a compile-time
rejection.

**Why.** Two forces. First, the promised-but-undelivered `project`: the rows learning committed the
language to an explicit narrowing operation and to closed-value records, but left the operation
unnamed and unwitnessed, so "records are rows" was a typing story with no value-level surface an author
could write. Second, a self-hosting compiler needs these constantly — narrow an environment record to
the fields a pass reads, stamp provenance onto an AST node, pop a scrutinee off a context, partition a
fixed argument tuple — and expressing each as a hand-rolled composition at every site is exactly the
"a helper over an object that has an `id`" friction the rows learning was written to remove. Pinning a
small primitive set plus derived ergonomics keeps the semantics minimal (a behavior-gate failure names
the primitive that broke) while giving the author the verbs they actually reach for.

**The requirement it drove.** `spec/capabilities/type-system.md` §"The Declarable Type Universe" gained
seven subsections after the rows subsection: §"A Record Row Is Reshaped Only Through An Explicit
Operation Yielding A New Value" (the umbrella: explicit, new-value, statically-shaped), §"A Record Is
Restricted To A Named Set Of Its Fields", §"A Record Is Reduced By Dropping A Named Set Of Its Fields",
§"Two Records Are Combined Only When Their Field Sets Are Disjoint", §"A Field Is Added To Or Replaced
In A Record By A Derived Operation", and the tuple pair §"A Tuple Is Reshaped Positionally By An
Explicit Operation Yielding A New Value" with §"Two Tuples Are Concatenated Into One Of Their Combined
Length" and §"A Tuple Is Split At A Position Into A Prefix And A Suffix". Two diagnostic codes were
added to the `CDZ02xx` band in `options/diagnostics-schema/coded-span-record.md`: `CDZ0211` (a shared
or already-present field) and `CDZ0212` (an absent field). The concrete surface is pinned in the new
decision `options/record-tuple-operations/` (default choice `namespaced-row-operations`), and the
behavior is witnessed by `(needs rows)` cases in `spec/semantics/15-rows-and-open-sums.sexp` (the same
tag the open-record cases carry, so the seed skips them until a generation realizes row inference —
`Record.*`/`Tuple.*` being unbound names to the seed, the [[symbol-interning-direction]]-style
`(needs …)` discipline the `Set` cases use with `(needs sets)`). Glossary terms **Row** and **Row
operation** were added.
