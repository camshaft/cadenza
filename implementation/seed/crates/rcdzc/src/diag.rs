//! Diagnostics — a "no" as a first-class value produced where the decision is made.
//!
//! The compiler distinguishes three kinds of "no", ordered by safety
//! (`reference-compiler.md` §Outcomes Are Ordered By Safety): a **reject** (the program is
//! ill-formed — carries a stable machine-readable [`Code`]), a **decline** (the compiler does not
//! yet realize the construct — uncoded, an honest "not built yet"), and a **trap** (a run-time halt;
//! its compile-provable form is a poison, which is a reject with a code). This module gives the
//! value that carries a reject/decline; the kind is fixed at the point it is produced and carried
//! concretely, never reconstructed downstream from an artifact's shape
//! (`reference-compiler.md` §The Kind Of A "No" Is Fixed Where It Is Produced).
//!
//! The kind is machine-branchable: [`Reject::code`] is `Some` for a reject and `None` for a decline,
//! and a trap's compile-provable form is a coded poison — so an agent branches on the outcome kind:
//!
//= spec/capabilities/diagnostics.md#a-diagnostic-names-its-kind
//# The compiler MUST expose a machine-branchable kind for each outcome distinguishing a rejection (the program is ill-formed), a decline (the compiler does not yet handle the construct), and a trap (a runtime halt), so an agent routes around compiler limits rather than chasing them.
//!
//! The taxonomy grows one variant per added check; its `str` form is the stable `CDZ####` string a
//! tool branches on.

/// The `DeclineId` catalog (the unsupported-error tracker's oracle: a stable, enumerable referent for
/// every construct rcdzc declines to compile) is GENERATED from `data/unsupported.sexp` by
/// `xtask-codegen-declines` (operator seq-106: sexpr source → xtask codegen → rust, no xtask→rcdzc edge).
/// rcdzc consumes it here; `Reject::id`/`declined()` reference it via this re-export
/// (`crate::diag::DeclineId`). Edit the sexpr + `cargo run -p xtask-codegen-declines` to regenerate.
mod declines_generated;
pub use declines_generated::DeclineId;

