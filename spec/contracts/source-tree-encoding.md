# Frozen Contract — Source Tree Encoding

> **FROZEN CONTRACT.** This document pins the canonical encoding of a program's source tree — a tree
> of modules each stored as its canonical binary AST — and the hash computed over it, so that "the
> same source" is a byte-exact, third-party-checkable notion. It is versioned and changed only by the
> coordinated act described in the constitution's Governance Floors. Its requirements realize
> [Core Principle I](../../constitution.md) and [Core Principle II](../../constitution.md) and trace
> to [overview §3](../overview.md) and [overview §7](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract composes the per-module canonical form fixed by the ast-encoding
> contract into a tree; the concrete hash is pinned at the declared-default location.

## Purpose And Scope

A program is compiled from a source tree of one or more modules. Each module's canonical form is its
binary AST, fixed by the ast-encoding contract; this contract composes those modules into a tree and
fixes the one byte sequence and the hash that represent the whole program, so that two parties
computing "the source" of a program compute identical bytes. It does not fix a module's own encoding,
which the ast-encoding contract governs, nor the storage or transport of source.

## Canonical Encoding

### A Module Is Stored As Its Canonical Binary AST

Each module in a source tree MUST be stored as the canonical binary AST fixed by the ast-encoding contract.

The canonical encoding of the tree MUST NOT depend on any textual rendering of a module, because a module's identity is its binary AST rather than a rendering of it.

### A Source Tree Has One Canonical Encoding

A source tree MUST have exactly one canonical byte encoding such that two trees with identical paths and identical per-module binary ASTs encode to identical bytes.

The canonical encoding MUST order modules by their tree-relative path under a fixed total byte-wise ordering, independent of the order in which a filesystem enumerates them.

The canonical encoding MUST include each module's tree-relative path together with its binary AST, so that moving a module's contents to a different path changes the encoding.

### The Encoding Ignores Filesystem Incidentals

The canonical encoding MUST NOT depend on any filesystem metadata beyond a module's tree-relative path and its binary AST.

## Source Hashing

### The Source Hash Is A Function Of The Canonical Encoding

The source hash MUST be a cryptographic hash computed over the canonical encoding of the source tree.

The source hash MUST NOT depend on file modification times, file ownership, or any filesystem attribute other than path and binary AST.

### The Source Hash Is Independently Reproducible

A third party MUST be able to recompute a program's source hash from the program's stored binary ASTs alone, obtaining the same hash the compiler computed.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-hashed source trees, or else carry an explicit version increment and a stated migration path.
