# Design/scoping: NEXT compiler-ml increment — MATCH + USER SUM TYPES (both greenfield)

**Scoped:** 2026-07-22 (tick 223), v-compiler-ml. After HIGHER-ORDER complete (HO-1..3 landed/queued), the
pipeline is still INTEGER/BOOL-only. The next big front-end surfaces, both DECLINE today:

## What's unsupported (probed tick-221/223)
- **`match`**: `(match 5 (5 1) (_ 0))` cdz-check-CLEAN but RUN → `declined`. The reader has NO `match`/`NMatch`
  handling (grep: no read-match/NMatch in sread.cdz) — it parses loosely (permissive check) but has no
  resolve/infer/lower/eval → declines. Match is ENTIRELY greenfield.
- **user sum types**: `(type Opt (Some Int64) (None))` — the reader has NO `type`-decl handling (no NType/
  read-type/variant in sread). Greenfield.
- **lists**: `(list 1 2 3)` + List.len → declined (a separate greenfield surface; likely AFTER match/sum, since
  match is more foundational).

## Why MATCH + SUM first (recommendation)
Match is THE foundational pattern-matching construct + sum types are the core ADT. Together they unlock most of
the rest of the language (Option/Result, tree walks, the compiler's own AST shapes). Bigger than any single HO
slice — a multi-stage effort across the WHOLE pipeline (sread → parse → resolve → infer → lower → eval), mirror
the HO slicing discipline (small gated MRs).

## Proposed slice plan (M = match)
- **M1 (reader):** sread reads `(type Name (Ctor Ty…) …)` → an NType decl (variant table: ctor-name → arg types
  + tag), and `(match scrut (pat body)…)` → an NMatch node (scrutinee + arm list; pat = literal | wildcard |
  `(Ctor.Name binders…)`). parse-db: NType/NMatch nodes + a variant table (like the def/param tables).
- **M2 (resolve):** resolve a match's scrutinee + each arm's pattern binders (a ctor-pattern binds its
  sub-binders in the arm body's scope) + arm bodies; resolve a ctor-name → its variant/type.
- **M3 (infer):** a sum type `TySum(name, ctors)` (extend Ty/Typed, thread through unify-ty/ty-eq/bridge like
  TyFn); a ctor application types as its sum; a match types as the JOIN of its arm bodies (all same type),
  scrutinee must match the pattern types; exhaustiveness (later).
- **M4 (lower):** a ctor value → a Core tagged-value node (CCtor(tag, args)); a match → a Core CMatch (scrutinee
  + arms) or lower to nested CIf on the tag (simpler first — integer-tag dispatch). 🪤 mirror rcdzc's match lowering.
- **M5 (eval):** eval a CCtor → a tagged heap/tuple value; eval CMatch → test the scrutinee's tag, bind the
  matched arm's binders, eval that arm. (⚠ eval values are Int64 today — a sum value needs a richer eval value
  OR a tag+payload encoding into the Int64/env model; DESIGN POINT — may need an eval Value ADT, the first
  non-scalar eval value. Coordinate w/ v-inference/v-runtime on the value representation.)
- Gate each: run-ml pins (`(match (Opt.Some 5) (…) …)` → 5, etc.).

## Sequencing
Start AFTER HO-3 lands + `cargo xtask fleet sync --force` (base is 28 behind; don't stack this big increment on
the lagged branch). Coordinate the sum-value eval representation with v-inference (infer/unify owner) early —
it's the one deep design point (eval's first non-scalar value). Consider an OPERATOR steer on match-vs-lists
priority if unsure, but match+sum is the stronger default (foundational).

## M1 hook points (pinned tick-226, read-only)
- **`(type …)` decl:** sread `read-do-form` (sread.cdz:493-498) dispatches do-block items — `if kw=="def" then
  read-do-def else (export/decline)`. M1 inserts `else if kw=="type" then read-do-type(...)` BEFORE the
  export/decline fallthrough. `read-do-type` mirrors `read-do-def`'s shape (scan the type name, then each
  `(Ctor Ty…)` variant, recording a variant table keyed like the def/param tables) then `read-do-next` to
  continue the do-sequence. parse-db gets a variant table (ctor-name → (typeName, tag, argTypes)) + accessors.
