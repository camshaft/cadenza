# Frozen Contract — AST Encoding

> **FROZEN CONTRACT.** This document pins the canonical stored form of a Cadenza program: a stable
> binary serialization of its abstract syntax tree. This is the form a program is stored as, hashed
> as, and handed to the compiler as; every textual syntax is a parser and printer to and from it, and
> none is privileged. It is versioned and changed only by the coordinated act described in the
> constitution's Governance Floors. Its requirements realize [Core Principle I](../../constitution.md)
> and [Core Principle X](../../constitution.md) and trace to [overview §3](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract states the encoding's properties; the concrete byte format is pinned
> at the declared-default location.

## Purpose And Scope

A Cadenza program's canonical representation is a homoiconic abstract syntax tree. For that tree to
be a durable, hashable, third-party-checkable artifact independent of any surface syntax, its stored
form must be fixed. This contract pins that the canonical stored form is a binary serialization of
the AST, that a node names its kind by referencing a symbol in a prelude the file itself carries so
that the file is self-contained, that the serialization is a bijection with one canonical byte form
per tree, that it carries everything a program means to preserve — including comments — and that
textual syntaxes are conversions to and from it rather than the stored form itself. It does not pin
the concrete byte format, which is a declared-default choice, nor the meaning of the symbols a node
references, which the capability specifications and the executable semantics govern.

## The Canonical Stored Form

### The Canonical Stored Form Is The Binary AST

A Cadenza program's canonical stored form MUST be the binary serialization of its abstract syntax tree.

A program MUST be stored as its binary AST rather than as a textual rendering.

A program MUST be hashed as its binary AST rather than as a textual rendering.

A program MUST be supplied to the compiler as its binary AST rather than as a textual rendering.

### The Encoding Is A Bijection With One Canonical Byte Form

Each abstract syntax tree MUST have exactly one canonical binary encoding.

Two abstract syntax trees that are equal MUST have identical binary encodings.

Decoding a canonical binary encoding MUST yield the abstract syntax tree it was encoded from.

### The Encoding Is General And Stable

The binary encoding MUST represent an abstract syntax tree as a tree of nodes, each a symbol applied to an ordered sequence of child nodes, so that the container form is independent of which node kinds the language currently defines.

The addition of a new node kind MUST be expressible as a new symbol without changing the binary encoding of a tree that does not reference it.

## The Symbol Prelude

### The File Carries Its Own Symbol Prelude

A stored binary AST MUST carry a prelude that lists every symbol its nodes reference, so that the file is self-contained and readable without an external registry.

A node MUST name its kind by referencing a symbol in the prelude by index rather than by carrying the symbol inline.

### A Prelude Symbol Is Namespaced And May Be Versioned

Each prelude symbol MUST carry the namespace it belongs to, so that a symbol defined by the language and a symbol introduced by a macro cannot collide.

Each prelude symbol MAY carry a version, so that the meaning of a construct can evolve while a file that references an earlier version continues to denote it.

### The Prelude Order Is Canonical

The order of symbols in the prelude MUST be a deterministic function of the set of symbols referenced, independent of the order in which nodes were constructed or discovered.

Two abstract syntax trees that reference the same set of symbols MUST produce identical preludes.

## What The Tree Carries

### The Tree Carries Comments And Documentation

The abstract syntax tree MUST be able to carry a comment attached to the node it annotates.

The abstract syntax tree MUST be able to carry documentation attached to a definition, as required by the agent-authoring capability.

A comment or documentation carried by the tree MUST survive encoding and decoding unchanged.

## Textual Syntaxes Are Conversions

### A Textual Syntax Parses To And Prints From The Canonical Form

A textual syntax MUST provide a parser that converts its text to the canonical binary AST.

A textual syntax MUST provide a printer that converts the canonical binary AST to its text.

No textual syntax MUST be privileged as the stored form, so that a program's identity is its binary AST and not any one rendering of it.

### Parsing And Printing Are Not In The Compiler's Trusted Path

The compiler MUST accept the canonical binary AST directly, without requiring a textual parser in the path that derives a component.

## Versioning

### The Encoding Is Versioned

The binary encoding MUST carry the version of the container encoding it conforms to.

A reader MUST refuse a binary AST whose container encoding version it does not implement rather than misinterpret it.

### New Constructs Do Not Bump The Encoding Version

The introduction of a new construct MUST be expressed as a new prelude symbol rather than as a change to the container encoding version.

A file that references a symbol or symbol version a reader does not understand MUST be refused by that reader rather than misinterpreted, without requiring a change to the container encoding version.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-stored binary ASTs, or else carry an explicit version increment.

A change to this contract that is not additive with respect to already-stored binary ASTs MUST carry a stated migration path.
