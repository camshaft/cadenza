# ii-c2b-2 nullary→TSum atomic slice — resolved design (build once v-inference rules X vs Z)

Owner: v-compiler-ml. Depends on: `21b97c14` (decl-is-enum-disc) + `20416688c` (TErr-scrutinee) landing.
Ruling (a) [v-inference, tick 398/402]: a nullary ctor is TSum, NOT an int — retire M2-a rep-transparency.

## The mechanism (all pinned empirically, ticks 404–405)

A nullary ctor use `(Red)` currently types as its NLit tag's Int and lowers to a bare `CNum`.
Making it type `TSum(declName)` broke 16 sum tests → **8** after the def-body-infer fix. The 8 split:
- **(A) rep-transparency, SHOULD decline per ruling (a)** — rewrite to expect None:
  `ss-ctor-tag-in-arith` (`(+ (Blue) (Green))`), `ss-ctor-tag-compared-in-match` (`(match (Green) (0 100)…)`).
- **(B) nullary ctor PATTERN, must keep running** — need the pattern-routing piece:
  `ss-nullary-sibling-of-payload-matches-on-tag`, `ss-match-on-ctor-pattern-{first,middle,wildcard}`,
  `ss-match-ctor-pattern-over-bound-var`, `ss-match-mixed-literal-and-ctor-patterns`.

## Pieces

### Piece 1 — infer (PINNED, tick 404)
NApp ctor arm: a ctor use (nullary OR payload) → `TSum(declName)`. For a NULLARY ctor, MUST still
`infer-node(calleeBody)` first (types the def-body NLit) — else lower's `lower-node(calleeBody)` finds no
type fact and declines. Then set the NApp's own type to `TSum(declName)`.

### Piece 2 — lower nullary ctor rep (per v-inference flag, tick 405): decl-is-enum-disc BRANCH
enum-disc is a per-DECL rep. A nullary ctor lowers as:
- `decl-is-enum-disc(declOf ctor)` (all-nullary, ≥2) → `CNum tag` (bare-tag inline, M2-a path).
- else (MIXED decl) → `CCtor(tag, [])` (boxed empty-payload handle) — shares payload siblings' store rep,
  so a match over the mixed sum reads it via `store-tag` correctly. (Matches rcdzc select.rs:2030/6196.)
This is why `decl-is-enum-disc` (21b97c14) is a LOWER consumer.

### Piece 3 — nullary ctor PATTERN (AWAITING X vs Z ruling)
- MIXED-decl nullary pattern `((None) body)`: route through NMatchCtor tag-compare on the BOXED handle
  (store-tag) — identical to a payload pattern, no new mechanism. SETTLED.
- ENUM-DISC-decl nullary pattern `((Green) body)` over a BARE-TAG scrutinee: CMatchSum's store-tag returns
  None on a bare tag → always-else → WRONG. Needs one of:
  - **Option X**: keep the integer-NMatch/CIf tag-compare; add a reader ctor-tag-pattern MARKER (distinguish
    `((Green) body)` from raw `(0 body)`) + a `match-type` arm admitting a TSum-enum-disc scrutinee vs a
    same-decl ctor-tag literal (still decline raw-int-over-TSum). Eval untouched.
  - **Option Z** (leaning): teach `CMatchSum` to compare a BARE-TAG scrutinee directly when `store-tag` is
    None but the value is an enum-disc tag → route ALL nullary patterns (enum-disc + mixed) through
    NMatchCtor uniformly, no reader marker. Trades a reader/infer change for an eval change.

### Piece 4 — test rewrites
- (A) 2 tests → expect None (decline), + add idiomatic ctor-pattern replacements.
- (B) 6 tests → keep running via piece 3.
- Verify (B) return the REAL int (e.g. `ss-nullary-sibling…` → 100), not just not-None.

## Verified soundness of the tag-compare (Option X viability)
`ss-match-on-ctor-pattern-first` (`(match (Red) (Red 100)…)` → 100) PASSES on current trunk — the
tag-compare lowering is already correct; X only adds the marker + type-admit.

## Cross-type reject reuse
Both X and Z reuse `ctor-pattern-cross-type` (landed `c86deab4e`) — a nullary pattern of the wrong decl
still rejects.

## Option Z soundness (why a bare-tag CMatchSum is safe)
Worry: at eval a value is just an Int64 with no runtime marker — would a PLAIN integer 0 match a
`((Red) …)` pattern (Red = tag 0)? NO — same argument as the cross-type reject / tag-only Core: a
tag-only compare is sound BECAUSE infer gates it. A nullary-ctor pattern match requires (`match-type`)
the scrutinee to be `TSum` of the pattern's decl; a plain-int scrutinee is `TInt ≠ TSum` → infer rejects
it BEFORE lower emits CMatchSum. So by eval, the type column already proved the scrutinee is a same-decl
enum-disc sum value — the bare Int IS a ctor tag, not an arbitrary integer. Eval needs no runtime
discriminator; infer's TSum gate is the guarantee. ⇒ Z = one eval-side bare-tag arm in CMatchSum (when
`store-tag` is None, compare the scrutinee value directly against `tag`), no reader marker, no match-type
ctor-tag-lit special case. Leaning Z (smaller, unifies the pattern path). X/Z still v-inference's call.