- **`(match …)` expr:** NOT a do-item — it's an expression, handled in the `read-form`/`read-paren-form` cycle
  (sread.cdz:156-ish, where `do`/prefix-forms dispatch). Add a `(match scrut (pat body)…)` arm → NMatch node.
  Correct s-expr form confirmed t223: arms are `(pat body)`, pat = literal | `_` | `(Ctor.Name binders…)`.

## M1 arena-ripple finding (tick-239) — design refinement
Tree.Arena has 11 fields (node map + next-id + 9 Int→Int tables). Adding a NEW arena TABLE (e.g. a variant/ctor
table) ripples to ~24 `Tree.Arena(...)` match sites (22 in parse-db = all accessors, +1 db.cdz, +1 lower-db) —
mechanical but large, and needs cdz TEST (not just check) to confirm no pipeline regression (silent-decline
class, cf. HO-3). REFINEMENT: MINIMIZE arena-field additions —
- NMatch/NType node DATA rides in the NODE MAP (the Node variant carries its children/arms), NO new arena field.
- ctor→(type,tag,argtypes) lookup: if match/infer must resolve a ctor by name, that DOES want a table — but see
  if it can be derived from the NType node itself (walk the type decls in the node map) before adding a 12th
  arena field. Prefer node-map-derived over a new arena table to avoid the 24-site ripple.
- Arm representation: an NMatch node holds scrutinee-id + a list of (patternId, bodyId) — patterns are their own
  nodes (NLit/NVar-as-binder/a ctor-pattern node). Keep it in the node map.
BUILD DISCIPLINE: M1's first sub-slice (Node variants + reader) is a core-reader change → build + `cdz check` +
`cdz test` (sread/parse-db) IN ONE SHOT on a CALM host (not piecemeal under load — the silent-decline risk).

## PROGRESS: M1-a DONE + MR'd `139e6d281` (tick-245) — integer match RUNS
2-arm literal/wildcard INTEGER match end-to-end: `match 5 (5 111)(_ 222)`→111, `match 3`→222. Node.NMatch +
read-match-form (read-paren-form keyword chain) + resolve (4 children same-env) + infer match-type + lower →
CIf(CBin(==,scrut,lit),then,else) (reuses Core; NO eval/emit change). Gate sread-eval-match 4/0 + regression
clean. KEY: desugar-to-CIf/CBin kept the ripple SMALL (only resolve+infer+lower NMatch arms; eval/emit untouched).
Stacked on doc-fix `48b600c00`.

## NEXT slices (post-M1-a)
- **M1-b: N-arm integer match — ✅ DONE + MR'd `bf9b05bfb` (tick-253, stacked on recursion-fix):** did it as a
  READER DESUGAR to a right-nested chain of 2-arm NMatch (NO resolve/infer/lower change — reader-only!) rather
  than a new arm-list node. read-match-arms recursion: `_`→terminal wildcard, else literal arm→NMatch+recurse.
  Gate sread-eval-match 9/0 (+3 N-arm pins). 🔑 desugar-to-existing-node kept it reader-only.
- **M2-a: USER SUM TYPES — NULLARY-CTOR slice ✅ DONE + MR'd `5960ab7d1` (tick-269, on landed trunk `eebde872b`):**
  `(type Name (C0)(C1)…)` with ALL-NULLARY ctors → READER-ONLY desugar (read-do-form `type` arm → read-do-type
  → read-do-ctors) to a nullary def per ctor `Ck→NLit(k)` (ordinal TAG). A ctor use `(Ck)` runs via the EXISTING
  nullary-call pipeline (NApp→inline NLit→eval) — ZERO new Node/arena/resolve/infer/lower/eval (same discipline as
  M1-b). Payload ctor `(Some Int64)` DECLINES cleanly (slice boundary). Gate sread-eval-sum 8/0 + regression clean.
  🔑 desugar-nullary-ctor-to-tagged-nullary-def kept it reader-only, deferring the whole value-rep design point.
