# Design: gap B — reject an unbound type NAME in a type-decl payload / record field (CDZ0101)

**Scoped:** 2026-08-01, v-compiler-ml. The last of breaker's reject-path gaps (companion to the landed
export-unbound + param-linearity + gap3/4 width fixes). Two faces, ONE fix (breaker's note):
- sum ctor payload: `(do (type T (Mk Bogus)) (def (main) 42) (export main))` → ml runs 42, rcdzc CDZ0101
  ("unknown type `Bogus`").
- record field:      `(do (type R (record (: field NoSuchField))) …)` → ml runs 42, rcdzc CDZ0101 ×3.

## Why it's harder than the other gaps
- There is NO declared-TYPE-NAME registry today. The `ct` (ctor) table is keyed by CTOR name, with the
  type's `declName` only as a VALUE field. So a payload/field type-atom `A` (a declared type) vs `Bogus`
  (unknown) cannot be distinguished at read time.
- 🔑 FORWARD REFS ARE VALID (verified vs oracle): `(type B (Mk A)) (type A (Av)) …` COMPILES OK. So the
  check MUST be WHOLE-PROGRAM: gather ALL declared type names first, THEN validate references — an
  incremental at-read-time check would FALSE-REJECT a forward ref. Not a single-pass reader guard.
- Record-decl `(type R (record (: field T)))` currently isn't specially read (runs to 42) — the reader
  treats it as some ctor shape. The fix walks record FIELD types too, not just sum ctor payloads.

## Approach (codegen-light — the guest emit-ceiling lesson)
Whole-program checks added to the run closure must be a SET/LOOKUP, not a fold-over-a-table (a ctor-table
fold tipped the self-host guest over the emit ceiling; a single `def-body-of`/marker lookup is fine).
1. **Registry:** record every `(type Name …)` name into a lookupable set. Options: (a) a new Arena field
   `Set(Int64)` of declared-type-name-ids (record in read-do-type); (b) reuse the def-table with a reserved
   key prefix; (c) a marker-def per type `__type__<nameId>`. Lean (a) — an explicit type-name set, cleanest.
2. **Validate references at read time INTO A MARKER (deferred to whole-program):** because forward refs are
   valid, don't reject a reference the moment it's read (the target may be declared later). Instead: after the
   WHOLE program is read (registry complete), walk the recorded payload/field type-atoms and, for each that
   is NEITHER a builtin (Int/UInt/Bool/Float — is-int-type/Bool/Float) NOR in the type-name registry, record
   the `__illwidth__`-style poison marker (reuse the has-illwidth-marker gate, or a new `__unboundtype__`
   marker). `run-src`/`run-src-typed` decline via a single marker lookup.
   - ⚠ but the payload type-atoms are stored as ENCODED Int64s in the ct argTypes (`-1` for unknown scalar),
     which LOSES the original name — so I can't re-check them post-read from argTypes alone. So EITHER (i)
     record the raw type-atom NAME per payload/field field (new storage), OR (ii) do the validation in a
     SECOND read pass that has the full registry. Decide at build time; (i) is likely cleaner.
3. **Record fields:** teach the reader to recognize `(type R (record (: f T) …))` and walk each field's type
   `T` through the same builtin-∪-registry validation.

## Verification (regress heavily — touches the type-decl reader used by the sum suites)
- `(Mk Bogus)` / `(record (: field NoSuchField))` DECLINE; `(Mk A)` with A declared (before OR after) RUNS;
  plain sums / `(Some 5)` / valid records RUN; guest stays healthy (`42` runs — under the emit ceiling);
  sread-eval-sum/-sum-payload green; ML round-trip clean.

## Record-face parse finding (2026-08-01) — ⚠ CORRECTED 2026-08-01 (breaker re-verify on trunk)
`record` is NOT a keyword the reader handles — `(type R (record (: f Int64)))` is misparsed: read-do-ctors
treats `(record …)` as a "ctor" named `record` with payload `(: f Int64)`.
🔴 **CORRECTION (the earlier "same flat payload atom" claim was WRONG):** the record's FIELD `(: field T)` is a
NESTED `(` group, NOT a top-level payload atom. So `ctor-payload-has-unbound` SKIPS it (nested = out of C's
narrow scope) and the bad field type `NoSuchField` is never checked → adv-48 record-face STILL RUNS 42
(breaker re-verified on trunk ad50154bd: both `(: field NoSuchField)` and no-anno `(field NoSuchField)` run).
gap-B-C correctly fixed the SUM-payload face (the differential-relevant one) — breaker released THAT pin.
- Oracle: BOTH record spellings reject CDZ0101 "unknown type `NoSuchField`" (node 5/6). So adv-48 is a real
  reject-gap, just not the flat-atom shape I assumed.
- To fix adv-48, `ctor-payload-has-unbound` must, when the ctor NAME is `record`, descend ONE level into each
  `(: field T)` / `(field T)` field group and check the LAST atom (the field type) instead of skipping. This is
  a SHALLOW, STRUCTURED, NON-recursive descent into a KNOWN shape (`record` fields are a flat list) — DISTINCT
  from the arbitrary nested applied-type recursion C excludes (`(Holder (Box Bogus2))` = generics frontier).
  → RAISED to concierge: is this shallow record-field descent within Option-C's narrow bar (a known non-recursive
  shape, same CDZ0101 class), or does it count as the nested/frontier work that's OUT? Not urgent (record isn't
  a real compiler-ml feature yet; sum-payload was the differential face). breaker holds the adv-48 pin.

