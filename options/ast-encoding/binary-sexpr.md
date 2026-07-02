# AST Encoding — Choice: binary-sexpr

> **The default choice for the `ast-encoding` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the canonical stored form as a binary
> s-expression.

## The choice

The canonical stored form is a **binary s-expression**: a minimal, general tree of tagged nodes,
deliberately simple so that it stays stable across compiler versions while the *meaning* of node tags
evolves with the language.

A node is one of:

- an **atom** — a tagged leaf carrying one primitive: an integer, a float, a string, a symbol, or a
  boolean, whose bytes follow the canonical value form in
  [`../hashing-and-encoding/`](../hashing-and-encoding/);
- a **list** — a tag followed by an ordered sequence of child nodes.

That is the whole container. A construct of the language — a function, a match, a module, a type
declaration — is a list whose tag names the construct and whose children are its parts. Because the
container is just tagged atoms and lists, adding a new construct is adding a new tag; it does not
change how any existing tree encodes.

## Concrete encoding

- The tree is serialized as **deterministic CBOR** (RFC 8949 §4.2), the same canonical encoding the
  value form uses, so there is one byte sequence per tree and equal trees encode identically.
- A node is a CBOR array `[tag, ...children]` for a list, or a tagged primitive for an atom; the tag
  is a stable small integer or symbol drawn from a registry the encoding version fixes.
- The stored artifact begins with the **encoding version** (ast-encoding.md §"The Encoding Is
  Versioned"), so a reader refuses a version it does not implement.

## Comments and documentation

Comments and documentation are **nodes in the tree**, not lexical trivia:

- a **doc node** attaches prose to the definition it documents (the `///` / `(doc ...)` projection);
- a **comment node** attaches a free comment to the node it annotates.

Both are ordinary tagged nodes, so they encode, hash, and round-trip like any other node, and a
printer renders them into whatever display it targets. This is how "store it however, format it
however" keeps comments: they live in the canonical form, not in a particular text rendering.

## Why binary-sexpr

- **Stable across versions:** the container is fixed; only the tag registry grows. A tree stored by
  one compiler generation is readable by another, which is what content-addressed, cross-generation
  programs require.
- **General:** any construct, and any future construct, is just a tagged list — the same reason
  s-expressions have outlived most syntaxes.
- **Reproducible:** deterministic CBOR gives one byte form per tree, so the source hash over the
  binary AST is exact and third-party-checkable, with no text-normalization fragility.
- **Cheap to bootstrap:** a reader is a small deterministic-CBOR decoder plus a tag table — the seed
  toolchain needs little more than it already needs for the value form.
