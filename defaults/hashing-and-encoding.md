# Hashing And Encoding — Declared Default

> **What this file is.** The concrete hash function and canonical encodings that realize three
> frozen contracts: the source-tree hashing rule (source-tree-encoding.md), the canonical value
> byte form (deterministic-value-form.md), and the content addressing that reproducible derivation
> depends on (reproducible-derivation.md). Those contracts state the *properties*
> technology-neutrally; this file names the algorithms.
>
> This is a **declared default** at the ABI/wire level: these choices fix bytes and hashes that
> identify source and components, so a change to them is a coordinated change under the
> constitution's Governance Floors, evaluated against already-derived components with a migration
> path.

## The default choices

| Concern | Default | Realizes |
|---|---|---|
| Cryptographic hash | **SHA-256** | content addressing of components and blobs; the source hash |
| Canonical value encoding | **deterministic CBOR** (RFC 8949 §4.2 core deterministic encoding) | deterministic-value-form.md §"The Canonical Byte Form" |
| Map/set member order | **byte-wise ordering of canonically-encoded members** | deterministic-value-form.md §"Ordering Of Aggregate Members Is Fixed" |
| Text normalization | **Unicode Normalization Form C**; line endings normalized to a single line-feed | source-tree-encoding.md §"Text Normalization Is Pinned" |
| Source-tree encoding | a deterministic sequence of `(tree-relative-path, contents)` entries ordered byte-wise by path | source-tree-encoding.md §"A Source Tree Has One Canonical Encoding" |
| Source hash | **SHA-256 over the canonical source-tree encoding** | source-tree-encoding.md §"The Source Hash Is A Function Of The Canonical Encoding" |

## Alignment with the host

These choices are deliberately aligned with the content-addressing conventions of the host that
runs Cadenza's output (SHA-256 content hashes, deterministic CBOR canonical form), so that a source
hash Cadenza computes and a component hash Cadenza binds interoperate with the host's own content
addressing without translation.

## Why these choices

- **One hash function everywhere** keeps content addressing uniform: source, components, and blobs
  are all identified the same way, and a third party needs only one primitive to verify.
- **Deterministic CBOR** gives every value exactly one byte encoding with a fixed member order,
  which is precisely the "one canonical byte form; equal values encode identically" the value-form
  contract requires — and it is binary, compact, and widely implementable.
- **NFC + single line-feed** makes "the same source" robust to editor and platform differences that
  would otherwise change a source hash without changing meaning.