## ✅ adv-48 RULED WITHIN-BAR (concierge 2026-08-01) + AUDIT CLEAN (pre-build)
RULING: YES — the shallow record-field descent is WITHIN Option-C's narrow bar. "One bounded level into a KNOWN
FIXED shape (record = flat field list, check the field-type atom)" ≠ "arbitrary nested applied-type RECURSION
(`Holder (Box Bogus2)` = the frontier C excluded)". C's "nested OUT" meant UNBOUNDED applied-type recursion, not
"never descend a known structure". Same hard-audit + stay-one-level + document-boundary + gated/de-prio.
### AUDIT (concierge's hard gate) — CLEAN, and stronger than expected:
- The covered corpus (01/02/06/07) has **ZERO `(type R (record …))` DECLARATIONS**. Its record usage is 99
  `(record …)` VALUE literals + 37 `(Record …)` TYPE annotations — NEITHER is a `(type …)` decl head, so
  `scan-source-unbound-type` (which fires ONLY on `(type` heads, sread:721) never touches them. ⟹ the record
  descent has **zero covered over-reject risk** (nothing to over-reject). Breaker's `(type R (record …))` decl
  shape is not a covered case — it's a breaker probe.
- Oracle: both `(type R (record (: field NoSuchField)))` + no-anno `(record (field NoSuchField))` reject CDZ0101;
  valid `(record (: field Int64))` runs. So the fix must decline the bad, accept the valid.
