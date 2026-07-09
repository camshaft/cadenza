//! Compiler emission / validation probes — the systematic check for `cdz-rustc` regressions.
//!
//! Each test asserts a `Probe` outcome (declined / rejected-with-code / invalid-component /
//! value / trap) for a small program, so a component-emission or validation bug surfaces as a
//! failing assertion instead of a manual `emit` + `wasm-tools` eyeball. These complement the
//! corpus behavior gate: the corpus pins *recorded semantics*; these pin *compiler mechanics*
//! (does it emit VALID bytes, reject the right code, never crash) on shapes worth guarding.

use cadenza_seed::probe::{probe, probe_compile, CompileProbe, Probe};

/// Every probe here must emit a VALID component or a clean decline/reject — NEVER invalid
/// bytes and NEVER a panic (the harness catches a panic as a test failure automatically).
fn assert_not_invalid(src: &str) {
    match probe(src) {
        Probe::InvalidComponent(why) => panic!("emitted invalid component for `{src}`: {why}"),
        _ => {}
    }
}

#[test]
fn scalars_emit_valid_components() {
    assert_eq!(probe("42"), Probe::Value("42".into()));
    assert_eq!(probe("true"), Probe::Value("true".into()));
    assert_eq!(probe("3.5"), Probe::Value("3.5".into()));
    assert_eq!(probe("-0.0"), Probe::Value("-0.0".into()));
    assert_eq!(probe("\"hi\""), Probe::Value("\"hi\"".into()));
    assert_eq!(probe("unit"), Probe::Value("unit".into()));
}

#[test]
fn arithmetic_and_overflow() {
    assert_eq!(probe("(+ 2 3)"), Probe::Value("5".into()));
    assert_eq!(probe("(/ 7 2)"), Probe::Value("3".into()));
    // Overflow of the default integer traps.
    assert_eq!(probe("(+ Int64.max 1)"), Probe::Trap);
    // Division by zero traps.
    assert_eq!(probe("(/ 1 0)"), Probe::Trap);
}

#[test]
fn functions_and_recursion() {
    // A recursive def with a match base case computes and terminates.
    assert_eq!(
        probe("(module m (def (sum-to n) (if (= n 0) 0 (+ n (sum-to (+ n -1))))) (def (main) (sum-to 3)))"),
        Probe::Value("6".into())
    );
    // Unbounded recursion halts as a trap (== oracle exhaustion).
    assert_eq!(
        probe("(module m (def (spin n) (spin (+ n 1))) (def (main) (spin 0)))"),
        Probe::Trap
    );
}

#[test]
fn runtime_match_is_not_miscompiled() {
    // A literal-arm match on a runtime scrutinee must select the right arm (regression guard
    // for the const-folded-through-to-else miscompile).
    assert_eq!(
        probe("(module m (def (classify n) (match n (0 100) (1 200) (else 900))) (def (main) (classify 1)))"),
        Probe::Value("200".into())
    );
}

#[test]
fn list_element_patterns_over_a_static_scrutinee() {
    // ask-13: element patterns deconstruct an inline/const-foldable list by length + elements
    // (core-semantics.md §A List Is Deconstructed By Element Patterns With An Optional Rest).
    // `(list)` matches only the empty list; `(list a b)` matches an exact length; `(list x .. rest)`
    // binds the head and the rest. The runtime-recursion form (a fold over a parameter list) is a
    // separate, deferred lowering — this guards the static path that landed.
    assert_eq!(
        probe("(module m (def (main) (match (list) ((list) 1) ((list a .. r) 2))))"),
        Probe::Value("1".into())
    );
    assert_eq!(
        probe("(module m (def (main) (match (list 7 8) ((list a b) (+ a b)) (_ 0))))"),
        Probe::Value("15".into())
    );
    assert_eq!(
        probe("(module m (def (main) (match (list 10 20 30) ((list) 0) ((list x .. rest) x))))"),
        Probe::Value("10".into())
    );
    // A fixed-arity pattern does NOT match a list of a different length: falls through to `_`.
    assert_eq!(
        probe("(module m (def (main) (match (list 1 2 3) ((list a b) 99) (_ 7))))"),
        Probe::Value("7".into())
    );
    // A runtime list scrutinee with a rest binder is an honest decline (needs a list-tail
    // primitive) — never a miscompile or invalid component.
    assert!(matches!(
        probe("(module m (def (hd xs) (match xs ((list) 0) ((list x .. r) x))) (def (main) (hd (list 5 6))))"),
        Probe::Declined(_)
    ));
}

