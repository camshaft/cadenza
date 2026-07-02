# Frozen Contract — Source Tree Encoding

> **FROZEN CONTRACT.** This document pins the canonical encoding of a program's source tree
> and the hash computed over it, so that "the same source" is a byte-exact, third-party-checkable
> notion. It is versioned and changed only by the coordinated act described in the constitution's
> Governance Floors. Its requirements realize [Core Principle I](../../constitution.md) and
> [Core Principle II](../../constitution.md) and trace to [overview §3](../overview.md) and
> [overview §7](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract fixes an encoding and a hash rule, not a storage medium; the
> concrete hash and encoding realizations are pinned at the declared-default location.

## Purpose And Scope

A program is compiled from a source tree of one or more files. Reproducible derivation and
content-addressed identity both depend on there being exactly one byte sequence that represents a
given tree, so that two parties computing "the source" of a program compute identical bytes. This
contract fixes the canonical encoding of a source tree and the hash over it. It does not fix the
*content* of source — that is governed by the capability specifications — nor the storage or
transport of source.

## Canonical Encoding

### A Source Tree Has One Canonical Encoding

A source tree MUST have exactly one canonical byte encoding such that two trees with identical file paths and identical file contents encode to identical bytes.

The canonical encoding MUST order files by their tree-relative path under a fixed total byte-wise ordering, independent of the order in which a filesystem enumerates them.

The canonical encoding MUST include each file's tree-relative path together with its contents, so that moving a file's contents to a different path changes the encoding.

### Text Normalization Is Pinned

The canonical encoding MUST apply a single fixed line-ending normalization so that source differing only in line-ending convention encodes identically.

The canonical encoding MUST fix a single text normalization form so that source differing only in that normalization encodes identically.

The canonical encoding MUST NOT depend on any filesystem metadata beyond a file's tree-relative path and its contents.

## Source Hashing

### The Source Hash Is A Function Of The Canonical Encoding

The source hash MUST be a cryptographic hash computed over the canonical encoding of the source tree.

The source hash MUST NOT depend on file modification times, file ownership, or any filesystem attribute other than path and content.

### The Source Hash Is Independently Reproducible

A third party MUST be able to recompute a program's source hash from the program's files alone, obtaining the same hash the compiler computed.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-hashed source trees, or else carry an explicit version increment and a stated migration path.
