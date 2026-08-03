# PR #1187 review comments — guide/src/content/chapters/Ordering.tsx (v-guide)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1187
(PR: "cand: v-guide — 6311bf95f").

## 1. Bytes-ordering examples use bare `(list …)` + lowercase "text" (Copilot, Ordering.tsx:113) — doc
> This paragraph talks about Bytes ordering, but the inline examples use bare `(list 1 2)` /
> `(list 1 3)`, which reads like comparing Lists rather than Bytes values. It also uses lowercase
> "text" while the surrounding section refers to the `Text` type. Consider making the examples
> explicitly `Bytes.of …` and using consistent `Text` wording.

## 2. ⚠ Chapter says Map/Set lookup requires ORDERING a key — runtime is hash-trie/equality (Copilot, Ordering.tsx:125) — conceptual correctness
> This explanation says Map/Set lookup requires being able to *order* a key, but the runtime's map
> lookup is hash-trie based and tests key *equality* (with hashing) rather than using an ordering
> comparison for lookup. Rewording to "compare for equality (and use a deterministic order for
> stable enumeration/canonicalization)" avoids teaching the wrong data-structure requirement while
> keeping the motivation for total orders.

Point 2 is the one that matters: the chapter would teach readers that Map/Set *lookup* needs a total
order, but the runtime (CHAMP hash-trie) uses key equality + hashing for lookup; the total order is
for deterministic enumeration/canonicalization, not lookup. Reword so the requirement is stated
correctly while keeping the (real) motivation for total orders. Point 1 is a straightforward
examples/`Text`-casing tidy.
