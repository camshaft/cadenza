# Design/scoping: compound (multi-field) + nested-sum payload ctors — the next sum increment

Owner: v-compiler-ml. Scoped tick-442 (2026-07-25). Status: DESIGN-READY + v-inference ANSWERED all 3 rep Qs
(t443) + construction VERIFIED already-works (t443). Build-gated on b127 landing (the ~380s flaky-cap-race
batch that froze trunk; this increment's END-TO-END correctness NEEDS the e2e sread-eval-sum gate, so do NOT
build the run-slices while that gate races the cap). This note makes execution mechanical once the gate is fast.

## v-inference REP ANSWERS (t443, all 3 confirmed)
- Q1 ✅ ct-argType → (declName, tag, List(Int64)) per-field encoded types (same 0/1/-1/≥2 space; nullary=[],
  1-field=[enc], (Pair Int64 Bool)=[65,1]). SAME 12th arena field (List inside the tuple) → ~4-6-site ripple.
  🪤 REFINEMENT: keep the single-field FAST PATH — `ctor-argtype-of` stays (return head-or-0) for the existing
  1-field callers; ADD `ctor-argtypes-of` (the list) for multi. Don't force every caller to index a list.
- Q2 ✅ (b) CONSTRUCT-first then multi-binder PATTERN. VERIFY (Pair 3 4) already lowers via arg2/3/4-of first.
  Then extend NMatchCtor to List(Int64) binderIds (option a) for the PATTERN slice only.
- Q3 ✅ multi-SCALAR-field NOW; DEFER nested-sum (it needs the argType to encode a DECL name — a different
  encoding axis: reserve a range e.g. negatives below -1 = declName, so infer seeds binder TSum(declName) +
  cross-type reject composes recursively — a LATER slice). Keep -1 covering nested-sum (declines) until then.

