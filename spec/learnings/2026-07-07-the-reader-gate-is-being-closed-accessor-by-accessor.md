# The reader gate is being closed accessor-by-accessor — `Bytes.at` crosses a boundary now, `String.from-bytes` is the next domino

*2026-07-07*

**What happened.** The "built-in fallible result loses its payload kind across a function boundary"
gate on the reader ([[2026-07-07-the-built-in-option-loses-its-payload-kind-across-a-boundary]]) is
being closed one accessor at a time, exactly as that learning warned. This cycle the seed fixed the
`Bytes.at` facet: `(def (bat b) (match (Bytes.at b i) ((Some x) x) ((None _) -1)))` — the reader's
per-byte idiom — now compiles and runs (→ 20 on a runtime byte sequence), where it previously declined
"runtime sum match arms differ in kind". But the fix was **accessor-specific**, and the probe set shows
the gate is only partly closed:

| fallible result | matched at `main` directly | matched through a helper boundary |
|---|---|---|
| `List.at` | works | **works** (already) |
| `Bytes.at` | works | **works** (fixed this cycle) |
| `String.from-bytes` | works (→ 2) | **declines** — *"unsupported dotted-application"* |
| a literal `(Some 42)` | works | **declines** — *"arms differ in kind"* |

So `String.from-bytes` — the reader's *symbol-table* decode idiom (`(def (dec b) (match (String.from-bytes
b) ((Some s) …) ((None _) …)))`) — is the next domino, and it declines with a *different* message
("unsupported dotted-application", not "arms differ in kind"), meaning it isn't a payload-kind-unify
problem at all: a runtime `String.from-bytes` in a called helper isn't lowered on that path. And the
literal `(Some 42)` through a helper still declines "arms differ in kind" — the general built-in-`Option`
facet from item 12 is untouched.

**Why.** This is the concrete vindication of item 12's core claim: patching per-accessor closes the
symptom where each accessor is used but leaves the class open, because the underlying issue is that the
**built-in polymorphic sums (`Option`/`Result`) carry no per-slot payload type** — so each producer that
returns one has to be taught, individually, to make its result match across a boundary. `Bytes.at` and
`List.at` are now taught; `String.from-bytes` is not, and the bare `Some` constructor is not. A reader is
written from *all* of these at once — it indexes bytes (`Bytes.at`), slices and decodes them into symbol
strings (`String.from-bytes`), and threads the resulting `Option`s between helper functions — so closing
the gate accessor-by-accessor means the reader compiles only when the *last* accessor it uses is fixed.
The durable lesson stands: the general fix is to give the built-in `Option`/`Result` the same
payload-type registration a user `type` gets (so any producer's result matches across a boundary
uniformly), and the accessor-by-accessor progress, while real, is a sequence of symptom fixes whose
end-state is the same general fix. The differing decline message on `String.from-bytes` ("unsupported
dotted-application") is a useful signal that it needs its own runtime-lowering step *in addition* to the
payload-kind work — it is not merely the same unify gap wearing a different hat.

**The requirement it drove.** A conformance case in `13-strings.sexp` — *"a helper decodes bytes to a
string and consumes the fallible result"* (`(def (dec b) (match (String.from-bytes b) ((Some s)
(String.byte-len s)) ((None _) -1)))`, `dec (Bytes.of (list 104 105)) → 2`) — pins the reader's
symbol-table decode idiom: `String.from-bytes` consumed through a function boundary. It records the true
oracle (2) and is tagged `fallible-access` (which the seed realizes), so it scores **todo** — declining
cleanly today ("unsupported dotted-application") and flipping green when a runtime `String.from-bytes`
result matches across a boundary. It is the companion of the existing round-trip case (which decodes at
`main` directly): the boundary crossing is what a self-hosted reader actually does. This joins the
already-pinned literal-`Some`-through-a-helper case (item 12) and the now-passing `Bytes.at`-through-a-helper
cases (a sibling pinned those with the fix) to map the whole gate: two facets green, two still todo. The
reader cannot be authored until both remaining facets close, so SPEC-BACKLOG item 12 is updated to track
the class — `String.from-bytes` and the bare `Option` constructor — not just `Bytes.at`.