/// A stable, machine-readable diagnostic code. Its `code()` string is the durable identity a
/// consumer matches on; the enum variant is the compiler-internal handle.
///
//= spec/capabilities/diagnostics.md#every-diagnostic-has-a-stable-code
//# Every diagnostic the compiler emits MUST carry a machine-readable code that is stable across changes to unrelated diagnostics.
///
//= constitution.md#xi-diagnostics-are-machine-actionable
//# Every diagnostic the compiler emits MUST carry a stable machine-readable code.
///
/// Each variant names exactly ONE rule/rejection — its own docstring cites the spec section it enforces,
/// and `code()` maps it to a stable `CDZ####` that IS the pinned code set's "the rejection each code
/// names". So the code a diagnostic carries names the rule it enforces, machine-readably: an agent
/// branches on the `CDZ####` to know which requirement was violated and act on it programmatically.
//= constitution.md#xi-diagnostics-are-machine-actionable
//# Every diagnostic the compiler emits MUST name the rule or requirement it enforces so that an agent can act on it programmatically.
//= spec/capabilities/diagnostics.md#every-diagnostic-attributes-a-rule
//# Every diagnostic the compiler emits MUST name the rule or requirement it enforces so that an agent can trace the diagnostic to its cause.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Code {
    /// A LEXICAL well-formedness defect the READER detected but cannot itself report through the
    /// artifact channel (the front-end's stderr is not the diagnostic surface) — an unrecognized string
    /// escape `\q` (`collections-and-text.md` §A String Literal's Escapes Are A Closed Set). The reader
    /// emits a marker leaf (`Leaf::BadEscape`) that survives the binary AST codec; the COMPILER turns
    /// that marker into this coded rejection, so a lexically-malformed literal fails the build with a
    /// stable code rather than silently reading `\q` as the bare `q`.
    BadEscape,
    /// A char literal that names a NON-scalar code point — a surrogate (`#\u+D800`) or a value outside
    /// `U+0000..=U+10FFFF` — or is otherwise malformed. Like `BadEscape`, a LEXICAL defect the READER
    /// detected (emitting a `Leaf::BadChar` marker) but cannot itself report; the COMPILER turns the
    /// marker into this coded rejection (`collections-and-text.md` §A Char Is A Single Unicode Scalar
    /// Value). The static companion of the dynamic `(Char.from-int 55296)` → None.
    BadChar,
    /// An `unquote` (`,x`) or `unquote-splicing` (`,@x`) OUTSIDE any quasiquote context — a SYNTAX error
    /// (`metaprogramming.md` §Quasiquote Constructs AST With Selective Evaluation: "Unquote and
    /// unquote-splicing outside a quasiquote context MUST be a syntax error"). `,`/`,@` only mean
    /// something inside a `` ` `` template; a bare `,x`, or one nested only under a PLAIN `(quote …)`
    /// (quote's body is inert data, NOT a selective-evaluation template), has no template to insert into.
    /// In the CDZ00xx reader/syntax band with `BadEscape`/`BadChar` — a structural defect in the quoting
    /// forms, distinct from `EffectNoHome` (CDZ0401), which the corpus formerly reused for this by mistake.
    UnquoteOutsideQuasiquote,
    /// A reference to a name with no binding in scope — the unbound-name rule, unconditional and not
    /// gated on reachability (`core-semantics.md` §Binding Is Lexical).
    Unbound,
    /// A binder position binds the same name more than once — a non-linear binder. A pattern
    /// (`core-semantics.md` §Patterns Compose: "A pattern MUST bind each name at most once … rather than
    /// silently shadowing an earlier binder") and a function's PARAMETER LIST are the same binder-linearity
    /// surface, so `(def (f x x) …)` (or `(tuple a a)` in a pattern) is this error rather than a last-wins
    /// shadow that makes the first binder unreachable.
    NonLinearBinder,
    /// A malformed or ill-typed construct that the compiler positively proves ill-formed.
    Malformed,
    /// A type mismatch (e.g. an `if` condition that is not a boolean; branches of differing type).
    TypeMismatch,
    /// A `?`/`try` operator with NO fallible boundary that admits it — the enclosing function's result
    /// type is neither `Result` nor `Option` (or the `?` is not inside a function at all), so there is no
    /// boundary for its short-circuit to exit to (`DESIGN-try-operator-rcdzc.md` §6). The fix hint tells
    /// the user to annotate the enclosing function's return type as `(Result _ e)` / `(Option _)` (or, in
    /// v2, wrap the expression in a `try { … }` block). DISTINCT from the CDZ0203 `TypeMismatch` a `?` on
    /// a non-fallible OPERAND raises: this is about the missing BOUNDARY, that one about the operand shape.
    TryNoBoundary,
    /// A comparison ACROSS THE NOMINAL BOUNDARY — comparing two values whose types are distinct NOMINAL
    /// types even when structurally identical (`(= (A.Mk 1) (B.Mk 1))` for two same-shape sums `A`/`B`;
    /// a nominal record vs a plain record of the same shape). A nominal type's identity is its
    /// declaration, so such a comparison is ill-typed, NOT `false` — the untagged structural comparison
    /// the nominal boundary forbids (`type-system.md` §Nominal Types Are Not Comparable Across Their
    /// Boundary). Distinct from `TypeMismatch`: the operands are the SAME structural shape, differing
    /// only in nominal tag.
    NominalMismatch,
    /// Two operands of different numeric types with no explicit conversion (no silent promotion).
    NumericMismatch,
    /// An integer literal that does not fit the width its use requires.
    IntOutOfRange,
    /// A `(pragma default-integer <T>)` directive naming a type OUTSIDE the integer domain the numeric
    /// model admits — a well-formed directive (recognized key, one type argument) whose argument is a
    /// valid type that simply is not an integer type (`Float64`, a record, …), so it fails the
    /// integer-domain predicate (`numeric-model.md` §A Module May Declare Its Default Integer Literal
    /// Type: the default MUST name an integer type). In the CDZ03xx NUMERIC band — a numeric-domain
    /// failure, not a structural one — DISTINCT from `MalformedDirective` (CDZ0602, wrong arity) and
    /// `UnknownDirective` (CDZ0601, an unknown key): the key and arity are right, only the numeric
    /// domain of the named type is wrong.
    //= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-integer-literal-type
    //# The type named by a default-integer-literal directive MUST be an integer type the numeric model admits, and a directive naming a non-integer type MUST be rejected with the machine-readable diagnostic for that unsatisfied constraint.
    NonIntegerDefault,
    /// A constant operation whose defined outcome on its (compile-time-known) operands is a trap —
    /// e.g. a provable overflow. A compile-provable trap fails the build rather than shipping a
    /// component that traps at run time (`reference-compiler.md` §A Compile-Provable Trap Fails The
    /// Build).
    ConstTrap,
    /// A `match` that does not cover its scrutinee — a coverage defect (the pattern engine's
    /// non-exhaustiveness rejection, distinct from a shape defect). For a scalar scrutinee this is a
    /// match with no wildcard tail; for a sum it is a missing variant (a later increment).
    NonExhaustive,
    /// A record ROW operation names a field the operand record ALREADY CONTAINS — `Record.merge` /
    /// `Record.extend` combining or adding a field whose name is already present (`type-system.md` §Two
    /// Records Are Combined Only When Their Field Sets Are Disjoint / §A Field Is Added To Or Replaced In
    /// A Record By A Derived Operation). Rejected so a combined record never has to choose which operand's
    /// value a shared field takes and `extend` never silently overwrites (the author means `Record.with`
    /// to replace). The row-operation companion of the duplicate-field literal `(record (a 1) (a 2))`
    /// (CDZ0201); in the CDZ021x types-and-patterns band with its dual, `AbsentField`.
    //= spec/capabilities/type-system.md#two-records-are-combined-only-when-their-field-sets-are-disjoint
    //# A combination of two records whose field sets share a name MUST be rejected at compile time with the machine-readable code for a field that is already present, so that a combined record never has to choose which operand's value a shared field takes and the fixed-field-set invariant is preserved.
    PresentField,
    /// A record ROW operation names a field the operand record DOES NOT CONTAIN — `Record.project` /
    /// `Record.without` / `Record.with` / `Record.pop` restricting to, dropping, updating, or popping a
    /// field absent from the operand (`type-system.md` §A Record Is Restricted To A Named Set Of Its
    /// Fields, §A Record Is Reduced By Dropping A Named Set Of Its Fields, §A Field Is Added To Or Replaced
    /// In A Record By A Derived Operation). Rejected at compile time so a projection cannot produce a field
    /// the operand never held, a drop/pop of an absent field is a static error not a no-op, and a `with`
    /// of an absent field stays distinct from `extend`. A record field name is a STATIC label (not a
    /// runtime index), so an absent one is this compile-time rejection, never a runtime `None`. The dual of
    /// `PresentField`.
    //= spec/capabilities/type-system.md#a-record-is-restricted-to-a-named-set-of-its-fields
    //# A projection that names a field the operand record does not contain MUST be rejected at compile time with the machine-readable code for a required field that is absent, so that a projection cannot silently produce a field the operand never held.
    //= spec/capabilities/type-system.md#a-record-is-reduced-by-dropping-a-named-set-of-its-fields
    //# A drop that names a field the operand record does not contain MUST be rejected at compile time with the machine-readable code for a required field that is absent, so that dropping a field the record never held is a static error rather than a no-op.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A field update whose named field is absent from the operand record MUST be rejected at compile time with the machine-readable code for a required field that is absent, so that updating a field the record never held is a static error rather than an addition.
    AbsentField,
    /// A `match` ARM that can never be reached because an EARLIER arm already covers every value it would
    /// — a duplicate variant/literal arm, or any arm after a catch-all binder/wildcard. The DUAL of
    /// non-exhaustiveness: where CDZ0210 flags a value NO arm covers, this flags an arm NO value reaches.
    /// A WARNING (not a rejection): the program is well-formed and runs correctly (first-match wins, so
    /// the shadowed arm is simply dead), but a redundant arm is almost always a defect (a typo in a
    /// variant name, a copy-paste, a misordered wildcard). The pattern analogue of the `DeadTrap` /
    /// `UnusedBinding` warnings — dead code the build surfaces rather than silently keeping.
    RedundantArm,
    /// A CONSTRUCTION or MATCH of a variant of an ABSTRACT type — a type whose HANDLE another module
    /// exported but whose CONSTRUCTORS it withheld (opaque/abstract types — `modules-and-namespaces.md`
    /// §Visibility Is Explicit). The importing file can NAME the type (annotate, hold, pass its values)
    /// but MUST NOT construct or take apart its variants; it builds and reads such a value only through
    /// the module's exported functions ("smart constructors"). DISTINCT from a plain unbound name
    /// (CDZ0101): the constructor is not merely absent, it is HIDDEN ON PURPOSE — so the diagnostic names
    /// the type, notes its handle is exported but its constructors are not, and (when the module exports a
    /// function returning the type) points at that function as the way in.
    ///
    /// This is the reject that keeps an abstract type's representation unobservable across its boundary:
    /// constructing or matching a variant would strip the name tag to the underlying structural value, so
    /// outside the declaring module that escape hatch is refused and a handle-only export never leaks the
    /// representation it withheld.
    //= spec/capabilities/type-system.md#an-abstract-type-s-representation-is-not-observable-across-its-boundary
    //# Stripping an abstract type's name tag to its underlying structural value MUST be rejected outside the declaring module, so that the escape hatch to a nominal type's structure is available only where that type's constructors are, and a handle-only export does not leak the representation it withheld.
    AbstractCtor,
    /// A `Record.extend`/`Record.with` whose FIELD-NAME-INTRODUCTION operand (the `#z` slot) is NOT a
    /// static `#field` label — a BARE identifier `z` (which the reader would otherwise PUN to a static
    /// label, letting an undeclared name silently become a field) or any runtime-value expression. The row
    /// op's field name is a compile-time LABEL, never a runtime value (`type-system.md` §A Record Row Is
    /// Reshaped Only Through An Explicit Operation; `prelude-and-resolution.md` §A Member Key Is A Label,
    /// Not A Value — the name must be written `#z`). DISTINCT from PresentField/AbsentField (those are a
    /// valid `#label` that is present/absent); here the operand is not a valid static label at ALL. Scoped
    /// to the NAME-INTRODUCTION operand of extend/with ONLY — the READ/DROP ops (`.`/pop/without/project)
    /// legitimately take a bare label and stay valid. (Concierge ruling on breaker's Record.extend pun.)
    RecordFieldNameNotLabel,
    /// A value of FUNCTION type is used where equality/order is required — a Map/Set KEY, a Set element,
    /// or a direct `(=)`/`compare` operand. A function/closure has no canonical value identity, so it is
    /// neither equatable nor orderable at ALL (unlike an abstract type, which COULD be compared but must
    /// not be across its boundary — that is `NominalMismatch`/CDZ0202). This is an INTRINSIC
    /// non-comparability, independent of any module boundary: a function is never a valid key/operand even
    /// inside its own module. Distinct code so the message names the real issue ("functions aren't
    /// comparable") rather than CDZ0202's boundary phrasing. (v-inference ruling, concierge-confirmed.)
    NotEquatable,
    /// A computation the compiler PROVES would trap (`ConstTrap`'s outcome) was ELIMINATED because its
    /// value is unobserved — an unprojected tuple/record element, an unreferenced `let` binding, an
    /// argument bound to an unused parameter. NOT a rejection: the build succeeds (the dead computation
    /// need not run — `core-semantics.md` §A Trap Occurs Only Where Its Computation Is Observed). This
    /// is the WARNING severity's code, emitted so a program does not silently discard a computation that
    /// could never have produced a value (almost always a defect). The error-severity companion is
    /// `ConstTrap` (CDZ0304), emitted when the same provable trap IS observed.
    DeadTrap,
    /// A compile-provable trap (a divide-by-zero / overflow / out-of-bounds the compiler discovered by
    /// CONST-FOLDING) sits in a CONDITIONALLY-reached position — an `if` branch or `match` arm guarded by a
    /// RUNTIME condition. Per the operator ruling (cn02), such a trap is NOT a compile error: it demotes to a
    /// runtime trap that fires only when the branch is taken (the program builds + runs). But since the trap
    /// was SYNTHESIZED by the fold — the author did not write an explicit `trap`/panic — a WARNING flags that
    /// the operation could trap at runtime along a reachable path (a likely defect). The conditional-branch
    /// companion of `ConstTrap` (CDZ0304, the UNCONDITIONAL / const-demanded trap that IS an error) and
    /// `DeadTrap` (CDZ0305, the trap in a DROPPED value). Emitted ONLY for a const-fold-origin trap, never for
    /// an explicit user `(trap …)` (which lowers to a plain `Core::Trap`, not a provable-trap poison).
    ReachableTrap,
    /// A binding is DECLARED but never referenced — a `let` binding, a `fn`/`def` parameter, or a
    /// top-level definition (not exported) that nothing uses. A WARNING (not a rejection): an unused
    /// binding is well-formed, just likely a defect (a typo, a leftover, a forgotten use). Suppressed
    /// when the name begins with `_` — the deliberate "intentionally unused" convention (as in Rust),
    /// so `_x`/`_` never warn. The reference check is the same resolution-column read `UsesOf` uses.
    UnusedBinding,
    /// A NON-FINAL form of a sequencing block computes a value that is DISCARDED — a `(do S… tail)` yields
    /// only its last form (`core-semantics.md` §A Sequencing Block Evaluates Its Forms In Order), so every
    /// earlier form is evaluated for its (thrown-away) value. When such a form is PURE (reaches no host
    /// call — nothing observable to sequence for) AND has a concrete NON-Unit type, its value can only have
    /// been meant to be used: in a pure language a pure statement whose result is dropped is almost always
    /// a bug (a call whose result the author forgot to bind, a misplaced expression). A WARNING (not a
    /// rejection): the program is well-formed and runs correctly (the compiler already drops the pure
    /// intermediate — this is exactly the form its DCE elides), just likely a defect. The sequencing-block
    /// analogue of `UnusedBinding`/`DeadTrap` — dead code the build surfaces rather than silently keeping.
    /// A Unit-typed statement (a host-effect-free `unit`) or an effectful one (a host call, kept by the
    /// `Core::Seq` lowering) never warns; a `_ =`-style intentional discard is spelled as a `let` binding.
    DiscardedValue,
    /// A conditional branch is PROVABLY never reached — the flow-sensitive value-facts analysis proves the
    /// guarding condition is a constant at that point (e.g. inside an `if x > 0` truthy branch, a nested
    /// `if x > 0` is always-true, so its else arm is dead). A WARNING (not a rejection): dead code is
    /// well-formed and runs correctly, just noteworthy — the reachability analogue of `DiscardedValue`/
    /// `DeadTrap`/`UnusedBinding` (the 03xx code-quality/dead-code band), surfaced rather than silently
    /// kept. Emitted ONLY when the interval facts PROVE the condition constant (conservative — a false
    /// positive that flagged a REACHABLE branch would mislead, so an unproven/open condition never warns);
    /// the emitting fact-analysis lives in the lower/emit tier (`v-value-facts`), this code + its message
    /// shape are owned here. The message names the proving fact (`<var> ∈ [lo,hi]`) so the claim is
    /// verifiable, not just asserted. Anchored at the dead branch, and MAY carry a heuristic delete fix.
    UnreachableBranch,
    /// An effect operation is reached at a point with NEITHER an enclosing handler for its effect NOR an
    /// enclosing host delegation of it — the merged "no home for a reached effect" check. This single
    /// code subsumes both the reached-but-undelegated host operation and the undischarged intra-program
    /// effect (the retired CDZ0402), because host-binding is an entrypoint routing decision, not a
    /// declaration-time property — an effect reached the entrypoint's top with no home.
    //= spec/capabilities/capabilities-and-effects.md#an-ungranted-effect-is-a-compile-time-error
    //# An operation performed at a point that has neither an enclosing handler for its effect nor an enclosing host delegation of its effect MUST be rejected at compile time, so that an effect is always either discharged by a handler or delegated to the host and never silently ambient, making "no ambient authority" a compile-time property.
    //= spec/capabilities/capabilities-and-effects.md#an-ungranted-effect-is-a-compile-time-error
    //# This single check MUST subsume both the reached-but-undelegated host operation and the undischarged intra-program effect, so that the two are one condition — an effect reached an entrypoint's top with no home — rather than two separate diagnostics keyed on a declaration-time host/intra distinction the effect no longer carries.
    //= spec/capabilities/capabilities-and-effects.md#undeclared-capability-is-a-compile-time-error
    //# A program that reaches an effect operation that no enclosing handler discharges and that its entrypoint does not delegate to the host MUST be rejected at compile time, so that every effect is either handled in-program or explicitly granted to the host and none is silently ambient.
    //= spec/capabilities/capabilities-and-effects.md#an-entrypoint-delegates-the-capabilities-it-grants-to-the-host
    //# An effect an entrypoint reaches but neither handles in-program nor delegates to the host MUST be treated as not granted.
    //= spec/contracts/host-interface-binding.md#ungranted-access-is-rejected-at-compile-time
    //# A program that reaches a host operation its manifest does not enumerate MUST be rejected at compile time.
    //= spec/contracts/host-interface-binding.md#ungranted-access-is-rejected-at-compile-time
    //# The compiler MUST NOT emit a component that would fail to instantiate because it imports an operation absent from its manifest.
    EffectNoHome,
    /// A handler arm names an operation the arm's effect does not declare — a closed-set violation. An
    /// effect's operations are a closed, statically-known set (like a sum's variants), so discharging an
    /// operation that does not exist is ill-formed.
    //= spec/capabilities/capabilities-and-effects.md#a-handler-arm-names-an-operation-its-effect-declares
    //# A handler arm that names an operation the arm's effect does not declare MUST be rejected at compile time, so that a handler discharges only operations that exist and the declaration remains the closed set of an effect's operations.
    HandlerUndeclaredOp,
    /// A handler does NOT bind every operation its effect declares — a non-exhaustive handler. A `handle
    /// E` names ONE effect and its arms ARE that effect's operations; because an effect's operations are a
    /// closed, statically-known set (like a sum's variants), a handle must discharge the WHOLE set — the
    /// effect analogue of match exhaustiveness. A handler missing an operation is ill-formed: it would
    /// leave an operation of the effect it claims to discharge silently without a home. (Discharging a
    /// subset across LAYERS is nested handles, each exhaustive for its own effect.)
    //= spec/capabilities/capabilities-and-effects.md#a-handler-discharges-its-effect
    //# A handler MUST bind every operation its effect declares, so that installing a handler for an effect discharges the whole of that effect's closed operation set — the effect analogue of a match covering every variant of its scrutinee's sum — and no operation of the effect a handler claims to discharge is left without a discharger under that handler.
    //= spec/capabilities/capabilities-and-effects.md#a-handler-discharges-its-effect
    //# A handler that omits an operation its effect declares MUST be rejected at compile time, so that a partially-handled effect is a compile-time error rather than an operation that silently escapes the handler that appears to discharge it, and the rejection SHOULD identify the omitted operations so the gap is mechanically repairable.
    //= spec/capabilities/capabilities-and-effects.md#a-handler-discharges-exactly-one-effect
    //# A handler MUST discharge exactly one effect — every arm of a single handler names an operation of the same declaring effect — so that a handler installs one effect's context and the effect a handler discharges is unambiguous, mirroring that an operation is reached through its declaring effect.
    //= spec/capabilities/capabilities-and-effects.md#a-handler-discharges-exactly-one-effect
    //# Discharging several effects over one sub-computation MUST be expressed by nesting a handler per effect, so that each handler in the nest discharges its own single effect and no handler mixes the operations of two effects, keeping the discharged effect a property of the handler rather than an open collection its arms enumerate.
    HandlerNotExhaustive,
    /// A host delegation names an effect the delegated computation never reaches — latent authority. The
    /// manifest must be exactly the effects that escape, no more and no fewer, so a granted-but-unexercised
    /// capability is rejected rather than carried.
    //= spec/capabilities/capabilities-and-effects.md#host-delegation-is-an-entrypoint-s-prerogative
    //# A delegation that names an effect the delegated computation never reaches MUST be rejected at compile time, so that a manifest carries no latent authority — a granted capability that is never exercised — and the manifest is exactly the effects that escape, no more and no fewer.
    LatentAuthority,
    /// A closure that PERFORMS AN EFFECT is passed across the host boundary (as an export's result, or a
    /// parameter) — a closure escaping its handler context (`capabilities-and-effects.md` §An Effect Is
    /// Discharged Within Its Dynamic Extent). A closure's effects are discharged by the `handle`/`(host …)`
    /// frame that was dynamically open where the closure was BUILT; a host-held closure is invoked LATER,
    /// outside that frame, so the effect would have no home at the call — the dynamic extent the effect
    /// system relies on is broken. Rejected as unsupported rather than declined: it is a positive design
    /// decision (a closure that captures effectful authority cannot be handed to a custodian that will run
    /// it in an unknown context), not a not-yet-built gap. Distinct from `EffectNoHome` (CDZ0401), which is
    /// an effect with NO delegation anywhere; here the effect IS delegated, but the delegation cannot travel
    /// with the escaping closure.
    ClosureEscapesEffect,
    /// An EFFECT / perform is executed in a GUARD position — a `(guard <pattern> <cond>)` match-arm
    /// condition (and, per the operator directive, any position required to be side-effect-free). A guard
    /// decides WHICH arm matches; the pattern engine may evaluate it speculatively, more than once, or not
    /// at all (a guard that fails falls through to the next arm), so an effect performed there has no
    /// well-defined single execution — it would perform zero, one, or several times depending on match
    /// order, a miscompile the fold cannot represent (the performing-guard class, breaker #9). Rejected at
    /// compile time so a guard stays a pure decision: lift the effect to a `let`-binding evaluated once
    /// BEFORE the `match`, then guard on the bound pure value. DISTINCT from `EffectNoHome` (CDZ0401): the
    /// effect may well have a handler — the defect is its POSITION (a guard), not a missing home. The
    /// PERFORM-IN-GUARD detection is v-effects' (perform-detection); this code + its message are the
    /// diagnostic surface. (The exact SCOPE — forbid all effects vs permit provably-non-mutating ones — is
    /// an operator decision the detection predicate encodes; this code names the rejection either way.)
    //= spec/capabilities/capabilities-and-effects.md#a-guard-is-side-effect-free
    //# An effect operation performed in a match-arm guard MUST be rejected at compile time, so that a guard is a pure decision the pattern engine may evaluate speculatively or repeatedly without observable effect.
    //= spec/capabilities/capabilities-and-effects.md#a-guard-is-side-effect-free
    //# An effect performed in that position therefore has no well-defined execution count or order — it would perform zero, one, or several times depending on the match strategy — so it MUST be a compile-time error rather than a computation with an unspecified effect schedule.
    //= spec/capabilities/capabilities-and-effects.md#a-guard-is-side-effect-free
    //# A program that must consult an effect to decide an arm MUST perform that effect once before the `match` — binding its result to a `let` — and guard on the bound pure value, so that the effect has a single well-defined execution and the guard stays a pure decision over its result.
    EffectInGuard,
    /// An ILL-FORMED binary form `(bin …)` — a compile-time well-formedness defect decidable from the
    /// segment list alone (`options/binary-syntax/`): bit-fields whose widths do not close a whole byte
    /// (the whole `bin` must be byte-aligned), a non-final unsized `(bytes …)` segment, or a `bits` width
    /// that is not a compile-time constant. The binary analogue of a non-exhaustive match — a static
    /// structural rejection, not a runtime surprise (a value that does not fit its segment traps at run
    /// time instead, "binary value does not fit segment"). The CDZ02xx types-and-patterns band.
    IllFormedBinary,
    /// A non-final `,@` SPLICE in a QUOTE PATTERN — `` `(f ,@init ,last) `` puts `,@init` (which binds the
    /// remaining elements) before a fixed `,last`, requiring a variable-length gap flanked by a fixed tail
    /// (`metaprogramming.md` §A `,@<name>` … MUST appear only as the final element). Ill-formed: a rest
    /// binds the whole tail, so it is meaningful only LAST. The quote-pattern companion of the binary-form
    /// `IllFormedBinary` (an unsized `(bytes …)` segment is legal only last), one code down the CDZ02xx
    /// types-and-patterns band.
    NonFinalSplice,
    /// A DIMENSIONAL mismatch — combining quantities of incompatible dimension: adding, subtracting, or
    /// comparing a length to a time; annotating a quantity at a dimension the expression does not derive.
    /// Units are checked THEN ERASED before the program runs, so a dimensional inconsistency is ALWAYS a
    /// compile-time rejection, never a runtime trap — which is why it opens the CDZ05xx VERIFICATION-LAYER
    /// band, not a numeric-trap code. The dimensional specialization of the annotation conflict:
    /// `TypeMismatch` (CDZ0203) names the general type conflict; this names it when it is dimensional.
    //= spec/capabilities/units-of-measure.md#dimensional-mismatch-is-an-error
    //# Combining quantities of incompatible dimension MUST be a compile-time error.
    //= spec/capabilities/units-of-measure.md#dimensional-mismatch-is-an-error
    //# A combination of quantities of incompatible dimension MUST be rejected at compile time with the machine-readable diagnostic for the unsatisfied dimensional constraint, rather than accepted or deferred to runtime.
    DimensionMismatch,
    /// A family UNIT is registered more than once with CONFLICTING conversions — the same unit name
    /// bound to a different reference dimension or a different scale: a named unit resolves to ONE
    /// dimension and ONE scale, so its name→conversion is a well-defined function. A redeclaration that
    /// AGREES is admissible; a disagreement is this rejection. In the CDZ05xx verification-layer band with
    /// the dimensional mismatch. (The built-in family table can't hit this — a conflict there is a
    /// compiler bug that panics at construction; this codes the future USER family-declaration conflict.)
    //= spec/capabilities/units-of-measure.md#a-named-unit-s-conversion-is-unique
    //# A named unit MUST resolve to a single dimension and a single scale, so that its conversion to and from its dimension's reference is a well-defined function rather than dependent on which of several declarations is consulted.
    //= spec/capabilities/units-of-measure.md#a-named-unit-s-conversion-is-unique
    //# Declaring a named unit more than once with conflicting conversions — a differing dimension or a differing scale — MUST be a compile-time error, while a redeclaration that agrees is admissible.
    UnitConflict,
    /// A module DIRECTIVE `(pragma <key> …)` names a key NOT in the fixed registry the specification
    /// defines: a directive's meaning must be fixed across generations, so an unknown key is rejected at
    /// compile time rather than ignored (a dropped meaning-changing directive would make one source mean
    /// two things on two toolchains). Opens the CDZ06xx MODULE-DIRECTIVE band.
    //= spec/capabilities/modules-and-namespaces.md#an-unrecognized-module-directive-is-rejected
    //# A module directive whose key is not one the fixed set defines MUST be rejected at compile time with a machine-readable diagnostic, rather than ignored, so that a directive can neither silently change a program's meaning on a toolchain that understands it while being dropped by one that does not, nor silently fail to take effect.
    //= spec/capabilities/modules-and-namespaces.md#a-module-directive-is-drawn-from-a-fixed-set
    //# A module MAY carry directives that instruct the compiler how to compile it, and every such directive's key MUST be drawn from a set fixed by this specification rather than invented per program, so that a directive has one fixed meaning across generations.
    UnknownDirective,
    /// A recognized module directive whose ARGUMENTS do not match the shape its key defines: the key is
    /// in the registry but the directive is structurally malformed (wrong arity), e.g. `(pragma
    /// default-integer)` omitting its one required type argument. Distinct from `UnknownDirective`
    /// (CDZ0601, an unknown KEY) and from a numeric-domain failure (CDZ0303, a well-formed directive
    /// whose type argument fails the integer-domain predicate).
    //= spec/capabilities/modules-and-namespaces.md#a-module-directive-is-drawn-from-a-fixed-set
    //# A module directive's arguments MUST match the shape the directive's key defines, and a directive whose arguments do not MUST be rejected with a machine-readable diagnostic.
    MalformedDirective,
    /// A member access naming a prelude collection operation that was RENAMED in the consistent-naming
    /// cutover — `Map.size` (now `Map.len`), `Tuple.cat` (now `Tuple.concat`), `Tuple.pop` (now
    /// `Tuple.remove`). The retired name genuinely no longer resolves (there is no transitional alias —
    /// one place a name resolves, per `no-keys-outside-the-prelude`); this code just supplies a BETTER
    /// message than the generic "no member" — it names the new spelling and carries a VERIFIED fix-it
    /// rewriting the key token to the canonical name. A plain typo (a name that was never a member) still
    /// gets the ordinary CDZ0201 unknown-member did-you-mean, so this fires ONLY on the fixed retired set.
    /// In the CDZ060x DIRECTIVE/SURFACE band alongside `UnknownDirective`/`MalformedDirective`.
    RenamedOp,
    /// A ROBUSTNESS decline: a well-formed program the compiler cannot reduce to a component because it
    /// hits a recursion/resource BOUND — an unproductive compile-time recursion (a nullary self-call
    /// `(def (f) (f))` with no base case: following it re-enters the same body without end, and a nullary
    /// def has no runtime-function form to specialize), or a pathological expression depth. The compiler
    /// MUST stop at the bound and DECLINE, never abort with a native stack overflow
    /// (`self-hosting-and-bootstrap.md` §An Unsupported Construct Is Declined, Not Miscompiled). The
    /// terminal CDZ09xx band — a "declined, not crashed" outcome, distinct from a type/well-formedness
    /// rejection (the program is well-formed; the COMPILER cannot yet derive it) and from a plain codeless
    /// decline (a not-yet-built construct): this names the specific "cannot reduce, must not crash" case
    /// the robustness corpus pins.
    RecursionBound,

    /// An UNSUPPORTED CONSTRUCT: a well-formed program the compiler cannot yet derive a component for
    /// because it uses a construct this generation does not build (the "not-yet-built construct" the
    /// `decline` doc names). Historically these were CODELESS declines; operator seq-286 requires every
    /// user-facing decline to carry a code, so this is the ONE umbrella code for that whole class
    /// (v-corpus-harness / C1 registry ruling: one code, granularity in the message — a clean drive-to-0
    /// metric as constructs get built). In the CDZ09xx "declined, not crashed" band with `RecursionBound`
    /// (CDZ0999), but a DISTINCT reason: unimplemented, not a resource wall. Emitted via
    /// `Reject::unsupported`; it is still a DECLINE (a safe reject, not a "the program is wrong"
    /// rejection — see `is_decline`), it just now carries an identity. Does NOT cover PERMANENT design
    /// rejections (those keep their own coded semantics) nor the recursion/resource bound (CDZ0999).
    UnsupportedConstruct,
}

