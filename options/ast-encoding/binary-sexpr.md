# AST Encoding — Choice: binary-sexpr

> **The default choice for the `ast-encoding` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the canonical stored form as a binary
> s-expression with a self-contained symbol prelude.

## The choice

The canonical stored form is a **binary s-expression**: a minimal, general tree of nodes, deliberately
simple so that the container stays stable across compiler versions while the *meaning* of the symbols
a node references evolves with the language. The file is **self-contained** — it carries its own
prelude of the symbols its nodes use, so a reader needs no external registry.

A stored file is a triple:

```
[ container-version, prelude, root-node ]
```

- **`container-version`** — the version of this container encoding (rarely changes; new constructs are
  new symbols, not container changes).
- **`prelude`** — the ordered list of symbols the tree references (below).
- **`root-node`** — the program's top node.

A node is one of:

- an **atom** — a leaf carrying one primitive: an integer, a float, a string, a character, or a
  boolean, whose bytes follow the canonical value form in
  [`../hashing-and-encoding/`](../hashing-and-encoding/);
- an **application** — `[symbol-index, child…]`, a reference to a prelude symbol by its index followed
  by an ordered sequence of child nodes.

That is the whole container. A construct of the language — a function, a match, a module, a type
declaration, a doc node — is an application whose head symbol names the construct and whose children
are its parts. Because a node names its kind by a prelude index and not by an inline tag, adding a new
construct is adding a symbol to a file's prelude; it changes neither the container nor how any other
tree encodes.

## The symbol prelude

- A **symbol** is `[namespace, name, version?]`: a namespace, a name within it, and an optional
  version. Language-defined symbols live in the `cadenza/core` namespace; a macro introduces symbols
  in its own namespace, so a macro symbol can never collide with a core one. A version lets a
  construct's meaning evolve while a file that references the earlier version keeps denoting it.
- The prelude lists every distinct symbol the tree references, and each application node references
  one by its **index** into this list.
- **Canonical order:** the prelude is sorted by `(namespace, name, version)` under a fixed byte-wise
  ordering, so two trees that reference the same set of symbols produce an identical prelude and thus
  identical indices — the property equal-trees-encode-identically depends on. The order is a function
  of the referenced set, never of construction or discovery order.

## Concrete encoding

- The triple is serialized as **deterministic CBOR** (RFC 8949 §4.2), the same canonical encoding the
  value form uses, so there is one byte sequence per tree and equal trees encode identically.
- An application node is a CBOR array `[symbol-index, ...children]`; an atom is a CBOR primitive
  tagged with its value-form kind; a symbol is a CBOR array `[namespace, name]` or
  `[namespace, name, version]`.
- The file begins with the `container-version` (ast-encoding.md §"The Encoding Is Versioned"), so a
  reader refuses a container version it does not implement, and refuses a file that references a
  symbol or symbol version it does not understand.

## Worked example

A documented `classify` function, shown first in a textual s-expression display and then as the
stored structure. The two are the same tree; the file stores the structure, and the display is one
projection of it.

Textual s-expression display:

```
(module math
  (doc "Small integer helpers.")
  (def (classify n)
    (doc "Sign of n as a tag.")
    (match n
      ((< n 0) Sign.Neg)
      (else    Sign.Pos))))
```

Stored structure (indices refer to the prelude below):

```
container-version: 1
prelude:
  0 = [cadenza/core, module]
  1 = [cadenza/core, doc]
  2 = [cadenza/core, def]
  3 = [cadenza/core, match]
  4 = [cadenza/core, <]
  5 = [cadenza/core, else]
  6 = [math, classify]
  7 = [math, n]
  8 = [math, Sign.Neg]
  9 = [math, Sign.Pos]
root:
  (0 "math"
     (1 "Small integer helpers.")
     (2 (6 7)
        (1 "Sign of n as a tag.")
        (3 7
           ((4 7 0) 8)
           (5 9))))
```

Comments and documentation are ordinary nodes here (symbols `cadenza/core:doc` and
`cadenza/core:comment`), so they encode, hash, and round-trip like any other node, and a printer
renders them into whatever display it targets — which is how "store it however, format it however"
keeps comments in the canonical form rather than in a text rendering.

## Why binary-sexpr with a prelude

- **Self-contained:** a file needs no external tag registry to read; the prelude is the registry, and
  it travels with the file. There is no global table two generations must agree on out of band.
- **Stable across versions:** the container is fixed; new constructs are new prelude symbols, so a
  tree stored by one compiler generation is readable by another — what content-addressed,
  cross-generation programs require.
- **General:** any construct, and any future construct, is just a symbol applied to children — the
  same reason s-expressions have outlived most syntaxes — with namespaces keeping core and macro
  symbols apart.
- **Reproducible:** deterministic CBOR plus a canonically-ordered prelude gives one byte form per
  tree, so the source hash over the binary AST is exact and third-party-checkable, with no
  text-normalization fragility.
- **Cheap to bootstrap:** a reader is a small deterministic-CBOR decoder plus prelude resolution — the
  seed toolchain needs little more than it already needs for the value form, and no node-kind table.