- **M2-a-ctor-patterns: nullary CTOR-PATTERNS in match ✅ DONE + MR'd `58c151186` (tick-270, 1-deep on landed trunk):**
  `(match c (Red 100)(Green 200)(_ 300))` — a nullary ctor NAME as a match pattern. Reader-only: a ctor pattern
  desugars to its tag literal (the ctor is a nullary def `Ck→NLit(tag)`), reusing the integer-match lowering, ZERO
  pipeline change. New `read-arm-pattern` on the match-arm PATTERN position: digit/`-digit`→literal (read-form);
  else scan the atom as a ctor NAME + `def-body-of`→`NLit(tag)`. 🪤🔑 ctor names are CAPITALIZED but read-form's
  bare-id arm leads on the LOWERCASE `is-alpha` set only (sread.cdz:48, a..z, no uppercase) — read PATTERN names via
  `scan-atom` NOT read-form (patterns read NAMES not exprs → scan-atom is correct). compiler-ml subset-reader limit,
  NOT a Cadenza-lang bug. Gate sread-eval-sum 13/0 (+5). Also verified `(let ((v (Y))) (match v …))` composes (my
  earlier "let gap" probes were WRONG SYNTAX — s-expr let is `(let ((x V)…) body)` Scheme-style, NOT `let x = V in`).
