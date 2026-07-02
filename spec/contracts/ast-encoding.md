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
the AST, that the serialization is a bijection with one canonical byte form per tree, that it carries
everything a program means to preserve — including comments — and that textual syntaxes are
conversions to and from it rather than the stored form itself. It does not pin the concrete byte
format, which is a declared-default choice, nor the set of node kinds, which the capability
specifications govern.

## The Canonical Stored Form

### The Canonical Stored Form Is The Binary AST

A Cadenza program's canonical stored form MUST be the binary serialization of its abstract syntax tree.

A program MUST be stored, hashed, and supplied to the compiler as its binary AST rather than as a textual rendering.

### The Encoding Is A Bijection With One Canonical Byte Form

Each abstract syntax tree MUST have exactly one canonical binary encoding.

Two abstract syntax trees that are equal MUST have identical binary encodings.

Decoding a canonical binary encoding MUST yield the abstract syntax tree it was encoded from.

### The Encoding Is General And Stable

The binary encoding MUST represent an abstract syntax tree as a tree of tagged nodes, so that the container form is independent of which node kinds the language currently defines.

The addition of a new node kind MUST NOT change the binary encoding of a tree that does not use it.

## What The Tree Carries

### The Tree Carries Comments And Documentation

The abstract syntax tree MUST be able to carry a comment attached to the node it annotates.

The abstract syntax tree MUST be able to carry documentation attached to a definition, as required by the agent-authoring capability.

A comment or documentation carried by the tree MUST survive encoding and decoding unchanged.

## Textual Syntaxes Are Conversions

### A Textual Syntax Parses To And Prints From The Canonical Form

A textual syntax MUST be defined as a parser that converts its text to the canonical binary AST and a printer that converts the canonical binary AST to its text.

No textual syntax MUST be privileged as the stored form, so that a program's identity is its binary AST and not any one rendering of it.

### Parsing And Printing Are Not In The Compiler's Trusted Path

The compiler MUST accept the canonical binary AST directly, without requiring a textual parser in the path that derives a component.

## Versioning

### The Encoding Is Versioned

The binary encoding MUST carry the version of the encoding it conforms to.

A reader MUST refuse a binary AST whose encoding version it does not implement rather than misinterpret it.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-stored binary ASTs, or else carry an explicit version increment and a stated migration path.