### BUILD RECIPE (adv-48, build-ready on the next clean base):
In `ctor-payload-has-unbound`, when the ENCLOSING ctor name is `record` (thread a `is-record: Bool` flag from
`type-ctors-have-unbound`, set when `_cname == "record"`), a nested `(` field is NOT skipped but DESCENDED ONE
level: read the field group `(: f T)` / `(f T)`, take its LAST atom (the field TYPE — skip the `:` and field
name), and apply the SAME predicate (upper-led ∧ ¬builtin ∧ ¬prelude ∧ ¬registry → unbound). STAY one level: if
the field type is itself `(Box X)` (nested applied), check the HEAD atom only, do NOT recurse into its args
(that's the frontier). Document the boundary in-code. Pins: `(type R (record (: f NoSuchField)))` declines,
`(record (f NoSuchField))` declines, `(record (: f Int64))` runs, `(record (: f R))` self/declared runs. RAISE
if the audit surprises (a covered record over-rejects) — don't widen. Low-urgency (record = no-op ctor).

## Design question RESOLVED (2026-08-01 read-only de-risk)
Inspected `read-payload-argtypes` (sread.cdz:633) + `read-do-ctors`/`read-do-type`:
- **Option (ii) is DEAD.** `read-payload-argtypes` collapses EVERY non-int/non-Bool scalar atom to `-1`
  (the UNSUPPORTED sentinel) — the raw name STRING is discarded at read time. So a declared type `A` and an
  unbound `Bogus` both encode as `-1` in argTypes; a post-read re-check of argTypes CANNOT distinguish them.
  Must capture the raw referenced NAME (option (i)).
- **No type-name registry exists.** `read-do-type` records the ctor names (`Some`/`None` → def-table) but NOT
  the type NAME `Opt` itself — nothing to look up a payload atom against today. Must ADD the registry.
- 🔑 **Where the fold is allowed.** The emit-ceiling lesson bounds the `run-src`/`run-src-typed` RUN CLOSURE
  (a ctor-table fold THERE tipped the guest). The READER (`read-do-*`) already folds freely. ⟹ the
  set-difference (referenced-names − builtins − registry) runs ONCE at do-block COMPLETION *in the reader*,
  and — if any referenced name is unbound — records a SINGLE shared `__unboundtype__` marker def (mirror the
  `__illwidth__` pattern, keyed under a fresh reserved key à la `illwidth-marker-key()=0`, e.g. `-1` or a
  second reserved constant). `run-src` then declines via ONE `def-body-of` lookup — codegen-light, guest-safe.
- **Build shape (option (i), for the clean base):**
  1. `read-do-type`: record the declared type NAME into a registry (def-table under a reserved-key namespace,
     e.g. `type-decl-key(nameId)`, so it can't collide with real def/ctor name-ids OR the illwidth key).
  2. Capture referenced payload type-atom NAMES: `read-payload-argtypes` currently returns only `List(Int64)`
     of encodings and does NOT thread the tree — so EITHER thread the tree through it to record each
     non-builtin referenced name-id into a "pending refs" namespace, OR add a sibling scan that collects them.
     (Threading the tree is cleaner; sibling scan avoids touching the hot argType path — decide at build.)
  3. At do-completion, fold the pending refs: any ref whose name is NEITHER a builtin (is-int-type/Bool/Float)
     NOR in the type-decl registry → record the shared `__unboundtype__` marker.
  4. `run-src`/`run-src-typed`: add `has-unboundtype-marker(tree)` (one lookup) to the decline guard, exactly
     alongside `has-illwidth-marker`.
- **adv-48 record-face:** free — records are misparsed as ctors (design finding below), so a record field's
  type atom flows through the SAME payload-atom capture; one validation covers both faces.

## ⚠ SCOPE RE-ASSESSED against the ORACLE (2026-08-01, clean base 21c15bbe4) — BIGGER + RISKIER than assumed
Ran the rcdzc oracle (`cdz convert → cdz compile`) on the exact reject/accept shapes. Ground truth:
- `(Mk Bogus)` uppercase UNDECLARED concrete → **CDZ0101 reject** ✓ (the target bug).
- `(Mk A)` with `A` declared, EVEN forward-ref → **ACCEPT**. `(Box a)` lowercase type-VAR → **ACCEPT**.
  `(Mk Int64)` builtin → **ACCEPT**. `(Box _x)` `_`-led → **REJECT** (unbound). `(Box foo)` lowercase → ACCEPT.
- 🔑 **DISCRIMINATOR:** lowercase-led atom = TYPE VARIABLE → always accept; uppercase- or `_`-led + not-builtin
  + not-in-registry → CDZ0101. (Matches "declare a type var vs reference a concrete type" ML rule.)
- 🚩 **RECURSIVE validation:** `(Holder (Box Bogus2))` → oracle rejects the INNER `Bogus2` (node 11). So the
  check is not just top-level payload atoms — it walks NESTED applied-type args too.
- 🚩 **PRELUDE type constructors:** the COVERED corpus (01/02/06/07) uses uppercase payload names that are
  NOT user-declared and NOT scalar builtins — `List`, `Record`, `Tuple` (`(FromList (List a))`,
  `(Box (Record (: v a) …))`, `(Tuple Int64 Env)`), plus cross-declared user types (`Iter`, `Ast`, `Env`,
  `Expr`). compiler-ml recognizes NONE of these as types today. A naive "reject uppercase-undeclared payload
  atom" would OVER-REJECT these covered cases → flip agree→decline → **DISAGREE = RED GATE** (worse than the
  currently-HELD, non-gating missing-reject). So a correct gap-B MUST: (a) register ALL user-declared type
  names, (b) allowlist the prelude type constructors (List/Map/Set/Tuple/Record/Option/Result/…), (c) treat
  lowercase as a type var, (d) recurse into nested applied-type args. (a)+(b)+(d) is real surface area.

### ⛔ CROSSES THE PAUSED GENERICS FRONTIER — operator gate
The prelude-type-constructor allowlist + nested applied-type walk (`(List a)`, `(Box Int64)`, `(Record …)`)
is exactly the generics/type-constructor territory the operator PAUSED under the cleanup mandate. Doing gap-B
RIGHT pulls that in; doing it NARROW (top-level uppercase-scalar only) risks a covered-corpus over-reject the
moment a case puts a bare cross-declared or prelude type in a top-level payload. → filed an operator ASK
(concierge) to confirm scope: (A) build the full registry+allowlist+recursion now (touches paused frontier),
or (B) keep gap-B HELD until generics unpause, or (C) a proven-safe NARROW slice (only reject an uppercase-led,
non-builtin, non-prelude, undeclared, NON-nested atom, with a corpus over-reject audit as the gate). Not
blocking — HOLDING gap-B (breaker already holds its corpus pins) pending the reply; frontier stays paused.

## ✅ RULED OPTION C (concierge 2026-08-01) + CORPUS OVER-REJECT AUDIT DONE (pre-build)
Ruling: build the NARROW slice (uppercase-led + non-builtin + non-prelude + undeclared + NON-nested → CDZ0101),
HARD corpus over-reject audit as the land gate, nested types OUT (leave to generics-unpause), respect de-prio,
RAISE-don't-widen if the audit surprises. A (full registry+prelude+recursion) is OUT (paused frontier).

### Audit — enumerated EVERY covered-corpus (01/02/06/07) type-decl's TOP-LEVEL payload atoms (C's only scope):
All top-level payload atoms are one of: a scalar BUILTIN (`Int64`/`Int8`/`Bool`), a LOWERCASE type-var (`a`),
an UPPERCASE atom that IS declared in the same program, or a NESTED `(…)` group (which C SKIPS). Detail:
- Builtin scalar payloads (accept): Ast.AInt, C.A, Id.Mk, Meters, N.*, P.Mk, T.Leaf/Node, F.S, P(Int8,Int8),
  W(Int8) — all `Int64`/`Int8`/`Bool`.
- Lowercase type-var (accept, NOT validated): `Box a`, `Cons a`.
- Nested `(…)` payload (C SKIPS — nested is explicitly OUT): AList `(List Ast)`, Box `(Record …)`, ECons
  `(Tuple …)`, Add `(Tuple …)`, Holder `(Box Int64)`, Pair `(Box …)` ×2, Iter `(FromList (List a))`, and the
  illwidth cases T `(Float 8)` / `(Int -8)` (handled by the existing __illwidth__ marker, not gap-B).
- Nullary ctors (no payload): C.B, F.C, Sel.A/B, Ast.ALeaf, Env.ENil, Iter.Nil.
- 🎯 UPPERCASE-DECLARED top-level payloads (the ONLY cases the C-predicate actually inspects + must ACCEPT):
  - `(type Expr (Lit Int64) (Neg Expr))` — `Neg`'s payload `Expr` = the type itself (self-ref). Registry accept.
  - `(type W (Wrap Id))` @ 02-binding-and-control.sexp:3814 — `Wrap`'s payload `Id` is declared ON THE LINE
    ABOVE (`(type Id (Mk Int64))` :3813), expected `(: 10 Int64)`. ⚠ THE decisive case: C's registry MUST be
    WHOLE-PROGRAM / forward-ref-safe (gather ALL decl names, THEN validate) — an at-read-time check would
    false-reject `Expr`/`Id`. With the whole-program registry: ACCEPT. ✅
- 🔴 ZERO covered cases put an UNDECLARED uppercase non-builtin non-prelude atom in a TOP-LEVEL payload → under
  a correct whole-program C-predicate, **over-rejection count = 0**. The audit is CLEAN (pre-build; must be
  RE-RUN as the executable gate on the actual C impl before land — this table is the expected-verdict oracle).

### C build recipe (verified against the audit):
1. Registry = whole-program declared type-names. **Already exists**: the ct-table records every decl's declName
   (nullary via record-ctor-tag, payload via -list). Enumerate ct values for the name set. (No new arena field.)
2. Predicate `payload-atom-unbound(atom, registry)`: TRUE iff `is-upper-led(atom)` AND `not is-int-type(atom)`
   AND `not is-prelude-type(atom)` AND `not in registry`. `is-prelude-type` = the bare-accepting set from the
   oracle probe: Option/Result/String/Bytes/Char/Unit (List/Map/Set/Tuple/Record only appear NESTED→skipped,
   and reject CDZ0203 arity not 0101 anyway, so not needed for C but harmless to include). Lowercase → type-var
   → never flagged. `_`-led → treat as upper-led (oracle rejects `_x`).
3. Capture: scan read-do-ctors' TOP-LEVEL payload atoms only (skip a `(`-led field via skip-to-close — that's
   the nested-OUT boundary). Record referenced uppercase non-builtin names.