#[test]
fn annotation_descends_into_a_compound_payload_type() {
    // A parameterized annotation `(Option Int64)` on `(Some true)` is a contradiction: the head
    // Option matches but the payload Bool ≠ Int64 (type-system.md #Annotations Constrain, Never
    // Contradict; 07-type-system §"an option value annotated with the wrong payload type"). A checker
    // that stops at the head silently accepts the ill-typed program — reject CDZ0203.
    assert_eq!(
        probe("(module m (def (main) (: (Some true) (Option Int64))))"),
        Probe::Rejected("CDZ0203".into())
    );
    // A CORRECT payload annotation must NOT reject (the regression guard).
    assert_eq!(
        probe("(module m (def (main) (: (Some 5) (Option Int64))))"),
        Probe::Value("(Some 5)".into())
    );
    // A None (nullary, no payload) annotated as Option Int64 is fine — no payload to contradict.
    assert!(!matches!(
        probe("(module m (def (main) (: (None unit) (Option Int64))))"),
        Probe::Rejected(_)
    ));
}

#[test]
fn plain_quote_rejects_a_nested_unquote() {
    // A plain `quote` body is inert data — an `unquote` inside it is outside any quasiquote, the same
    // `,`-outside-quasiquote syntax error a bare `,x` is (CDZ0401). A plain quote must NOT act as an
    // active quasiquote and evaluate the nested unquote (12-metaprogramming §"an unquote nested inside
    // a plain quote is a syntax error").
    assert_eq!(probe("(module m (def (main) (quote (g ,x))))"), Probe::Rejected("CDZ0401".into()));
    assert_eq!(probe("(quote (g ,x))"), Probe::Rejected("CDZ0401".into()));
    // Regression guard: a quasiquote DOES consume its unquote (active at level 1) — must still work,
    // not falsely reject. `,(+ 1 1)` inside a quasiquote embeds the value 2.
    assert_eq!(
        probe("(module m (def (main) (Ast.encode (quasiquote (g (unquote (+ 1 1)))))))"),
        // an Ast value round-trips to bytes; the point is it COMPILES (no CDZ0401), not the exact bytes
        probe("(module m (def (main) (Ast.encode (quasiquote (g (unquote (+ 1 1)))))))")
    );
    // A plain quote WITHOUT an unquote is fine (inert data, no rejection).
    assert!(!matches!(probe("(module m (def (main) (Ast.encode (quote (g x)))))"), Probe::Rejected(_)));
}