impl Code {
    /// The stable `CDZ####` string. These are the identities a tool and the corpus branch on, so
    /// they change only by the coordinated act a code taxonomy change is. This table IS the pinned
    /// code set: each variant maps to one `CDZ####` a build emits for that rejection, and the string is
    /// a function of the variant alone — independent of any diagnostic's human wording, so re-wording a
    /// message never moves a code:
    ///
    //= spec/capabilities/diagnostics.md#the-code-set-is-pinned-outside-the-specification
    //# The set of diagnostic codes and the rejection each code names MUST be pinned at the declared-default location so that two builds emit the same code for the same rejection.
    ///
    //= spec/capabilities/diagnostics.md#every-diagnostic-has-a-stable-code
    //# The code a diagnostic carries MUST NOT change when the diagnostic's human-readable wording changes.
    pub fn code(self) -> &'static str {
        match self {
            Code::BadEscape => "CDZ0001",
            Code::BadChar => "CDZ0002",
            Code::UnquoteOutsideQuasiquote => "CDZ0003",
            Code::Unbound => "CDZ0101",
            Code::NonLinearBinder => "CDZ0102",
            Code::Malformed => "CDZ0201",
            Code::NominalMismatch => "CDZ0202",
            Code::TypeMismatch => "CDZ0203",
            Code::NumericMismatch => "CDZ0301",
            Code::IntOutOfRange => "CDZ0302",
            Code::NonIntegerDefault => "CDZ0303",
            Code::ConstTrap => "CDZ0304",
            Code::DeadTrap => "CDZ0305",
            Code::ReachableTrap => "CDZ0309",
            Code::UnusedBinding => "CDZ0306",
            Code::DiscardedValue => "CDZ0307",
            Code::UnreachableBranch => "CDZ0308",
            Code::NonExhaustive => "CDZ0210",
            Code::PresentField => "CDZ0211",
            Code::AbsentField => "CDZ0212",
            Code::RedundantArm => "CDZ0213",
            Code::AbstractCtor => "CDZ0214",
            Code::RecordFieldNameNotLabel => "CDZ0215",
            Code::NotEquatable => "CDZ0216",
            Code::TryNoBoundary => "CDZ0230",
            Code::EffectNoHome => "CDZ0401",
            Code::HandlerUndeclaredOp => "CDZ0403",
            Code::HandlerNotExhaustive => "CDZ0405",
            Code::LatentAuthority => "CDZ0404",
            Code::ClosureEscapesEffect => "CDZ0406",
            Code::EffectInGuard => "CDZ0407",
            Code::IllFormedBinary => "CDZ0220",
            Code::NonFinalSplice => "CDZ0221",
            Code::DimensionMismatch => "CDZ0501",
            Code::UnitConflict => "CDZ0502",
            Code::UnknownDirective => "CDZ0601",
            Code::MalformedDirective => "CDZ0602",
            Code::RenamedOp => "CDZ0603",
            Code::RecursionBound => "CDZ0999",
            Code::UnsupportedConstruct => "CDZ0900",
        }
    }
}

/// The canonical [`Code::UnreachableBranch`] (CDZ0308) message — the one wording the value-facts emitter
/// (`v-value-facts`, lower/emit tier) uses so the phrasing stays a single owned shape rather than drifting
/// per emit site. `cond` is the branch's guarding condition as written (e.g. `` `x > 0` `` — pass it
/// pre-quoted or bare, this wraps nothing), `always_true` picks the constant the facts proved, and `fact`
/// names the proving interval (e.g. `x ∈ [1, 127]`) so the claim is VERIFIABLE, not just asserted — the
/// property that makes a dead-code warning trustworthy (a reader can check the interval against the code).
///
/// Shape: `` this branch is never reached — `<cond>` is always <true|false> here (<fact>) ``. Kept as a
/// helper (not inlined at the emit site) so this ONE test-pinned string is the whole surface: a re-word is
/// a one-place edit + a test update, and the emitter never hand-rolls a divergent variant. The emitter
/// wraps it in `Diagnostic::warning(Code::UnreachableBranch, msg, dead_branch_node)` — warning severity,
/// anchored at the DEAD branch (not the condition), per the 03xx code-quality/dead-code band convention.
pub fn unreachable_branch_message(cond: &str, always_true: bool, fact: &str) -> String {
    format!(
        "this branch is never reached — `{cond}` is always {} here ({fact})",
        if always_true { "true" } else { "false" }
    )
}