4. At do-completion (reader-time fold OK — ceiling bounds only the run closure), for each captured ref not in
   registry∪builtins∪prelude → record a shared `__unboundtype__` marker (mirror illwidth-marker-key; a NEW
   reserved key, e.g. a second small constant — NOT a magic negative, per the operator's neg-sentinel watch).
5. `run-src`/`run-src-typed`: add `has-unboundtype-marker(tree)` (one lookup) to the decline guard.
6. Pins: `(Mk Bogus)`→decline; `(Wrap Id)` w/ Id declared before OR after→run; `(Neg Expr)` self-ref→run;
   `(Box a)` type-var→run; nested `(Holder (Box Int64))`→run (nested not inspected). Re-run the audit table.

## adv-48 STATUS — ✅ BUILT + QUEUED (1d4aaee7d), AUDIT CLEAN (oracle-exact)
Built per the recipe + within-bar ruling. `ctor-payload-has-unbound` threads `is-record` (set when ctor
name==`record`); a record's nested field group is descended ONE level via `record-field-type-unbound` (checks the
field-TYPE atom); non-record nested still skipped; a nested applied field type checked HEAD-only. Audit: covered
corpus has ZERO `(type R (record …))` decls → zero over-reject risk; verified via `cdz run-ml`: both bad record
spellings→declined, valid/declared/nested-applied→value 42 (oracle-exact, nested confirms no frontier-widening).
sread-eval 41/0 (+4 pins). Queued MR 1d4aaee7d. Closes breaker's LAST held reject-pin (adv-48 + record-face).