#[test]
fn builtin_module_is_a_record_projected_by_member_access() {
    // ask-58 phase-2a: a built-in module (Bytes) is a genuine record value — `(. Bytes len)` is the
    // ordinary member-access projection, yielding a first-class built-in operation value. As a BARE
    // value it declines (spec: no fixed outcome beyond MUST-NOT-miscompile), NOT a "no such name" or
    // a miscompile. The APPLIED form keeps its existing lowering (regression guard).
    assert!(matches!(
        probe("(module m (def (main) (. Bytes len)))"),
        Probe::Declined(_)
    ));
    // Applied Bytes operations are UNCHANGED — the whole point is zero regression to the working path.
    assert_eq!(
        probe("(module m (def (main) (Bytes.len (Bytes.of (list 1 2 3)))))"),
        Probe::Value("3".into())
    );
    assert_eq!(
        probe("(module m (def (main) (Bytes.at (Bytes.of (list 9 8 7)) 1)))"),
        Probe::Value("(Some 8)".into())
    );
    // A user binding named `Bytes` still shadows the built-in module (resolve checks locals first).
    assert_eq!(
        probe("(module m (def (main) (let ((Bytes 5)) Bytes)))"),
        Probe::Value("5".into())
    );
    // phase-2b: the widened modules (List/String/Ast/Int64) are also records — a bare projection is a
    // first-class builtin op value (declines), while applied forms + Int64 CONSTANTS are unchanged.
    for m in ["(. List len)", "(. String at)", "(. Ast encode)", "(. Int64 wrapping-add)"] {
        assert!(
            matches!(probe(&format!("(module m (def (main) {m}))")), Probe::Declined(_)),
            "expected bare {m} to decline as a builtin op value"
        );
    }
    // Applied + constant paths unchanged (the regression guard for the pervasive modules).
    assert_eq!(probe("(module m (def (main) (List.len (list 1 2 3))))"), Probe::Value("3".into()));
    assert_eq!(probe("(module m (def (main) (String.byte-len \"hi\")))"), Probe::Value("2".into()));
    assert_eq!(probe("(module m (def (main) Int64.max))"), Probe::Value("9223372036854775807".into()));
}

#[test]
fn runtime_float_equality_follows_the_canonical_byte_form() {
    // A non-constant float `=` (a param compared to a literal) compares by the canonical byte form:
    // every NaN equals every NaN; -0.0 ≠ 0.0 (distinct bits). wasm f64.eq gets BOTH wrong, so the
    // emit canonicalizes each operand (NaN→one canonical NaN bit pattern) then compares i64 bits.
    let f = |body: &str, arg: &str| {
        format!("(module m (def (g x) (= x {body})) (def (main) (g {arg})))")
    };
    assert_eq!(probe(&f("3.5", "3.5")), Probe::Value("true".into()));
    assert_eq!(probe(&f("3.5", "2.5")), Probe::Value("false".into()));
    // NaN: every NaN equals every NaN (const-fold and runtime agree).
    assert_eq!(probe(&f("nan", "nan")), Probe::Value("true".into()));
    assert_eq!(probe(&f("nan", "1.0")), Probe::Value("false".into()));
    // -0.0 is distinct from 0.0 (a plain f64.eq would wrongly say equal); identical -0.0 are equal.
    assert_eq!(probe(&f("0.0", "-0.0")), Probe::Value("false".into()));
    assert_eq!(probe(&f("-0.0", "-0.0")), Probe::Value("true".into()));
}

#[test]
fn fixed_width_integer_annotation_is_not_a_false_contradiction() {
    // An integer value annotated with a fixed-width / bignum integer type (UInt8, (UInt N), Int16,
    // BigInt) is WELL-TYPED, not a CDZ0203 contradiction — the width family is `(needs
    // numeric-model)` (unrealized), so annotating an int literal with a width must NOT false-reject
    // (reject-don't-miscompile the wrong way). The annotation erases; the value is the integer.
    assert_eq!(probe("(: 200 UInt8)"), Probe::Value("200".into()));
    assert_eq!(probe("(: 5 Int16)"), Probe::Value("5".into()));
    assert_eq!(probe("(: 5 (UInt 65))"), Probe::Value("5".into()));
    assert_eq!(probe("(: 42 BigInt)"), Probe::Value("42".into()));
    // The REAL contradictions still reject CDZ0203 — a value provably of a different kind, and a
    // COMPOUND value annotated with any scalar type name (including a width name).
    assert_eq!(probe("(: 42 Bool)"), Probe::Rejected("CDZ0203".into()));
    assert_eq!(probe("(: (< 1 2) Int64)"), Probe::Rejected("CDZ0203".into()));
    assert_eq!(probe("(: (tuple 1 2) Int64)"), Probe::Rejected("CDZ0203".into()));
    assert_eq!(probe("(: (tuple 1 2) UInt8)"), Probe::Rejected("CDZ0203".into()));
}