/// Render the proving-interval FACT clause for a CDZ0308 message from the structured facts the value-facts
/// analysis holds — so the emitter (`v-value-facts`, `compile.rs`) passes `(var, lo, hi)` and the interval
/// wording stays a single owned+pinned shape rather than each call site hand-formatting `∈ [..]`. `hi` is
/// `None` for an OPEN upper bound (a `>= lo` fact with no proven ceiling) → `<var> ≥ <lo>`; a closed bound
/// → `<var> ∈ [<lo>, <hi>]`. Feed the result as the `fact` arg of [`unreachable_branch_message`]:
/// `unreachable_branch_message(cond, always, &unreachable_branch_fact("x", 1, Some(127)))`
/// → `` this branch is never reached — `<cond>` is always true here (x ∈ [1, 127]) ``.
pub fn unreachable_branch_fact(var: &str, lo: i64, hi: Option<i64>) -> String {
    match hi {
        Some(hi) => format!("{var} ∈ [{lo}, {hi}]"),
        None => format!("{var} ≥ {lo}"),
    }
}

/// How confident the compiler is that a proposed [`Fix`] is the RIGHT edit — the branch an agent
/// reads before applying it blind (`spec/capabilities/diagnostics.md` §An Unconfirmed Fix Carries An
/// Applicability Marker). This is the rustc `Applicability` distinction, minus `Unspecified`: a fix
/// the compiler recompiled clean is `Verified` (apply it without review); a fix derived by a heuristic
/// — the nearest-name "did you mean", a wrapping suggestion — is `Heuristic` (an agent may apply it but
/// should confirm it matches intent, and a batch tool should default to leaving it for a human).
///
/// The default is `Heuristic`: a producer must positively PROVE a fix correct (by recompiling with it
/// applied) to mark it `Verified`, so an unproven fix never masquerades as machine-applicable.
///
/// The two variants together are the constitution's machine-actionable-diagnostics obligation: a
/// confirmed route is marked `Verified`, and a route the compiler cannot confirm carries the `Heuristic`
/// marker — so an agent distinguishes a guaranteed repair from a suggested one (the diagnostics.md pair
/// below is this same rule, stated per-variant, in the capability spec).
//= constitution.md#xi-diagnostics-are-machine-actionable
//# A route whose application the compiler has confirmed recompiles the program clean and clears the diagnostic MUST be marked verified, and a route the compiler cannot so confirm MUST carry an applicability marker declaring it a heuristic, so that an agent can distinguish a guaranteed repair from a suggested one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applicability {
    /// The compiler confirmed applying this fix recompiles the program clean and clears the diagnostic
    /// — apply it without review. Machine-applicable.
    //= spec/capabilities/diagnostics.md#a-confirmed-fix-is-marked-verified
    //# A fix whose application the compiler has confirmed recompiles the program clean and clears the diagnostic MUST be marked verified.
    Verified,
    /// A best-effort suggestion the compiler could NOT so confirm — a nearest-name replacement, a
    /// wrapping edit. Likely right, but an agent should confirm it matches intent before applying.
    //= spec/capabilities/diagnostics.md#an-unconfirmed-fix-carries-an-applicability-marker
    //# A fix the compiler cannot so confirm MUST carry an applicability marker declaring it a heuristic, so an agent can branch on it.
    Heuristic,
}

/// A proposed repair for a rejection, expressed as a STRUCTURAL edit of the program's AST rather than a
/// textual patch (`spec/capabilities/diagnostics.md` §A Rejection Carries A Structural Fix). This is the
/// rustc "suggestion" — the thing that makes a diagnostic actionable rather than merely descriptive: an
/// agent (or the front-end's fix-it UI) applies the named edit directly instead of re-deriving the
/// repair from prose.
///
/// The edit targets a NODE (`at`, a `StructId` — the same span-free identity a [`Reject`] anchors on),
/// so the consumer that holds the span table maps it to a text region; the compiler never emits a
/// source offset. The `replacement` is the SURFACE spelling of the node to put there (e.g. the
/// suggested name for a "did you mean", the wrapped form for a coercion) — a rendered s-expression the
/// front-end splices over the target node's span, which keeps the fix a tree edit at the point of
/// application while remaining a compact, printable payload here.
///
//= spec/capabilities/diagnostics.md#a-rejection-carries-a-structural-fix
//# A diagnostic that reports a rejection MUST carry a proposed fix expressed as a structural edit of the program's abstract syntax tree, not a textual patch.
///
//= constitution.md#xi-diagnostics-are-machine-actionable
//# Every diagnostic that reports a rejection MUST carry a machine-applicable route to a compliant program, expressed as a structural edit of the program's abstract syntax tree rather than a textual patch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fix {
    /// A one-line human label for the edit (`replace with `foo``, `wrap in `(Some …)``) — what an
    /// editor shows in its lightbulb menu. The machine-actionable part is `edit`, not this.
    pub label: String,
    /// The concrete structural edit. One variant today (replace a node's surface spelling); the enum
    /// leaves room for insert/delete/wrap edits as later checks gain fixes.
    pub edit: Edit,
    /// Whether the compiler proved this fix correct (`Verified`) or offers it as a heuristic.
    pub applicability: Applicability,
}

/// The structural edit a [`Fix`] carries. A tree operation keyed on a node identity — never a textual
/// diff (`spec/capabilities/diagnostics.md` §A Rejection Carries A Structural Fix).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    /// Replace the node `at` with the surface spelling `replacement` — the "did you mean `foo`?"
    /// edit (swap the misspelled reference for the candidate) and, later, a wrap/coerce edit whose
    /// `replacement` embeds the original as a sub-form.
    ReplaceNode {
        at: crate::ast::StructId,
        replacement: String,
    },
    /// Append `arms` (each a rendered `(pattern body)` s-expression) as new children at the END of the
    /// list node `at` — the "add the missing match arms" edit for a non-exhaustive match. `at` is the
    /// `(match …)` form; the consumer splices each arm in before the form's closing paren. An INSERT,
    /// not a replace: the existing arms are untouched, so the edit is additive.
    InsertArms {
        at: crate::ast::StructId,
        arms: Vec<String>,
    },
    /// WRAP the node `at` in an enclosing form — the consumer replaces `at`'s span with `prefix` + the
    /// node's ORIGINAL text + `suffix` (e.g. `prefix = "(Some "`, `suffix = ")"` → `(Some <expr>)`).
    /// The "try wrapping the expression in `Some`" edit: unlike `ReplaceNode` it PRESERVES the original
    /// sub-expression, embedding it in a constructor call, so the consumer needs the original text (which
    /// it has, from the span) rather than a re-rendered copy.
    Wrap {
        at: crate::ast::StructId,
        prefix: String,
        suffix: String,
    },
    /// DELETE the node `at` from its enclosing list — the consumer removes the node's span plus one
    /// adjacent separating space, so the list stays well-formed (`(a b)` → `(b)`, not `( b)`). The
    /// "remove the unused element" edit — a latent-authority effect dropped from a `host` manifest, a
    /// dead item. No payload: the edit is fully described by the target node.
    Delete { at: crate::ast::StructId },
}

impl Edit {
    /// The node this edit TARGETS — the node replaced/wrapped/deleted, or the list appended into. Every
    /// variant carries exactly one `at`; this reads it uniformly (a consumer that dedups or validates fixes
    /// asks "which node does this touch?" without matching each variant).
    pub fn target(&self) -> crate::ast::StructId {
        match self {
            Edit::ReplaceNode { at, .. }
            | Edit::InsertArms { at, .. }
            | Edit::Wrap { at, .. }
            | Edit::Delete { at } => *at,
        }
    }
}

impl Fix {
    /// A heuristic node-replacement fix — the "did you mean `replacement`?" repair. Heuristic because
    /// the nearest-name match is a guess at intent, not a proof; an agent confirms it before applying.
    pub fn replace_heuristic(at: crate::ast::StructId, replacement: impl Into<String>) -> Fix {
        let replacement = replacement.into();
        Fix {
            label: format!("replace with `{replacement}`"),
            edit: Edit::ReplaceNode { at, replacement },
            applicability: Applicability::Heuristic,
        }
    }

    /// A VERIFIED node-replacement fix — one the producer knows is behaviour-preserving and clears the
    /// diagnostic by construction, so an agent applies it WITHOUT review (`spec/capabilities/
    /// diagnostics.md` §A Confirmed Fix Is Marked Verified). `label` states the concrete action (e.g.
    /// "prefix with `_`"). Use ONLY when the edit's correctness follows from a rule, not a guess — the
    /// caller vouches for it (there is no free lunch: an UNPROVEN edit must stay
    /// [`replace_heuristic`]).
    pub fn replace_verified(
        at: crate::ast::StructId,
        replacement: impl Into<String>,
        label: impl Into<String>,
    ) -> Fix {
        Fix {
            label: label.into(),
            edit: Edit::ReplaceNode {
                at,
                replacement: replacement.into(),
            },
            applicability: Applicability::Verified,
        }
    }

    /// A heuristic "add the missing match arms" fix — append `arms` to the `(match …)` form `at`. The
    /// arms COVER the missing variants (so applying it makes the match exhaustive, clearing CDZ0210),
    /// but their BODIES are placeholders the author must fill — hence Heuristic, not Verified: the shape
    /// is right, the intent is not the compiler's to guess.
    pub fn insert_arms_heuristic(at: crate::ast::StructId, arms: Vec<String>) -> Fix {
        Fix {
            label: format!(
                "add the missing match arm{}",
                if arms.len() == 1 { "" } else { "s" }
            ),
            edit: Edit::InsertArms { at, arms },
            applicability: Applicability::Heuristic,
        }
    }

    /// A heuristic "wrap the expression" fix — enclose the node `at` in `prefix` … `suffix` (e.g.
    /// `(Some ` … `)`). Heuristic: the wrap resolves the type mismatch the compiler saw, but whether the
    /// author MEANT to wrap (vs. change the annotation, vs. a different variant) is a guess. `label`
    /// states the action ("wrap in `(Some …)`").
    pub fn wrap_heuristic(
        at: crate::ast::StructId,
        prefix: impl Into<String>,
        suffix: impl Into<String>,
        label: impl Into<String>,
    ) -> Fix {
        Fix {
            label: label.into(),
            edit: Edit::Wrap {
                at,
                prefix: prefix.into(),
                suffix: suffix.into(),
            },
            applicability: Applicability::Heuristic,
        }
    }

    /// A heuristic "remove this element" fix — delete the node `at` from its enclosing list. `label`
    /// states what is removed ("remove the unused effect `log`"). Heuristic: dropping the element
    /// resolves the diagnostic, but whether the author meant to remove it (vs. add a use for it) is a
    /// guess.
    pub fn delete_heuristic(at: crate::ast::StructId, label: impl Into<String>) -> Fix {
        Fix {
            label: label.into(),
            edit: Edit::Delete { at },
            applicability: Applicability::Heuristic,
        }
    }
}

/// A produced "no": either a coded rejection or an uncoded decline, each carrying a human message and
/// the AST node it is about. The `code` is `Some` for a rejection/poison (an ill-formed program) and
/// `None` for a decline (a construct the compiler does not yet realize) — the branch a downstream sink
/// must preserve rather than collapse (`reference-compiler.md` §The Kind Of A "No" Is Fixed Where It
/// Is Produced).
///
/// **The origin is a node identity, not a span.** `at` is the `StructId` the fault is about. The
/// compiler holds NO source positions — a `StructId` is a stable node identity, and the FRONT-END
/// (which parsed the text into the binary AST and holds the span table keyed by that same identity)
/// maps the id back to a text region (`query-engine.md` §Provenance Is Recovered By Back-Reference).
/// So the whole compiler — and its eventual Cadenza port — stays span-free: a diagnostic names a node,
/// the application layer resolves it to a region. Program node ids are preserved through decode (the
/// prelude appends its nodes AFTER the program's), so an `at` on a program node IS the id the
/// front-end's span table is keyed by. `None` = an un-anchored "no" (a synthesized node, or a
/// producer that did not stamp one).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reject {
    /// `Some(code)` = a rejection (ill-formed); `None` = a decline (not yet built).
    pub code: Option<Code>,
    /// The stable catalog id for this decline's REASON, once migrated (`declined(id, …)`). `None` for a
    /// bare `decline()`/`unsupported()` not yet folded into the `DeclineId` catalog (migration in
    /// progress) and for coded rejections (which are tracked by `Code`, not the decline catalog).
    pub id: Option<DeclineId>,
    pub message: String,
    /// The AST node this "no" is about — the front-end maps it to a text region. `None` if unstamped.
    pub at: Option<crate::ast::StructId>,
    /// A proposed structural repair, if the producer knows one — the "route to a fix"
    /// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). `None` when the
    /// compiler has no actionable suggestion (most declines, and rejections whose repair is not
    /// mechanical). Carried alongside the message so a consumer applies the edit rather than parsing
    /// prose. BOXED so a fix-less `Reject` (the overwhelming majority — and the `Err` of the many
    /// `Result<_, Reject>` the compiler threads) stays pointer-small; the fix's several strings live on
    /// the heap only when one is actually attached.
    pub fix: Option<Box<Fix>>,
}

