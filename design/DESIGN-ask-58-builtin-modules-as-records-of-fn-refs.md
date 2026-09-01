## 58. 🟢 DESIGN (operator direction) — Make built-in modules REAL RECORDS of builtin-function refs, so `Bytes.len` is just `(. Bytes len)` const-folded — no name-specialization anywhere

**Operator's framing (2026-07-07, verbatim intent).** *"Have generic const folding on records handle a lot of the
built-in stuff. Ideally the `Bytes` module were just a record of functions — then `(. Bytes len)` would just const
fold and return the function to call, and we'd have some way to bind to a built-in function. That way we don't
specialize names everywhere — we rely on const folding to project to the right thing, and it works the same as
user-defined modules, so there's really nothing special."*

**This is the concrete realization of the standing `member-access-and-modules-as-records` decision** ("`.` is the
sole record accessor; modules / records / prelude are all records"). Today that's *asserted* but not *realized*:
`Bytes`/`Int64`/`String`/… are SPECIAL-CASED as dotted-application heads, not actual record values. Evidence
(reference seed, 2026-07-07): `(. Bytes len)` UNAPPLIED → `declined: unsupported bare form/constructor: Bytes`;
a runtime dotted-application → `decline("unsupported dotted-application")` (`codegen.rs:5987`). So `Bytes` is not a
first-class value in EITHER compiler — it's a name the front-end recognizes in a specific syntactic position.

**Scope (operator clarification, explicit): ALL built-in modules — not just `Bytes`.** This is a WHOLESALE
replacement of the entire built-in-module surface, not a per-module feature. EVERY module the language ships —
`Bytes`, `Int64` (and the width family), `String`, `Char`, `List`, `Set`, `Map`, `Rational`, `Qty`, `Option`,
`Result`, `Ast`, `Symbol`, and any others — becomes a prelude RECORD of builtin-function refs, and the front-end's
per-module recognition (the dotted-application special-casing, the builtin-name table) is DELETED entirely. The
value of the design is precisely that it's uniform across all of them at once: one projection rule + one
builtin-ref lowering table subsumes every `Mod.method` in the language. (`Bytes` below is just the worked example.)

**The proposal.** A built-in module is a genuine RECORD whose field VALUES are built-in-function references:

    Bytes  = (record (len   <builtin bytes-len>)
                     (at    <builtin bytes-at>)
                     (slice <builtin bytes-slice>) …)
    Int64  = (record (wrapping-add <builtin i64-wrapping-add>)
                     (checked-add  <builtin i64-checked-add>) …)
    String = (record (at <builtin string-at>) (scalar-len <builtin string-scalar-len>) …)
    …every built-in module, identically — a record of builtin-refs in the prelude.

Then `(. Bytes len)` is an ORDINARY record projection that const-folds to `<builtin bytes-len>` (exactly the
projection fold compiler.cdz already does for `(. (record …) f)` and `(. r f)` on a let-bound record — ask-57
landings). Applying it — `((. Bytes len) x)` = the desugaring of `(Bytes.len x)` — lowers the builtin-ref to its
intrinsic/opcode. **One projection rule replaces N name-specializations**, and a USER module (a record of user
functions) goes through the identical path — nothing special.

**The single new primitive this hinges on: a BUILTIN-FUNCTION REFERENCE value.** Const-folding `(. Bytes len)`
must yield *some value*, but `len`'s implementation is a wasm intrinsic / hand-emitted opcode, not Cadenza source.
So the value model needs an opaque `(builtin <id>)` node that:
- is a first-class value a projection can PRODUCE (so `(. Bytes len)` folds to it);
- when APPLIED at the right arity — `((builtin bytes-len) b)` — lowers to that builtin's specific emission
  (an opcode, an intrinsic call, or the existing operator lowering — e.g. `Int64.wrapping-add` is already emitted
  as an operator; the builtin-ref for it just routes to that same lowering);
- DECLINES (never miscompiles) in any other position — stored in a data structure, compared, partially applied
  without closure support, applied at wrong arity. Decline-don't-miscompile is preserved by construction.

**Where the records come from (operator's call): PRELUDE-AS-SOURCE.** `Bytes`/`Int64`/… are literally
`(record (len <builtin …>) …)` in a prelude the reader prepends — so a built-in module IS a record value by the
same mechanism a user `(record …)` is, and "nothing special" is *literally* true (not "recognized name → synthetic
record"). This needs (a) the `builtin` primitive above, and (b) a prelude-injection path that seeds these records
before user code. The per-builtin knowledge (which id → which opcode/intrinsic) then lives in ONE place: the
builtin-ref lowering table, not scattered across the reader's dotted-application special cases.

**Why this is high-value (not just cleanup).**
- **Deletes special-casing from 3 places at once:** the reader's dotted-application handling, the resolver's
  builtin-name recognition, and (in compiler.cdz) the per-builtin plans I keep declining to write because
  specializing each builtin name is fragmented and contortion-adjacent. This ask is the principled alternative.
- **Unlocks a large decline cluster by construction:** every `Bytes.len` / `Int64.wrapping-add` / `String.at` /
  `Set.contains` / `Char.to-int` corpus case (the dotted-method builtins — ~20+ scalar-output cases the loop has
  been unable to touch because they desugar to `(apply (. Mod method) args)` = a computed-callee application both
  compilers decline) becomes: project the field (fold) → apply (lower the builtin-ref). No per-name code.
- **Self-hosting relevance:** compiler.cdz itself will want to be organized as modules-as-records; this makes the
  built-in surface uniform with that, so the eventual self-hosted compiler doesn't carry a separate builtin table.

**compiler.cdz is READY for its half.** The projection-fold (direct, let-bound, and nested — ask-57) already
folds `(. <record> field)` to the field value. Once `Bytes` is a real prelude record of builtin-refs, `(. Bytes
len)` folds with ZERO new reader code. The only compiler.cdz addition is "apply a builtin-ref → emit its
lowering," and for the builtins already emitted as operators (`wrapping-add`, `checked-*`, bitwise, shifts, `=`,
comparisons) that's a small id→existing-lowering table. So this is a SEED/SPEC change first (the `builtin` value
+ the prelude), and compiler.cdz follows cheaply.

**Acceptance signal.** `(. Bytes len)` folds to a builtin-ref (a first-class value); `(Bytes.len (Bytes.of (list
1 2 3)))` compiles via project-then-apply with no `Bytes`-specific reader branch; a user module `(record (f <fn>))`
projected+applied works identically; a builtin-ref used as a bare value / wrong arity DECLINES (no miscompile);
the `member-access-and-modules-as-records` spec decision is now realized, not asserted. Corpus: the dotted-method
builtin cluster (Bytes/Int64/String/Set/Char scalar-output cases) moves decline→agree.

**Status.** 🟢 DESIGN ask, operator-directed. SEED + SPEC change (the `builtin` value + prelude-as-source records)
must land before compiler.cdz wires the apply-a-builtin-ref lowering. Related: `member-access-and-modules-as-
records` (the decision this realizes), ask-57 (the projection-fold this rides on), ask-13 (sum/list patterns — a
sibling piece of the "compounds are ordinary" unification). Not a workaround — it REMOVES special-casing; the loop
has repeatedly declined the per-builtin alternative as contortion-adjacent, and this is the principled replacement.

---

## ✅ NATIVE PHASE LANDED (2026-07-07) + 🔑 a finding that shapes compiler.cdz's half

**Native now realizes ask-58 (verified 21:31 build):** applied builtin-methods COMPILE — `(Bytes.len (Bytes.of
(list 1 2 3)))`→3, `(Int64.wrapping-add 5 3)`→8, `(UInt8.wrap 65)`→65. A BARE `(. Bytes len)` declines with the
new, correct message *"bare built-in operation value not representable (apply it)"* — i.e. `Bytes` IS a record now,
projecting `len` yields a built-in operation VALUE, which is fine unapplied-declines and lowers on APPLY. Binding
it to a name (`(do (def f (. Bytes len)) (f …))`) not yet supported (the first-class-ref-as-value case). So the
core ask-58 semantics are live native-side.

**🔑 KEY FINDING for whoever wires compiler.cdz's half — the builtin-module record is NOT in the AST bytes.** I
decoded `(. Bytes len)`: it encodes as `[., <name-tag Bytes>, <name-tag len>]` — `Bytes` is just a bare NAME tag
(`d8 27 <idx>`). The builtin-module RECORD (`Bytes = (record (len <builtin bytes-len>) …)`) is a SEED-SIDE RESOLVE
concept — the seed's resolver knows the name `Bytes` denotes a builtin record and projects accordingly. It does
NOT appear in the canonical AST. **Consequence:** compiler.cdz can't ride its existing `(. record f)` projection-
fold (ask-57) directly on `(. Bytes len)`, because there's no record literal in the bytes to project — it sees a
member access on an unbound name. compiler.cdz's half therefore needs its OWN builtin-module PRELUDE (the
prelude-as-source records + the `builtin` value + the id→lowering table), matching what the seed's resolver has.
That is the substantial part — NOT a gap-independent slice — so the loop is NOT wiring it piecemeal (a per-method
`(. Bytes len)`-special-case in compiler.cdz's reader would be exactly the per-builtin contortion ask-58 exists to
avoid). It waits for a deliberate prelude-as-source pass in compiler.cdz (or for the prelude to be shared/derivable
so both compilers read the same builtin-record source).

**Remaining ask-58 work:** (1) first-class builtin-ref as a bindable/storable VALUE (native declines it today);
(2) the no-arg builtin constants (`Int64.max`/`Int64.min` still `declined: unsupported dotted-application` native-
side); (3) compiler.cdz's prelude-as-source half (the above). The APPLIED-method core is done native-side.
