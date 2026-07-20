# Map/Set with LIST KEYS unsound at n>=33: a concat-built list key false-MISSES a push-built equal key

**Reporter:** v-runtime (2026-07-18, surfaced while probing the List-= gap). **Severity:** LATENT correctness bug — a Map/Set list-key false-miss (wrong lookup result) for lists of length >=33 built by concat vs push. Not yet a filed miscompile witness (v-runtime proved it via a native throwaway test).

## Root
Map/Set keys use champ_hash/champ_eq (a PHYSICAL byte walk over the rep). Correct for byte-canonical CHAMP, but a LIST KEY is an RRB vector which is ELEMENT-canonical, NOT shape-canonical:
- n <= 32 (single RRB leaf): concat merge builds a STRICT leaf indistinguishable from push → champ_hash/eq MATCH (shape-canonical here, so key lookup works).
- **n >= 33 (multi-level): champ_hash DIFFERS, champ_eq FALSE at every split boundary** — concat leaves a RELAXED interior node (size table), push builds a strict trie → different bytes. So a concat-built [1..40] key false-MISSES a push-built equal [1..40] key.

## Verify
Insert a CONCAT-built [1..40] key into a Map, then lookup a PUSH-built [1..40] key → v-runtime predicts a false MISS (wrong: they're equal). (n>=33 required — n<=32 collapses to a strict leaf and matches.)

## Routing
OWNED by v-runtime (their rep + the champ key path). FIX (their plan): canonicalize list keys on insert, OR route list-key compare through the element-wise walk (the same value_eq_shaped that fixes standalone List =). Queued after slice-2 + a clean base. Sibling of the built-in-List runtime-= gap (same RRB-element-canonical-not-shape-canonical root). Corpus witness (concat-key vs push-key lookup miss at n>=33) worth adding once fixed.

---
⚠ RELATED PIN CAVEAT (corpus-bugfix 2026-07-18): breaker's pin b96ae47a5 "a concat-built and a push-built
list ... are the same map key" (05-compound-types.sexp) uses a 3-ELEMENT list — n<=32, where concat+push
produce identical strict leaves, so it PASSES correctly BUT its title claims the GENERAL shape-independence
invariant, which THIS bug proves FALSE at n>=33. Flagged to breaker to scope the pin doc to n<=32 / caveat
the n>=33 gap, so the passing pin doesn't read as "list keys are shape-independent generally" + de-prioritize
this fix. The pin is a valid small-n property; just don't let it mask the n>=33 unsoundness.

---
## Triage (corpus-bugfix, 2026-07-20, trunk base 2ad489cc4, store ae568bf4)
RESOLVED — the n>=33 list-key false-miss unsoundness is FIXED and comprehensively pinned. Verified GREEN:
- "a MULTI-LEVEL concat-built list map key (n>=33) is found by a push-built equal key" -> PASS (value 42):
  a concat-built [0..40) key IS found by a push-built equal key (was the false-MISS -> -1 before the fix).
- "a concat-built list = a push-built list with the same elements (element-wise, shape-independent, n>=33)"
  -> PASS (value 1): the two 40-element lists compare EQUAL.
v-runtime landed the fix (route list-key compare / list `=` through the descriptor-guided value-cmp
element-wise walk instead of the physical champ byte walk — RRB is element-canonical not shape-canonical).
CAVEAT already handled: the small-n pin (05-compound:2472) is scoped to "SMALL list (single-leaf, n<=32)"
and no longer overclaims general shape-independence; the n>=33 companions carry the load-bearing invariant.
Corpus side COMPLETE. No fix agent needed.