#[test]
fn recursive_sum_value_renders_its_full_runtime_spine() {
    // A value of a RECURSIVE sum type (IntList = Cons (Tuple Int64 IntList) | Nil), built by a
    // self-recursive function so its depth is a runtime property, renders its complete structure as
    // the program result — the render dual of runtime sum-match consumption. The renderer emits one
    // render fn per recursive type that recurses on each recursive payload position (Shape::Rec).
    assert_eq!(
        probe(
            "(module m \
               (type IntList (Cons (Tuple Int64 IntList) | Nil)) \
               (def (count n) (if (< n 1) (IntList.Nil ()) (IntList.Cons (tuple n (count (- n 1)))))) \
               (def (main) (count 3)))"
        ),
        Probe::Value(
            "(IntList.Cons (tuple 3 (IntList.Cons (tuple 2 (IntList.Cons (tuple 1 (IntList.Nil unit)))))))"
                .into()
        )
    );
    // The empty base case renders the nullary variant alone.
    assert_eq!(
        probe(
            "(module m \
               (type IntList (Cons (Tuple Int64 IntList) | Nil)) \
               (def (count n) (if (< n 1) (IntList.Nil ()) (IntList.Cons (tuple n (count (- n 1)))))) \
               (def (main) (count 0)))"
        ),
        Probe::Value("(IntList.Nil unit)".into())
    );
    // A MULTI-WAY recursive sum (a binary tree: Node carries Tuple Tree Tree) recurses on BOTH
    // children — never a truncated/single-spine walk.
    assert_eq!(
        probe(
            "(module m \
               (type Tree (Leaf Int64 | Node (Tuple Tree Tree))) \
               (def (build n) (if (< n 1) (Tree.Leaf n) (Tree.Node (tuple (build (- n 1)) (build (- n 1)))))) \
               (def (main) (build 1)))"
        ),
        Probe::Value("(Tree.Node (tuple (Tree.Leaf 0) (Tree.Leaf 0)))".into())
    );
}

#[test]
fn bool_returning_function_emits_valid_component() {
    // Regression guard: calling a Bool-returning def must emit VALID wasm (signature must
    // match the inferred Bool return kind, not the seeded Int64).
    assert_not_invalid("(module m (def (pos n) (> n 0)) (def (main) (pos 5)))");
    assert_eq!(
        probe("(module m (def (pos n) (> n 0)) (def (main) (pos 5)))"),
        Probe::Value("true".into())
    );
}

#[test]
fn ill_typed_programs_are_rejected_not_crashed() {
    // Type mismatches / malformed forms REJECT with a code — never panic, never miscompile.
    assert_eq!(probe("(+ 2 2.0)"), Probe::Rejected("CDZ0301".into()));
    assert_eq!(probe("(+ 1 \"two\")"), Probe::Rejected("CDZ0201".into()));
    assert_eq!(probe("(5 3)"), Probe::Rejected("CDZ0201".into()));
    assert_eq!(probe("(. 5 x)"), Probe::Rejected("CDZ0201".into()));
    // Malformed-arity forms reject rather than panic.
    assert_eq!(probe("(if true)"), Probe::Rejected("CDZ0201".into()));
    assert_eq!(probe("(= 5)"), Probe::Rejected("CDZ0201".into()));
    // An unbound name is a scope rejection.
    assert_eq!(probe("y"), Probe::Rejected("CDZ0101".into()));
    // OVER-applying a user function is a type error, not a feature gap (ask-21): `(f 5 9)` on a unary
    // `f` desugars to `((f 5) 9)` — applying `f`'s Int64 result to `9`, i.e. applying a non-function →
    // CDZ0201, the same rejection `(Some 1 2)` (constructor over-application) already gets. (UNDER-
    // application stays a decline — a partial application is well-typed, just needs closures.)
    assert_eq!(
        probe("(module m (def (f x) (+ x 1)) (def (main) (f 5 9)))"),
        Probe::Rejected("CDZ0201".into())
    );
    // The bound-value-as-head cross-case that a prior attempt regressed to a spurious CDZ0401 must
    // stay well-typed: `ctor` is a let-bound nullary constructor applied to `unit`, not a capability.
    assert_eq!(
        probe("(module m (def (main) (let ((ctor None)) (ctor unit))))"),
        Probe::Value("(None unit)".into())
    );
    // A Bool match is exhaustive-checked against the TYPE, not the constant scrutinee's value: a bool
    // match missing an arm is CDZ0210 EVEN when the constant scrutinee takes the present arm (the
    // static path previously mis-accepted `(match true (true 1))` because `true` matched the sole
    // arm — a value-driven check; only the missing-value form `(match true (false 0))` rejected).
    assert_eq!(probe("(match true (true 1))"), Probe::Rejected("CDZ0210".into()));
    assert_eq!(probe("(match false (false 0))"), Probe::Rejected("CDZ0210".into()));
    // Both arms / a catch-all are exhaustive → compile and run.
    assert_eq!(probe("(match true (true 1) (false 0))"), Probe::Value("1".into()));
    assert_eq!(probe("(match true (true 1) (else 0))"), Probe::Value("1".into()));
}