- **M2-b (NEXT, the DEEP slice): PAYLOAD ctors + TySum + ctor-PATTERNS-WITH-BINDERS** — `(type Name (Ctor Ty…)…)`
  with args + a ctor/variant representation + ctor-application-with-args + payload-binding ctor-patterns in match.
  THE deep design point: eval's first non-scalar value. ⚠ GATE on: (1) v-inference confirms value-rep steer still
  holds (asked t270 note; no reply yet), (2) ctor-patterns MR `58c151186` LANDS (M2-b re-touches sread.cdz + adds
  eval/infer — must NOT stack on the queued reader MR; build on CLEAN trunk).
  **HOOK POINTS scoped t271 (read-only, so M2-b builds fast):**
  - **Node (parse-db.cdz:41):** payload ctor needs the ctor's arg types recorded. A ctor `(Some Int64)` in read-do-ctors
    currently DECLINES; M2-b records it (ctor-name→(tag, argTypeList)) — PREFER node-map-derived over a 12th arena
    field (t239 refinement). A ctor USE with args `(Some 5)` reads like an NApp; a ctor PATTERN with binders
    `((Some x) body)` needs a new pattern node OR a reader desugar.
  - **Resolved (resolve-db.cdz:23):** RBound/RPoison/RDef today; a ctor-name-as-value may want `RCtor(tag)` (or ride
    RDef since a ctor is already a nullary def for the nullary case). Pattern binders resolve in the arm-body scope.
  - **Typed (infer-db.cdz:22) + Ty (ty.cdz:36):** add `TSum(name, ctors)` / `TySum(name, ctors)` threaded through
    unify-ty/ty-eq/ty-bridge EXACTLY like TFn/TyFn (HO-1 pattern). 🪤 decl-identity compare at recursion points.
  - **Core (lower-db.cdz:23):** add `CCtor(tag, List(Core))` (ctor value w/ payloads) + `CMatch`/reuse nested-CIf on
    tag. eval-db: a sum value = Int64 HANDLE into a side `SumStore` (sum-store.cdz, M2-b-1 `9e44b2567`, empty-store/
    store-alloc/store-tag/store-payload) threaded LIKE `defs` (keeps eval-core-d's Int64 sig; adds CCtor/CMatch arms).
    Defer full Value-ADT ripple.
  - **PARSE-PATH finding (t280, read-only):** a payload ctor USE `(Some 5)` ALREADY parses as `NApp(name-id Some,
    argId)` (read-app-or-bin → op-code-of miss → read-param-call), then DECLINES LATE (unknown def-name → no callee).
    So M2-b-2 needs: (1) read-do-ctors RECORDS a payload ctor in a ctor-table `name→(tag, arity)` (prefer node-map-
    derived / a minimal table over the 24-site arena ripple — t239 refinement), (2) lower turns `NApp(ctorName, args)`
    into `CCtor(tag, args)` when ctorName is a recorded ctor (else the existing def-call/decline path), (3) eval's
    CCtor arm store-allocs, (4) eval threads the SumStore like defs, (5) payload-binding patterns `((Some x) body)`
    read a parenthesized pattern → CMatch that store-reads payload[i] to bind x. v-inference reviews infer/unify (TySum).
  - **v-RUNTIME interface:** a sum value as the SINGLE export crosses the host boundary as a heap value — loop them in
    BEFORE wiring run-src to return a sum (their lane).
  **KNOWN M2-a-tag gaps that M2-b's TYPES will close (probed t274, currently decline/mis-match — DO in M2-b, not now):**
  - FORWARD-REF ctor PATTERN: `(match (Y) …) … (type C (X)(Y))` (type decl AFTER the match) DECLINES, because
    read-arm-pattern resolves ctor-name→tag EAGERLY at read time via def-body-of (not yet recorded). Ctor USES
    forward-ref fine (NApp keys by name, resolves late). FIX = defer pattern→tag resolution out of the reader into
    resolve/lower (M2-b adds RCtor + a real ctor-pattern node instead of the reader tag-desugar).
  - CROSS-TYPE ctor pattern: `(match (Red) (Off 1)(On 2)…)` — Color's Red (tag 0) MIS-MATCHES Bit's Off (tag 0)
    because M2-a compares raw untyped tags. UNSOUND but inherent to the tagless rep; M2-b's TySum makes infer REJECT
    a pattern whose ctor isn't of the scrutinee's type. (Not fixable without types — squarely M2-b.)
  **v-inference STEER (tick-251, their lane):**
  - VALUE REP: rcdzc = (B)-shaped tag+heap-payload (Core::SumNew{disc,payloads} + sum-new/disc/payload ops,
    deconstructed via a MatchSum decision tree, Maranget). BUT for eval-db (an interpreter, Value=Int64 today),
    steer the **MIDDLE path (A-ish)**: a sum value = a heap HANDLE threaded as an Int64 = an index into a SIDE
    `Map(Int64, (tag, List(Int64)))` threaded through eval LIKE `defs` — keeps eval-core-d's Int64 signature,
    M2 only adds CCtor/CMatch arms + the side-map. Defers the full Value-ADT (eval-core-d:Option(Value)) ripple
    until multi-type payloads at the boundary genuinely need it (if ever, do THAT as its own increment, not M2).
  - TYPE REP: `TySum(name, ctors)` threaded through unify-ty/ty-eq exactly like TyFn (HO-1). 🪤 GOTCHA: a RECURSIVE
    sum (Tree/List) — the folded (annotation) path collapses to Sum{decl} while the unfolded (value) path holds
    Nominal{decl} at the recursion spot → compare by DECL IDENTITY at the recursion point, NOT structurally,
    else infinite-loop/mis-compare. Bake in from the start (rcdzc uses Ty::Nominal for the named-decl wrapper +
    a Nominal back-edge to close recursive sums).
  - v-RUNTIME: a sum value returned as the program's SINGLE export crosses the host boundary as a heap value
    (the "single heap export" rule Record.with also hит) — loop v-runtime in on the render/marshalling BEFORE
    wiring run-src to return a sum value. Their lane, a real interface.
  - v-inference offered to review the M2 infer/unify diff. This is the bigger half; M1-a/M1-b prove match on scalars first.

## Status
M1-a LANDED-pending (MR `139e6d281`, stacked on doc-fix). M1 hooks/refinement pinned. Execute M1-b then M2 on a
clean base. Operator's "widen the language surface" direction after generics.

## M2-b-2b DECISIONS (v-inference reviewed + greenlit, tick-307/308) — build M2-b-2b-ii on these
- **M2-b-2b-i ✅ DONE + MR'd `603a9fa66` (tick-306):** CCtor(tag,args) + CMatchSum(scrut,tag,binders,then,else)
  eval mechanism, direct-Core tested (eval-db 64/0), eval-only (can-emit=false). Uses M2-b-2a's SumStore threading.
- **Q1 ANSWER — nullary ctors type as TySum, NOT Int.** The tag Int is the EVAL rep (SumStore/M2-a); the TYPE is
  the sum. rcdzc: `(Color.Red)` types as `Color` — that's WHY `(match c ((Color.Red) 1)…)` type-checks (scrutinee
  + pattern share the sum type). ⚠ M2-a currently types nullary=Int → M2-b-2b-ii MUST change that to TySum (a real
  change, needed for cross-type soundness — an Int tag has no sum identity to reject a foreign ctor against).
