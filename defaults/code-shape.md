# Code Shape — Declared Default

> **What this file is.** The concrete resolution of the one design question the specification
> deliberately leaves open at the surface level: what the canonical representation of a Cadenza
> program *is*, and how the forms a human reads relate to it. The constitution requires a canonical
> textual form that round-trips byte-for-byte and a structural interface for manipulating programs
> (constitution X), but it does not fix a representation or a surface syntax family, because those
> are replaceable choices this location pins.
>
> This is a **declared default**, not a requirement. Accept it, tune it, or delete `defaults/` to
> reinvestigate from first principles. The whole specification is written against "the canonical
> representation" and "the canonical textual form," so changing the representation or a display is an
> edit to this file, a projection, and a formatter — it touches no frozen contract and no capability
> requirement.

## The requirements this choice must satisfy (from the spec — do not weaken)

- **A canonical textual form that round-trips** (constitution X: formatting yields byte-identical
  canonical bytes; parse-then-format reproduces them).
- **A structural interface** for reading and rewriting program structure without re-parsing
  unrelated code (constitution X; agent-authoring.md).
- **Written and read by agents; read by humans** — the top two north-star priorities.
- **Reproducible codegen** — the representation and its displays must not make canonical round-trip
  or structural edits fragile (reproducible-derivation.md).

## The insight: homoiconicity decouples display from representation

If the canonical representation is **homoiconic** — the program *is* a uniform data structure, code
as data — then display and representation come apart cleanly. The representation is the one durable,
hashable, manipulable thing; a *display* is any deterministic rendering of it. There is no single
"the syntax" that both humans and agents and the hash must agree on; there is one representation and
as many displays as are useful, each a projection. This is a better answer than picking one surface
and taxing whichever north-star priority it serves worst, because it serves all of them at once
through different projections of the same core.

## The default: a homoiconic canonical representation with decoupled displays

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

## Why this default, against the north star

- **Written by agents (priority #1):** because code is data, an agent produces and transforms a
  program by manipulating the homoiconic representation directly through the structural interface —
  the strongest form of "easy to write," independent of any surface's whitespace or delimiters.
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

## What is frozen vs. chosen

- **Frozen (requirements, in the spec):** that a canonical form exists and round-trips, and that a
  structural interface exists. These hold regardless of representation or display.
- **Chosen (here, replaceable):** that the canonical representation is homoiconic, that display is a
  decoupled projection, the set of displays offered, and which display is the canonical textual form.
  Adding, removing, or changing a display is a change to this file and its projection; it touches no
  contract and no capability requirement, precisely because display is decoupled from representation.