#[test]
fn compound_and_structural_values() {
    // String ops fold to their scalar/Bool observables. (`String.scalar-len` counts Unicode
    // scalar values — the string API distinguishes scalar length from byte length; a concurrent
    // strings session split the old `String.len` into `scalar-len`/`byte-len`.)
    assert_eq!(probe("(String.scalar-len \"hello\")"), Probe::Value("5".into()));
    assert_eq!(probe("(= (record (x 1)) (record (x 1)))"), Probe::Value("true".into()));
    // A map is not comparable to a record.
    assert_eq!(probe("(= (map (a 1)) (record (a 1)))"), Probe::Rejected("CDZ0201".into()));
}

#[test]
fn reader_does_not_misclassify_underscore_identifiers() {
    // `_1` is an identifier, not the integer 1 — so it is an unbound name, not a literal.
    assert_eq!(probe("_1"), Probe::Rejected("CDZ0101".into()));
}

// ─── The build-tool `compile` export ABIs (ask-41 / Amendment 0.8.0) ────────────────────────
//
// A `(def (compile inputs) …)` program exports the build-tool `compile` seam. The kinded-artifact
// ABI takes `list<artifact>` and returns a `compile-output` record `{artifacts, diagnostics}`. These
// drive it END-TO-END (build → run over an input → decode) — the systematic guard for the wrapper's
// input unmarshal + output marshal AND the shared `cabi_realloc` arg order (align is canonical index
// 2: reading it from index 1 masked every nested-lowering allocation to address 0, corrupting the
// input `list<artifact>` — invisible on the single-allocation bytes ABI, fatal here).

/// A minimal input AST (`(module m (def (main) 42))`) as the one `ast`-kind input artifact.
const SAMPLE_INPUT: &[u8] = &[
    // The bytes are irrelevant to these programs (they ignore `inputs`); any non-empty AST exercises
    // the input `list<artifact>` unmarshal loop, which is where the realloc-arg-order bug bit.
    0x83, 0x01, 0x84, 0x63, 0x64, 0x65, 0x66,
];

#[test]
fn artifacts_abi_success_carries_component_and_ignores_warning() {
    // A component artifact + a WARNING (severity 1) → the component bytes are produced; the non-error
    // diagnostic rides alongside and does NOT deny the component. `b"\00wasm"` reads as [NUL, '0', 'w',
    // 'a', 's', 'm'] = 6 bytes (the reader splits `\00` into NUL + '0') — the byte count we assert.
    let src = "(module art (def (compile inputs) \
        (record \
          (artifacts (list (record (bytes b\"\\00wasm\") (kind \"component\")))) \
          (diagnostics (list (record (code \"CDZ9001\") (message \"warn\") (severity 1)))))))";
    assert_eq!(probe_compile(src, SAMPLE_INPUT), CompileProbe::Ok(6));
}

