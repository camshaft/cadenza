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

/// A stable, machine-readable diagnostic code. Its `code()` string is the durable identity a
/// consumer matches on; the enum variant is the compiler-internal handle.
///
//= spec/capabilities/diagnostics.md#every-diagnostic-has-a-stable-code
//# Every diagnostic the compiler emits MUST carry a machine-readable code that is stable across changes to unrelated diagnostics.
///
//= constitution.md#xi-diagnostics-are-machine-actionable
//# Every diagnostic the compiler emits MUST carry a stable machine-readable code.
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
    /// A computation the compiler PROVES would trap (`ConstTrap`'s outcome) was ELIMINATED because its
    /// value is unobserved — an unprojected tuple/record element, an unreferenced `let` binding, an
    /// argument bound to an unused parameter. NOT a rejection: the build succeeds (the dead computation
    /// need not run — `core-semantics.md` §A Trap Occurs Only Where Its Computation Is Observed). This
    /// is the WARNING severity's code, emitted so a program does not silently discard a computation that
    /// could never have produced a value (almost always a defect). The error-severity companion is
    /// `ConstTrap` (CDZ0304), emitted when the same provable trap IS observed.
    DeadTrap,
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
            Code::UnusedBinding => "CDZ0306",
            Code::DiscardedValue => "CDZ0307",
            Code::NonExhaustive => "CDZ0210",
            Code::PresentField => "CDZ0211",
            Code::AbsentField => "CDZ0212",
            Code::RedundantArm => "CDZ0213",
            Code::EffectNoHome => "CDZ0401",
            Code::HandlerUndeclaredOp => "CDZ0403",
            Code::HandlerNotExhaustive => "CDZ0405",
            Code::LatentAuthority => "CDZ0404",
            Code::ClosureEscapesEffect => "CDZ0406",
            Code::IllFormedBinary => "CDZ0220",
            Code::NonFinalSplice => "CDZ0221",
            Code::DimensionMismatch => "CDZ0501",
            Code::UnitConflict => "CDZ0502",
            Code::UnknownDirective => "CDZ0601",
            Code::MalformedDirective => "CDZ0602",
            Code::RecursionBound => "CDZ0999",
        }
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
            message: message.into(),
            at: None,
            fix: None,
        }
    }

    /// An uncoded decline: a construct the compiler does not yet realize. NOT a statement that the
    /// program is wrong — it is the compiler declining to compile it, the safe outcome
    /// (`reference-compiler.md` §Outcomes Are Ordered By Safety).
    pub fn decline(message: impl Into<String>) -> Reject {
        Reject {
            code: None,
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

    /// Whether this "no" is a decline (uncoded) rather than a coded rejection.
    pub fn is_decline(&self) -> bool {
        self.code.is_none()
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

/// A stable SUBSTRING unique to the BUILT-IN-OPERATION wrong-arity decline (`<op> is applied at the wrong
/// arity — a built-in operation must be applied to exactly its arguments …`, in `lower`). Fires on BOTH
/// an under-application (no coded sibling — the decline is the primary "no") and an OVER-application
/// (where `infer`'s coded CDZ0203 over-application reject is primary — then this decline is redundant).
/// `dedup_faults` matches this to drop the decline ONLY when that coded over-application reject is present.
pub const BUILTIN_WRONG_ARITY_DECLINE: &str = "a built-in operation must be applied to exactly";

/// A stable SUBSTRING unique to the coded CDZ0201 resume-value/result-type mismatch (`a handler resumes
/// with a value of type X but the operation's result type is Y`). An ill-typed resume ALSO makes the
/// handler unfoldable, so `lower` emits the uncoded [`HANDLER_NOT_REDUCIBLE_DECLINE`] alongside — a
/// CONSEQUENCE, not an independent limit. `dedup_faults` matches this to drop that decline, like it does
/// for a malformed handler (CDZ0403/0405), so a mistyped resume is ONE primary error (carrying its
/// coercion fix when applicable).
pub const RESUME_RESULT_MISMATCH_MARKER: &str = "a handler resumes with a value of type";

/// The stable PREFIX of the coded CDZ0201 "this handle is not in canonical form" reject — a source
/// `handle` still headed `handle` after `effects::desugar_handles` (the retired effect-name-less shape,
/// or a too-short handle). Shared as a const so `compile::dedup_faults` can recognize it and drop the
/// CONSEQUENT CDZ0401 (`EffectNoHome`) the rejected handle's un-discharged perform triggers — the perform
/// has no home ONLY because its handler was rejected, so one root cause yields ONE primary `error:` (the
/// CDZ0201, which says how to fix the handle), not a coded reject shadowed by a "you have no handler" that
/// misdirects (the author DID write a handler). Matched as a prefix so the shape-carrying tail can vary.
pub const HANDLE_NONCANONICAL_PREFIX: &str = "this handle is not in canonical form";

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

/// The message the emit path (`lower`) attaches to the UNCODED decline it returns when `reduce_handle`
/// cannot fold a `handle` form. A MALFORMED handler — one whose arm names an operation its effect does
/// not declare (CDZ0403), or that does not discharge every operation (CDZ0405) — cannot fold, so this
/// decline rides ALONGSIDE the coded reject as a second `error:` for the same root cause (the misspelled
/// / missing arm). Shared as a const so `compile::dedup_faults` drops it whenever a CDZ0403/CDZ0405 is
/// present on the program — ONE primary, actionable "no" (the coded reject carries the fix), not a coded
/// rejection shadowed by an emit-path decline (`reference-compiler.md` §Outcomes Are Ordered By Safety).
/// A WELL-FORMED handler that is genuinely not-yet-reducible (a real cross-function / non-tail resume,
/// with NO coded reject) keeps this honest decline — there is nothing stronger to defer to.
pub const HANDLER_NOT_REDUCIBLE_DECLINE: &str = "this handler is not yet reducible by the tail-resumptive fold (cross-function \
     or non-tail resume arrives in a later increment)";

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
pub const TYPE_VALUE_NO_RUNTIME_DECLINE: &str = "a type value has no runtime form";
pub const NULLARY_LAMBDA_NO_CLOSURE_DECLINE: &str = "a nullary lambda has no runtime closure form";
pub const PRIM_AS_VALUE_DECLINE: &str =
    "a built-in operation used as a value needs runtime closures (not yet built)";

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
    /// 1 edit, a 9-char name up to 3. Ties break on the smaller distance, then the
    /// lexicographically-smaller name, so the result is a DETERMINISTIC function of the candidate SET —
    /// independent of the order they are supplied in (a hash-map iteration order never leaks through).
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
        let mut best: Option<(usize, String)> = None;
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
            let better = match &best {
                None => true,
                Some((bd, bn)) => d < *bd || (d == *bd && cand < bn.as_str()),
            };
            if better {
                best = Some((d, cand.to_string()));
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
                    let mut best: Option<(usize, &str)> = None;
                    for c in cands {
                        if c == q || c == "_" || c.is_empty() {
                            continue;
                        }
                        let d = super::edit_distance(q, c);
                        if d > max_dist {
                            continue;
                        }
                        if best.is_none_or(|(bd, bn)| d < bd || (d == bd && c < bn)) {
                            best = Some((d, c));
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
