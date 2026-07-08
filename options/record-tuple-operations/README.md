# Decision — Record And Tuple Reshaping Operations

**The decision.** The concrete operation surface an author writes to reshape a record or a tuple —
projecting a record onto a set of fields, dropping fields, combining two records, adding, updating,
and popping a field; and the positional analogues on tuples: concatenating and splitting. The
type-system.md capability fixes the *semantics* these operations must have (each yields a new closed
value, reshapes only through an explicit operation, is statically shaped, and rejects an absent or
already-present field). This decision pins the concrete *forms* an author writes and how the derived
convenience operations reduce to a small set of primitives.

**Why the language wants it.** The record surface already commits to row polymorphism — a record type
is a row that MAY carry a row variable, and inference types a function open over the fields it does not
use (type-system.md §"Records Are Rows, Open By Default Under Inference";
`spec/learnings/2026-07-04-records-are-rows-open-by-default.md`). That learning names an explicit
`project`/narrowing operation as "the only thing that changes the shape" so that subset comparison is
projection-then-`=` and never an overloaded `=` — but no such operation was ever pinned as a form or
witnessed in the corpus (`15-rows-and-open-sums.sexp` fakes subset comparison with plain `.` field
access). This decision completes that story: it gives the row-reshaping operations the learning
promised a concrete surface, and extends the same discipline to tuples positionally. A self-hosting
compiler wants these constantly — narrow an environment record to the fields a pass reads, stamp a
provenance field onto an AST node, pop a scrutinee off a context, partition a fixed argument tuple.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A shape change is only ever an explicit operation the program wrote, never an implicit widening or
  narrowing inference introduces (type-system.md §"A Record Row Is Reshaped Only Through An Explicit
  Operation Yielding A New Value"; §"A Tuple Is Reshaped Positionally By An Explicit Operation Yielding
  A New Value").
- Every operation yields a **new** value and never mutates its operands, consistent with the immutable
  acyclic value heap (same sections; memory-and-resource-model.md;
  `spec/learnings/2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete.md`).
- Every operation's result shape (a record's field set, a tuple's arity) is determined **statically**
  from the operands' shapes, so the emitted component carries a concrete closed shape and no
  runtime-determined field set or tuple length (same sections; consistent with monomorphization and the
  component ABI, type-system.md §"Records Are Rows" 3rd sentence).
- Combining two records, or adding a field, is rejected when a field name would be shared — the row
  analogue of the duplicate-field-literal rejection — carrying `CDZ0211` (type-system.md §"Two Records
  Are Combined Only When Their Field Sets Are Disjoint"; §"A Field Is Added To Or Replaced In A Record
  By A Derived Operation"; the duplicate-field literal case of `CDZ0201`, core-semantics.md §"A Record
  Has A Fixed Set Of Named Fields").
- Projecting onto, dropping, or updating a field the operand record does not contain is rejected,
  carrying `CDZ0212` (type-system.md §"A Record Is Restricted To A Named Set Of Its Fields"; §"A Record
  Is Reduced By Dropping A Named Set Of Its Fields"; §"A Field Is Added To Or Replaced In A Record By A
  Derived Operation").
- A tuple split at a position outside the operand's static arity range is rejected as a type error,
  consistent with an out-of-arity positional access being rejected (type-system.md §"A Tuple Is Split
  At A Position Into A Prefix And A Suffix"; the `tuple.N` static bounds rule, core-semantics.md
  §Tuples).
- The result of each operation is an ordinary structural value with the record/tuple boundary
  representation the type mapping already carries (options/type-mapping/component-model-types.md); the
  operations add no new boundary or canonical form — they are additive over the existing record and
  tuple value model.

## Choices

- [`namespaced-row-operations`](./namespaced-row-operations.md) — the operations are reached as member
  access into the `Record` and `Tuple` prelude records (`Record.project`, `Record.without`,
  `Record.merge`, `Record.extend`, `Record.with`, `Record.pop`; `Tuple.cat`, `Tuple.split-at`,
  `Tuple.pop`), with three record primitives (`project`, `without`, `merge`) and two tuple primitives
  (`cat`, `split-at`) from which the rest reduce by a meaning-preserving rewrite. Field names and
  positions are static operands (like the `.` accessor's key and `tuple.N`'s index), so the operations
  are special forms under a prelude-record prefix, not ordinary functions over runtime values. **The
  default.**

DEFAULT: namespaced-row-operations
