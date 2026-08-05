# PR #2093 review — cadenza-ast/src/codec.rs (v-syntax) — MERGED — 2 MED [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2093 (I2 — encode_with_dict, honor-supplied-dict transport).
Copilot 2 inline, both MED on the I2 encode/graft path. NOTE: relates to the #2082 design-review
determinism point + is a sibling of the #2086 decode cycle-DoS v-syntax just fixed (03e86220d).

## `by_bytes.entry(key).or_insert((*hash, node))` makes the chosen dict node HashMap-iteration-order dependent → nondeterministic transport bytes when two dict nodes share subtree bytes (Copilot, codec.rs:309) — determinism [VERIFIED, MED]
> `by_bytes.entry(key).or_insert((*hash, node))` makes the chosen dict node for a given subtree depend on
> `DictSet`'s internal `HashMap` iteration order. If two dict nodes … encode to the same subtree bytes,
> the resulting transport bytes can become nondeterministic across runs even with the same `DictSet`
> contents. Prefer a deterministic tie-breaker (e.g., pick the smallest `(hash, node)` pair).

VERIFIED in the #2093 diff: `by_bytes` is a `HashMap<Vec<u8>, (Hash, u32)>` (diff:75) built by iterating
the DictSet and `by_bytes.entry(key).or_insert((*hash, node))` (diff:82) — `or_insert` keeps the FIRST
inserted mapping, and "first" depends on DictSet/HashMap iteration order. So when two dict nodes encode to
the same subtree bytes, WHICH one wins (and thus the emitted DictRef → the transport bytes) is
nondeterministic run-to-run for identical DictSet contents. This is the CONCRETE impl of the "deterministic
transport bytes" concern from the #2082 design review (points 5/6). MED — a transport codec should be
reproducible. Fix per Copilot: deterministic tie-break — `by_bytes.entry(key).and_modify(|e| if (*hash,node)
< *e { *e = (*hash,node) }).or_insert((*hash,node))` (pick the smallest `(hash, node)`), so the pick is
stable regardless of iteration order.

## `decode_with_dicts` is doc'd total/never-panic but grafting assumes each imported dict is a tree — a cyclic dict (callers build `Arenas` directly; `DictSet::insert` doesn't validate) can make graft non-terminate (Copilot, codec.rs:811) — DoS [VERIFIED, MED]
> …grafting assumes every imported dict arena is a genuine tree. Since `DictSet::insert` doesn't validate
> and callers can construct `Arenas` directly, a cyclic dict could make `graft_dict_subtree`
> non-terminating. Consider defensively running the existing `verify_tree` check once per imported dict
> before grafting so malformed dicts fail with `DecodeError` instead of hanging/DoS-ing.

VERIFIED-with-context: this is a SIBLING of the #2086 decode cycle-DoS I flagged (id 3715282992), which
v-syntax fixed in 03e86220d (a per-graft visited guard on the DECODE graft). This #2093 finding is on the
I2 `graft_transport`/encode-side path + points at the DICT-validation-at-insert gap: `DictSet::insert`
doesn't verify the arena is a tree, and callers can build `Arenas` directly, so a cyclic imported dict can
non-terminate the graft. Copilot's fix (run `verify_tree` ONCE per imported dict before grafting) is the
clean systemic guard — arguably better than only a per-graft visited guard, since it rejects a malformed
dict up front for BOTH encode + decode paths. MED. v-syntax: check whether 03e86220d's per-graft guard
already covers this graft_transport path; if not (or even if so), the per-dict verify_tree-at-import is the
belt-and-suspenders that makes "total/never-panic" honest. v-syntax owns cadenza-ast.
