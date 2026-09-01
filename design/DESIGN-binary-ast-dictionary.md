# Vertical-ready: binary-AST dictionary (hashed dict-imports + node-by-index transport codec)

**Design doc:** `implementation/design/DESIGN-binary-ast-dictionary.md` (landing via MR on branch
`fleet/design-ast-dictionary`, commit `93995a5cc`).

**Operator ruling (the load-bearing fork, already resolved — do NOT re-open):** option **A** (seq-122)
— a dict-bearing AST is **non-canonical**, "and that's fine". The frozen `ast-encoding.md` bijection and
the canonical `cdzast\x00\x01` identity / content-addressing / `Event::hash` stay UNCHANGED. Dict-bearing
bytes are a NEW `cdzast\x00\x02` transport plane that `decode_with_dicts` resolves + grafts into a normal
dict-free arena (canonicalization to the inline form is done by `encode`/`canon`, NOT by the decoder —
matches landed I1).

**seq-125 refinement (fold in):** dict identity = its content hash; a dict-bearing AST is a **DAG** whose
content hash is a **hash-of-hashes** (own structure + referenced dict content-hashes) computed **without
deref** (`hash_dag`, doc §4.5). Full-deref+canonicalize stays available (the transform) but is off the
hashing hot path. So there are TWO content-address bases (inline frozen identity vs cheap DAG hash);
which one a subsystem keys on is its choice (doc §2.1). One point flagged to the operator (non-blocking):
whether a NEW dict-bearing program's stored identity is the DAG hash (default) or the deref-canonical
inline hash.

**Subsystem split:**
- **wire half → `v-syntax`** (area = `cadenza-ast`).
- **model / resolution → `v-metaprogramming`** (area = `cadenza-ast` / `rcdzc`).
- **first consumer → `v-agent-harness`** (invoke wire; waits on the invoke primitive's generic
  marshalling landing).

**I1 — LANDED** (v-syntax, PR #2086/#2093): `cadenza-ast/src/codec.rs` + `ast.rs` now carry the
`cdzast\x00\x02` header, `TAG_DICT_REF=2`, a value-only `Hash([u8;32])` defined IN cadenza-ast (the
BOTTOM crate — cdz-kernel depends on it, so it does NOT reference `cdz-kernel::Hash`), the transport-only
`DictRef{dict,node}` + `DictSet` types, `DecodeError::MissingDict(Hash)`, and `decode_with_dicts(&[u8],
&DictSet)` (hermetic resolve + tree-guard-before-graft + graft to a normal dict-free arena; NO post-graft
canon — canonicalization stays in `encode`/`canon`). Canonical `encode`/`decode`/`canon` + `cdzast\x00\x01`
bytes UNTOUCHED; canonical `decode`/`decode_detailed` REFUSE `\x00\x02` (`BadHeader`). **Next up:** I2
(`encode_with_dicts`) / I2b
(`hash_dag` DAG hash-of-hashes) / I3 (model API) / I4 (invoke wire) stack after — see the doc §5.

**Sequencing:** start AFTER the in-flight bytes-literal arc (operator seq 113, B2a/B2b) — composes
cleanly + strictly additive (a dict entry can be a `Bytes` leaf). `v-syntax` has been holding all dict
wire code pending this ruling; it can now proceed on I1. `v-metaprogramming` (model, I3) + `v-agent-harness`
(invoke-wire beneficiary/constraint, I4) want to be in the build loop from the start.
