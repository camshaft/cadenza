# PR #2086 review — cadenza-ast/src/codec.rs (v-syntax) — MERGED — untrusted-transport-decode: 2 substantive + 1 LOW [VERIFIED-PLAUSIBLE]

https://github.com/camshaft/cadenza/pull/2086 (I1 — binary-AST dictionary transport plane, `cdzast\x00\x02`
+ `decode_with_dicts`). Copilot 3 inline on the NEW untrusted-transport decoder. This path decodes
UNTRUSTED transport artifacts + attacker-influenceable dictionaries, so DoS/corruption hardening matters.

## `graft_dict_subtree` may walk a dict's structure WITHOUT a per-subtree visited guard → a malicious dict with a cycle/shared node under `d_root` can diverge/explode (Copilot, codec.rs:690) — DoS [VERIFIED-PLAUSIBLE, MED]
> `graft_dict_subtree` assumes `dict` is already a valid tree, but `decode_detailed` explicitly permits
> unreachable structure nodes (which can still contain cycles/shared subtrees). Since transport dict-refs
> can target any `node_id` … a malicious dictionary can make this loop diverge or explode. Add a
> per-subtree visited guard (like the canonical tree check) so cycles/shared nodes under `d_root` are
> rejected with `NotATree` instead of looping.

VERIFIED-PLAUSIBLE against the diff. `decode_with_dicts` DOES have a `visited` guard (diff:196-202) that
rejects cycles in the TRANSPORT tree — BUT its own comment says "A `DictRef` is a leaf for this walk (its
expansion is a fresh copy of a dict's subtree)" — i.e. that guard treats DictRefs as leaves and does NOT
walk INTO the dict's structure. The actual graft (the `jobs`/`results` worklist that expands a DictRef into
the dict's subtree) has no visible per-dict-subtree visited guard. And Copilot's premise holds:
`decode_detailed` permits UNREACHABLE dict nodes (which can carry cycles/sharing), and a transport dict-ref
can target any `node_id`. So a crafted dict with a cycle under the referenced `d_root` could make the graft
worklist loop/explode — a decode-time DoS on untrusted input. (I can't fully trace the worklist's
termination from the diff alone — hence PLAUSIBLE — but the asymmetry (transport tree guarded, dict-subtree
graft not) is the exact shape.) MED. Fix per Copilot: a per-graft visited guard over the dict's structure
under `d_root`, rejecting a re-reached node with `NotATree`.

## `Grafter::push` / `graft_dict_subtree` truncate lengths to `u32` via `as u32` → an artifact expanding beyond `u32::MAX` nodes silently WRAPS + corrupts the arena (Copilot, codec.rs:629) — correctness/robustness [VERIFIED, MED]
> `Grafter::push` truncates `self.out.len()` to `u32` with `as u32`. If a transport artifact expands
> beyond `u32::MAX` structure nodes (or leaves via the similar `as u32` cast in `graft_dict_subtree`), ids
> will silently wrap and corrupt the output arena. Since `StructId`/`LeafId` are `u32`, this should be
> checked and reported as an error instead of truncating.

VERIFIED in the diff: `fn push(&mut self, s: Struct) -> u32 { … self.out.len() … as u32 }` and
`new_leaf = self.leaves.len() as u32`. A transport artifact whose dict-expansion produces > u32::MAX
structure nodes (or leaves) silently wraps the id → the output arena's node ids collide/corrupt (a decoded
tree pointing at wrong nodes). Extreme size, but it's UNTRUSTED input + a silent-corruption failure mode
(worse than an error). MED. Fix per Copilot: `u32::try_from(self.out.len()).map_err(|_| overflow-error)?`
at each `as u32` id cast, reporting a bounded DecodeError instead of wrapping.

## `verify_tree` doc says it's shared by `decode_detailed` + `decode_with_dicts` but `decode_detailed` has its own inline check (Copilot, codec.rs:727) — doc-accuracy [VERIFIED, LOW]
> …`decode_detailed` currently has its own inline tree-check block and does not call this helper. Either
> refactor `decode_detailed` to use `verify_tree`, or update this comment.
LOW/doc — reword (or refactor `decode_detailed` to call `verify_tree`, dedup'ing the tree check — arguably
the better fix so both decoders share ONE cycle-rejection path, which also relates to finding #1). v-syntax
owns cadenza-ast.
