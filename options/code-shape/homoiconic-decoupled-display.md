# Code Shape — Choice: homoiconic-decoupled-display

> **A choice for the `code-shape` decision** (see [README.md](./README.md) for the decision and the
> requirements a choice must satisfy). This is the **default** choice. It is a declared choice, not a
> requirement; the whole specification is written against "the canonical representation" and "the
> canonical textual form," so adopting a different choice touches no frozen contract and no capability
> requirement.

## The insight: homoiconicity decouples display from representation

If the canonical representation is **homoiconic** — the program *is* a uniform data structure, code
as data — then display and representation come apart cleanly. The representation is the one durable,
hashable, manipulable thing; a *display* is any deterministic rendering of it. There is no single
"the syntax" that both humans and agents and the hash must agree on; there is one representation and
as many displays as are useful, each a projection. This is a better answer than picking one surface
and taxing whichever north-star priority it serves worst, because it serves all of them at once
through different projections of the same core.

## The choice: a homoiconic canonical representation with decoupled displays

The **canonical representation is a homoiconic, typed term** — a uniform code-as-data structure that
is content-addressable and is the sole target of structural manipulation, hashing, the executable
semantics, and verification. **Display is decoupled from it:** a program is shown through a
projection of the representation, and more than one projection may exist —

- a **conventional display** in the ML/Rust family (expression-oriented, keyword- and
  brace-delimited, indentation-insensitive) for humans to read and write comfortably;
- the **homoiconic display** itself, the direct code-as-data rendering, for metaprogramming and for
  agents that manipulate structure literally.

Exactly one display is designated the **canonical textual form** for the round-trip the constitution
requires; the others are alternative renderings of the same representation. Every display admits a
lossless projection to and from the canonical representation, so moving between displays never
changes the program.

## The two displays, shown

The same program — a documented function, a match, and a module with a capability declaration — in
both displays. Both project losslessly to the one representation.

**Homoiconic display** (the direct code-as-data rendering; also the corpus form):

```
(module math
  (doc "Small integer helpers.")
  (use (capability emit-event))
  (def (classify n)
    (doc "Sign of n as a tag.")
    (: (-> Int64 Sign))
    (match n
      ((< n 0) Sign.Neg)
      ((= n 0) Sign.Zero)
      (else    Sign.Pos))))
```

**Conventional display** (a projection of the very same representation):

```
module math

/// Small integer helpers.
use capability emit-event

/// Sign of n as a tag.
fn classify(n: Int64) -> Sign =
  match n {
    n < 0 => Sign.Neg
    n = 0 => Sign.Zero
    else  => Sign.Pos
  }
```

Documentation is a node in the representation (the `doc` form / `///` projection), not lexical
trivia, so it survives the round-trip in either direction (agent-authoring.md §Documentation).

## Why this choice, against the north star

- **Written by agents (priority #1):** because code is data, an agent produces and transforms a
  program by manipulating the homoiconic representation directly through the structural interface —
  the strongest form of "easy to write," independent of any display's whitespace or delimiters.
- **Read by agents and humans (priorities #1 and #2):** display is decoupled, so humans read the
  conventional display while agents may read either; neither priority is sacrificed to the other,
  because they are different projections of one representation rather than one contested syntax.
- **Verify properties (priority #3):** verification, the type system, and the executable-semantics
  corpus all operate on the uniform homoiconic representation — the property a homoiconic core is
  prized for — while humans still get a conventional display.
- **Reproducible codegen:** the hash and the round-trip are defined against the canonical
  representation and its one canonical textual form; because the representation is uniform and the
  canonical display is indentation-insensitive and delimiter-explicit, the byte-identical round-trip
  the constitution requires is straightforward, and alternative displays cannot affect a program's
  identity.

## What this choice fixes vs. leaves to the spec

- **Fixed by the spec (requirements):** that a canonical form exists and round-trips, and that a
  structural interface exists. These hold regardless of representation or display.
- **Fixed by this choice (replaceable):** that the canonical representation is homoiconic, that
  display is a decoupled projection, the set of displays offered, and which display is the canonical
  textual form. Adding, removing, or changing a display is a change to this choice and its
  projection; it touches no contract and no capability requirement, precisely because display is
  decoupled from representation.