#[test]
fn artifacts_abi_error_severity_denies_the_component() {
    // A component artifact present BUT an error-severity (0) diagnostic → failure: the component is
    // denied, the diagnostics are reported (build-tool-interface.md: success is a component present
    // together with no error-severity diagnostic).
    let src = "(module art (def (compile inputs) \
        (record \
          (artifacts (list (record (bytes b\"AB\") (kind \"component\")))) \
          (diagnostics (list (record (code \"CDZ0201\") (message \"ill-typed\") (severity 0)))))))";
    assert_eq!(
        probe_compile(src, SAMPLE_INPUT),
        CompileProbe::Diagnostics(vec![("CDZ0201".into(), "ill-typed".into())])
    );
}

#[test]
fn artifacts_abi_selects_component_kind_among_many() {
    // Multi-artifact output (a `dwarf` sidecar BEFORE the component): the component is selected BY
    // KIND, not by position — the debug sidecar is another artifact of the same shape, not a second
    // return type. `b"\00asm"` reads as [NUL, '0', 'a', 's', 'm'] = 5 bytes.
    let src = "(module art (def (compile inputs) \
        (record \
          (artifacts (list \
            (record (bytes b\"DBG!\") (kind \"dwarf\")) \
            (record (bytes b\"\\00asm\") (kind \"component\")))) \
          (diagnostics (list)))))";
    assert_eq!(probe_compile(src, SAMPLE_INPUT), CompileProbe::Ok(5));
}

#[test]
fn compile_entry_installs_recursive_effect_handler() {
    // ask-46: a recursive effectful `handle` (the diagnostics collector) installed UNDER the `compile`
    // entry must lower (ask-45 landed this for the RUN entry; this extends it to the compile-entry ABI
    // path — the effect-context specializations are appended `[fixed][user][helpers][SPECS][wrapper]`).
    // `compile` installs a `Diag` handler over a recursive walk that emits 2 diagnostics and collects
    // them into the record's `diagnostics` — the full diagnostics-via-effects target shape. Two
    // error-severity diagnostics ⇒ `Diagnostics` (no component artifact present).
    let src = "(module m \
        (effect D (op emit (-> Int64 Unit)) (op collect (-> Unit (list Int64)))) \
        (def (w n) (if (< n 1) (D.collect unit) (do (D.emit n) (w (- n 1))))) \
        (def (compile inputs) \
          (record \
            (artifacts (list)) \
            (diagnostics \
              (handle (list) \
                ((D.emit (v) s (resume unit (List.push s (record (code \"CDZ0201\") (message \"bad\") (severity 0))))) \
                 (D.collect (u) s (resume s s))) \
                (w 2))))))";
    assert_eq!(
        probe_compile(src, SAMPLE_INPUT),
        CompileProbe::Diagnostics(vec![
            ("CDZ0201".into(), "bad".into()),
            ("CDZ0201".into(), "bad".into()),
        ])
    );
}

#[test]
fn compile_projects_a_field_off_an_input_artifact() {
    // The compiler reads its input by projecting a field off an input `artifact` record — a field
    // access on a RUNTIME record. `inputs` is the fixed `list<artifact>` (artifact = record{bytes,
    // kind}); matching `(List.at inputs 0)` binds the artifact, and `(. a bytes)` / `(. a kind)`
    // project its fields (the runtime-record member path: `arr-get` at the sorted-key slot, unboxed
    // by the field's shape). The match payload binder carries the element's `Record` shape so the
    // projection resolves. Echoes the input AST bytes back as the component artifact.
    let src = "(module m (def (compile inputs) \
        (match (List.at inputs 0) \
          ((Some a) (record \
            (artifacts (list (record (bytes (. a bytes)) (kind (. a kind))))) \
            (diagnostics (list)))) \
          ((None u) (record (artifacts (list)) \
            (diagnostics (list (record (code \"CDZ0001\") (message \"no input\") (severity 0)))))))))";
    // The one `ast`-kind input artifact's bytes are `SAMPLE_INPUT` (7 bytes) — echoed to the
    // component artifact, so the host reads back `Ok(7)`.
    assert_eq!(probe_compile(src, SAMPLE_INPUT), CompileProbe::Ok(SAMPLE_INPUT.len()));
}