- **Q2 ANSWER — add the parse-db ctor-table arena field** `name→(typeName, tag, argTypeList)` (rcdzc uses exactly
  `variant_ctor_index: Map<ctorName, declOccurrence>`). Take the 24-site Tree.Arena ripple; it's reference-faithful.
  (Fallback if ripple too heavy: a side Map keyed by ctor-name → NType decl node-id, derive tag/argtypes from node.)
- **infer/unify plan CONFIRMED sound:** TySum(name, ctors=List((tag, List(Ty)))) through unify-ty/ty-eq like TyFn;
  DECL-IDENTITY compare at recursion (unify iff same decl AND args unify pairwise, NEVER structural inner). 🪤 the
  FOLD/UNFOLD μ arm: a recursive sum's back-edge presents as bare TySum{decl} vs declared Nominal{decl} — SAME decl
  — so a projected recursive field must unify with the folded declared param on EQUAL DECL. ctor-app types as TySum
  (arity + pairwise-unify args to declared arg types); match-arms join to one type; ctor-pattern MUST belong to the
  scrutinee's sum (closes t274 cross-type gap — rcdzc REJECTS `(match (b:Bit) ((Color.Red)…))` CDZ0203); payload
  binders get the ctor's declared arg types in the arm-body scope.
- **v-inference will SAME-TICK review the infer/unify DIFF** against its checklist (unify decl+args+μ arm / ctor-app
  types TySum / match-join / ctor-pattern→payload-binder-types / cross-type reject). Send the diff when up.
- **M2-b-2b-ii build order (once M2-b-2b-i lands):** (1) parse-db ctor-table + read-do-ctors records payload ctors
  (name→tag,argtypes) [the SKIP arm from the payload-skip fix becomes a RECORD arm]; (2) Ty/Typed TySum + bridge +
  unify; (3) lower payload-ctor NApp(name,args)→CCtor(tag,args); (4) lower payload-binding match pattern
  `((Some x) body)`→CMatchSum(scrut,tag,[binder-ids],then,else); (5) reader for `((Ctor x…) body)` parenthesized
  patterns in read-arm-pattern; (6) nullary ctor typing Int→TySum. Slice further if one MR is too big.

## ii-c REQUIREMENTS (v-inference re-confirmed t335, my review target)
- ii-b (payload-ctor CONSTRUCTION, Int64 placeholder typing) = SOUND intermediate — no reachable cross-type gap
  yet because deconstruct isn't wired. Good runnable-first slicing.