## 🔑 VERIFIED t443 (fast unit probe, off the cliff): multi-field CONSTRUCTION ALREADY WORKS — zero Core change
Hand-built `(Pair 3 4)` = NApp(860, a1) + record-arg2(callId, a2) + ct entry for Pair(860), typed TSum(800),
→ `lower-node` produces `CCtor(0, [CNum 3, CNum 4])`. So `lower-rec-args`' existing arg2/3/4-of splice ALREADY
builds the multi-element payload list. ⇒ SLICE 1 SCOPE COLLAPSES: construction needs NO lower/Core/eval change.
The ONLY missing pieces for a multi-field ctor to CONSTRUCT + type are:
  (1) ct schema → List(Int64) (so a multi-field ctor is recorded, not -1);
  (2) reader (read-do-ctors) parses N payload atoms → the List (currently reads ONE then skip-to-close);
  (3) infer types the multi-field ctor-app node TSum (the NApp-ctor arm already types payload ctors TSum via
      ctor-tag-of — VERIFY it doesn't gate on single-arg; likely already fine).
Then the e2e pin `(match (Pair 3 4) ((Pair x y) (+ x y)) …)` needs the PATTERN slice (multi-binder, slice 2).
So even slice-1 (construct + type) is testable at fast unit level for the lower/type shape, but the RUN pin
`run-src("(do (type P (Pair Int64 Int64)) (def (main) (Pair 3 4)) …)")` returning a handle needs e2e.

## Where we are (ground-payload COMPLETE)
Single-payload ctors work end-to-end: `(type Box (BB Int64))`, `(BB 5)` constructs, `((BB x) …)` matches +
binds `x`, int-width + Bool payloads type correctly. Non-single-scalar payloads (multi-field `(Pair Int64 Int64)`,
nested-sum `(Wrap (Box Int64))`) currently record ct-argType **-1 (unsupported)** → the binder infers TErr →
any use declines cleanly (the sound slice boundary, PR#849-style).

## What's already READY (no change needed) — the CORE + eval
- **Core `CCtor(Int64, List(Core))`** (lower-db.cdz:52) already holds a payload LIST — multi-element ready.
- **Core `CMatchSum(Core, Int64, List(Int64), Core, Core)`** (lower-db.cdz:58) — binders is a `List(Int64)`,
  and eval's `store-payload(h, i)` indexes slot `i`. Multi-binder deconstruct is already wired at the Core/eval
  layer; `bind-payload` binds each `binders[i]` from payload slot `i`.
- So the DEEP eval/rep design point (SumStore, multi-slot payload) is ALREADY DONE. This increment is a
  FRONT-END-only extension (reader + ct-table + infer binder-typing + lower binder-LIST construction).

## THE BLOCKER (the one real design decision) — ct-argType is single-valued
The ct table value is `Tuple(Int64, Int64, Int64)` = `(declName, tag, argType)` (parse-db.cdz:396). `argType`
is ONE encoded type (0=none/Int64, 1=Bool, -1=unsupported, ≥2=int via encode-param-type). It structurally
CANNOT record 2+ payload field types. Extending it is the crux.

### OPTION A — ct-argType becomes a LIST of encoded types
Change ct value to `(declName, tag, List(Int64))` where the list is the per-field encoded argTypes (each 0/1/-1/≥2
as today). `record-ctor-tag-typed` takes a `List(Int64)`; `ctor-argtype-of` returns the list; a single-payload
ctor is `[enc]`, nullary is `[]`, a 2-field `(Pair Int64 Bool)` is `[65, 1]`.
- PRO: reference-faithful (rcdzc's `argTypeList`), directly feeds CMatchSum's binder-LIST, no encoding tricks.
- CON: another Tree.Arena value-shape change — but it's the SAME ct table (the 12th field), so the ripple is
  only the ct accessors (record-ctor-tag-typed / ctor-argtype-of / ctor-payload-binder-type / decl-enum-disc-go),
  NOT the 24-site arena-field ripple (no NEW field). ~4-6 sites. Manageable.
- 🪤 `decl-enum-disc-go` matches on `Tuple(Int64, Tuple(Int64, Int64, Int64))` (parse-db.cdz:439) — must update
  to the list shape. `ctor-tag-of`/`ctor-decl-of` read slots 1/0 — unaffected by slot-2 becoming a list.

### OPTION B — keep single argType, add a PARALLEL ct-arity + per-field side lookup
Rejected: splits the ctor's type info across two structures (v-inference's t365 steer explicitly says co-locate
tag+argTypes in ONE ct entry — "splitting doubles lookup + risks drift"). Option A honors that.

**RECOMMENDATION: Option A.** Confirm the encoding with v-inference (their infer/unify lane); it's the natural
list-of-encoded-types and matches the CMatchSum binder-list they already reviewed.

## Slice plan (each gated; the e2e gate is sread-eval-sum, so these need b127's cap relief first)
1. **ct schema (Option A):** ct value `(declName, tag, List(Int64))`; update record-ctor-tag-typed (+ the
   argType=0 shorthand record-ctor-tag → `[]` or `[0]`?  — decide: `[]` for nullary, `[enc]` for 1-field),
   ctor-argtype-of → `List(Int64)`, ctor-payload-binder-type → indexed (see 3), decl-enum-disc-go arm.
   FAST-GATEABLE at parse-db unit level (like tick-437/439).
2. **reader (read-do-ctors):** parse ALL payload type atoms between the ctor name and the pattern's `)` (loop
   scan-atom until `)`), encoding each → the List(Int64). Currently `scan-atom(s, c2, "")` reads ONE then
   skip-to-close. FAST-GATEABLE at sread unit level (read-source + ctor-argtype-of assertion, off the cliff).
3. **infer (ctor-payload-binder-type → per-field):** today returns ONE Typed from the single argType; extend to
   type the i-th binder from argTypes[i]. The NMatchCtor arm binds a SINGLE binderId today — a multi-field
   pattern `((Pair x y) …)` needs a binder LIST. ⚠ this needs a NEW Node shape or a binder-list on NMatchCtor
   (currently `NMatchCtor(scrut, patCtorName, binderId, body, rest)` — ONE binderId). DESIGN SUB-POINT: extend
   to a binder-id list, OR (reader desugar) — TBD, see below. FAST-GATEABLE for the per-field decode (infer unit).
4. **reader (multi-binder pattern `((Pair x y) body)`):** read N binder names into N NVar nodes; NMatchCtor
   carries them. Needs the binderId→binder-LIST change (co-design w/ #3).
5. **lower:** `lower-binder-list` already maps -1→[] / else→[binderId]; extend to the full binder list. CCtor
   construction `(Pair a b)` = NApp with 2 args → already have arg2-of/arg3-of/arg4-of; lower-rec-args builds
   the List(Core). So CONSTRUCTION may already work for ≤4 fields once ct records them! VERIFY.
6. **e2e pins (sread-eval-sum):** `(type P (Pair Int64 Int64))`, `(match (Pair 3 4) ((Pair x y) (+ x y)) …)`→7;
   nested `(type W (Wrap (Box Int64)))` if nested-sum payloads are in scope (may be a further slice).

## The NMatchCtor binder-list sub-decision (the one to settle with v-inference before building #3-4)
NMatchCtor currently carries ONE binderId. Two paths:
- (a) extend the node to `NMatchCtor(scrut, patCtorName, List(Int64) binderIds, body, rest)` — clean, matches
  CMatchSum's binder-list, but ripples resolve/infer/lower NMatchCtor arms (each binds/types the list).
- (b) keep single-binder for now, ship multi-FIELD CONSTRUCTION (which may already work via arg2/3/4-of) +
  multi-field pattern as a SEPARATE follow-on. Runnable-first slicing (like ii-b construct-before-deconstruct).
RECOMMEND (b) for the first sub-slice: verify multi-field CONSTRUCTION `(Pair 3 4)` already lowers to CCtor(tag,
[3,4]) via the existing multi-arg NApp path (arg2-of…), gate that alone (a ctor value that constructs + a
single-field-or-nullary match over it), THEN do the binder-list pattern as slice 2. Smaller MRs, each e2e-gated.

## Open question to v-inference (SENT t442)
1. ct-argType rep: Option A (`List(Int64)` of per-field encoded types) — confirm?
2. NMatchCtor binder-list: extend the node to a binder-id list (a), or runnable-first construct-only then a
   separate multi-binder-pattern slice (b)? I lean (b).
3. Nested-SUM payload `(Wrap (Box Int64))`: is the payload-field-type encoding meant to cover a SUM field
   (encode a declName?), or is nested-sum a further increment beyond multi-SCALAR-field? (The -1 unsupported
   currently covers both; multi-scalar-field is the smaller first step.)

## Build discipline
Do NOT build under the flaky ~380s cap-race (b127). When b127 lands (v-cdz-tooling's 4→2 + cap relief) and the
e2e gate is reliable: slices 1-2 are fast-tier (parse-db/sread unit, off the cliff) — can even start those
BEFORE full e2e relief since they don't import sread-eval; slices 3-6 need the e2e gate. Start with the ct
schema + reader (fast-gateable) once my current 6-stack lands (don't deepen an unmerged stack).