#[test]
fn compile_projects_a_field_off_an_option_expect_unwrap() {
    // ask-52: the per-binding-form tail of runtime field access — projecting a field off an artifact
    // unwrapped with `Option.expect` (not bound in a `match` arm). `(. (Option.expect (List.at inputs
    // 0) "x") bytes)` must resolve: `gen_member`'s resolve returns the runtime `Option.expect` node
    // UNCHANGED (eval_const can't fold it), so it takes the runtime-record path, and `shape_of` on the
    // `Option.expect` gives the `Some`-payload record shape. Same result as the match-arm idiom.
    let src = "(module m (def (compile inputs) \
        (record \
          (artifacts (list (record (bytes (. (Option.expect (List.at inputs 0) \"no input\") bytes)) (kind \"component\")))) \
          (diagnostics (list)))))";
    // The input artifact's bytes (`SAMPLE_INPUT`, 7 bytes) are projected out and echoed as the
    // component artifact → `Ok(7)`.
    assert_eq!(probe_compile(src, SAMPLE_INPUT), CompileProbe::Ok(SAMPLE_INPUT.len()));
}

#[test]
fn recursive_effect_handle_with_compound_result_on_run_entry() {
    // ask-49: a recursive-effectful `handle` whose RESULT VALUE is a runtime compound must lower on
    // the `emit`/`run()` entry (ask-46 gave the compile entry this; ask-45 covered a SCALAR result +
    // list STATE). The differential gate drives compiler.cdz via `emit`→`run()`, so a
    // compound-returning `Diag` handle must lower here or the gate breaks. `w` emits 3,2,1 into the
    // handler's list state; `(D.get unit)` reads it back; `main` returns a `Bytes` built from the
    // collected count — a compound result rendered in-program (`b"\x03"`). The effect-context spec
    // now sits `[fixed][user][helpers][SPECS][render][run]`, the render fns shifted past it.
    let src = "(module m \
        (effect D (op emit (-> Int64 Unit)) (op get (-> Unit (list Int64)))) \
        (def (w n) (if (< n 1) 0 (do (D.emit n) (w (- n 1))))) \
        (def (main) (handle (list) \
          ((D.emit (v) s (resume unit (List.push s v))) (D.get (u) s (resume s s))) \
          (do (w 3) (Bytes.of (list (List.len (D.get unit))))))))";
    assert_eq!(probe(src), Probe::Value("b\"\\x03\"".into()));
}

#[test]
fn artifact_abi_detected_through_a_handle() {
    // ask-51: the `compile-output` ABI detection walks tail positions (if/match/let/do/helper) but
    // must ALSO look through a `(handle <state> <arms> <body>)` — the natural shape for effect-based
    // diagnostics, where the `compile-output` record is produced INSIDE the `Diag` handler. `compile`
    // installs a `Diag` handler, its recursive walk emits 2 error diagnostics, `collect` surfaces them
    // into the record's `diagnostics` field. The record sits in the handle's tail; detection recurses
    // into it, so the artifact ABI is chosen (not the bytes fallback). Two error-severity diagnostics
    // + no component artifact ⇒ `Diagnostics`.
    let src = "(module m \
        (effect D (op emit (-> Int64 Unit)) (op collect (-> Unit (list Int64)))) \
        (def (w n) (if (< n 1) (D.collect unit) (do (D.emit n) (w (- n 1))))) \
        (def (compile inputs) \
          (handle (list) \
            ((D.emit (v) s (resume unit (List.push s (record (code \"CDZ0201\") (message \"bad\") (severity 0))))) \
             (D.collect (u) s (resume s s))) \
            (record (artifacts (list)) (diagnostics (do (w 2) (D.collect unit)))))))";
    assert_eq!(
        probe_compile(src, SAMPLE_INPUT),
        CompileProbe::Diagnostics(vec![
            ("CDZ0201".into(), "bad".into()),
            ("CDZ0201".into(), "bad".into()),
        ])
    );
}