- 🔴 ii-c MUST FULLY REPLACE the Int64 placeholder with TySum for a payload-ctor construction — NO residual
  Int-typing of a ctor node past ii-c (else a ctor value unifies with a plain Int → cross-type-pattern reject
  can't fire). NULLARY ctors ALSO become TySum in ii-c (not just payload) so scrutinee+pattern share sum identity.
- v-inference SAME-TICK reviews the ii-c infer/unify diff vs checklist: unify decl+args+μ arm / ctor-app types as
  TySum (incl nullary) / match-join / ctor-pattern→payload-binder-types / cross-type reject. Send diff or ping.
- ii-c pieces: (1) payload-binding match pattern `((Some x) body)` reader (a new pattern node carrying ctorName +
  binder-ids) → CMatchSum lower; (2) Ty/Typed TySum(name, ctors=List(tag,List Ty)) + bridge + unify (like TFn,
  decl-identity μ arm); (3) ctor-app + nullary-ctor + match all type TySum; (4) cross-type ctor-pattern reject.

## ii-c2b DECISIONS (v-inference reviewed ii-c2a + answered, t365) — build ii-c2b on these
- ii-c2a (TySum/TSum decl-identity type-layer) REVIEWED CORRECT, ship it. 🔑 the decl-ONLY model is SIMPLER than
  + SUBSUMES the μ fold/unfold arm v-inference earlier flagged: no structural inner, no Sum-vs-Nominal split, so a
  recursive sum's back-edge AND declared form are BOTH TySum(sameDecl) → unify trivially by decl-identity, no
  fold/unfold divergence. NO μ-arm needed. (Confirm a Tree/List recursive sum types+runs at ii-c2b — should Just Work.)
- 🔴 ct-table: EXTEND the ONE ct table to `name→(tag, argTypes)` co-located (NOT a separate table). rcdzc precedent:
  tag + payload-type both come from the SAME `(type Name (Ctor Ty…))` decl occurrence, read together at ctor-use/
  pattern-binder typing; splitting doubles lookup + risks drift. Record BOTH at read-do-type in one ct entry.
  → so read-do-ctors must PARSE the payload type atoms (currently skip-to-close skips them) into a List(Ty)/encoded.
- ii-c2b CHECKLIST (v-inference same-tick review target): nullary AND payload ctor-app → TySum(decl); ctor-PATTERN
  → TySum(scrutinee's decl) or REJECT if different decl (cross-type); payload binder gets ct argTypes[i]; match arms
  join to one type. NO residual Int-typing of any ctor node (t335). Send v-inference the infer/unify diff.
- ii-c2b touches: parse-db ct table (tag→(tag,argTypes)) + read-do-ctors (parse payload arg-types) + infer (NApp-ctor
  arm ii-b: Int64→TySum; NMatchCtor arm ii-c1: binder Int64→argTypes[i], match→TySum-aware join + cross-type reject).

## ii-c2b-2 DECISIONS (v-inference reviewed ii-c2b-1 + answered nullary Q, t377)
- ii-c2b-1 (payload ctor-app→TySum) REVIEWED CORRECT, ship it.
- 🔑 NULLARY ctor: TSum at INFER + NLit-tag at LOWER = EXACTLY the reference type/rep split. rcdzc types a nullary
  ctor value as its SUM type (so scrutinee+pattern share the sum) BUT lowers to Core::SumNew{disc, payloads:[]} =
  a disc/tag (the runtime rep IS the tag). So for ii-c2b-2: change ONLY the INFER type of a nullary ctor-use
  (Int→TSum(declName)); LOWER stays the NLit-tag/def-body-inline path (do NOT route through a TSum-aware lower).
  Keep the tag available for M2-a's (match c (Red 1)…) integer-tag desugar exactly as-is. No residual Int-TYPING
  of ctor nodes (t335) — the nullary tag at LOWER is fine, that's the REP not the type.
- ii-c2b-2 CHECKLIST (v-inference review target, soundness payoff): (1) nullary ctor-use INFER→TSum(declName)
  [lower unchanged=tag]; (2) ctor-PATTERN typed TSum(scrutinee's decl) or REJECT if different decl (cross-type,
  via unify-None-on-different-decl — Color.Red vs Bit scrutinee); (3) payload binder ← ct argTypes[i] (needs ct
  extended to (declName,tag,argTypes) + read-do-ctors to PARSE payload type atoms); (4) match arms join to one type.
- 🪤 the nullary ctor-use node is NApp(name,-1) → currently infer's def-body-inline path types it as the NLit
  tag's Int. ii-c2b-2 must intercept it (ctor-tag-of hits) BEFORE the def-body path to type it TSum, while LOWER
  keeps taking the def-body path (they diverge: infer=TSum, lower=inline tag). Mirror the payload argId!=-1 arm.