impl Reject {
    /// A coded rejection: the program is ill-formed, and this is why. Unanchored — a caller with the
    /// faulting node in hand attaches it via [`Reject::at`] (or a collector stamps it with
    /// [`Reject::set_origin_if_absent`]).
    pub fn coded(code: Code, message: impl Into<String>) -> Reject {
        Reject {
            code: Some(code),
            id: None,
            message: message.into(),
            at: None,
            fix: None,
        }
    }

    /// An uncoded decline: a construct the compiler does not yet realize. NOT a statement that the
    /// program is wrong — it is the compiler declining to compile it, the safe outcome
    /// (`reference-compiler.md` §Outcomes Are Ordered By Safety). This IS the mechanism by which an
    /// incrementally-grown compiler declines an unsupported construct rather than emitting a component
    /// whose behavior would diverge from the oracle:
    //= spec/capabilities/self-hosting-and-bootstrap.md#an-unsupported-construct-is-declined-not-miscompiled
    //# A generation whose compiler does not yet compile a construct a program uses MUST decline to derive a component for that program rather than emit a component whose observable behavior diverges from the oracle.
    pub fn decline(message: impl Into<String>) -> Reject {
        Reject {
            code: None,
            id: None,
            message: message.into(),
            at: None,
            fix: None,
        }
    }

    /// A CODED decline — the same "the compiler does not yet realize this construct" outcome as
    /// [`decline`], but carrying the umbrella [`Code::UnsupportedConstruct`] (`CDZ0900`) that operator
    /// seq-286 requires on every user-facing decline. Semantically STILL a decline ([`is_decline`]
    /// returns true for it), so the safety-ordering / dedup logic that branches on `is_decline` is
    /// unaffected; the code just gives the "not-yet-built construct" class a stable identity a tool and
    /// the corpus can pin (`(declines CDZ0900 …)`). Use this for a construct the compiler does not yet
    /// build; keep [`decline`] only where a code is not yet assigned, and use [`coded`] for a rejection
    /// that says the program is WRONG.
    pub fn unsupported(message: impl Into<String>) -> Reject {
        Reject {
            code: Some(Code::UnsupportedConstruct),
            id: None,
            message: message.into(),
            at: None,
            fix: None,
        }
    }

    /// A decline naming its stable catalog [`DeclineId`] — the id-carrying form the unsupported-tracker
    /// migration switches sites to (`declined(id, msg)` replacing `decline(msg)`/`unsupported(msg)`). The
    /// umbrella `code` comes from `id.code()` (so a still-codeless reason stays codeless, and a coded one
    /// carries `CDZ0900`), while `message` keeps carrying the runtime SPECIFICS (the offending type,
    /// arity, name). Semantically a decline: [`is_decline`] holds whenever `id.code()` is `None` or
    /// `CDZ0900` (every seeded reason), so safety-ordering / dedup logic is unaffected.
    pub fn declined(id: DeclineId, message: impl Into<String>) -> Reject {
        Reject {
            code: id.code(),
            id: Some(id),
            message: message.into(),
            at: None,
            fix: None,
        }
    }

    /// Attach a proposed structural fix — the fluent form a producer uses when, alongside the "no", it
    /// can name the repair: `Reject::coded(..).at(id).with_fix(Fix::replace_heuristic(id, "foo"))`.
    pub fn with_fix(mut self, fix: Fix) -> Reject {
        self.fix = Some(Box::new(fix));
        self
    }

    /// Attach (or replace) the node this "no" is about — the fluent form a producer uses when it holds
    /// the precise faulting node: `Reject::coded(..).at(id)`.
    pub fn at(mut self, id: crate::ast::StructId) -> Reject {
        self.at = Some(id);
        self
    }

    /// Stamp the origin node ONLY if none is set yet — so a precise producer's anchor is never
    /// overwritten, while a collector walking node `id` can supply a default origin for a "no" that
    /// was produced without one. The innermost frame that has the faulting node wins.
    pub fn set_origin_if_absent(&mut self, id: crate::ast::StructId) {
        if self.at.is_none() {
            self.at = Some(id);
        }
    }

    /// Whether this "no" is a DECLINE (a safe "the compiler does not yet build this construct" outcome)
    /// rather than a coded rejection that says the PROGRAM is wrong. A decline is either a codeless
    /// [`decline`] or the umbrella-coded [`unsupported`] (`CDZ0900`) — both are the same "not-yet-built"
    /// class (operator seq-286 gave the class a code without changing what it MEANS), so the safety-
    /// ordering / dedup logic that branches on `is_decline` treats them identically. A `CDZ0999`
    /// recursion/resource bound and any permanent design rejection are NOT declines here.
    pub fn is_decline(&self) -> bool {
        self.code.is_none() || self.code == Some(Code::UnsupportedConstruct)
    }
}

/// The message the emit path (`lower`) attaches to the UNCODED decline it returns when it reaches an
/// effect operation performed with no enclosing handler at STANDALONE lowering — the lowering-side
/// consequence of the very condition the entrypoint `check_no_home` reports authoritatively as CDZ0401.
/// Shared as a const so `compile::dedup_faults` can recognize (and drop) this decline whenever a CDZ0401
/// is also present for the same ungranted effect, keeping ONE primary "no" for one root cause rather
/// than an `error:` decline shadowing the coded rejection (`reference-compiler.md` §Outcomes Are Ordered
/// By Safety: a coded rejection is the stronger, more actionable report).
pub const NO_HOME_STANDALONE_DECLINE: &str = "this effect operation is performed with no enclosing handler here; its home is \
     determined by the handler or delegation enclosing its callers";

/// The message the emit path (`lower`) attaches to the UNCODED decline it returns when a head has no
/// `(meta apply)` — a value applied as if it were a function. When the head is a DEFINITE non-function
/// (`(5 3)`, `(true 1)`), `infer` reports the authoritative CDZ0201 `NOT_A_FUNCTION_PREFIX` reject at
/// the same node; this decline then shadows it as a second `error:`. Shared as a const so
/// `compile::dedup_faults` drops it whenever that reject is present — ONE primary "no" for one root
/// cause. (A head whose non-function-ness `infer` can't prove — e.g. an unresolved type — keeps this
/// honest decline: there is no coded reject to defer to.)
pub const NOT_APPLYABLE_DECLINE: &str = "value is not applyable";

/// The stable PREFIX of the coded CDZ0201 "applying a non-function" reject (`cannot apply a value of
/// type <T> — it is not a function`). `dedup_faults` matches this prefix to recognize the reject that
/// makes the [`NOT_APPLYABLE_DECLINE`] redundant, without pinning the whole (type-name-bearing) text.
pub const NOT_A_FUNCTION_PREFIX: &str = "cannot apply a value of type";

/// The message the evaluator (`eval::apply_lambda`) returns, and `lower` turns into an UNCODED decline,
/// when a function is applied to MORE arguments than its arity. When the over-application is provable,
/// `infer` reports the authoritative CDZ0203 `OVER_APPLICATION_PREFIX` reject at the same node; this
/// decline then shadows it as a second `error:`. Shared as a const so `compile::dedup_faults` drops it
/// whenever that reject is present — one primary "no" for one over-application.
pub const OVER_APPLICATION_DECLINE: &str = "applied more arguments than the function accepts";

/// A stable SUBSTRING unique to the coded CDZ0203 over-application reject (`applied N arguments to a
/// function of arity M — …`) — chosen NOT to match the [`OVER_APPLICATION_DECLINE`] (which also begins
/// "applied "). `dedup_faults` matches this to recognize the reject that makes the decline redundant,
/// without pinning the count-bearing text.
pub const OVER_APPLICATION_MARKER: &str = "arguments to a function of arity";

/// A stable SUBSTRING of the NAMED-member-op over-application message (`` `List.push` takes N argument(s),
/// but M were given ``) — the member-op variant of [`OVER_APPLICATION_MARKER`]. `dedup_faults` matches
/// BOTH so a member-op over-application (`(Map.size m x)`) also drops the redundant emit-path wrong-arity
/// decline, reporting ONE primary error carrying the delete fix. Uses "were given" (an over-application
/// always supplies ≥2 arguments, so the plural "were" is invariant regardless of the arity's plural).
pub const MEMBER_OVER_APPLICATION_MARKER: &str = "were given";

/// A stable SUBSTRING unique to the BUILT-IN-OPERATION wrong-arity decline (`<op> is applied at the wrong
/// arity — a built-in operation must be applied to exactly its arguments …`, in `lower`). Fires on BOTH
/// an under-application (no coded sibling — the decline is the primary "no") and an OVER-application
/// (where `infer`'s coded CDZ0203 over-application reject is primary — then this decline is redundant).
/// `dedup_faults` matches this to drop the decline ONLY when that coded over-application reject is present.
pub const BUILTIN_WRONG_ARITY_DECLINE: &str = "a built-in operation must be applied to exactly";

/// A stable SUBSTRING of the emit-path (`lower`) CONVERSION/arith arity reject `<op> takes exactly N
/// operand(s)` — the coded CDZ0201 a conversion op (`Int64.of`, `Float64.of`, `of-int`, …) or a binary op
/// returns when applied to the wrong operand count. On an OVER-application `infer` already reports the
/// coded CDZ0203 "`Int64.of` takes 1 argument, but 2 were given" (naming the op + carrying a
/// delete-surplus fix) — MORE actionable than the bare "of takes exactly 1 operand". So `dedup_faults`
/// drops this emit-path arity reject when the over-application CDZ0203 is present (both anchor near the
/// same call), keeping the coded, fixable one as the ONE primary. The resolve-path arity rejects for the
/// grammar forms (`if`/`and`/`not`) share the "takes exactly … operand" wording but are the PRIMARY for
/// those forms (no over-application CDZ0203 accompanies them), so the `has_over_application_reject` gate
/// leaves them untouched.
pub const EMIT_OPERAND_ARITY_MARKER: &str = "takes exactly";

/// The stable PREFIX of the UNCODED decline `lower`'s `Resolved::Try` arm returns when a `?`'s operand
/// core is not a constant `SumNew` — `the ?/try operator lowers only a constant operand …`. This fires
/// on TWO shapes: a genuinely-RUNTIME fallible operand (the honest BRICK-3b decline — the primary "no",
/// kept) AND an ILL-TYPED operand (`(try 3.14)`, `(try "hi")`) whose non-sum constant core also misses the
/// `SumNew` arm. In the ill-typed case `infer` already reports the authoritative CDZ0203
/// [`TRY_NON_FALLIBLE_PREFIX`] naming the real defect (the operand's type), so this decline is the same
/// fault reported more weakly (and MISLEADINGLY — it blames "constant operand" when the operand IS constant,
/// its problem being the TYPE). Shared as a const so `compile::dedup_faults` drops it whenever that CDZ0203
/// is present — ONE primary `error:` per ill-typed `?` (`reference-compiler.md` §Outcomes Are Ordered By
/// Safety), while a runtime operand with no such reject keeps its honest decline.
pub const TRY_RUNTIME_OPERAND_DECLINE_PREFIX: &str =
    "the `?`/`try` operator lowers only a constant operand";

/// The stable PREFIX of the coded CDZ0203 a `?` on a non-fallible operand reject (`` `?` operand must be a
/// fallible `Result`/`Option`, found <T>``). `dedup_faults` matches this to recognize the reject that makes
/// the [`TRY_RUNTIME_OPERAND_DECLINE_PREFIX`] redundant, without pinning the type-name-bearing tail.
pub const TRY_NON_FALLIBLE_PREFIX: &str = "`?` operand must be a fallible `Result`/`Option`, found";

/// A `(bind …)` directive whose shape is not `(bind <Effect> "cadenza:pkg/iface")` — a missing/non-string
/// interface, or the wrong arity. Reported at the form so a malformed peer-binding directive is named, not
/// silently dropped (the peer-binding analogue of the malformed-export reject).
pub const MALFORMED_BIND_MESSAGE: &str = "a `(bind …)` binds an EFFECT to a peer interface string — write \
     `(bind Effect \"cadenza:pkg/iface\")` (the effect is a declared effect's name, the interface a string literal)";

/// A `(bind Name …)` whose `Name` is not a declared effect. Reported at the name so binding a non-effect
/// (a def, a type, an unbound name) to a peer is named rather than silently ignored.
pub const BIND_NOT_AN_EFFECT_MESSAGE: &str = "a `(bind …)` names a declared EFFECT — this name is not an \
     effect, so there is nothing to route to a peer";

/// A `(bind E "…")` (or `--component-name`) interface string that is not a valid component-model
/// interface name. Without this reject the string is emitted verbatim as a component import/export
/// extern name, so a non-conforming one (`"Math/API"`) produces a component `wasmtime` rejects at LOAD
/// with NO compiler diagnostic — a silent invalid-component miscompile. Reported at the string, with the
/// required shape spelled out so the fix is mechanical.
pub const MALFORMED_INTERFACE_NAME_MESSAGE: &str = "a peer interface name must be \
     `namespace:package/interface` in kebab-case (lowercase package, e.g. `cadenza:math/api`, with an \
     optional `@version`) — this string is not a valid component interface name, so the emitted \
     component would fail to load";