## gap-B-C STATUS — ✅ BUILT + QUEUED (38e7cdca8), AUDIT CLEAN
gap-B-C BUILT on clean base (trunk 2ecd10086, after S-A1 landed batch #128) per the verified recipe + Option-C
ruling. Impl: parse-db `decl-name-registered` (registry from ct-table, no new arena field) + sread
`is-upper-led`/`is-prelude-or-builtin-type`/`scan-source-unbound-type` (whole-program reader-time walk, nested
SKIPPED) + `unboundtype-marker-key`=1 (reserved non-negative, not a magic negative) + `has-unboundtype-marker`
in the run guard. AUDIT executed via `cdz run-ml` (the differential's ML path): `(Mk Bogus)`→declined; the 5
accept-guards (Wrap-Id-declared / fwd-ref / type-var / self-ref / nested) → all value 42 = ORACLE-EXACT,
over-rejection count 0. sread-eval 37/0 (+6 pins), parse-db 56/0, guest under emit ceiling. Queued MR 38e7cdca8;
asked pr-sync to re-run the ML differential as the land gate. Closes breaker's differential reject-ledger
(gap B + adv-48). ONE deliberate boundary: NESTED payloads (`Holder (Box Bogus2)`) NOT validated — that
recursion is A-scope (paused generics frontier); documented in-code + to breaker.