/// A peer-bound effect (`(bind E "iface")`) whose operation signature involves a CLOSURE — a function
/// type `(-> …)` in an argument or result position. Peers exchange VALUE-HEAP HANDLES (a tuple/record/
/// sum/list/map/string/…), and a closure is not a value-heap value, so it has no peer-boundary form.
/// (A closure crosses the HOST boundary as a component-model RESOURCE — `closures-across-host` — but the
/// peer/shared-runtime path does not carry one.) Without this check the op type-checks, then APPLYING a
/// peer-returned closure declines at lower time with the opaque "value is not applyable"; reject it at
/// the binding with the real reason. Reported at the `(bind …)` name.
pub const CLOSURE_ACROSS_PEER_MESSAGE: &str = "a peer-bound effect operation cannot take or return a \
     CLOSURE — peers exchange value-heap handles (a tuple/record/sum/list/map/string/…), and a closure \
     has no peer-boundary form (a closure crosses the HOST boundary as a resource, not a peer); give the \
     operation a value type, or handle the effect in-program instead of binding it to a peer";

// (STRING_ARG_ACROSS_PEER_MESSAGE was removed once a peer String/Bytes ARGUMENT became emittable — it
// crosses as a runtime rope HANDLE like any compound, so the decline is gone. See the peer-aware
// `collect_used_ops`/`collect_host_arg_strings` and the `a_string_argument_crosses_to_a_peer_*` tests.)
/// A stable SUBSTRING unique to the coded CDZ0201 resume-value/result-type mismatch (`a handler resumes
/// with a value of type X but the operation's result type is Y`). An ill-typed resume ALSO makes the
/// handler unfoldable, so `lower` emits the uncoded [`HANDLER_NOT_REDUCIBLE_DECLINE`] alongside — a
/// CONSEQUENCE, not an independent limit. `dedup_faults` matches this to drop that decline, like it does
/// for a malformed handler (CDZ0403/0405), so a mistyped resume is ONE primary error (carrying its
/// coercion fix when applicable).
pub const RESUME_RESULT_MISMATCH_MARKER: &str = "a handler resumes with a value of type";

/// A stable SUBSTRING unique to the coded CDZ0201 handler-arm parameter-arity mismatch (`handler arm for
/// operation X binds N parameters but the operation declares M`). An arm that binds the wrong number of
/// parameter binders ALSO makes the handler unfoldable, so `lower` emits the uncoded
/// [`HANDLER_NOT_REDUCIBLE_DECLINE`] alongside — a CONSEQUENCE of the arity defect, not an independent
/// limit. `dedup_faults` matches this to drop that decline, exactly as it does for a malformed handler
/// (CDZ0403/0405) or a mistyped resume, so a wrong-arity arm is ONE primary error naming the real defect.
pub const HANDLER_ARM_ARITY_MARKER: &str = "an arm binds exactly its operation's parameters";

/// The stable PREFIX of the coded CDZ0201 "this handle is not in canonical form" reject — a source
/// `handle` still headed `handle` after `effects::desugar_handles` (the retired effect-name-less shape,
/// or a too-short handle). Shared as a const so `compile::dedup_faults` can recognize it and drop the
/// CONSEQUENT CDZ0401 (`EffectNoHome`) the rejected handle's un-discharged perform triggers — the perform
/// has no home ONLY because its handler was rejected, so one root cause yields ONE primary `error:` (the
/// CDZ0201, which says how to fix the handle), not a coded reject shadowed by a "you have no handler" that
/// misdirects (the author DID write a handler). Matched as a prefix so the shape-carrying tail can vary.
pub const HANDLE_NONCANONICAL_PREFIX: &str = "this handle is not in canonical form";

/// A stable PREFIX of the coded CDZ0201 "a handle's head must name an EFFECT" reject — a `(handle foo …)`
/// whose head names a VALUE definition, not an effect. The head is desugared into each arm's `(. foo op)`
/// projection, so a value head otherwise surfaces ONLY as a leaky cascade — a CDZ0201 "member access
/// requires a record, found <T>" (from `(. foo op)` where `foo` is that scalar) plus the uncoded
/// "not yet reducible by the tail-resumptive fold" decline — neither naming the real problem. `dedup_faults`
/// drops both cascade faults whenever this reject is present, so a value-headed handle is ONE primary
/// `error:` at the head, not shadowed by desugar-artifact diagnostics the author cannot act on.
pub const HANDLE_VALUE_HEAD_PREFIX: &str = "a handle's head must name an EFFECT";

/// A stable PREFIX of the coded CDZ0201 "an operation's type must be an arrow" reject — a
/// `(op get Int64)` whose type is a well-formed NON-arrow. The op-value's `(meta t)` is then wrapped as
/// `(fn () Int64)`, so PERFORMING it types the projected op-VALUE record against the arg, leaking the
/// internal `(Record (apply Any) (effect-op Any) (t Type))` in a consequent CDZ0203 at the perform site.
/// That leak is a CONSEQUENCE of the malformed declaration, not an independent fault — `dedup_faults`
/// drops it (a fault whose message names the internal op-record) whenever this reject is present, so a
/// malformed op type is ONE primary `error:` at the declaration (carrying the wrap fix), not shadowed by
/// a downstream internal-record type error the author cannot act on.
pub const NON_ARROW_OP_TYPE_PREFIX: &str = "an operation's type must be an arrow";

/// The internal op-VALUE record signature that leaks into a perform-site type-mismatch message when an
/// operation was declared with a non-arrow type (see [`NON_ARROW_OP_TYPE_PREFIX`]). No legitimate user
/// type renders these synthesized meta-channel field names, so `dedup_faults` uses this substring to
/// recognize (and drop) the consequent leak when the primary malformed-op-type reject is present.
pub const OP_VALUE_RECORD_LEAK: &str = "(effect-op Any)";

/// A stable PREFIX of the coded CDZ0201 "this operation has no type" reject — an operation declared with
/// NO type at all (`(op get)`, `op.ty == None`). Like the non-arrow case, PERFORMING such an op leaks the
/// internal op-record (`OP_VALUE_RECORD_LEAK`) into a consequent, AND the perform reaches the entrypoint
/// with no home (a consequent CDZ0401) — both CONSEQUENCES of the untyped declaration. `dedup_faults`
/// drops that cascade whenever this reject is present, so an untyped op is ONE primary `error:` at the
/// declaration (carrying the add-a-type fix), not shadowed by faults the author cannot act on.
pub const MISSING_OP_TYPE_PREFIX: &str = "this operation has no type";

/// A stable PREFIX shared by every coded CDZ0201 a MALFORMED `host` form produces (`resolve_host`): a
/// missing effect-list/body, a non-list effect slot, or too many operands — every message begins "this
/// host". Because the malformed host never resolved as a delegation, its body's perform is seen by the
/// entrypoint no-home walk as reached with NO delegation → a CONSEQUENT CDZ0401 that misdirects (the
/// author DID write a `host`, it is just malformed). `dedup_faults` drops that CDZ0401 whenever any
/// malformed-host reject is present, keeping the CDZ0201 that says how to fix the host as the ONE primary
/// error — the `host` analogue of the noncanonical-handle CDZ0401 suppression.
pub const MALFORMED_HOST_PREFIX: &str = "this host";

/// The message for a STRAY `resume` — one outside any handler arm's body (a top-level def body, a plain
/// expression). A resume is meaningful only inside a handler arm, so its PLACEMENT is the root defect
/// regardless of its ARITY: a stray `(resume 5)` is BOTH malformed (missing next-state) AND misplaced,
/// and the resolve-path arity poison + this placement reject BOTH anchor the same `resume` node. The
/// same-node fault dedup keeps only ONE anchored fault per (code, node), so `dedup_faults` DROPS the
/// arity poison whenever this placement reject is present at that node — a misplaced resume then reports
/// the fundamental "not in a handler" cause, not the misleading "missing next-state" (which reads as if
/// adding an argument would fix it, when the resume simply does not belong here). Shared as a const so
/// the stray-loop producer and the dedup suppressor cannot drift.
pub const STRAY_RESUME_MESSAGE: &str = "a `resume` is only meaningful inside a handler arm's body — this one has no \
     enclosing handler arm to resume into";

/// The UNCODED decline the emit path (`lower`) returns for a `<`/`=`/… comparison whose operand is a
/// COMPOUND value it cannot fold to a scalar and cannot heap-walk yet. When the two operands are a
/// genuine TYPE MISMATCH (`(< 1 "x")` — Int64 vs String, one side a compound/text), `infer` already
/// reports the coded CDZ0201/CDZ0203 "… are different types" naming the kind boundary; this decline then
/// rides alongside as a misleading second error (it reads as an unbuilt feature, but the real defect is
/// the mismatch). `dedup_faults` drops it whenever a comparison type-mismatch reject (recognized by this
/// substring) is present, keeping the coded reject as the ONE primary. A WELL-TYPED compound comparison
/// that genuinely needs the not-yet-built heap walk (`(< (tuple 1 2) (tuple 3 4))`) keeps its honest
/// decline — no "different types" reject accompanies it.
pub const COMPOUND_COMPARISON_DECLINE: &str = "comparison of a compound value needs a heap walk";

/// The stable SUBSTRING of the ORDERING (`<`/`<=`/`>`/`>=`) carve-out decline: a compound whose leaf is a
/// float / set / map has NO total order (a float offers only the IEEE partial order; set/map carry no
/// blessed order), so it cannot be ordered — a PERMANENT carve-out, NOT the "not yet built" heap walk that
/// [`COMPOUND_COMPARISON_DECLINE`] names for a genuinely-unbuilt equality. `dedup_faults` recognizes THIS
/// message too (alongside [`COMPOUND_COMPARISON_DECLINE`]) so a mismatched-type ORDERING comparison still
/// drops the consequent decline for the coded "different types" primary.
pub const COMPOUND_ORDERING_NO_TOTAL_ORDER_DECLINE: &str =
    "has no total order, so it cannot be ordered";

/// The stable SUBSTRING shared by every coded cross-kind / mismatched-operand comparison reject `infer`
/// produces — `<a> and <b> are different types …` (a text-vs-scalar / compound-vs-atom kind boundary, a
/// Bool-vs-other-scalar pair, or a map-vs-record pair). `dedup_faults` uses it to recognize such a reject
/// and drop the consequent [`COMPOUND_COMPARISON_DECLINE`].
pub const DIFFERENT_TYPES_COMPARISON_MARKER: &str = "are different types";

/// The message the emit path (`lower`) attaches to the CDZ0900 unsupported-construct decline it returns
/// when `reduce_handle` cannot fold a `handle` form (seq-286: every decline carries a code; seq-280: the
/// text is a clean capability statement, no "not yet" / "later increment" deferral framing). A MALFORMED
/// handler — one whose arm names an operation its effect does not declare (CDZ0403), or that does not
/// discharge every operation (CDZ0405) — cannot fold, so this decline rides ALONGSIDE the coded reject as
/// a second `error:` for the same root cause (the misspelled / missing arm). Shared as a const so
/// `compile::dedup_faults` drops it (by `message ==` this const, still `is_decline()`) whenever a
/// CDZ0403/CDZ0405 is present on the program — ONE primary, actionable "no" (the coded reject carries the
/// fix), not a rejection shadowed by an emit-path decline (`reference-compiler.md` §Outcomes Are Ordered By
/// Safety). A WELL-FORMED handler that genuinely needs a cross-function / non-tail resume (with NO coded
/// reject) keeps this CDZ0900 — there is nothing stronger to defer to; the "later increment" that would
/// fold it is tracked internally, NOT in the user-facing text.
pub const HANDLER_NOT_REDUCIBLE_DECLINE: &str = "this handler is not reducible by the tail-resumptive fold: it \
     requires a cross-function or non-tail resume, which the effect specializer does not lower";

/// The CDZ0900 message the emit path attaches to a `Resolved::Resume` node lowered STANDALONE (out of its
/// handler-fold context) — `lower/compute.rs`'s `Resolved::Resume` fallthrough. DISTINCT from
/// [`HANDLER_NOT_REDUCIBLE_DECLINE`] (that says "this handler…"; this says "this `resume`…") so the two are
/// separable. Shared as a const so `compile::collect_reached_poisons_at` can recognize + SKIP it: the
/// reached-poison walk must never independently fault a resume node — its real diagnostic is always
/// reported elsewhere (the enclosing handle's fold outcome, or the upstream `STRAY_RESUME` CDZ0201), so a
/// bare/nested resume poison surfacing in the walk is spurious (it would wrongly block a program whose
/// handle folds + emits — the mutual-group `…_folds` case).
pub const RESUME_NOT_REDUCIBLE_DECLINE: &str = "this `resume` is not reducible by the tail-resumptive fold: it \
     is a cross-function or non-tail continuation the effect specializer does not reify";

/// The three UNCODED declines the emit path (`lower::lower_lambda_value`) returns when a closure that must
/// cross the component boundary has a non-representable part — an `Any` (never-fixed) parameter or result,
/// or a captured value with no machine type. When such a closure is an EXPORT'S result, `compile::
/// collect_faults` reports the authoritative coded CDZ0201 ("returns a closure that cannot cross the
/// component boundary … a parameter inference never fixed to a concrete scalar") at the export clause; the
/// emit-path decline then rides alongside it as a second `error:` for the SAME root cause (the
/// unrepresentable closure). Shared as consts so `dedup_faults` drops the decline whenever that CDZ0201 is
/// present — ONE primary, actionable "no". A non-exported closure with an unrepresentable part (no CDZ0201
/// covering it) keeps its honest decline.
/// The UNCODED declines the emit path (`lower`) returns for a value with no runtime form — a bare type
/// name (`Int64`), a nullary lambda, a bare built-in op, a closure whose param type has none. When such a
/// value is an EXPORT'S result — e.g. `(def (main) Int64)` — `compile::collect_faults` reports the
/// authoritative coded CDZ0201 "export `<name>` is a TYPE, not a runtime value" at the export clause; the
/// emit path then declines the same body through SEVERAL of these paths at once (a 4-error cascade for one
/// root cause). Shared as consts so `dedup_faults` drops them whenever that CDZ0201 is present, keeping the
/// ONE coded, actionable reject. A non-export use of such a value (no CDZ0201) keeps its honest decline.
/// The coded CDZ0201 shape reject for a list pattern whose `..` rest slot holds anything other than a bare
/// name or `_` — a nested `(list …)`/`(tuple …)`/ctor sub-pattern OR a literal. SHARED by two paths that
/// must agree byte-for-byte so their same-node reports dedup into ONE diagnostic: (a) resolve's `Case 6mr`
/// (`binder_in`), reached when the arm BODY references an inner binder of the nested rest pattern; and (b)
/// lowering (`lower_match_list` / the binding-position check), reached STRUCTURALLY regardless of the body —
/// which is why lowering uses a CODED reject here (not a reachability-gated decline), so the invalid shape
/// surfaces even when the body ignores the inner binders (the body-dependent gap v-guide flagged). The rule:
/// core-semantics.md:149 grants nested patterns to ELEMENT positions only; :135 a binding position holds an
/// irrefutable pattern (a nested list rest is refutable on the empty tail), so the rest binder is name/`_`.
pub const LIST_REST_BINDER_NAME_ONLY: &str = "the rest binder of a list pattern must be a name or `_` (it binds the whole tail sublist) — \
     a nested pattern or literal is not allowed here; bind the tail to a name and destructure it \
     in a nested `match` (e.g. `(list a .. rest)`, then `(match rest …)`)";

pub const TYPE_VALUE_NO_RUNTIME_DECLINE: &str = "a type value has no runtime form";
pub const NULLARY_LAMBDA_NO_CLOSURE_DECLINE: &str = "a nullary lambda has no runtime closure form";
// seq-280: user-facing text is a clean capability statement — the "not yet built" runtime-closure
// framing (a built-in used as a value would need a synthesized runtime closure the compiler does not
// yet emit) stays here in-comment, NOT in the message. (The compute.rs:1020 emit stays a codeless
// decline for now; v-compiler-primitives flips it to CDZ0900 once v-inference's dedup/test-path fix lands.)
pub const PRIM_AS_VALUE_DECLINE: &str =
    "a built-in operation used as a value needs a runtime closure";

/// A stable SUBSTRING of the coded CDZ0201 type-export reject (`export <name> is a TYPE, not a runtime
/// value …`). `dedup_faults` matches this to recognize the reject that makes the type-value decline family
/// redundant, without pinning the whole (name-bearing) text.
pub const TYPE_EXPORT_MARKER: &str = "is a TYPE, not a runtime value";

/// A stable SUBSTRING of the coded CDZ0201 EFFECT-valued-export reject (`export <name> is an effect, not a
/// runtime value …`). Its twin for effects: exporting a bare effect name evaluates the effect's SYNTHESIZED
/// record, leaking a cascade ("unknown intrinsic", unbound `effect-op`/`effect`, nullary-lambda-no-closure).
/// `dedup_faults` matches this to drop that cascade, keeping the one clean category reject.
pub const EFFECT_EXPORT_MARKER: &str = "is an effect, not a runtime value";

/// The uncoded DECLINE the resolver emits for an effect-op/intrinsic name reached as a runtime value (a
/// bare effect name evaluated). Leaked verbatim when an effect is exported; `dedup_faults` drops it (and
/// the `effect-op`/`effect` unbound-field CDZ0101s) when the effect-export reject is present.
pub const UNKNOWN_INTRINSIC_DECLINE: &str = "unknown intrinsic";

/// The shared PREFIX of the CDZ0201 "record has no field `<key>`" reject. A member access `(. r key)`
/// with an absent field is reported by BOTH the infer-side member check (`infer::no_field_reject`, which
/// adds a did-you-mean fix) AND — when that access is the head of an application `(R.make …)` — the
/// emit-side member fold (`lower`). Both anchor the SAME construct at DIFFERENT nodes (the `.key`
/// projection vs the enclosing apply), so `dedup_faults` sees two `(code, node)` keys for ONE fault.
/// Sharing the prefix lets `dedup_faults` recognize the pair by message and keep only the RICHER infer
/// copy (the one carrying the fix). NOTE the trailing space before the backticked key.
pub const NO_FIELD_PREFIX: &str = "record has no field ";

/// The BARE member-access decline the LOWERING path (`lower.rs`) returns when a projection cannot fold and
/// the operand is not a runtime record — "member access requires a record" with NO ", found <T>" tail.
/// Distinct from `infer`'s richer "member access requires a record, found <T>" (which names the type and
/// is the primary). The bare form is emitted only by lowering, so when a member-access `infer` reject is
/// present (e.g. the tuple-by-position message for `(. t name)`, or a "found <T>" reject), this bare
/// decline is the same defect reached again through the emit path — a consequent `dedup_faults` drops.
pub const MEMBER_NOT_RECORD_DECLINE: &str = "member access requires a record";

/// A stable SUBSTRING of `infer`'s tuple-accessed-by-name reject (`(. t name)` on a `(Tuple …)`): "a tuple
/// is accessed by position, not by name". The precise, actionable primary; used by `dedup_faults` to
/// recognize the same-defect [`MEMBER_NOT_RECORD_DECLINE`] the emit path leaks at a CALL SITE (the reduced
/// body lowers `(. (tuple …) name)`, which cannot fold) and drop that weaker consequent.
pub const TUPLE_BY_NAME_MARKER: &str = "a tuple is accessed by position, not by name";

pub const CLOSURE_PARAM_NO_REPR_DECLINE: &str =
    "a closure's parameter type has no machine representation";
pub const CLOSURE_RESULT_NO_REPR_DECLINE: &str =
    "a closure's result type has no machine representation";
pub const CLOSURE_CAPTURE_NO_REPR_DECLINE: &str =
    "a closure captures a value with no machine representation";

/// A stable SUBSTRING of the coded CDZ0201 closure-boundary reject (`export <name> returns a closure that
/// cannot cross the component boundary …`). `dedup_faults` matches this to recognize the reject that makes
/// the [`CLOSURE_PARAM_NO_REPR_DECLINE`] family redundant, without pinning the whole (name/type-bearing) text.
pub const CLOSURE_BOUNDARY_MARKER: &str = "cannot cross the component boundary";

/// The shared "did you mean?" machinery — the ONE nearest-name search every suggestion draws on
/// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). A producer that
/// rejected an unknown name (an unbound reference, an absent record field, a mistyped variant) hands
/// this its candidate set — the names that WOULD have been valid there — and gets back the nearest
/// plausible typo, or `None` when nothing is close enough (a false suggestion is worse than none: an
/// agent would apply the wrong edit). Kept in `diag` so resolve/infer/… share one implementation and
/// one cutoff rather than each rolling its own.
pub mod suggest {
    /// A fresh binder name derived from `base` by appending the lowest integer suffix (starting at 2) that
    /// `taken` does not already contain — `x` → `x2`, or `x3` if `x2` is also taken. The rename fix for a
    /// NON-LINEAR binder (CDZ0102): a duplicated parameter / pattern binder (`(f x x)`, `(tuple a a)`) is
    /// made linear by renaming its second occurrence to a name that collides with nothing in scope
    /// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). `taken` is the names
    /// already bound at that position (the linearity check's `seen` set), so the result is guaranteed
    /// distinct; a deterministic function of `base` + `taken` (lowest free suffix), never order-dependent.
    pub fn fresh_suffixed_name(base: &str, taken: &std::collections::HashSet<String>) -> String {
        let mut n = 2u32;
        loop {
            let candidate = format!("{base}{n}");
            if !taken.contains(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// Pick the closest of `candidates` to `name` under a length-relative edit-distance cutoff, or
    /// `None` if none is close enough. The cutoff (`max(1, len/3)`, rustc's `find_best_match_for_name`
    /// heuristic) keeps a suggestion only when the candidate is a plausible typo: a 3-char name tolerates
    /// 1 edit, a 9-char name up to 3. Ties break on the smaller distance, then a SHARED FIRST CHARACTER
    /// with the query (a typo rarely changes the leading letter, so `Lst` → `List`, not the equidistant
    /// `Ast`), then the lexicographically-smaller name, so the result is a DETERMINISTIC function of the
    /// candidate SET — independent of the order they are supplied in (a hash-map iteration order never
    /// leaks through).
    /// The candidate set is itself a function of the source (the names in scope), so the whole
    /// suggestion — and the fix built from it, with its fixed verified/heuristic status — is a
    /// deterministic function of the source:
    ///
    //= spec/capabilities/diagnostics.md#a-fix-is-a-deterministic-function-of-the-source
    //# A proposed fix and its verified-or-heuristic status MUST be a deterministic function of the source.
    //= constitution.md#xi-diagnostics-are-machine-actionable
    //# The route a diagnostic carries and its verified-or-heuristic status MUST be a deterministic function of the source, like every other compiler output.
    pub fn nearest<I, S>(name: &str, candidates: I) -> Option<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let name_len = name.chars().count();
        // A one-char query has no meaningful typo neighbour: with `max_dist` floored at 1, EVERY one-char
        // candidate is one substitution away, so `z` would "suggest" an unrelated `a` — a confident but
        // baseless "did you mean?". Suppress it for ALL sites (unbound name, record field, import, effect
        // op); the field/import paths otherwise leaked exactly this noise the unbound path already guarded.
        if name_len < 2 {
            return None;
        }
        let max_dist = (name_len / 3).max(1);
        let name_first = name.chars().next();
        // The tie-break KEY for a candidate at edit distance `d`: prefer the smaller distance, then a
        // candidate SHARING the query's first character (a typo rarely changes the leading letter — `Lst`
        // means `List`, not the equidistant `Ast`), then the lexicographically-smaller name. `false` sorts
        // before `true`, so `!first_char_match` puts a first-char match first. A pure function of (query,
        // candidate) → the whole selection stays a deterministic function of the candidate set.
        let key = |d: usize, cand: &str| (d, cand.chars().next() != name_first, cand.to_string());
        let mut best: Option<((usize, bool, String), String)> = None;
        for cand in candidates {
            let cand = cand.as_ref();
            // Never suggest the name itself (a shadowed / out-of-scope exact match is not a typo), the
            // wildcard, nor the EMPTY name. An empty candidate arises from a nameless malformed binder
            // (e.g. `(def)` registers a def with an empty name); "did you mean ``?" is never useful, and
            // its edit distance to any 1-char name is 1 (≤ max_dist), so it would otherwise win.
            if cand == name || cand == "_" || cand.is_empty() {
                continue;
            }
            // LENGTH-DIFFERENCE PREFILTER: the edit distance is at least the difference in char length
            // (each insertion/deletion changes length by exactly one), so a candidate whose length differs
            // from `name` by more than `max_dist` can NEVER be within the cutoff — skip it WITHOUT the
            // O(name·cand) Levenshtein matrix. This is what keeps a "did you mean?" over a large candidate
            // set cheap: `cdz check` on a file with many distinct unbound names ran `edit_distance` against
            // EVERY in-scope name for EACH, O(names²·len); the prefilter rejects almost all pairs in O(len).
            let cand_len = cand.chars().count();
            if name_len.abs_diff(cand_len) > max_dist {
                continue;
            }
            let d = edit_distance(name, cand);
            if d > max_dist {
                continue;
            }
            let k = key(d, cand);
            let better = match &best {
                None => true,
                Some((bk, _)) => k < *bk,
            };
            if better {
                best = Some((k, cand.to_string()));
            }
        }
        best.map(|(_, n)| n)
    }

    /// The `limit` candidates CLOSEST to `name` by edit distance, nearest first — the FALLBACK tier for a
    /// "did you mean?" when [`nearest`] finds no confident typo (the query is too far off the strict
    /// cutoff). Unlike `nearest` this applies NO distance cutoff: it always returns the available
    /// candidates (up to `limit`) so a diagnostic can offer "the closest options" rather than nothing when
    /// a name is wrong but no single candidate is a plausible typo. Sorted by (distance, then
    /// lexicographic name) so the result is a DETERMINISTIC function of the candidate SET, independent of
    /// supply order — the same determinism `nearest` guarantees (diagnostics.md §A Fix Is A Deterministic
    /// Function Of The Source). The name itself, `_`, and the empty name are never offered (same as
    /// `nearest`); duplicates collapse (a candidate appearing twice is listed once). Returns an empty Vec
    /// when there are no usable candidates.
    pub fn closest_matches<I, S>(name: &str, candidates: I, limit: usize) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut scored: Vec<(usize, String)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for cand in candidates {
            let cand = cand.as_ref();
            if cand == name || cand == "_" || cand.is_empty() || !seen.insert(cand.to_string()) {
                continue;
            }
            scored.push((edit_distance(name, cand), cand.to_string()));
        }
        // Nearest first; ties break lexicographically for determinism.
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        scored.truncate(limit);
        scored.into_iter().map(|(_, n)| n).collect()
    }

    /// A "did you mean?" HINT suffix for a wrong `name`, two-tiered: a CONFIDENT single suggestion when a
    /// candidate is a plausible typo (`nearest`, within the edit-distance cutoff — "` — did you mean
    /// \`X\`?`"), else the CLOSEST few candidates without a cutoff ("` — closest matches: \`a\`, \`b\`,
    /// \`c\``) so a diagnostic never shows NOTHING when candidates exist. Empty string only when there are
    /// no usable candidates at all. `limit` bounds the fallback list. A deterministic function of the
    /// candidate set (both tiers are), so the message is reproducible like every compiler output.
    pub fn did_you_mean<I, S>(name: &str, candidates: I, limit: usize) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str> + Clone,
    {
        // Collect once so both tiers see the same set (an iterator is single-pass).
        let cands: Vec<String> = candidates
            .into_iter()
            .map(|c| c.as_ref().to_string())
            .collect();
        if let Some(near) = nearest(name, cands.iter()) {
            return format!(" — did you mean `{near}`?");
        }
        let close = closest_matches(name, cands.iter(), limit);
        match close.as_slice() {
            [] => String::new(),
            names => {
                let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
                format!(" — closest matches: {}", quoted.join(", "))
            }
        }
    }

    /// Levenshtein–Damerau edit distance (insertions, deletions, substitutions, and ADJACENT
    /// TRANSPOSITIONS) between two names, in Unicode scalar values. Transpositions count as one edit so a
    /// `fodl`→`fold` swap reads as a single typo. O(a·b) time, O(b) space — names are short, so this is
    /// negligible and only runs on a reject path.
    pub fn edit_distance(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        if a.is_empty() {
            return b.len();
        }
        if b.is_empty() {
            return a.len();
        }
        // Two rolling rows would suffice for plain Levenshtein; the transposition rule needs the row two
        // back, so keep three rows.
        let mut prev2: Vec<usize> = vec![0; b.len() + 1];
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        let mut cur: Vec<usize> = vec![0; b.len() + 1];
        for i in 1..=a.len() {
            cur[0] = i;
            for j in 1..=b.len() {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
                // Adjacent transposition: `a[i-1] a[i-2]` swapped matches `b[j-2] b[j-1]`.
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    v = v.min(prev2[j - 2] + 1);
                }
                cur[j] = v;
            }
            std::mem::swap(&mut prev2, &mut prev);
            std::mem::swap(&mut prev, &mut cur);
        }
        prev[b.len()]
    }

    #[cfg(test)]
    mod tests {
        use super::nearest;

        #[test]
        fn tie_break_prefers_a_shared_first_character() {
            // On an edit-distance tie, a candidate sharing the query's first character wins — a typo
            // rarely changes the leading letter. `Lst` is 1 edit from both `List` (insert `i`) and `Ast`
            // (substitute `L`→`A`); `List` shares the `L`, so it is the intended suggestion (was `Ast`
            // under the old lexicographic-only tie-break).
            assert_eq!(nearest("Lst", ["Ast", "List", "Set"]), Some("List".into()));
            assert_eq!(nearest("Lst", ["List", "Ast"]), Some("List".into())); // order-independent
            // No first-char match among the tied → fall back to lexicographic (still deterministic).
            assert_eq!(nearest("xy", ["az", "ay"]), Some("ay".into())); // both dist 1, neither shares `x`
        }

        #[test]
        fn nearest_keeps_real_near_matches_across_length_deltas() {
            // The length-difference PREFILTER (skip a candidate whose char-length differs from the query
            // by more than `max_dist` before running the O(len²) Levenshtein) must NOT drop a genuine
            // typo whose length legitimately differs within the cutoff. Cover a substitution (same len),
            // an insertion and a deletion (len ±1), and a transposition (same len).
            let cands = ["compute", "fold", "helper", "value", "length"];
            assert_eq!(nearest("computee", cands), Some("compute".into())); // 1 insertion (len +1)
            assert_eq!(nearest("folds", cands), Some("fold".into())); // 1 deletion (len -1)
            assert_eq!(nearest("helpe", cands), Some("helper".into())); // 1 deletion (len -1)
            assert_eq!(nearest("lenght", cands), Some("length".into())); // transposition (same len)
            // A candidate whose length is far from the query (beyond max_dist) is correctly NOT suggested —
            // `x` (would need max_dist ≥ 5 to reach `length`); the prefilter and the distance cutoff agree.
            assert_eq!(nearest("zzzzzzzz", cands), None);
        }

        #[test]
        fn nearest_prefilter_agrees_with_the_unfiltered_distance_cutoff() {
            // The prefilter is a pure optimization: for EVERY (query, candidate) pair the result must be
            // identical to the unfiltered "closest within max(1, len/3)" search — the prefilter only skips
            // pairs the distance cutoff would reject anyway (a length delta lower-bounds the edit distance).
            let names = [
                "fold",
                "folder",
                "f",
                "compute",
                "compote",
                "abcdefghij",
                "xy",
            ];
            let cands = ["fold", "folder", "compute", "abcdefghij", "value", "kez"];
            for q in names {
                // Reference implementation: the same cutoff, WITHOUT the length prefilter.
                let name_len = q.chars().count();
                let want = if name_len < 2 {
                    None
                } else {
                    let max_dist = (name_len / 3).max(1);
                    let q_first = q.chars().next();
                    let mut best: Option<((usize, bool, &str), &str)> = None;
                    for c in cands {
                        if c == q || c == "_" || c.is_empty() {
                            continue;
                        }
                        let d = super::edit_distance(q, c);
                        if d > max_dist {
                            continue;
                        }
                        let k = (d, c.chars().next() != q_first, c);
                        if best.is_none_or(|(bk, _)| k < bk) {
                            best = Some((k, c));
                        }
                    }
                    best.map(|(_, n)| n.to_string())
                };
                assert_eq!(
                    nearest(q, cands),
                    want,
                    "prefilter changed the result for {q:?}"
                );
            }
        }

        #[test]
        fn closest_matches_always_offers_the_nearest_even_beyond_the_typo_cutoff() {
            use super::closest_matches;
            let cands = ["fold", "map", "length", "value", "compute"];
            // A query too far off for a confident `nearest` typo still gets the closest few, nearest first.
            let got = closest_matches("fxld", cands, 3);
            assert_eq!(got.first().map(String::as_str), Some("fold")); // distance 2, the nearest
            assert_eq!(got.len(), 3); // limited to 3
            // The name itself, `_`, empty, and duplicates are never offered.
            let dedup = closest_matches("fold", ["fold", "map", "map", "_", ""], 5);
            assert_eq!(dedup, vec!["map".to_string()]); // "fold" (self), dup "map", "_", "" all dropped
            // No candidates → empty.
            assert!(closest_matches("x", Vec::<&str>::new(), 3).is_empty());
        }

        #[test]
        fn did_you_mean_is_two_tiered() {
            use super::did_you_mean;
            let cands = ["compute", "fold", "length"];
            // A plausible typo → the CONFIDENT single suggestion.
            assert_eq!(
                did_you_mean("computee", cands, 3),
                " — did you mean `compute`?"
            );
            // Too far for a confident typo → the FALLBACK closest-matches list (never nothing here).
            let hint = did_you_mean("xyzzy", cands, 3);
            assert!(hint.starts_with(" — closest matches: "), "got {hint:?}");
            assert!(hint.contains('`'), "lists candidates: {hint:?}");
            // No candidates at all → empty string (nothing to suggest).
            assert_eq!(did_you_mean("xyzzy", Vec::<&str>::new(), 3), "");
        }
    }
}

#[cfg(test)]
mod cdz0308_tests {
    use super::{Code, unreachable_branch_fact, unreachable_branch_message};

    #[test]
    fn unreachable_branch_maps_to_cdz0308() {
        // The new dead-branch reachability warning is CDZ0308, the next slot in the 03xx code-quality/
        // dead-code band (0305 dead-trap, 0306 unused-binding, 0307 discarded-value). Pins the code so a
        // future taxonomy edit can't silently move it, and confirms it doesn't collide with a sibling.
        assert_eq!(Code::UnreachableBranch.code(), "CDZ0308");
        // Distinct from the sibling dead-code codes it must not be confused with.
        assert_ne!(Code::DeadTrap.code(), "CDZ0308");
        assert_ne!(Code::DiscardedValue.code(), "CDZ0308");
    }

    #[test]
    fn unreachable_branch_message_is_the_pinned_shape() {
        // The canonical CDZ0308 wording the value-facts emitter uses. Pins the shape end-to-end so a
        // re-word is a deliberate one-place edit + this update — the emitter never diverges. The
        // `<var> ∈ [lo,hi]` fact clause is load-bearing: it names the interval that PROVES the branch dead,
        // which is what makes the warning trustworthy (a reader verifies the claim against the code).
        assert_eq!(
            unreachable_branch_message("x > 0", true, "x ∈ [1, 127]"),
            "this branch is never reached — `x > 0` is always true here (x ∈ [1, 127])"
        );
        // The false case picks `false`; the condition + fact are echoed verbatim.
        assert_eq!(
            unreachable_branch_message("n == 0", false, "n ∈ [1, 9]"),
            "this branch is never reached — `n == 0` is always false here (n ∈ [1, 9])"
        );
    }

    #[test]
    fn unreachable_branch_fact_renders_closed_and_open_intervals() {
        // The proving-interval clause the value-facts emitter passes as `fact`. A closed bound → ∈ [lo, hi];
        // an OPEN upper bound (hi=None, a `>= lo` fact with no proven ceiling) → `<var> ≥ <lo>`. Pinned so
        // the interval wording stays one owned shape across every emit site.
        assert_eq!(unreachable_branch_fact("x", 1, Some(127)), "x ∈ [1, 127]");
        assert_eq!(unreachable_branch_fact("n", 0, None), "n ≥ 0");
        // Composes with the message helper into the full CDZ0308 wording.
        assert_eq!(
            unreachable_branch_message("x > 0", true, &unreachable_branch_fact("x", 1, Some(127))),
            "this branch is never reached — `x > 0` is always true here (x ∈ [1, 127])"
        );
    }
}

#[cfg(test)]
mod decline_catalog_tests {
    use super::{Code, DeclineId, Reject};

    #[test]
    fn catalog_is_enumerable_with_stable_kebab_keys() {
        // Inc-1: the DeclineId catalog exists + is enumerable via ALL. The registry generator (Inc-2)
        // iterates ALL, so this list is the source of truth. Pin the seed count so accidentally dropping
        // a variant from ALL (breaking the "complete by construction" contract) reds here.
        assert_eq!(
            DeclineId::ALL.len(),
            10,
            "seed catalog size (grow deliberately as sites migrate)"
        );
        // Every key is stable kebab-case (lowercase ASCII + hyphens; no spaces/underscores/caps) — the
        // durable referent a tool/registry pins, independent of the Rust variant name.
        for &id in DeclineId::ALL {
            let k = id.key();
            assert!(!k.is_empty(), "empty key for {id:?}");
            assert!(
                k.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "key `{k}` for {id:?} is not kebab-case"
            );
            assert!(
                !k.starts_with('-') && !k.ends_with('-'),
                "key `{k}` has a boundary hyphen"
            );
            assert!(!id.reason().is_empty(), "empty reason for {id:?}");
        }
    }

    #[test]
    fn catalog_keys_are_unique() {
        // Two reasons must never share a key — the key is the registry's primary referent.
        let mut keys: Vec<&str> = DeclineId::ALL.iter().map(|id| id.key()).collect();
        keys.sort_unstable();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "duplicate DeclineId key(s) in the catalog");
    }

    #[test]
    fn declined_carries_the_ids_code_and_stays_a_decline() {
        // `declined(id, msg)` takes its umbrella code from `id.code()` and records the id; the message
        // keeps the runtime specifics. Every seeded reason is either codeless (None) or CDZ0900 — so it
        // is ALWAYS a decline (is_decline holds), leaving safety-ordering / dedup logic unaffected.
        for &id in DeclineId::ALL {
            let r = Reject::declined(id, "specifics");
            assert_eq!(r.id, Some(id));
            assert_eq!(
                r.code,
                id.code(),
                "declined() code must derive from id.code() for {id:?}"
            );
            assert!(
                matches!(id.code(), None | Some(Code::UnsupportedConstruct)),
                "a seeded decline reason must be codeless or CDZ0900, got {:?} for {id:?}",
                id.code()
            );
            assert!(r.is_decline(), "declined({id:?}) must be a decline");
            assert_eq!(r.message, "specifics");
        }
    }
}
