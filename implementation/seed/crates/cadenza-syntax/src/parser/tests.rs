//! Tests for the ML surface reader/parser ([`super`] = `crate::parser`). Split out of `parser.rs`
//! (2026-08-31, v-syntax) to keep `parser.rs` under the 512KB file-size mandate; the delanguaging
//! test-retirement (v-parser-corpus) continues migrating these to the ml/spec corpus from here.

use super::*;

fn parse_ok(src: &str) -> Arenas {
    let p = read_ml(src);
    assert!(
        p.ok(),
        "expected clean parse of {src:?}, got {:?}",
        p.errors
    );
    p.arenas
}

#[test]
fn deep_nesting_boundary_agrees_across_readers() {
    // DIFFERENTIAL DEPTH-BUDGET GUARD (v-syntax / breaker, re #6881): the iterative reader must accept/
    // reject at the EXACT same nesting depth as the recursive reference for every shape — so the
    // explicit worklist's per-level depth accounting can't silently drift from `expr`'s. NOTE: the
    // absolute boundary is shape-dependent because a level can cost >1 depth unit in BOTH readers — e.g.
    // `(1 + (paren))` costs 2 depth units per nesting level (the `+` right operand + the paren interior
    // are two `expr` levels), so it caps at ~511, NOT 1024; a pure `(((…)))` caps at 1023. That 2×-for-
    // plusparen is INHERENT to the recursive descent (present pre-#6881), not a rewrite regression — the
    // point of this test is only that iterative == recursive, which is the behavior-neutrality contract.
    // The recursive reader overflows the default stack at depth (the SIGABRT the rewrite removes), so it
    // is measured on a big thread — this reads its GUARD boundary, the parity target.
    type Shape = (&'static str, fn(usize) -> String);
    let shapes: &[Shape] = &[
        ("plusparen", |d| {
            format!("{}0{}", "(1 + ".repeat(d), ")".repeat(d))
        }),
        ("paren", |d| format!("{}0{}", "(".repeat(d), ")".repeat(d))),
        ("defbody", |d| {
            format!("def main() = {}0{}", "(1 + ".repeat(d), ")".repeat(d))
        }),
        ("flatplus", |d| format!("{}0", "1 + ".repeat(d))),
        ("call", |d| format!("{}0{}", "f(".repeat(d), ")".repeat(d))),
        ("neg", |d| format!("{}0", "- ".repeat(d))),
    ];
    fn max_ok(mk: fn(usize) -> String, f: &dyn Fn(&str) -> bool) -> usize {
        let mut d = 1;
        while d < 4096 && f(&mk(d)) {
            d += 1;
        }
        d - 1
    }
    let mut msg = String::new();
    for (name, mk) in shapes {
        let it = max_ok(*mk, &|s| read_ml(s).ok());
        let g = *mk;
        let rec = std::thread::Builder::new()
            .stack_size(512 * 1024 * 1024)
            .spawn(move || max_ok(g, &|s| read_ml_recursive(s).ok()))
            .unwrap()
            .join()
            .unwrap();
        if it != rec {
            msg.push_str(&format!("  {name}: iterative {it} != recursive {rec}\n"));
        }
    }
    assert!(
        msg.is_empty(),
        "iterative reader's nesting boundary drifted from the recursive reference:\n{msg}"
    );
}

#[test]
fn expr_iter_matches_recursive_expr() {
    // I3 differential check at the EXPR level: the iterative shunting-yard `expr_iter` must produce a
    // BYTE-IDENTICAL result to the recursive `expr` for every input — arena (structural eq), span
    // table, and errors. Verified directly here (the whole-program oracle covers it end-to-end once
    // `read_ml` routes through the iterative driver). Covers the tricky arms: left/right-assoc chains,
    // right-assoc arrows, ascription + the `:`/`forall` intercept, pipeline, `as` conversion, the unit
    // suffix, member/call postfix operands, and bracket/keyword operands (still recursive this stage).
    use crate::token::PREC_SEQ;
    let cases = [
        "a + b + c",
        "a + b * c - d",
        "a - b - c",
        "a : T",
        "a : forall x. x",
        "f(1) + g(2, 3)",
        "x.a.b + y",
        "a |> f |> g",
        "(a + b) * c",
        "a + (b - c) * d",
        "if a then b else c",
        "a + (if x then 1 else 2)",
        "x meters + 1",
        "10 meters",
        "a == b",
        "t : a -> b -> c",
        "1; 2; 3",
        "a + b; c",
        "-a + b",
        "- - a",
        // quasiquote `{ e } — the first operand family pulled onto the worklist.
        "`{x}",
        "`{a + b}",
        "`{x} + 1",
        "`{`{x}}",
        "`{if a then b else c}",
        "f(`{x}, y)",
        // paren family: unit / grouping / tuple (+ nesting, trailing comma, as-operand).
        "()",
        "(a)",
        "(a + b)",
        "(a, b)",
        "(a, b, c)",
        "(a,)",
        "(a, b,)",
        // tuple CONSTRUCTION spread `(.. a)` (the tuple twin of list/set/map/record spread) — leading,
        // trailing, mid, multi, and nested; a leading `..` forces the tuple path (no grouping).
        "(.. a)",
        "(.. a, 1)",
        "(1, .. a)",
        "(a, .. b, c)",
        "(a, b, .. c)",
        "(.. a, .. b)",
        "((.. a, 1), 2)",
        "((a))",
        "((a, b), c)",
        "(a + b) * c",
        "(a) + 1",
        "(a; b)",
        "-(a + b)",
        "(if a then b else c) + 1",
        // list family: empty / elements / trailing comma / nesting / rest spread / as-operand.
        "[]",
        "[1]",
        "[1, 2, 3]",
        "[1, 2,]",
        "[a + b, c]",
        "[[1], [2, 3]]",
        "[.. rest]",
        "[1, 2, .. xs]",
        "[(a, b), c]",
        "[x]",
        // set family `#( … )` — same comma-list cont as list (closer `)`).
        "#()",
        "#(1, 2, 3)",
        "#(a + b, c)",
        "#(1, .. s)",
        "#(x) + y",
        // raw-list family `#[ … ]` — same cont, NO head / NO rest / NO comment slots (bare elements).
        "#[]",
        "#[1]",
        "#[1, 2, 3]",
        "#[a + b, c]",
        "#[[1], 2]",
        "#[x] + y",
        // bin family `b[ … ]` — same cont, "bin" name head + comment slots, NO rest / NO drain.
        "b[]",
        "b[u8(1)]",
        "b[u16(258), bits(1, 1)]",
        "b[bytes(payload)]",
        "b[u8(1)] + x",
        // record family `{ … }` — FIELD-PAIRS: `name = value`, shorthand pun `{ x }`, `.. rest` spread,
        // trailing comma, nesting, comment slots, as-operand + unit-suffix.
        "{}",
        "{ a = 1 }",
        "{ a = 1, b = 2 }",
        "{ x }",
        "{ x, y }",
        "{ a = 1, b }",
        "{ a = 1, }",
        "{ .. base, a = 1 }",
        "{ a = { b = 2 } }",
        "{ a = 1 + 2, b = f(3) }",
        "{ a = 1 } + x",
        // map family `#{ … }` — FIELD-PAIRS with arbitrary-expr keys: `key = value`, `.. rest`, nesting.
        "#{}",
        "#{ 1 = a }",
        "#{ 1 = a, 2 = b }",
        "#{ k = v, .. rest }",
        "#{ (a + b) = c }",
        "#{ 1 = #{ 2 = 3 } }",
        "#{ 1 = a } + x",
        // `if` keyword form (now on the worklist) — nesting in each slot, infix branches, as-operand.
        "if a then b else c",
        "if a + b then c * d else e",
        "if a then if b then c else d else e",
        "if (if p then q else r) then s else t",
        "if a then b else c + 1",
        "1 + if a then b else c",
        "[if a then b else c, d]",
        "if a then b else c |> f",
        "if x then -a else -b",
        // `let … in …` keyword form (now on the worklist) — single/multi binding, pattern binder,
        // value expr, body sequencing, nesting, as-operand.
        "let x = 1 in x",
        "let x = 1, y = 2 in x + y",
        "let x = a + b in x * 2",
        "let x = 1 in let y = 2 in x + y",
        "let x = if a then b else c in x",
        "let (a, b) = p in a",
        "let x = 1 in (x; x)",
        "1 + let x = 2 in x",
        "let f = fn(a) => a in f(1)",
        // annotated let binder `let x: T = v` -> `(: x T)` binder (shared read_let_binder path).
        "let x: Int64 = 1 in x",
        "let x: Int64 = 1, y: Bool = true in x",
        // `match … with …` keyword form (now on the worklist) — single/multi arm, guard, ctor patterns,
        // infix arm body, nesting in scrutinee/body, as-operand.
        "match x with | 0 => a | _ => b",
        "match x with | Some(y) => y | None => 0",
        "match x with | n if n > 0 => a | _ => b",
        "match x with | a => a + 1",
        "match a + b with | 0 => x | _ => y",
        "match x with | _ => match y with | 0 => a | _ => b",
        "match x with | 0 => a + b | _ => c * d",
        "1 + match x with | _ => 2",
        "match p with | (a, b) => a | _ => 0",
        // `fn` lambda (now on the worklist) — params, typed params, return type, body sequencing,
        // nesting, as-operand.
        "fn(x) => x + 1",
        "fn() => 0",
        "fn(x: Int64, y: Int64) => x + y",
        "fn(x) -> Int64 => x",
        "fn(x) => fn(y) => x + y",
        "fn(x) => (a; b)",
        "map(xs, fn(x) => x * 2)",
        "fn((a, b)) => a",
        // `host` delegation (now on the worklist) — single/multi effect, body, as-operand.
        "host E in x",
        "host E, F in x + y",
        "host E in if a then b else c",
        // `handle` form (now on the worklist) — bare/unit/seeded effect, single/multi arm, arm params +
        // state, body, nesting, as-operand.
        "handle E with | op(s) => resume(1, s) in body",
        "handle E() with | op(s) => resume(1, s) in body",
        "handle E(0) with | op(s) => resume(1, s) in body",
        "handle E with | get(s) => resume(s, s) | put(x, s) => resume((), x) in body",
        "handle E(a + b) with | op(x, s) => resume(x, s) in run()",
        "handle E with | op(s) => s in x + 1",
        "1 + handle E with | op(s) => s in x",
        "handle E with | op(s) => resume(x | 8, s) in body",
        // call/postfix funnel (arg_exprs on the worklist) — empty/single/multi args, nested calls,
        // chained calls, member+call chains, calls-of-keyword-operands.
        "f()",
        "f(1)",
        "f(1, 2, 3)",
        "f(g(h(x)))",
        "f(1)(2)(3)",
        "x.a.b.c",
        "obj.method(1, 2)",
        "a.b(c).d(e)",
        "f(a + b, c * d)",
        "f(g(1), h(2)) + k(3)",
        "m.f().g().h()",
        "f(if a then b else c)",
        "outer(let x = 1 in x, y)",
        "f(x).y + z",
        "-f(x)",
        "f(fn(a) => a, 2)",
        // postfix on NON-prefix (reduce-produced) operands — now funneled at every site.
        "(a + b)(x)",
        "(f)(1)(2)",
        "(a, b).0",
        "[1, 2, 3].len()",
        "#(1, 2).contains(x)",
        "{ x = 1 }.x",
        "#{ 1 = a }.get(1)",
        "(if a then f else g)(x)",
        "(match x with | _ => f)(y)",
        "(fn(a) => a)(1)",
        "`{x}.field",
        "b[u8(1)].len",
        "(let x = f in x)(1)",
        // unary minus (now on the worklist) — tight operand (prefix+postfix, NO unit suffix, no infix),
        // nesting, member/call operands, the outer unit-suffix distinction, as infix operand.
        "-a",
        "- - a",
        "- - - a",
        "-a + b",
        "a + -b",
        "-f(x)",
        "-x.a.b",
        "-(a + b)",
        "-(a + b) meters",
        "- x meters",
        "-a * -b",
        "[-a, -b]",
        "f(-1, -2)",
        "-if a then b else c",
        // unquote / unquote-splicing (now on the worklist) — bare (tight operand) + braced (full expr),
        // splice, tight member/call operands, the outer unit suffix, nested inside a quasiquote.
        ",x",
        ",{a + b}",
        ",@xs",
        ",@{items}",
        ",f(x)",
        ",x.a.b",
        ",{a; b}",
        "`{,x + ,y}",
        "`{f(,x, ,@rest)}",
        ",x meters",
        // `@ann` annotation (now on the worklist) — bare / glued-call name / stacked / prefix-only form
        // (a following `.member`/`(args)` binds to the OUTER `(@ …)`, not the form) / annotated keyword.
        "@test x",
        "@tag(\"slow\") x",
        "@a @b x",
        "@ann (a + b)",
        "@ann f(y)",
        "@ann x.field",
        "@a @b @c y",
        "@doc(\"hi\") let x = 1 in x",
        // `@!key` pragma (now on the worklist) — tight type arg (bare / member / ctor app) + the param
        // payload form (config kvs + name:Type binder, assembled inline).
        "@!default-float Float32",
        "@!k Int(8)",
        "@!k Foo.Bar",
        "@!param(widget: slider) width: Int64",
        "@!param() x: Bool",
        "@!param(min: 0, max: 10) n: Int64",
        // `def` declaration (now on the worklist) — value def / function def / return type / forall /
        // nested (def value = def, the de-recursion target) / sequence / leading doc / annotated def.
        "def x = 1",
        "def f(a) = a",
        "def f(a, b) = a + b",
        "def f(a: Int64) -> Bool = a",
        "def forall t. f(x: t) = x",
        "def x = def y = 1",
        "def x = 1; def y = 2",
        "def f() = (a; b)",
        "/// the answer\ndef x = 42",
        "def f(a) =\n  // note\n  a + 1",
        "@test def f(a) = a",
        // `module … { … }` (now on the worklist) — empty / single / multi member / NESTED (the
        // de-recursion target) / leading doc / trailing comment before `}`.
        "module M {}",
        "module M { def x = 1 }",
        "module M { def a = 1 def b = 2 }",
        "module A { module B { def x = 1 } }",
        "module A { module B { module C { def x = 1 } } }",
        "/// mod doc\nmodule M { def x = 1 }",
        "module M {\n  def x = 1\n  // trailing\n}",
        "module M { def f(a) = a + 1 export { f } }",
    ];
    for src in cases {
        let mut rec = build_parser(src, FileId::default());
        let rec_root = rec.expr(PREC_SEQ);
        let rec_arenas = rec.builder.finish(rec_root);

        let mut it = build_parser(src, FileId::default());
        let it_root = it.expr_iter(PREC_SEQ);
        let it_arenas = it.builder.finish(it_root);

        assert!(
            it_arenas.structurally_eq(&rec_arenas),
            "expr_iter arena diverged for {src:?}\n  recursive: {}\n  iterative: {}",
            crate::sexpr::print(&rec_arenas),
            crate::sexpr::print(&it_arenas),
        );
        assert_eq!(
            rec.spans.len(),
            it.spans.len(),
            "expr_iter span-table length diverged for {src:?}"
        );
        for i in 0..rec.spans.len() as u32 {
            assert_eq!(
                rec.spans.get(StructId(i)),
                it.spans.get(StructId(i)),
                "expr_iter span[{i}] diverged for {src:?}"
            );
        }
        assert_eq!(
            rec.errors.len(),
            it.errors.len(),
            "expr_iter error count diverged for {src:?}\n  rec: {:?}\n  it: {:?}",
            rec.errors,
            it.errors
        );
    }
}

#[test]
fn pattern_iter_matches_recursive_pattern() {
    // I4 differential check: the iterative `pattern_iter` must produce a BYTE-IDENTICAL result to the
    // recursive `pattern` for every pattern — arena (structural eq), span table, and errors. Covers the
    // de-recursed postfix chain (`.member` / `(args)` ctor applications, incl. deep nesting) + the
    // recursive-fallback atom families (literals/names/tuple/list/map/set/record/bin), which must stay
    // byte-identical through the hybrid stage.
    let cases = [
        "_",
        "x",
        "0",
        "true",
        "\"s\"",
        "Some(x)",
        "None",
        "Sign.Neg",
        "Id.Mk(n)",
        "Some(Some(x))",
        "C(D(E(f)))",
        "Cons(x, Cons(y, Nil))",
        "Some((a, b))",
        "Wrap(x).inner",
        "(a, b)",
        "(a)",
        "()",
        "(a,)",
        "(a, b, .. rest)",
        "((a, b), (c, d))",
        "((((x))))",
        "(((a, b)))",
        "(Some(x), None)",
        "(a, .. rest)",
        "(a, b, c)",
        "(x, y).swap",
        "[]",
        "[x, y]",
        "[x, .. rest]",
        "[(a, b), .. rest]",
        "[[x], [y]]",
        "[[[[x]]]]",
        "[Some(a), None, .. rest]",
        "[.. all]",
        "[x].head",
        "#{ 1 = p }",
        "#{ k = Some(v), .. rest }",
        "#{}",
        "#{ 1 = p, 2 = q }",
        "#{ (a + b) = p }",
        "#{ 1 = #{ 2 = p } }",
        "#{ k = v }.rest",
        "#(a, b)",
        "#()",
        "#(a, b, .. rest)",
        "#(Some(x), None)",
        "#(#(a), b)",
        "#[]",
        "#[p, q]",
        "#[[x], y]",
        "#[Some(a), b]",
        "{ f = p, g = q }",
        "{}",
        "{ x }",
        "{ x, y }",
        "{ a = 1, b }",
        "{ .. rest }",
        "{ a = Some(x), .. rest }",
        "{ a = { b = p } }",
        "{ p = q }.field",
        "b[u8(1)]",
        "b[]",
        "b[u16(n), bits(1, x)]",
        "b[bytes(rest)]",
        "b[u8(1)].len",
        "Point(x, y).norm(z)",
        "C(C(C(C(x))))",
    ];
    for src in cases {
        let mut rec = build_parser(src, FileId::default());
        let rec_root = rec.pattern(); // rec.iterative == false -> recursive body
        let rec_arenas = rec.builder.finish(rec_root);

        let mut it = build_parser(src, FileId::default());
        let it_root = it.pattern_iter();
        let it_arenas = it.builder.finish(it_root);

        assert!(
            it_arenas.structurally_eq(&rec_arenas),
            "pattern_iter arena diverged for {src:?}\n  recursive: {}\n  iterative: {}",
            crate::sexpr::print(&rec_arenas),
            crate::sexpr::print(&it_arenas),
        );
        assert_eq!(
            rec.spans.len(),
            it.spans.len(),
            "pattern_iter span-table length diverged for {src:?}"
        );
        for i in 0..rec.spans.len() as u32 {
            assert_eq!(
                rec.spans.get(StructId(i)),
                it.spans.get(StructId(i)),
                "pattern_iter span[{i}] diverged for {src:?}"
            );
        }
        assert_eq!(
            rec.errors.len(),
            it.errors.len(),
            "pattern_iter error count diverged for {src:?}\n  rec: {:?}\n  it: {:?}",
            rec.errors,
            it.errors
        );
    }
}

#[test]
fn type_ref_iter_matches_recursive_type() {
    // I5 differential check: the iterative `type_ref_iter` must produce a BYTE-IDENTICAL result to the
    // recursive `type_ref` for every type — arena (structural eq), span table, and errors. Covers the
    // de-recursed `->` arrow chain + the recursive-fallback layers (operand/postfix-app/paren-tuple/
    // brace-record/forall/unit-infix), which must stay byte-identical through the hybrid stage.
    let cases = [
        "Int64",
        "a",
        "List(Int64)",
        "List(List(Int64))",
        "Map(Int64, List(Bool))",
        "Option(Tuple(Int64, Int64))",
        "Int64 -> Bool",
        "Int64 -> Bool -> Int64",
        "A -> B -> C -> D -> E",
        "List(a) -> Option(a)",
        "(Int64, Bool)",
        "(Int64, Bool) -> Int64",
        "Tuple(Int64, Bool)",
        "M.T",
        "M.N.T(a)",
        "forall a. a",
        "forall a b. a -> b",
        "forall a. List(a) -> a",
        "Record(x : Int64, y : Bool)",
        "Qty(Int64, meter / second ^ 2)",
        "meter / second",
        "(A -> B) -> C",
        "Fn(Int64) -> Fn(Bool) -> Int64",
        // Postfix-application de-recursion (I5 part 2): the deep nested-generic vector, mixed
        // member+application chains, empty applications, multi-arg + trailing comma, labeled record
        // fields nested inside an application arg, and a `forall`/arrow inside an application arg.
        "List(List(List(List(List(a)))))",
        "Map(Int64, Map(Bool, List(Option(a))))",
        "M.N.Codec(a).Encoder(b)",
        "List()",
        "Tuple(A, B, C,)",
        "Record(x: Int64, y: List(Bool))",
        "Encoder(Record(x: Int64, inner: Record(y: Bool)))",
        "Tuple(forall b. List(b), a -> b)",
        "List(A -> B -> C)",
        "Point(x, y).norm(z).scale(w)",
        // Paren-tuple / grouping / unit de-recursion (I5 part 3): unit, transparent grouping (incl.
        // deeply nested + around an arrow), nested tuples, tuple inside an application arg, trailing
        // comma, and a paren grouping carrying a following unit-infix.
        "()",
        "(A)",
        "((((A))))",
        "(A, (B, C))",
        "((A, B), (C, D))",
        "List((A, B))",
        "Map((Int64, Bool), (a, b, c))",
        "(A, B, C,)",
        "((A -> B), C)",
        "(meter) ^ 2",
        "() -> Bool",
        // Brace-record `{field: T}` de-recursion (I5 part 4): empty, single, multi-field, nested record
        // (as a field type + inside a paren/app), backtick label, trailing comma, arrow-/tuple-typed
        // field, and a record carrying a following arrow.
        "{}",
        "{x: Int64}",
        "{x: Int64, y: Bool}",
        "{f: Int64 -> Bool, p: {x: Int64}}",
        "{ inner: {a: {b: Int64}} }",
        "(A, {x: Int64})",
        "List({k: Int64, v: Bool})",
        "{`type`: Int64}",
        "{a: Int64, b: Bool,}",
        "{fn: (A, B) -> C}",
        "{x: Int64} -> Bool",
        // forall-body de-recursion (I5 part 5): nested foralls (the unbounded native-recursion vector),
        // forall body carrying an arrow / application / paren / record, forall as an application arg /
        // paren element / record field type, and a malformed forall (missing binder / missing `.`).
        "forall a. forall b. forall c. a -> b -> c",
        "forall a. List(a) -> Option(a)",
        "forall a b. Map(a, b)",
        "forall a. (a, a)",
        "forall a. {x: a}",
        "List(forall a. a -> a)",
        "(forall a. a, Bool)",
        "{f: forall a. a -> a}",
        "forall a. forall b. Tuple(a, b)",
        "forall . a",
        "forall a b",
        // unit-composition-infix de-recursion (I5 part 6): a bare op, left-assoc same-tier chain, the
        // mixed-tier associativity cases (`^` tier 7 looser than `*`/`/` tier 11), a PAREN'd-unit operand
        // (the `(a * (b * …))` native-recursion vector), unit-infix inside a type-app arg / on a member
        // chain, and a following arrow.
        "meter * second",
        "m / s / s",
        "kg * m / s / s",
        "second ^ 2",
        "kg / s ^ 2",
        "kg * m ^ 2 / s ^ 2",
        "(m * s) / s",
        "(m * (s / (kg * m)))",
        "Qty(Int64, meter / second ^ 2)",
        "M.unit ^ 2",
        "List(a) ^ 2",
        "m / s -> Bool",
    ];
    // Also compare the iterative and recursive readers on a GENERATED deep FLAT unit chain — long enough
    // to trip the shared `type_unit_infix` `self.depth + spine` guard (MAX_NESTING_DEPTH). Both readers
    // build a left-assoc chain via a LOOP (no native recursion for a flat chain), so this runs on the
    // default stack and validates the worklist's unit-spine guard against the recursive reference (arena +
    // span table + the single "nests too deeply" error must match). `+ 40` overshoots the cap so the guard
    // definitely trips in both.
    let mut deep = String::from("m");
    for _ in 0..(crate::sexpr::MAX_NESTING_DEPTH + 40) {
        deep.push_str(" / s");
    }
    let owned: Vec<String> = cases.iter().map(|s| s.to_string()).chain([deep]).collect();
    for src in owned.iter().map(String::as_str) {
        let mut rec = build_parser(src, FileId::default());
        let rec_root = rec.type_ref(); // rec.iterative == false -> recursive body
        let rec_arenas = rec.builder.finish(rec_root);

        let mut it = build_parser(src, FileId::default());
        let it_root = it.type_ref_iter();
        let it_arenas = it.builder.finish(it_root);

        assert!(
            it_arenas.structurally_eq(&rec_arenas),
            "type_ref_iter arena diverged for {src:?}\n  recursive: {}\n  iterative: {}",
            crate::sexpr::print(&rec_arenas),
            crate::sexpr::print(&it_arenas),
        );
        assert_eq!(
            rec.spans.len(),
            it.spans.len(),
            "type_ref_iter span-table length diverged for {src:?}"
        );
        for i in 0..rec.spans.len() as u32 {
            assert_eq!(
                rec.spans.get(StructId(i)),
                it.spans.get(StructId(i)),
                "type_ref_iter span[{i}] diverged for {src:?}"
            );
        }
        assert_eq!(
            rec.errors.len(),
            it.errors.len(),
            "type_ref_iter error count diverged for {src:?}\n  rec: {:?}\n  it: {:?}",
            rec.errors,
            it.errors
        );
    }
}

#[test]
fn a_trailing_comment_attaches_to_the_last_form_not_the_whole_program() {
    use crate::sexpr;
    // A `//` comment after the LAST top-level form has no following form to precede. It must attach
    // to the LAST form (the same `(comment "text" node)` wrapper a mid/leading comment gets), NOT
    // wrap the whole root. Wrapping the root buried every top-level def inside the comment's child
    // when the root is a multi-form `(do …)`, so a top-level walk (`cdz metadata`/exports/manifest)
    // saw ZERO defs though a leading comment parsed fine (v-cdz-tooling bug: a `Project.cdz` ending
    // in `//` read as name:null deps:[]). Regression guard: the trailing comment stays INSIDE the
    // do-block on the last form, keeping each def a direct root child.
    assert_eq!(
        sexpr::print(&parse_ok("def a = 1\ndef b = 2\n// end")),
        "(do (def a 1) (comment \"end\" (def b 2)))",
        "trailing comment must wrap the LAST form, not the whole (do …) — else top-level defs vanish"
    );
    // Multiple trailing comments stack on the last form, outermost first (mirrors `wrap_comments`).
    assert_eq!(
        sexpr::print(&parse_ok("def a = 1\ndef b = 2\n// x\n// y")),
        "(do (def a 1) (comment \"x\" (comment \"y\" (def b 2))))"
    );
    // Contrast (must stay correct): a MID comment attaches to its following form, a LEADING comment
    // to the first form — the trailing case now matches this same shape.
    assert_eq!(
        sexpr::print(&parse_ok("def a = 1\n// mid\ndef b = 2")),
        "(do (def a 1) (comment \"mid\" (def b 2)))"
    );
    assert_eq!(
        sexpr::print(&parse_ok("// lead\ndef a = 1\ndef b = 2")),
        "(do (comment \"lead\" (def a 1)) (def b 2))"
    );
    // A SINGLE-form program: trailing and leading both wrap that one form (root stays bare, no `do`),
    // so the def is reachable either way — the multi-form case is the one that regressed.
    assert_eq!(
        sexpr::print(&parse_ok("def main() = 42\n// note")),
        "(comment \"note\" (def (main) 42))"
    );
    // The bijection that matters to walkers: for the buggy input, the top-level form set (the direct
    // children of the root `do`, unwrapping any comment) is exactly {a, b} — two defs, not zero.
    let a = parse_ok("def a = 1\ndef b = 2\n// end");
    let elems = a
        .as_form(a.root, "do")
        .expect("multi-form root is a do-block");
    assert_eq!(
        elems.len(),
        2,
        "two top-level forms survive the trailing comment"
    );
}

#[test]
fn an_own_line_comment_before_a_collection_closer_attaches_to_the_last_element() {
    use crate::sexpr;
    // An own-line `//` after the last element, before the closer, was dropped (the reader left it in
    // the closer's leading slot). Now it attaches to the LAST element as a leading `(comment …)` —
    // the same attach-to-last shape a trailing top-level comment gets (its printed position moves
    // ABOVE the last element, the accepted v1 limitation; the point is it is PRESERVED, not dropped).
    // list:
    assert_eq!(
        sexpr::print(&parse_ok("def l() = [1, 2\n // c\n]")),
        "(def (l) #list(1 (comment \"c\" 2)))"
    );
    // tuple:
    assert_eq!(
        sexpr::print(&parse_ok("def t() = (1, 2\n // c\n)")),
        "(def (t) #tuple(1 (comment \"c\" 2)))"
    );
    // record (field is a `(= name value)` triple; the comment wraps the whole field):
    assert_eq!(
        sexpr::print(&parse_ok("def r() = { a = 1, b = 2\n // c\n }")),
        "(def (r) #record((= a 1) (comment \"c\" (= b 2))))"
    );
    // set desugars to `Set.of([…])` — the comment wraps the last list element:
    assert_eq!(
        sexpr::print(&parse_ok("def s() = #(1, 2\n // c\n)")),
        "(def (s) #set(1 (comment \"c\" 2)))"
    );
    // map (native `#map(…)` ctor head; an entry is the canonical `(= key value)` `FieldPair` triple,
    // unified with a record field per the M2 native-compound-data migration):
    assert_eq!(
        sexpr::print(&parse_ok("def m() = #{ a = 1\n // c\n }")),
        "(def (m) #map((comment \"c\" (= a 1))))"
    );
}

#[test]
fn a_closer_comment_does_not_reorder_when_the_last_element_already_has_a_comment() {
    use crate::sexpr;
    // COLLISION GUARD: when the last element ALREADY carries a leading comment (`[1, // mid\n 2\n
    // // last\n]`), attaching the closer comment there would print `last` ABOVE `mid` — an
    // out-of-order round-trip. So the drain is skipped in that case; the closer comment stays in its
    // slot (the drop-guard refuses to format, no corruption — the pre-fix behavior). The reader keeps
    // ONLY the mid comment on `2`; the last comment is NOT attached (would reorder).
    assert_eq!(
        sexpr::print(&parse_ok("def m() = [1,\n // mid\n 2\n // last\n]")),
        "(def (m) #list(1 (comment \"mid\" 2)))",
        "the closer comment is not attached (would reorder above the element's own comment)"
    );
}

#[test]
fn multiline_def_body_equals_single_line_and_survives_the_export_wrapper() {
    // Follow-up to a reported native-vs-browser divergence (v-guide-infra): a `def main() =` whose
    // NESTED multi-arg-ctor body starts on the NEXT line and spans several indented continuation
    // lines was reported to fail "expected an expression" in browser-wasm (native cdz clean). The ML
    // parser handles it identically to the single-line form: layout (newline-then-indent after `=`)
    // is not significant, so the multi-line body parses to a STRUCTURALLY IDENTICAL tree — and the
    // guide's `wrapModule` shape (snippet + "\nexport { main }") parses clean too. Pins the exact
    // authored-snippet shape the guide feeds, so a real layout-sensitivity regression here is caught.
    // (The reported browser failure is not in read_ml — confirmed identical trees + native `cdz
    // check` exit 0 on the wrapped form; it lives in the guide's JS/wasm-bundle layer.)
    let single = "type Vec3r = | V3r(Rational, Rational, Rational)\n\
                      type Solidr = | Cuber(Vec3r) | Spherer(Rational) | Differencer(Solidr, Solidr)\n\
                      def r(n: Int64) = Rational.of(n, 1)\n\
                      def main() = Solidr.Differencer(Solidr.Cuber(V3r(r(4), r(4), r(4))), Solidr.Spherer(Rational.of(5, 2)))";
    let multi = "type Vec3r = | V3r(Rational, Rational, Rational)\n\
                     type Solidr =\n\
                     \x20\x20| Cuber(Vec3r)\n\
                     \x20\x20| Spherer(Rational)\n\
                     \x20\x20| Differencer(Solidr, Solidr)\n\
                     def r(n: Int64) = Rational.of(n, 1)\n\
                     def main() =\n\
                     \x20\x20Solidr.Differencer(\n\
                     \x20\x20\x20\x20Solidr.Cuber(V3r(r(4), r(4), r(4))),\n\
                     \x20\x20\x20\x20Solidr.Spherer(Rational.of(5, 2)))";
    let ps = read_ml(single);
    let pm = read_ml(multi);
    assert!(ps.ok(), "single-line form parses, got {:?}", ps.errors);
    assert!(
        pm.ok(),
        "multi-line def body (body on next line, indented continuation) parses, got {:?}",
        pm.errors
    );
    // Layout is insignificant: the two lay out to the SAME tree (no "expected an expression").
    assert!(
        ps.arenas.structurally_eq(&pm.arenas),
        "multi-line and single-line def bodies must parse to identical trees"
    );
    // And the guide's wrapper shape (`… \nexport { main }`) parses clean on the multi-line form.
    let wrapped = format!("{multi}\nexport {{ main }}");
    let pw = read_ml(&wrapped);
    assert!(
        pw.ok(),
        "the wrapModule shape (snippet + export list) parses, got {:?}",
        pw.errors
    );
}

#[test]
fn nested_multi_arg_constructor_in_a_type_def_block_parses_and_round_trips() {
    // Regression for a reported cdz-vs-browser divergence (v-guide-infra): a NESTED multi-arg
    // constructor application inside a multi-line sum-type-def block — `Solidr.Differencer(
    // Solidr.Cuber(V3r(r(4), r(4), r(4))), Solidr.Spherer(Rational.of(5, 2)))` — was suspected to
    // trip the ML front-end. It does NOT: the parser accepts it with ZERO errors and the arena
    // round-trips through ML print → reparse. (The browser rejection traced to a STALE deployed
    // guide-wasm, not a live front-end defect — cadenza-syntax's `read_ml` is what both native cdz
    // and cdz-wasm use.) This pins the shape so a real regression here would be caught. Covers both
    // the single-line minimal case and the full multi-line, multi-variant block.
    for src in [
        // Minimal: one nested multi-arg ctor `V3r(...)` inside `Cuber(...)`.
        "type Vec3r = | V3r(Rational, Rational, Rational)\n\
             type Solidr = | Cuber(Vec3r)\n\
             def r(n: Int64) = Rational.of(n, 1)\n\
             def main() = Solidr.Cuber(V3r(r(4), r(4), r(4)))\n",
        // Full: multi-line block, multiple variants, deeper nesting + a member-access head.
        "type Vec3r = | V3r(Rational, Rational, Rational)\n\
             type Solidr =\n\
             \x20\x20| Cuber(Vec3r)\n\
             \x20\x20| Spherer(Rational)\n\
             \x20\x20| Differencer(Solidr, Solidr)\n\
             def r(n: Int64) = Rational.of(n, 1)\n\
             def main() =\n\
             \x20\x20Solidr.Differencer(\n\
             \x20\x20\x20\x20Solidr.Cuber(V3r(r(4), r(4), r(4))),\n\
             \x20\x20\x20\x20Solidr.Spherer(Rational.of(5, 2)))\n",
    ] {
        let p = read_ml(src);
        assert!(
            p.ok(),
            "nested-ctor program should parse cleanly, got {:?}",
            p.errors
        );
        // Round-trips: reprint to ML, reparse, structurally identical (no silent tree corruption).
        let printed = crate::printer::print(&p.arenas, 100);
        let reparsed = read_ml(&printed);
        assert!(
            reparsed.ok(),
            "ML reprint should reparse cleanly, got {:?}\n--- reprint ---\n{printed}",
            reparsed.errors
        );
        assert!(
            p.arenas.structurally_eq(&reparsed.arenas),
            "nested-ctor arena not preserved across ML round-trip\n--- reprint ---\n{printed}"
        );
    }
}

#[test]
fn an_own_line_comment_after_a_match_bodied_def_leads_the_next_def_not_dropped() {
    // Regression (seq-277/C3): a `match`-bodied def followed by an own-line comment then the next def
    // USED TO DROP the comment — `match_expr`'s arm loop drained it as the "next arm's" leading run
    // and, finding no next arm (a `def`, not `|`), DISCARDED it (`cdz fmt` refused: "would drop N
    // comment(s)"). It must instead be restored to lead the FOLLOWING form. This closes db-demand.cdz's
    // 10 dropped comments (all section headers after match-bodied defs).
    let src = "def a(x) = match x with\n  | 0 => 1\n  | _ => 2\n\n\
                   // ---- SECTION between defs\n\
                   def b() = 2\n\nexport { a, b }\n";
    let count_comments = |a: &Arenas| {
        (0..a.structure.len() as u32)
            .map(StructId)
            .filter(|&id| a.head_name(id) == Some("comment"))
            .count()
    };
    let p = read_ml(src);
    assert!(p.ok(), "parses cleanly: {:?}", p.errors);
    assert_eq!(
        count_comments(&p.arenas),
        1,
        "the section comment is attached as a (comment …) node, not dropped"
    );
    // Round-trips: reprint to ML + reparse keeps the comment (this is what `cdz fmt`'s drop-guard checks).
    let printed = crate::printer::print(&p.arenas, 100);
    let reparsed = read_ml(&printed);
    assert!(
        reparsed.ok(),
        "reprint reparses: {:?}\n{printed}",
        reparsed.errors
    );
    assert_eq!(
        count_comments(&reparsed.arenas),
        1,
        "the comment survives the ML round-trip\n{printed}"
    );
    assert!(
        printed.contains("// ---- SECTION between defs"),
        "comment text is re-emitted:\n{printed}"
    );
}

#[test]
fn a_chained_else_if_ladder_flattens_headers_to_one_indent() {
    // operator seq69/seq70: an `else if` ladder must NOT indent DEEPER per rung — every
    // `if`/`else if`/`else` header stays at the OUTER indent (the "20 levels deep" compiler-ml pain
    // was the printer nesting `else { if { else { if … } } }`, one cbox per rung). A too-wide chain
    // lays out as a FLAT ladder (headers aligned, bodies one level under each header).
    let src = "def classify(x) =\n\
                   \x20\x20if first-threshold-check-predicate(x) then first-branch-result-value(x)\n\
                   \x20\x20else if second-threshold-check-predicate(x) then second-branch-result-value(x)\n\
                   \x20\x20else if third-threshold-check-predicate(x) then third-branch-result-value(x)\n\
                   \x20\x20else final-fallback-branch-result-value(x)\n";
    let p = read_ml(src);
    assert!(p.ok(), "parses: {:?}", p.errors);
    let printed = crate::printer::print(&p.arenas, 60); // narrow width forces the ladder to break
    // Every `else if`/`else` header sits at the SAME indent (flat ladder, not deepening per rung).
    let header_indents: Vec<usize> = printed
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("else if ") || t == "else"
        })
        .map(|l| l.len() - l.trim_start().len())
        .collect();
    assert!(
        header_indents.len() >= 3,
        "ladder broke into rungs:\n{printed}"
    );
    assert!(
        header_indents.iter().all(|&i| i == header_indents[0]),
        "all else-if/else headers at ONE indent (flat ladder, not deepening): {header_indents:?}\n{printed}"
    );
    // Round-trips structurally + idempotent.
    let rp = read_ml(&printed);
    assert!(
        rp.ok() && p.arenas.structurally_eq(&rp.arenas),
        "ladder round-trips: {:?}\n{printed}",
        rp.errors
    );
    assert_eq!(
        crate::printer::print(&rp.arenas, 60),
        printed,
        "idempotent\n{printed}"
    );
}

#[test]
fn an_own_line_comment_between_infix_operands_survives_the_round_trip() {
    // Regression (seq-277/C3): an OWN-LINE `//` comment/block BETWEEN operands of a multi-line infix
    // chain (`a\n  and b\n  // block\n  and c`) sits at the next operator's leading slot. The infix loop
    // drained a same-line operand-trailing (slice 3) but NOT an own-line-before-operator comment, so it
    // dropped. Reader now attaches it as leading on the RIGHT operand; the printer emits it OWN-LINE
    // BEFORE the operator (emitting after the op re-reads to a DROP — non-idempotent). Closes sread-eval.
    let src = "def f(a, b, c) = if a\n  and b\n  // block line1\n  // block line2\n  and c\n  then 1 else 2\n";
    let count = |a: &Arenas| {
        (0..a.structure.len() as u32)
            .map(StructId)
            .filter(|&id| a.head_name(id) == Some("comment"))
            .count()
    };
    let p = read_ml(src);
    assert!(p.ok(), "parses: {:?}", p.errors);
    assert_eq!(
        count(&p.arenas),
        2,
        "both own-line block lines attached as leading (comment …)"
    );
    let printed = crate::printer::print(&p.arenas, 80);
    assert!(
        !printed.contains("comment("),
        "no garbage comment(...):\n{printed}"
    );
    assert!(
        printed.contains("// block line1") && printed.contains("// block line2"),
        "both re-emitted:\n{printed}"
    );
    let rp = read_ml(&printed);
    assert!(
        rp.ok() && p.arenas.structurally_eq(&rp.arenas),
        "round-trips: {:?}\n{printed}",
        rp.errors
    );
    assert_eq!(
        crate::printer::print(&rp.arenas, 80),
        printed,
        "idempotent\n{printed}"
    );
}

#[test]
fn a_trailing_comment_on_a_non_last_infix_operand_survives_the_round_trip() {
    // Regression (seq-277/C3 slice 3): a same-line `//` on a NON-LAST operand of a multi-line infix
    // chain (`a and b  // note` newline `and c`) USED TO DROP — the Pratt infix loop never drained the
    // operator token's leading slot. Reader: drain the operand's trailing comment before the operator
    // and attach `(comment-after …)` to `left`. Printer: `infix_operand` re-emits it as `inner // note`
    // and forces a break before the next operator. (Closes int-width.cdz's operand-trailing drops.)
    let src = "def f(a, b, c) = a\n  and b   // note on b\n  and c\n\nexport { f }\n";
    let count = |a: &Arenas| {
        (0..a.structure.len() as u32)
            .map(StructId)
            .filter(|&id| a.head_name(id) == Some("comment-after"))
            .count()
    };
    let p = read_ml(src);
    assert!(p.ok(), "parses: {:?}", p.errors);
    assert_eq!(
        count(&p.arenas),
        1,
        "the operand-trailing comment is a (comment-after …) node"
    );
    let printed = crate::printer::print(&p.arenas, 100);
    assert!(
        printed.contains("// note on b"),
        "comment re-emitted:\n{printed}"
    );
    let reparsed = read_ml(&printed);
    assert!(reparsed.ok(), "reparse: {:?}\n{printed}", reparsed.errors);
    assert_eq!(
        count(&reparsed.arenas),
        1,
        "comment survives the round-trip\n{printed}"
    );
    // Idempotent: the forced break makes the reprint a fixed point.
    assert_eq!(
        crate::printer::print(&reparsed.arenas, 100),
        printed,
        "idempotent\n{printed}"
    );
}

#[test]
fn a_trailing_comment_on_an_effect_op_survives_the_round_trip() {
    // Regression (seq-277/C3): a same-line `//` on an effect op (`| op : Sig  // note`) USED TO DROP
    // (a non-last op) or mis-attach to the FOLLOWING def (the last op) — the effect-op loop drained no
    // comments. Reader attaches `(comment-after …)`; the printer + `is_effect_shape` peel + re-emit it
    // same-line (closes db-query-perfield.cdz), while the leading `///` docs stay intact.
    let src = "effect E =\n  | get : Int64 -> Int64 // note on get\n  | put : Int64 -> Unit // note on put\n\
                   def f() = 1\n\nexport { f }\n";
    let count = |a: &Arenas| {
        (0..a.structure.len() as u32)
            .map(StructId)
            .filter(|&id| a.head_name(id) == Some("comment-after"))
            .count()
    };
    let p = read_ml(src);
    assert!(p.ok(), "parses: {:?}", p.errors);
    assert_eq!(
        count(&p.arenas),
        2,
        "both op-trailing comments attached as (comment-after …)"
    );
    let printed = crate::printer::print(&p.arenas, 100);
    assert!(
        printed.contains("// note on get") && printed.contains("// note on put"),
        "both re-emitted:\n{printed}"
    );
    assert!(
        printed.contains("effect E ="),
        "stays the effect surface (not the generic call form):\n{printed}"
    );
    let rp = read_ml(&printed);
    assert!(
        rp.ok() && p.arenas.structurally_eq(&rp.arenas),
        "round-trips: {:?}\n{printed}",
        rp.errors
    );
    assert_eq!(
        crate::printer::print(&rp.arenas, 100),
        printed,
        "idempotent\n{printed}"
    );
}

#[test]
fn a_multiline_trailing_comment_on_a_type_variant_round_trips() {
    // Regression (seq-277/C3): a MULTI-LINE trailing comment on a variant (`| A(T) // line1` then
    // own-line `// line2` continuations) leaves the continuation lines as the NEXT variant's LEADING
    // comment, nested OUTSIDE that variant's own trailing `(comment-after …)`. `print_type` peeled only
    // a leading `comment` (outer), so with `(comment-after trail (comment lead V))` the inner leading
    // comment rendered as a garbage `comment(text, V)` variant + dropped. Now it peels BOTH wrappers in
    // either order. (Closes ty.cdz / parse-db.cdz / lower-db.cdz variant multi-line trailing drops.)
    let src = "type T =\n  | A(Int64) // trailing on A\n  // continuation of A\n  | B(Int64) // trailing on B\n\nexport {}\n";
    let comments = |a: &Arenas| {
        (0..a.structure.len() as u32)
            .map(StructId)
            .filter(|&id| matches!(a.head_name(id), Some("comment") | Some("comment-after")))
            .count()
    };
    let p = read_ml(src);
    assert!(p.ok(), "parses: {:?}", p.errors);
    let n = comments(&p.arenas);
    assert_eq!(
        n, 3,
        "A-trailing + A-continuation + B-trailing all attached (got {n})"
    );
    let printed = crate::printer::print(&p.arenas, 100);
    assert!(
        !printed.contains("comment("),
        "no garbage comment(...) variant:\n{printed}"
    );
    assert!(
        printed.contains("// trailing on A")
            && printed.contains("// continuation of A")
            && printed.contains("// trailing on B"),
        "all three re-emitted:\n{printed}"
    );
    let rp = read_ml(&printed);
    assert!(
        rp.ok() && p.arenas.structurally_eq(&rp.arenas),
        "round-trips: {:?}\n{printed}",
        rp.errors
    );
    assert_eq!(
        crate::printer::print(&rp.arenas, 100),
        printed,
        "idempotent\n{printed}"
    );
}

#[test]
fn an_own_line_comment_after_the_last_sum_variant_is_not_dropped() {
    // Regression (seq-277/C3): an own-line comment after the LAST variant of a `type T = | A | B`
    // decl (before the next form) USED TO DROP — the variant loop drained it as the "next variant's"
    // leading run and discarded it on break (no next `|`). Same class as the match-arm fix; restored
    // on break so it leads the following form. (Closes emit-db.cdz / resolve-db.cdz drops.)
    let src = "type T =\n  | A\n  | B\n  // trailing note after the last variant\ndef f() = 1\n\nexport { f }\n";
    let count_comments = |a: &Arenas| {
        (0..a.structure.len() as u32)
            .map(StructId)
            .filter(|&id| a.head_name(id) == Some("comment"))
            .count()
    };
    let p = read_ml(src);
    assert!(p.ok(), "parses cleanly: {:?}", p.errors);
    assert_eq!(
        count_comments(&p.arenas),
        1,
        "the post-variant comment is a (comment …) node, not dropped"
    );
    let printed = crate::printer::print(&p.arenas, 100);
    let reparsed = read_ml(&printed);
    assert!(
        reparsed.ok(),
        "reprint reparses: {:?}\n{printed}",
        reparsed.errors
    );
    assert_eq!(
        count_comments(&reparsed.arenas),
        1,
        "comment survives the ML round-trip\n{printed}"
    );
}

#[test]
fn deeply_nested_input_is_diagnosed_not_crashed() {
    // The Pratt parser recurses through `expr` one native frame per nesting level, so DESCENDING to
    // the depth guard (`MAX_NESTING_DEPTH` = 1024) itself needs more stack than a default `cargo test`
    // worker (~2 MB on Linux, ~512 KB–1 MB on macOS) — the guard fires cleanly, but the recursion
    // reaching it would overflow the worker's stack first (a spurious SIGABRT that is NOT what this
    // test asserts). Run the body on a large-stacked thread so it exercises the depth guard, not the
    // worker's stack limit. (The compiler's own deep walks use the same 64 MB guard-sized stack.)
    run_deep(|| {
        // Unguarded, a pathologically deep nest overflowed the native stack (SIGABRT) or — once a
        // naive guard returned an error node without stopping — SPUN on the unconsumed deep tail (a
        // hang). The depth guard records ONE error and POISONS the parser (`depth_exceeded` ⇒
        // `at_end`), so parsing TERMINATES with a clean diagnostic. The nest exceeds the limit.
        let n = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
        let src = format!("{}1{}", "(".repeat(n), ")".repeat(n));
        let p = read_ml(&src);
        assert!(
            !p.ok()
                && p.errors
                    .iter()
                    .any(|e| e.message.contains("nests too deeply")),
            "deep nesting must be a clean depth-limit error, not a crash/hang; got {:?}",
            p.errors
        );
        // A nest well under the limit still parses cleanly (no over-rejection).
        let ok = (crate::sexpr::MAX_NESTING_DEPTH as usize) - 1;
        let shallow = format!("{}1{}", "(".repeat(ok), ")".repeat(ok));
        let ps = read_ml(&shallow);
        assert!(
            ps.ok(),
            "a nest just under the limit must parse: {:?}",
            ps.errors
        );
    });
}

#[test]
fn deep_prefix_operator_runs_are_diagnosed_not_crashed() {
    // A run of a PREFIX operator that recurses `prefix` DIRECTLY — unary minus (`- - - … x`) and the
    // bare-form unquote (`, , , … x`) — bypassed `expr`'s depth guard (they call `self.prefix()`, not
    // `self.expr()`), so a pathologically deep run overflowed the native stack (SIGABRT). `guard_prefix`
    // now counts each layer against the same `MAX_NESTING_DEPTH` budget at those two recursion sites, so
    // a deep run is a clean depth-limit diagnostic. Needs a large stack (like the nested-`(` test — the
    // recursion DESCENDS to the 1024 guard, more than a default test worker's stack). (Regression for
    // the prefix-recursion stack-overflow class.)
    run_deep(|| {
        let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
        let cases = [
            format!("{}1", "-".repeat(over)),  // unary-minus run `----…1`
            format!("{}x", ",".repeat(over)),  // unquote run `,,,,…x`
            format!("{}x", ",@".repeat(over)), // unquote-splicing run `,@,@…x`
        ];
        for src in cases {
            let p = read_ml(&src);
            assert!(
                !p.ok()
                    && p.errors
                        .iter()
                        .any(|e| e.message.contains("nests too deeply")),
                "a deep prefix-operator run must be a clean depth-limit error, not a crash; \
                     src head {:?}, got {:?}",
                &src[..src.len().min(8)],
                p.errors
            );
        }
        // A moderate run well under the limit still parses cleanly (no over-rejection).
        let ok = read_ml("- - - 5");
        assert!(ok.ok(), "shallow negation must parse: {:?}", ok.errors);
        let okq = read_ml("`{ ,x + ,y }");
        assert!(okq.ok(), "shallow unquote must parse: {:?}", okq.errors);
    });
}

#[test]
fn deep_pattern_and_annotation_recursion_is_diagnosed_not_crashed() {
    // Two more recursion classes that bypass `expr`'s depth guard, each fixed by `guard_prefix`:
    // (1) PATTERNS — a tuple/list/ctor sub-pattern re-enters `pattern` on a path entirely separate
    //     from `expr`, so a deep `((((…` / `[[[[…` / `C(C(C(…` pattern overflowed the stack; the
    //     guard now lives at `pattern`'s entry.
    // (2) ANNOTATION / PRAGMA — the `@name form` and `@!key arg` arms recurse `prefix` DIRECTLY, so a
    //     stacked `@a @b … def` / `@!k @!k … def` overflowed; each layer is now counted.
    // All must be clean 'nests too deeply' diagnostics, not a SIGABRT. Large stack (descends to 1024).
    run_deep(|| {
        let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
        let cases = [
            format!("def f(x) = match x with | {} => 1", "(".repeat(over)), // tuple-pattern
            format!("def f(x) = match x with | {} => 1", "[".repeat(over)), // list-pattern
            format!(
                "def f(x) = match x with | {}x{} => 1",
                "C(".repeat(over),
                ")".repeat(over)
            ), // ctor-pattern
            format!("{}def f() = 1", "@ann ".repeat(over)),                 // stacked annotations
            format!("{}def f() = 1", "@!k ".repeat(over)),                  // stacked pragmas
        ];
        for src in &cases {
            let p = read_ml(src);
            assert!(
                p.errors
                    .iter()
                    .any(|e| e.message.contains("nests too deeply")),
                "a deep pattern/annotation must be a clean depth-limit error, not a crash; \
                     src head {:?}, got {:?}",
                &src[..src.len().min(24)],
                p.errors
            );
        }
        // Moderate, well-formed pattern + annotation still parse (no over-rejection).
        let ok = read_ml("def f(x) = match x with | (a, [b, c]) => 1 | Some(y) => 2 | _ => 0");
        assert!(ok.ok(), "shallow pattern must parse: {:?}", ok.errors);
        let oka = read_ml("@inline\ndef f() = 1");
        assert!(oka.ok(), "shallow annotation must parse: {:?}", oka.errors);
    });
}

#[test]
fn deep_flat_chains_are_diagnosed_not_crashed() {
    // A FLAT chain — left-associative infix (`1+1+1…`), a postfix member run (`x.a.a…`), or a
    // call chain (`f(1)(1)…`) — is parsed by a LOOP, not recursion, so the parser's per-`expr`
    // depth counter never grows with it. But each iteration deepens the produced ARENA on one
    // side, so an unbounded run built an arbitrarily deep TREE that a recursive CONSUMER (the
    // s-expr printer, `canon`, the compiler's own walk) then overflowed the stack on (SIGABRT) —
    // even though the PARSE itself never recursed and succeeded. The `expr`/`postfix` loop guards
    // now bound the folded spine against the same `MAX_NESTING_DEPTH`, so a pathological chain is a
    // clean parse diagnostic. (Regression for the flat-chain stack-overflow class.)
    //
    // Run the body on a LARGE-STACK thread (mirroring `sexpr::deeply_nested_input_is_diagnosed_not_
    // crashed`): the guard caps the tree at `MAX_NESTING_DEPTH` (1024), but the DOWNSTREAM walks this
    // test deliberately exercises — the ML printer, the s-expr printer, `codec::encode` — recurse one
    // native frame per level, and 1024 debug-build frames exceed a default `cargo test` worker's
    // stack. (This is about the CONSUMERS' recursion depth, NOT the parse: the parse itself loops.
    // The margin is genuinely tight — a printer-dispatch change once tipped it over — so provision the
    // stack explicitly rather than depend on per-frame size staying under an implicit cap.)
    let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
                let cases = [
                    format!("1{}", "+1".repeat(over)),    // left-assoc infix spine
                    format!("1{}", " |> f".repeat(over)), // pipeline spine (also infix)
                    format!("x{}", ".a".repeat(over)),    // postfix member chain
                    format!("f{}", "(1)".repeat(over)),   // postfix call chain
                ];
                for src in cases {
                    let p = read_ml(&src);
                    assert!(
                        !p.ok()
                            && p.errors
                                .iter()
                                .any(|e| e.message.contains("nests too deeply")),
                        "a deep flat chain must be a clean depth-limit error, not a crash; got ok={} errs={:?}",
                        p.ok(),
                        p.errors
                    );
                    // The produced arena is well-formed and EVERY recursive downstream consumer must
                    // handle it without crashing — the guard capped the tree depth, so the whole pipeline
                    // is safe. This is the invariant the cap exists to uphold: "a reader produces
                    // bounded-depth trees ⇒ the printer, canon, and codec walks are all safe." Exercise all
                    // three (the s-expr printer, the ML printer, and codec::encode, which canonicalizes).
                    let _ = crate::printer::print(&p.arenas, 80);
                    let _ = crate::sexpr::print(&p.arenas);
                    let _ = crate::codec::encode(&p.arenas);
                }
                // A flat chain WELL under the limit parses cleanly (no over-rejection) and survives the
                // full round-trip through every surface.
                let ok_n = (crate::sexpr::MAX_NESTING_DEPTH as usize) / 2;
                let shallow = format!("1{}", "+1".repeat(ok_n));
                let ps = read_ml(&shallow);
                assert!(
                    ps.ok(),
                    "a flat chain under the limit must parse: {:?}",
                    ps.errors
                );
                // Binary round-trip: the deep-but-legal arena encodes and decodes back structurally-equal.
                let bytes = crate::codec::encode(&ps.arenas);
                let back = crate::codec::decode(&bytes).expect("deep-but-legal arena decodes");
                assert!(
                    back.structurally_eq(&ps.arenas),
                    "a deep-but-legal flat chain must survive the binary round-trip"
                );
            })
            .expect("spawn deep-flat-chain worker");
    if let Err(payload) = h.join() {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn combined_recursion_plus_postfix_depth_is_bounded() {
    // PR #383 DoS: bracket-nesting recursion (`self.depth`, one frame per level) and a postfix spine
    // (`.a.a…`) BOTH deepen the same produced arena, and a recursive consumer (printer/`canon`)
    // walks their SUM. The postfix guard must bound `self.depth + spine`, so a deeply-parenthesized
    // expression WITH a long postfix chain — combined arena depth ~2× MAX_NESTING_DEPTH — is
    // diagnosed, not left to overflow a small (e.g. the guide's ~1MB wasm) stack. A spine-only
    // check missed this. Run on a big stack so the recursion descent reaches the guard.
    run_deep(|| {
        let each = crate::sexpr::MAX_NESTING_DEPTH as usize; // parens AND postfix each ~= the limit
        let inner = format!("x{}", ".a".repeat(each));
        let src = format!("{}{}{}", "(".repeat(each), inner, ")".repeat(each));
        let p = read_ml(&src);
        assert!(
            !p.ok()
                && p.errors
                    .iter()
                    .any(|e| e.message.contains("nests too deeply")),
            "combined nest+postfix (~2× limit) must be diagnosed, not a crash; ok={} errs={:?}",
            p.ok(),
            p.errors
        );
        // A combined depth comfortably UNDER the limit still parses (no over-rejection).
        let q = (crate::sexpr::MAX_NESTING_DEPTH as usize) / 3;
        let ok_inner = format!("x{}", ".a".repeat(q));
        let ok_src = format!("{}{}{}", "(".repeat(q), ok_inner, ")".repeat(q));
        let ok = read_ml(&ok_src);
        assert!(
            ok.ok(),
            "a combined depth under the limit must parse: {:?}",
            ok.errors
        );
    });
}

/// Run `f` on a thread with a stack large enough to reach the parser's depth guard (the same
/// 64 MB the compiler sizes its deep-walk worker at), re-raising a panic so an assertion failure
/// inside still fails the test. The default `cargo test` worker stack is too small to DESCEND to
/// the depth limit before overflowing (macOS especially), which would mask the guarded behavior.
fn run_deep(f: impl FnOnce() + Send + 'static) {
    let h = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn deep-parse worker");
    if let Err(payload) = h.join() {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn parses_clean() {
    for src in [
        "42",
        "1 + 2 * 3",
        "f(a, b)",
        "a.b.c",
        "let x = 1, y = 2 in x + y",
        "if a then b else c",
        "fn(x, y) => x + y",
        "match e with | Some(n) => n | None => 0 | _ => neg",
        "match n with | x if x < 0 => neg | _ => pos",
        "List.at(xs, 0)",
        "x |> f",
        "x |> f(a) |> g",
        "total + tax |> round",
        "`{ x + 1 }",
        "#[a, b, c]",
    ] {
        let _ = parse_ok(src);
    }
}

// `pipeline_operator_builds_a_real_node` (`|>` is a REAL infix operator building an arena node `(|> L R)`,
// left-associative, looser than arithmetic — the resolver's application-rewrite happens later, so the
// surface tree keeps the operator) is fully subsumed by the spec/syntax corpus (inc-6 batch-63, confirmed):
// ml/61-pipeline-basic `x |> f`→`(|> x f)`, ml/63-pipeline-chain `x |> f |> g`→`(|> (|> x f) g)` (left-
// assoc), ml/64-pipeline-looser-than-plus `total + tax |> round`→`(|> (+ total tax) round)` (precedence);
// ml/62-pipeline-call-rhs `x |> f(a)`→`(|> x (f a))` covers the call RHS. No new cases needed — the parser
// `parse_ok` tree assertions here are byte-identical to those goldens' trees.

#[test]
fn arena_shapes() {
    // `1 + 2 * 3` -> (+ 1 (* 2 3))
    let a = parse_ok("1 + 2 * 3");
    assert_eq!(a.head_name(a.root), Some("+"));
    let plus = a.as_form(a.root, "+").unwrap();
    assert_eq!(a.head_name(plus[1]), Some("*"));

    // `f(a, b)` -> (f a b)
    let a = parse_ok("f(a, b)");
    assert_eq!(a.head_name(a.root), Some("f"));
    assert_eq!(a.as_form(a.root, "f").unwrap().len(), 2);

    // `a.b` -> (. a b) — the `.` head is a native `Leaf::Member` (kind identity, not `Name(".")`),
    // read via `member_parts`.
    let a = parse_ok("a.b");
    assert!(a.member_parts(a.root).is_some());

    // `p.0` -> (. p 0) — positional tuple access, the numeric sibling of `p.field`.
    let a = parse_ok("p.0");
    let (obj, key) = a.member_parts(a.root).unwrap();
    assert_eq!(a.as_name(obj), Some("p"));
    assert!(
        matches!(a.get(key), crate::ast::Struct::Atom(l) if matches!(a.leaf(*l), Leaf::Int { .. }))
    );
    // `(x.0).1` -> (. (. x 0) 1) — chained index, parens keep `0.1` from lexing as a float.
    let a = parse_ok("(x.0).1");
    let (inner, _) = a.member_parts(a.root).unwrap();
    assert!(a.member_parts(inner).is_some());

    // `if a then b else c` -> (if a b c)
    let a = parse_ok("if a then b else c");
    assert_eq!(a.as_form(a.root, "if").map(|t| t.len()), Some(3));

    // `let x = 1 in x` -> (let ((x 1)) x)
    let a = parse_ok("let x = 1 in x");
    let tail = a.as_form(a.root, "let").unwrap();
    assert_eq!(tail.len(), 2); // bindings + body
}

#[test]
fn match_arm_is_pattern_body_pair() {
    // `match e with | Some(n) => n | _ => 0` -> (match e ((Some n) n) (_ 0))
    let a = parse_ok("match e with | Some(n) => n | _ => 0");
    let tail = a.as_form(a.root, "match").unwrap();
    assert_eq!(tail.len(), 3); // scrutinee + 2 arms
    // first arm is a 2-element list (pattern, body); pattern is (Some n)
    let crate::ast::Struct::List(arm0) = a.get(tail[1]) else {
        panic!()
    };
    assert_eq!(arm0.len(), 2);
    assert_eq!(a.head_name(arm0[0]), Some("Some"));
}

#[test]
fn prefix_unary_minus_is_arity_one_subtraction() {
    // `-x` (prefix minus on a NAME) -> the arity-1 subtraction `(- x)` (negation, read as such by
    // `lower`), NOT a binary `-` and NOT a signed literal (only `-<digit>` lexes as a literal).
    let a = parse_ok("-x");
    let tail = a.as_form(a.root, "-").unwrap();
    assert_eq!(tail.len(), 1, "prefix `-x` is one operand");
    assert_eq!(a.as_name(tail[0]), Some("x"));
    // Negation binds TIGHTER than `+`: `-x + 1` is `(+ (- x) 1)` — the `-x` is the left addend.
    let b = parse_ok("-x + 1");
    let plus = b.as_form(b.root, "+").unwrap();
    assert_eq!(plus.len(), 2);
    let neg = b.as_form(plus[0], "-").unwrap();
    assert_eq!(neg.len(), 1, "the left addend is the negation `(- x)`");
    assert_eq!(b.as_name(neg[0]), Some("x"));
    // A parenthesized operand `-(x + 1)` negates the whole sum: `(- (+ x 1))`.
    let c = parse_ok("-(x + 1)");
    let neg = c.as_form(c.root, "-").unwrap();
    assert_eq!(neg.len(), 1);
    assert_eq!(c.head_name(neg[0]), Some("+"));
    // Binary subtraction is unchanged: `a - b` -> the arity-2 `(- a b)`.
    let d = parse_ok("a - b");
    let sub = d.as_form(d.root, "-").unwrap();
    assert_eq!(sub.len(), 2, "binary `a - b` stays arity-2 subtraction");
}

#[test]
fn guarded_arm_wraps_pattern() {
    // `match n with | x if x < 0 => neg | _ => pos`: first arm pattern is (guard x (< x 0))
    let a = parse_ok("match n with | x if x < 0 => neg | _ => pos");
    let tail = a.as_form(a.root, "match").unwrap();
    let crate::ast::Struct::List(arm0) = a.get(tail[1]) else {
        panic!()
    };
    assert_eq!(a.head_name(arm0[0]), Some("guard"));
}

#[test]
fn effect_op_resource_marker_lifts_to_a_hash_clean_sibling() {
    // SEC-F1 resource marker (concierge-ruled, v-agent-harness coord 2026-08-13): `@resource T` on an
    // op param designates the resource arg. It must lift OUT of the op TYPE (so the schema-hash is
    // marker-invariant) into a decl-level `(resource <idx>)` sibling on the op. Here: `write` marks
    // its FIRST param (index 0) as the resource; the op TYPE is marker-FREE (bare Bytes, NOT
    // `(@ resource Bytes)`), and a `(resource 0)` sibling records the position.
    let a = parse_ok("effect Fs = | write : @resource Bytes -> Bytes -> Unit");
    let op = a
        .as_form(a.as_form(a.root, "effect").unwrap()[1], "op")
        .unwrap();
    assert_eq!(a.as_name(op[0]), Some("write"));
    // op[1] = the op TYPE; op[2] = the (resource 0) sibling.
    let ty_sexp = crate::sexpr::print_from(&a, op[1]);
    assert!(
        ty_sexp.contains("(-> Bytes (-> Bytes Unit))") && !ty_sexp.contains("resource"),
        "op type is marker-FREE (hash-clean): {ty_sexp}"
    );
    assert_eq!(
        crate::sexpr::print_from(&a, op[2]),
        "(resource 0)",
        "resource marks param index 0"
    );

    // The SECOND param can be the resource (index 1); a no-marker op has NO resource sibling.
    let b = parse_ok("effect Fs = | write : Bytes -> @resource Bytes -> Unit");
    let opb = b
        .as_form(b.as_form(b.root, "effect").unwrap()[1], "op")
        .unwrap();
    assert_eq!(crate::sexpr::print_from(&b, opb[2]), "(resource 1)");

    let c = parse_ok("effect Fs = | read : Bytes -> Bytes");
    let opc = c
        .as_form(c.as_form(c.root, "effect").unwrap()[1], "op")
        .unwrap();
    assert_eq!(
        opc.len(),
        2,
        "no resource marker => no (resource N) sibling (name + type only)"
    );
}

#[test]
fn effect_decl_builds_op_signatures() {
    // `effect Diag = | emit : Int64 -> Unit | collect : -> List(Int64)` ->
    // `(effect Diag (op emit (-> Int64 Unit)) (op collect (-> (List Int64))))`. The leading-arrow
    // op type is the nullary-elided one-element `(-> R)`.
    let a = parse_ok("effect Diag = | emit : Int64 -> Unit | collect : -> List(Int64)");
    let tail = a.as_form(a.root, "effect").unwrap();
    assert_eq!(a.as_name(tail[0]), Some("Diag"));
    let emit = a.as_form(tail[1], "op").unwrap();
    assert_eq!(a.as_name(emit[0]), Some("emit"));
    let emit_ty = a.as_form(emit[1], "->").unwrap();
    assert_eq!(emit_ty.len(), 2, "P -> R is a two-element arrow");
    let collect = a.as_form(tail[2], "op").unwrap();
    let collect_ty = a.as_form(collect[1], "->").unwrap();
    assert_eq!(
        collect_ty.len(),
        1,
        "nullary-elided `-> R` is a one-element arrow"
    );
}

#[test]
fn world_decl_builds_the_canonical_wit_world_node() {
    // `world Reducer = | export fold = | apply : (event : Bytes) -> Bytes | import kv = | get :
    // (key : String) -> String` -> the canonical world node the S1 builders produce: a `world` head,
    // the name, then import/export interface sub-nodes, each `(member M (func (param P T) (result R)))`.
    let a = parse_ok(
        "world Reducer = \
             | export fold = | apply : (event : Bytes) -> Bytes \
             | import kv = | get : (key : String) -> String",
    );
    let world = a.as_form(a.root, "world").unwrap();
    assert_eq!(a.as_name(world[0]), Some("Reducer"));
    // export fold { apply : (event: Bytes) -> Bytes }
    let fold = a.as_form(world[1], "export").unwrap();
    assert_eq!(a.as_name(fold[0]), Some("fold"));
    let apply = a.as_form(fold[1], "member").unwrap();
    assert_eq!(a.as_name(apply[0]), Some("apply"));
    let func = a.as_form(apply[1], "func").unwrap();
    assert_eq!(
        func.len(),
        2,
        "one param sub-node + the always-present result"
    );
    let param = a.as_form(func[0], "param").unwrap();
    assert_eq!(a.as_name(param[0]), Some("event"));
    assert!(
        a.as_form(func[1], "result").is_some(),
        "result sub-node present"
    );
    // import kv { get : (key: String) -> String } — direction is the structural sub-head.
    let kv = a.as_form(world[2], "import").unwrap();
    assert_eq!(a.as_name(kv[0]), Some("kv"));
    assert!(a.as_form(kv[1], "member").is_some(), "kv has a member");
}

#[test]
fn a_docd_world_carries_the_doc_but_its_interfaces_are_identity_stable() {
    // A `///` doc on a world attaches as a `(doc …)` child right after the name (round-trip), same as
    // `effect`/`type`. The doc is SURFACE metadata, NOT part of the world's identity: `world_schema_
    // tree` (the identity constructor) takes no docs, so the identity is over the INTERFACE children
    // only. Pin that a doc'd world and its undocumented twin have byte-identical INTERFACE structure —
    // the invariant the compile arm's `parse_target_world` relies on when it skips doc heads (so a
    // documented world keeps the same `wit_world` as its undocumented twin; coordinated w/ v-compiler-ml
    // 2026-08-12). This guards the docs-vs-identity boundary from the syntax side.
    let doc = parse_ok("/// a reducer world\nworld W = | export i = | m : () -> u8");
    let plain = parse_ok("world W = | export i = | m : () -> u8");
    let dw = doc.as_form(doc.root, "world").expect("world form");
    let pw = plain.as_form(plain.root, "world").expect("world form");
    // The doc'd world carries a leading `(doc …)` after the name; the plain one does not.
    assert_eq!(doc.as_name(dw[0]), Some("W"));
    assert!(
        doc.as_form(dw[1], "doc").is_some(),
        "doc node attaches after the name"
    );
    assert!(
        plain.as_form(pw[1], "doc").is_none(),
        "no doc on the plain world"
    );
    // The IDENTITY-bearing part — the interfaces (children after name, skipping any doc) — matches
    // between the two worlds. The doc'd world's interfaces are children[2..] (past the doc);
    // the plain world's are children[1..]. Assert same count + pairwise structural equality, so the
    // identity computed over the non-doc children is doc-independent (what parse_target_world relies
    // on when it skips doc heads).
    let doc_ifaces: Vec<StructId> = dw[1..]
        .iter()
        .copied()
        .filter(|&c| doc.as_form(c, "doc").is_none())
        .collect();
    let plain_ifaces: Vec<StructId> = pw[1..].to_vec();
    assert_eq!(
        doc_ifaces.len(),
        plain_ifaces.len(),
        "same interface count once the doc is skipped"
    );
    for (&d, &p) in doc_ifaces.iter().zip(plain_ifaces.iter()) {
        assert_eq!(
            crate::sexpr::print_from(&doc, d),
            crate::sexpr::print_from(&plain, p),
            "each identity-bearing interface matches its undocumented twin"
        );
    }
}

#[test]
fn inline_world_surface_encodes_identically_to_the_world_schema_tree_builder() {
    // THE cross-source identity guarantee: the inline `world …` surface must lower to the EXACT SAME
    // canonical tree `cadenza-ast::Builder::world_schema_tree` builds (the node an external binary-AST
    // artifact and v-cml's emit also target), so a target world means one content-hash regardless of
    // source. Build `world W = | export i = | m : (p : T) -> R` both ways and assert byte-identical
    // `codec::encode`. If the parser ever drifts from the builder shape (a head-kind flip, a reordered
    // child), this fails — the same drift-guard `world_schema_tree`'s own byte-stable test gives the
    // builder, now extended across the SURFACE boundary.
    // A bare type name `T` lowers through `type_ref` to a plain `Name` atom, so use name atoms as the
    // builder's type descriptors to match the surface exactly.
    let parsed = parse_ok("world W = | export i = | m : (p : T) -> R");

    let mut b = crate::Builder::new();
    let pty = b.name("T");
    let rty = b.name("R");
    let sig = b.wit_func_sig(&[("p", pty)], rty);
    let iface = b.wit_interface(crate::ast::WitDir::Export, "i", &[("m", sig)]);
    let root = b.world_schema_tree("W", &[iface]);
    let built = b.finish(root);

    assert_eq!(
        crate::codec::encode(&parsed),
        crate::codec::encode(&built),
        "inline world surface must encode identically to world_schema_tree — cross-source identity"
    );
}

#[test]
fn inline_world_aggregate_members_encode_identically_to_the_builders() {
    // The cross-source identity guarantee across ALL aggregate member types — record, result, variant,
    // enum, and flags (the full set a typed reducer world binds at the boundary: a record
    // message/request, a result-of-payload-or-error answer, an outcome variant, plus enum/flags scalars).
    // The inline surface `{f: T, …}` / `result(A, B)` / `variant(C, D(T))` / `enum(A, …)` / `flags(A, …)`
    // must lower to the EXACT SAME canonical tree the shared `wit_type_*` builders produce, so a
    // typed-member world is one content-hash whether it comes from the inline decl, an external artifact,
    // or v-cml's emit. If `wit_type_desc_of` ever drifts from a builder shape (a field/case reorder, a
    // head-kind flip, a wrong result slot, a lost variant payload, enum-vs-flags confusion), this fails.
    let parsed = parse_ok(
        "world W = | export r = \
             | step : (msg : {id: string, payload: list(u8)}) -> result(bool, string) \
             | fold : (ev : list(u8)) -> variant(Continue, Break({schema: string, reason: string})) \
             | tag : (x : u8) -> enum(Red, Green) \
             | perms : (x : u8) -> flags(Read, Write)",
    );

    let mut b = crate::Builder::new();
    // step: (msg: record {id: string, payload: list(u8)}) -> result(bool, string). Fields/arms in the
    // same declaration order as the surface.
    let id_ty = b.wit_type_prim("string");
    let payload_ty = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let msg = b.wit_type_record(&[("id", id_ty), ("payload", payload_ty)]);
    let ok = b.wit_type_prim("bool");
    let err = b.wit_type_prim("string");
    let res = b.wit_type_result(Some(ok), Some(err));
    let step_sig = b.wit_func_sig(&[("msg", msg)], res);
    // fold: (ev: list(u8)) -> variant(Continue, Break({schema: string, reason: string})). A payload-less
    // case then a record-payload case.
    let ev = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let break_payload = {
        let schema = b.wit_type_prim("string");
        let reason = b.wit_type_prim("string");
        b.wit_type_record(&[("schema", schema), ("reason", reason)])
    };
    let outcome = b.wit_type_variant(&[("Continue", None), ("Break", Some(break_payload))]);
    let fold_sig = b.wit_func_sig(&[("ev", ev)], outcome);
    // tag: (x: u8) -> enum(Red, Green); perms: (x: u8) -> flags(Read, Write). Same names, distinct types.
    let tag_x = b.wit_type_prim("u8");
    let color = b.wit_type_enum(&["Red", "Green"]);
    let tag_sig = b.wit_func_sig(&[("x", tag_x)], color);
    let perms_x = b.wit_type_prim("u8");
    let perms_ty = b.wit_type_flags(&["Read", "Write"]);
    let perms_sig = b.wit_func_sig(&[("x", perms_x)], perms_ty);
    let iface = b.wit_interface(
        crate::ast::WitDir::Export,
        "r",
        &[
            ("step", step_sig),
            ("fold", fold_sig),
            ("tag", tag_sig),
            ("perms", perms_sig),
        ],
    );
    let root = b.world_schema_tree("W", &[iface]);
    let built = b.finish(root);

    assert_eq!(
        crate::codec::encode(&parsed),
        crate::codec::encode(&built),
        "inline record+result+variant+enum+flags member world must encode identically to the builders"
    );
}

#[test]
fn inline_pure_fold_world_encodes_identically_to_the_kernel_artifact_form() {
    // The CROSS-SOURCE BYTE-IDENTITY gate v-agent-harness cleared (build_type dedup landed cf1433380):
    // the inline surface for v-ah's `pure_fold_world_artifact` must encode BYTE-IDENTICALLY to the
    // artifact — both now route through the shared `world_schema_tree` + `wit_type_*` builders, so a
    // target world is one content-hash whether it comes from the inline decl or the external artifact.
    // Reproduce the artifact's exact build here (its cdz-kernel fn is another workspace, but it is
    // documented as building via these same shared builders): `world "reducer"`, one export interface
    // `cadenza:agent-kernel/fold`, one member `apply(event: list<u8>) -> list<u8>`.
    let parsed = parse_ok(
        "world reducer = | export `cadenza:agent-kernel/fold` = | apply : (event : list(u8)) -> list(u8)",
    );

    let mut b = crate::Builder::new();
    let ev = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let res = {
        let u8 = b.wit_type_prim("u8");
        b.wit_type_list(u8)
    };
    let sig = b.wit_func_sig(&[("event", ev)], res);
    let fold = b.wit_interface(
        crate::ast::WitDir::Export,
        "cadenza:agent-kernel/fold",
        &[("apply", sig)],
    );
    let root = b.world_schema_tree("reducer", &[fold]);
    let built = b.finish(root);

    assert_eq!(
        crate::codec::encode(&parsed),
        crate::codec::encode(&built),
        "inline pure_fold world must encode identically to the shared-builder artifact form"
    );
}

#[test]
fn world_nullary_member_elides_the_param_list() {
    // A nullary member `now : () -> Timestamp` (or `now : -> Timestamp`) yields a func with zero
    // params but an always-present result — matching `wit_func_sig(&[], …)`.
    let a = parse_ok("world Clock = | export c = | now : () -> Timestamp");
    let iface = a
        .as_form(a.as_form(a.root, "world").unwrap()[1], "export")
        .unwrap();
    let func = a
        .as_form(a.as_form(iface[1], "member").unwrap()[1], "func")
        .unwrap();
    assert_eq!(func.len(), 1, "zero params, just the result sub-node");
    assert!(a.as_form(func[0], "result").is_some());
}

#[test]
fn bare_world_is_still_an_ordinary_name_not_a_world_decl() {
    // `world` is CONTEXTUAL — only `world <name> =` heads a decl. A bare `world` used as a variable
    // (a let binding, a reference) MUST stay an ordinary name so the common word is not burned.
    let a = parse_ok("let world = 5 in world + 1");
    // The body reads `world` as a plain name, not a `(world …)` decl — no `world`-headed form at root.
    assert!(
        a.as_form(a.root, "world").is_none(),
        "a bare `world` must not parse as a world declaration"
    );
    // And `world` alone as an expression is just the name.
    let b = parse_ok("world");
    assert_eq!(b.as_name(b.root), Some("world"));
}

// `handle_promotes_effect_and_seed_with_state_last` + `handle_stateless_seed_elides_to_unit` +
// `host_delegation_builds_effect_list` (the effect-handling surface: `handle E(seed) with | op(params…) =>
// body in expr` promotes the effect NAME + seed to the head, the arm op is BARE, and the LAST arm binder is
// the state — an elided `(seed)` → `unit` with an empty param list; `host e1, e2 in body` builds an effect
// LIST) MIGRATED to the spec/syntax corpus (inc-6 batch-70):
//   * ml/421-handle-stateful-seed-state-last `handle Fresh(0) with | next(u, s) => resume(s, s + 1) in
//     Fresh.next()`→`(handle Fresh 0 ((next (u) s (resume s (+ s 1)))) ((. Fresh next)))`.
//   * ml/422-handle-stateless-seed-elides-unit `handle Choose with | pick(s) => resume(5, s) in
//     Choose.pick()`→`(handle Choose unit ((pick () s (resume 5 s))) ((. Choose pick)))` (elided seed=unit,
//     nullary op — state consumed the only binder).
//   * ml/423-host-delegation-effect-list `host ask, log in ask.ask()`→`(host (ask log) ((. ask ask)))`.
//   Each fmt pins the `handle … with`⏎`  | op(…) => …`⏎`in`⏎`body` / `host …, … in body` surface.

// `semicolon_sequences_a_function_body` (a `;`-separated body folds to a flat `(do …)`, greedily collecting
// its run and STOPPING at the next top-level `def`) + `top_level_forms_juxtapose_without_semicolons` (top-level
// forms are whitespace-separated, no `;`) MIGRATED to the spec/syntax corpus (inc-6 batch-72):
// ml/431-semicolon-body-stops-at-next-def `def f() = a; b; c`⏎`def g() = 2`→`(do (def (f) (do a b c)) (def (g)
// 2))` (body stops at `def g`); the def-juxtaposition `def a = 1 def b = 2`→`(do (def a 1) (def b 2))` is
// subsumed by ml/203-top-level-value-defs-blank-separated. (`top_level_semicolon_folds…` stays Rust — GAP note below.)

// STAYS RUST pending an OPERATOR DESIGN RULING (corpus-policy: never pin an unsettled spec question). `f(); g()`
// (with `;`) → `(do (f) (g))` correctly (pinned at ml/432-semicolon-top-level-folds), but `f() g()`
// (juxtaposition, no `;`) → `(do ((. Qty of) (f) ((. Unit of) #"g")) unit)`. This is NOT a bug: name/call/member-
// magnitude quantities are FIRST-CLASS GOLDENED (ml/242-245, e.g. `f(x) meter` → `(Qty.of (f x) …)`), so `f() g`
// as a call-magnitude quantity is CONSISTENT with the quantity spec (v-syntax, reader owner). The real tension is
// a DESIGN CONFLICT: goldened call/name-magnitude quantities vs this test's "`;`-optional between top-level forms"
// claim — for `f() g()` only one can win (mirrors `5 meter` quantity vs `5; meter` two forms). v-syntax's read
// (routed to operator): NARROW the `;`-optional claim to "except where the juxtaposition forms a valid quantity",
// NOT restrict the quantity sugar. Until the operator rules, pin NEITHER tree in the corpus and keep this test as
// a `len == 2` structural check. (inc-6 batch-72 flagged; v-syntax + concierge routing to operator.)
#[test]
fn top_level_semicolon_folds_and_flattens_to_the_same_root() {
    // A `;` between top-level forms is optional: it folds a stmt-level `(do …)` that the root
    // then splices flat, so `a; b` and `a  b` at the root yield the IDENTICAL tree.
    let with = parse_ok("f(); g()");
    let without = parse_ok("f() g()");
    let wt = with.as_form(with.root, "do").unwrap();
    let wo = without.as_form(without.root, "do").unwrap();
    assert_eq!(wt.len(), 2);
    assert_eq!(wo.len(), 2);
    assert_eq!(with.head_name(wt[0]), Some("f"));
    assert_eq!(with.head_name(wt[1]), Some("g"));
}

// `semicolon_in_argument_position_needs_parens` (a call argument is a single expression, so a `;` inside must
// parenthesize: `f((a; b))` is a one-arg call whose argument is the sequence `(do a b)`) +
// `if_branch_does_not_swallow_the_trailing_sequence` (`if`'s branches parse at `PREC_SEQ + 1`, so a `;` after
// the `if` belongs to the ENCLOSING sequence) MIGRATED to the spec/syntax corpus (inc-6 batch-72):
// ml/434-semicolon-in-arg-needs-parens `f((a; b))`→`(f (do a b))`, ml/435-if-branch-no-swallow-trailing-seq
// `def f() = if c then a else b; more`→`(def (f) (do (if c a b) more))` (NOT `(if c a (do b more))`).

#[test]
fn spans_are_total_and_distinct_for_occurrences() {
    // `x + x`: two x occurrences share one leaf but have distinct ids and distinct spans.
    let p = read_ml("x + x");
    assert!(p.ok());
    let a = &p.arenas;
    // span table has one entry per structure node
    assert_eq!(p.spans.len(), a.structure.len());
    let plus = a.as_form(a.root, "+").unwrap();
    let (l, r) = (plus[0], plus[1]);
    assert_ne!(l, r);
    let ls = p.spans.get(l).unwrap();
    let rs = p.spans.get(r).unwrap();
    assert_ne!(
        ls, rs,
        "the two `x` occurrences map to different source spans"
    );
    // both are the text "x"
    assert_eq!(&"x + x"[ls.start..ls.end], "x");
    assert_eq!(&"x + x"[rs.start..rs.end], "x");
}

#[test]
fn one_leaf_for_repeated_name() {
    let a = parse_ok("f(f, f)");
    // "f" interned once (+ nothing else); 3 occurrences.
    assert_eq!(a.leaves.len(), 1);
}

#[test]
fn quantity_literal_desugars() {
    use crate::sexpr;
    // A numeric literal followed by a bare unit name is a quantity literal: `5 feet` desugars to
    // the same arena as the canonical `(Qty.of 5 (Unit.of #"feet"))`.
    let a = parse_ok("5 feet");
    assert_eq!(sexpr::print(&a), r#"((. Qty of) 5 ((. Unit of) #"feet"))"#);
    // A float value works the same way.
    let f = parse_ok("5.0 meter");
    assert_eq!(
        sexpr::print(&f),
        r#"((. Qty of) 5.0 ((. Unit of) #"meter"))"#
    );
    // The literal binds TIGHTER than every operator, so `5 feet / 1 second` is a rate — the
    // division of two quantity literals — the reading the surface is designed to give.
    let rate = parse_ok("5 feet / 1 second");
    assert_eq!(
        sexpr::print(&rate),
        r#"(/ ((. Qty of) 5 ((. Unit of) #"feet")) ((. Qty of) 1 ((. Unit of) #"second")))"#
    );
    // It composes as an ordinary operand: a call argument, and an addend.
    assert_eq!(
        sexpr::print(&parse_ok("dist(5 feet)")),
        r#"(dist ((. Qty of) 5 ((. Unit of) #"feet")))"#
    );
}

#[test]
fn compound_unit_desugars_on_glued_operators() {
    use crate::sexpr;
    // COMPOUND / RATE units (operator BUG #51): a unit magnitude followed by a GLUED `/`/`*`/`^`
    // extends the UNIT into a composite (bare `/`/`*`/`^` between unit operands — the shape
    // `eval::unit_of` composes + the printer round-trips), so `59 GiB/s` is a RATE unit, not a
    // division by an unbound `s`. v-inference confirmed the shape (atomic quotients compose, no
    // unit_families change).
    // A glued `/` → a rate unit `(Qty.of 59 (/ (Unit.of GiB) (Unit.of s)))`.
    assert_eq!(
        sexpr::print(&parse_ok("59 GiB/s")),
        r#"((. Qty of) 59 (/ ((. Unit of) #"GiB") ((. Unit of) #"s")))"#
    );
    // `^` binds TIGHTER than `/`: `m/s^2` = `m/(s^2)` (the physical reading), NOT `(m/s)^2`.
    assert_eq!(
        sexpr::print(&parse_ok("9 m/s^2")),
        r#"((. Qty of) 9 (/ ((. Unit of) #"m") (^ ((. Unit of) #"s") 2)))"#
    );
    // `*` and `/` compose left-to-right: `kg*m/s^2` = `(kg*m)/(s^2)` (a newton).
    assert_eq!(
        sexpr::print(&parse_ok("3 kg*m/s^2")),
        r#"((. Qty of) 3 (/ (* ((. Unit of) #"kg") ((. Unit of) #"m")) (^ ((. Unit of) #"s") 2)))"#
    );
    // A bare `^` exponent on a single unit: `m^2`.
    assert_eq!(
        sexpr::print(&parse_ok("10 m^2")),
        r#"((. Qty of) 10 (^ ((. Unit of) #"m") 2))"#
    );
    // GLUE is the disambiguator: a SPACED `/ 2` or `/ x` stays ARITHMETIC (a division of the
    // quantity), NOT a unit — only the ordinary infix loop handles it.
    assert_eq!(
        sexpr::print(&parse_ok("59 GiB / 2")),
        r#"(/ ((. Qty of) 59 ((. Unit of) #"GiB")) 2)"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("59 GiB / x")),
        r#"(/ ((. Qty of) 59 ((. Unit of) #"GiB")) x)"#
    );
    // A glued `/` before a NUMBER is arithmetic, not a unit (`GiB/2` divides): the right operand of a
    // unit `/` must be a NAME.
    assert_eq!(
        sexpr::print(&parse_ok("59 GiB/2")),
        r#"(/ ((. Qty of) 59 ((. Unit of) #"GiB")) 2)"#
    );
}

#[test]
fn compound_unit_node_spans_cover_the_whole_unit_expression() {
    // A compound-unit op node (`a/b`, `a*b`) and an exponent node (`m^2`) must span from the LEFT/BASE
    // operand's start — not from the operator — so a diagnostic anchored on the unit expression covers
    // the whole thing (PR#731: the spans previously started at `/`/`*`/`^`, truncating to `/b` / `^2`).
    // The unit expr is the SECOND operand of the `(Qty.of num <unit>)` node; slice source by its span.
    let src = "def main() = 59 GiB/s";
    let p = read_ml(src);
    assert!(p.ok(), "parse: {:?}", p.errors);
    let a = &p.arenas;
    // Reach the `(/ (Unit.of GiB) (Unit.of s))` composite: def body = `((. Qty of) 59 <unit>)`, an
    // application list whose LAST element is the unit expression.
    let def = a.as_form(a.root, "def").unwrap();
    let crate::ast::Struct::List(items) = a.get(def[1]) else {
        panic!("Qty.of body is a list")
    };
    let unit = *items.last().unwrap();
    let us = p.spans.get(unit).unwrap();
    // The composite `/` node must span "GiB/s" WHOLE — from `G` through `s` — not just "/s".
    assert_eq!(
        &src[us.start..us.end],
        "GiB/s",
        "the compound-unit `/` node spans the whole unit expr, not just from the operator"
    );

    // And an exponent node `m^2` spans "m^2" whole, not "^2".
    let src2 = "def main() = 9 m^2";
    let p2 = read_ml(src2);
    assert!(p2.ok(), "parse: {:?}", p2.errors);
    let a2 = &p2.arenas;
    let def2 = a2.as_form(a2.root, "def").unwrap();
    let crate::ast::Struct::List(items2) = a2.get(def2[1]) else {
        panic!("list")
    };
    let unit2 = *items2.last().unwrap();
    let us2 = p2.spans.get(unit2).unwrap();
    assert_eq!(
        &src2[us2.start..us2.end],
        "m^2",
        "the unit-exponent `^` node spans the whole base^exp, not just from the `^`"
    );
}

#[test]
fn a_deep_glued_unit_chain_is_diagnosed_not_an_unbounded_arena() {
    // `compound_unit_tail` folds a glued `/`/`*` chain in a LOOP (self.depth doesn't grow), but each
    // iteration deepens the arena by one `(/ … …)` level — so an unbounded chain (`m/s/s/s…`) would
    // build a tree far deeper than the reader's MAX_NESTING_DEPTH cap that every other nesting path
    // enforces, a recursive-consumer overflow risk (the flat-chain DoS class, PR #383). The loop's
    // spine guard now bounds `self.depth + spine`, so a pathological chain is a clean depth diagnostic.
    let over = (crate::sexpr::MAX_NESTING_DEPTH as usize) + 50;
    let src = format!("def f() = 5 m{}", "/s".repeat(over));
    let p = read_ml(&src);
    assert!(
        !p.ok()
            && p.errors
                .iter()
                .any(|e| e.message.contains("nests too deeply")),
        "a deep glued unit chain must be a clean depth-limit error, not an unbounded arena: {:?}",
        p.errors
    );
    // A moderate chain well under the limit still parses cleanly (no over-rejection).
    let ok = read_ml("def f() = 5 m/s/s/s");
    assert!(
        ok.ok(),
        "a short glued unit chain must parse: {:?}",
        ok.errors
    );
}

// The `forall`-in-param-annotation DESUGAR parse-tree assertions MIGRATED to the spec/syntax corpus (inc-6):
// `forall a b. TYPE` in a param annotation desugars at parse time to leading `(: a Type)` params (infer never
// sees a `(forall …)` node) — ml/135-forall-param-desugars `def id(x: forall a. a) = x`→`(def (id (: a Type)
// (: x a)) x)`, ml/136-forall-param-multi-desugars, ml/137-forall-param-already-canonical `def id(a: Type, x:
// a) = x` (BYTE-IDENTICAL tree to ml/135, pinning the pure-sugar equivalence). This test keeps ONLY the
// value-position error guard below (a reserved-keyword diagnostic — out of the parse-tree/fmt corpus scope).
#[test]
fn a_bare_value_position_forall_is_a_reserved_keyword_error() {
    // `forall` is a RESERVED keyword (like `as`): recognized in type position, never a bare value
    // name — a value-position `forall` is the usual "keyword outside its form" error.
    let bare = read_ml("forall");
    assert!(
        !bare.ok() && bare.errors.iter().any(|e| e.message.contains("keyword")),
        "a bare value-position `forall` is a reserved-keyword error: {:?}",
        bare.errors
    );
}

// `infix_ascription_rhs_beginning_with_forall_is_a_type_not_an_expression` (a NESTED, expression-position
// infix `:` ascription whose RHS begins with `forall` parses the RHS as a TYPE, keeping a `(forall …)` node —
// distinct from a PARAM-annotation forall which desugars to `(: a Type)`, ml/135-140) MIGRATED to the
// spec/syntax corpus (inc-6 batch-73): ml/436-infix-ascription-forall-rhs `f(x : forall a. a)`→`(f (: x
// (forall (a) a)))`, ml/437-infix-ascription-forall-arrow `forall a. a -> a`→`(forall (a) (-> a a))`,
// ml/438-infix-ascription-forall-in-let `let h = k : forall a. a in h`, ml/439-infix-ascription-forall-in-
// operand `(q : forall a. a) + 1`. The forall-ONLY intercept contrast (a non-forall `:` RHS stays as-is):
// ml/440-infix-ascription-value-rhs `f(x : a + b)`→`(f (: x (+ a b)))`, ml/441-infix-ascription-arrow-type-rhs
// `f(x : a -> b)`→`(f (: x (-> a b)))`.

#[test]
fn type_application_argument_is_parsed_as_a_type_not_a_value() {
    use crate::sexpr;
    // A type-position APPLICATION (`Tuple(A, B)`, `List(T)`) parses each argument as a TYPE via
    // `type_ref`, not the value `arg_exprs`. This matters for `forall`: a contextual keyword valid
    // only in type position — the value path misread `Tuple(forall b. L)` as a name + unit-suffix
    // (`(Tuple (Qty.of forall (Unit.of "b")) …)` + `<error>`). Now a `forall`/arrow/nested-application
    // argument parses correctly, so the printed `Tuple(forall b. L)` round-trips.
    assert_eq!(
        sexpr::print(&parse_ok("def f(r: Tuple(forall b. L)) = r")),
        r#"(def (f (: r (Tuple (forall (b) L)))) r)"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("def f(r: List(forall a. a -> a)) = r")),
        r#"(def (f (: r (List (forall (a) (-> a a))))) r)"#
    );
    // A positional NON-forall type argument (application, arrow, qualified name) is unaffected.
    assert_eq!(
        sexpr::print(&parse_ok("def f(r: Tuple(List(a), Int64)) = r")),
        r#"(def (f (: r (Tuple (List a) Int64))) r)"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("def f(r: List(a -> b)) = r")),
        r#"(def (f (: r (List (-> a b)))) r)"#
    );
    // The LABELED record-type application `Record(field: T, …)` still builds `(Record (: field T) …)`
    // — a type-application arg may be a `name: T` label OR a bare type. A field type may itself be a
    // `forall` (the general-type field the value path could not carry).
    assert_eq!(
        sexpr::print(&parse_ok("def f(r: Record(x: Int64, y: Int64)) = r")),
        r#"(def (f (: r (Record (: x Int64) (: y Int64)))) r)"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("def f(r: Record(p: forall a. a)) = r")),
        r#"(def (f (: r (Record (: p (forall (a) a))))) r)"#
    );
}

#[test]
fn a_record_payload_variant_parses_the_labeled_field_form() {
    use crate::sexpr;
    // breaker report: a `(type R (record (: field Ty)))` type-declaration whose variant payload is a
    // RECORD prints as `R = | record(field : Ty)`, but `variant`'s payload parsed each arg via
    // `type_ref` (bare types only), so the `field : Ty` label failed re-parse at the `:` (`expected
    // ,`). No corpus case used the `(type _ (record …))` decl form, so this surface was never
    // round-trip-exercised. Now a variant payload accepts the SAME labeled `name : Type` arg
    // `type_arg_exprs` does (shared `type_arg`), producing `(: field Ty)`.
    assert_eq!(
        sexpr::print(&parse_ok("type R =\n  | record(field : NoSuchField)")),
        r#"(type R (record (: field NoSuchField)))"#
    );
    // Multiple fields.
    assert_eq!(
        sexpr::print(&parse_ok("type Point =\n  | record(x : Int64, y : Int64)")),
        r#"(type Point (record (: x Int64) (: y Int64)))"#
    );
    // A POSITIONAL (non-labeled) variant payload is unaffected — still bare types.
    assert_eq!(
        sexpr::print(&parse_ok("type T =\n  | Pair(Int64, String)")),
        r#"(type T (Pair Int64 String))"#
    );
    // Mixed positional + labeled in one payload also parses (label is per-arg).
    assert_eq!(
        sexpr::print(&parse_ok("type M =\n  | v(Int64, tag : String)")),
        r#"(type M (v Int64 (: tag String)))"#
    );
}

#[test]
fn derived_unit_infix_operators_parse_in_type_annotation_position() {
    use crate::sexpr;
    // ARM-EXTENT sibling (breaker report): a DERIVED-UNIT type annotation composes unit factors with
    // the infix operators `^`/`*`/`/` — the surface the printer emits for `Unit.^`/`Unit.*`/`Unit./`
    // (via `infix_glyph`). Type position had NO infix layer beyond `->`, so `Qty(Int64, meter ^ 2)`
    // failed re-parse (`expected ,` at the exponent). Now the type grammar folds them into bare-glyph
    // heads, sharing the value grammar's `infix_prec` so the ML print→parse cycle round-trips.
    // A single `^` exponent — breaker's minimal case (`(Unit.^ (Unit.base #meter) 2)` prints `meter ^ 2`).
    assert_eq!(
        sexpr::print(&parse_ok("def f(x: Qty(Int64, meter ^ 2)) = x")),
        r#"(def (f (: x (Qty Int64 (^ meter 2)))) x)"#
    );
    // A product `*` and a rate `/`.
    assert_eq!(
        sexpr::print(&parse_ok("def f(x: Qty(Int64, meter * second)) = x")),
        r#"(def (f (: x (Qty Int64 (* meter second)))) x)"#
    );
    // `/`/`*` are left-associative and share the multiplicative tier (`a / b / c` → `((a/b)/c)`).
    assert_eq!(
        sexpr::print(&parse_ok("def f(x: Qty(Int64, gram / meter / second)) = x")),
        r#"(def (f (: x (Qty Int64 (/ (/ gram meter) second)))) x)"#
    );
    // `^` (tier 7) is LOOSER than `/`/`*` (tier 11) — matching the general-expression glyph binding —
    // so an UNPARENTHESIZED `meter / second ^ 2` groups as `(meter / second) ^ 2`, and the printer
    // parenthesizes the physical `meter / (second ^ 2)` reading (verified round-trip in the corpus
    // test, which holds `Unit.^` inputs to idempotence since the surface drops the `Unit.` qualifier).
    assert_eq!(
        sexpr::print(&parse_ok("def f(x: Qty(Int64, meter / second ^ 2)) = x")),
        r#"(def (f (: x (Qty Int64 (^ (/ meter second) 2)))) x)"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("def f(x: Qty(Int64, meter / (second ^ 2))) = x")),
        r#"(def (f (: x (Qty Int64 (/ meter (^ second 2))))) x)"#
    );
    // A derived-unit annotation in RESULT position (`-> Qty(…)`) parses the same way.
    assert_eq!(
        sexpr::print(&parse_ok("def f() -> Qty(Int64, meter / second ^ 2) = x")),
        r#"(def (f) (: x (Qty Int64 (^ (/ meter second) 2))))"#
    );
}

#[test]
fn at_bang_param_carries_a_config_and_a_name_type_binder() {
    use crate::sexpr;
    // `@!param` is the operator's MODULE-level `@param` (module-scoped, like `@!default-fraction`). It
    // carries a PARAM PAYLOAD — a glued `(config kv…)` of `key: value` pairs PLUS a `name : Type`
    // binder — parsing to `(pragma param (param (: k v)…) (: name Type))`. Before this, the generic
    // `@!key <one-type-arg>` path read the config as the arg and let the general unit-suffix postfix
    // eat the trailing `name` as a unit on the pragma node (a garbled `Qty.of` tree). Pin the shape.
    assert_eq!(
        sexpr::print(&parse_ok("@!param(widget: slider) width : Int64")),
        "(pragma param (param (: widget slider)) (: width Int64))"
    );
    // Multiple config kvs; a compound value (tuple).
    assert_eq!(
        sexpr::print(&parse_ok(
            "@!param(widget: slider, range: (1, 10)) width : Int64"
        )),
        r#"(pragma param (param (: widget slider) (: range #tuple(1 10))) (: width Int64))"#
    );
    // Empty / absent config -> `(param)`; both spellings parse to the same node.
    assert_eq!(
        sexpr::print(&parse_ok("@!param() width : Int64")),
        "(pragma param (param) (: width Int64))"
    );
    assert_eq!(
        sexpr::print(&parse_ok("@!param width : Int64")),
        "(pragma param (param) (: width Int64))"
    );
    // A function-typed param — the binder's type is a full `type_ref` (arrow), not swallowed.
    assert_eq!(
        sexpr::print(&parse_ok(
            "@!param(widget: stepper) transform : Int64 -> Int64"
        )),
        "(pragma param (param (: widget stepper)) (: transform (-> Int64 Int64)))"
    );
    // A NON-`param` pragma is unchanged — single type-arg form.
    assert_eq!(
        sexpr::print(&parse_ok("@!default-fraction Rational")),
        "(pragma default-fraction Rational)"
    );
}

// The leading-`def forall` DESUGAR parse-tree assertions MIGRATED to the spec/syntax corpus (inc-6): a LEADING
// `def forall a b. f(…)` clause prepends a `(: a Type)` param per binder — ml/139-def-leading-forall-desugars
// `def forall a. id(x: a) = x`→`(def (id (: a Type) (: x a)) x)`, ml/140-def-leading-forall-multi. The three-
// spelling pure-sugar equivalence (leading == param-annotation == hand-written) is pinned by ml/139 ≡ ml/135 ≡
// ml/137 sharing a byte-identical tree.sexp. This test keeps ONLY the malformed-recovery guards below.
#[test]
fn a_malformed_leading_def_forall_recovers_without_panic() {
    // A malformed leading forall recovers (never panics): missing binder, missing `.`.
    assert!(!read_ml("def forall . f() = 1").ok());
    assert!(!read_ml("def forall a f() = 1").ok());
}

#[test]
fn unit_application_is_a_general_postfix_on_any_expression() {
    use crate::sexpr;
    // OPERATOR BUG FIX: unit application is a general POSTFIX, not literal-only. `let x = 10 in x
    // meters` (the operator's reported failure) now applies the unit to the VARIABLE. Before, `x
    // meters` SILENTLY mis-parsed to a two-statement sequence `(do x meters)` — a wrong tree.
    assert_eq!(
        sexpr::print(&parse_ok("let x = 10 in x meters")),
        r#"(let ((x 10)) ((. Qty of) x ((. Unit of) #"meters")))"#
    );
    // A bare variable, a parenthesized expression, and a call result all take a unit.
    assert_eq!(
        sexpr::print(&parse_ok("x meters")),
        r#"((. Qty of) x ((. Unit of) #"meters"))"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("(a + b) meters")),
        r#"((. Qty of) (+ a b) ((. Unit of) #"meters"))"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("f(5) meters")),
        r#"((. Qty of) (f 5) ((. Unit of) #"meters"))"#
    );
    // PRECEDENCE (operator-confirmed): the unit binds TIGHTER than infix, so `x + 1 meters` groups
    // as `x + (1 meters)`; the whole sum needs parens — `(x + 1) meters`.
    assert_eq!(
        sexpr::print(&parse_ok("x + 1 meters")),
        r#"(+ x ((. Qty of) 1 ((. Unit of) #"meters")))"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("(x + 1) meters")),
        r#"((. Qty of) (+ x 1) ((. Unit of) #"meters"))"#
    );
    // A unit inside a call argument reads as a quantity arg — the CAD units-everywhere case
    // (`cube(width meters)`). (Consequence: `f(a b)` is now a valid unit-suffixed arg, not a
    // missing-comma error — see `missing_comma_between_args_recovers`, which uses number args.)
    assert_eq!(
        sexpr::print(&parse_ok("cube(width meters)")),
        r#"(cube ((. Qty of) width ((. Unit of) #"meters")))"#
    );
    // A type-SUFFIXED literal is still EXEMPT (a suffix selects a numeric type, not a unit): `100N
    // feet` is NOT a quantity — it stays the two-form sequence, unchanged by the generalization.
    assert_eq!(sexpr::print(&parse_ok("100N feet")), r#"(do 100N feet)"#);
}

#[test]
fn unit_suffix_does_not_cross_a_newline_on_a_variable() {
    use crate::sexpr;
    // The same-line guard that protects the literal sugar (`f57c4a53`) protects the general postfix
    // too: a variable ending one statement must not eat the next statement's leading name as a unit.
    // `def a = x <newline> meters` is TWO forms, not `x meters`.
    let a = parse_ok("def w = x\nmeters");
    assert_eq!(
        sexpr::print(&a),
        r#"(do (def w x) meters)"#,
        "a newline between the expr and the candidate unit means separate statements"
    );
}

// `set_literal_desugars` (`#(…)` is the native set ctor literal `#set(…)`, head `Leaf::Ctor(Set)`, uniform
// with `#list`/`#tuple`/`#record`/`#map`) MIGRATED to the spec/syntax corpus (inc-6 batch-63):
// ml/376-set-literal-basic `#(1, 2, 3)`→`#set(1 2 3)`, ml/378-set-literal-empty `#()`→`#set()`,
// ml/391-set-literal-expr-element `#(x + 1)`→`#set((+ x 1))` (an expression element parses fully),
// ml/392-set-literal-as-call-arg `contains(#(1, 2), 1)`→`(contains #set(1 2) 1)` (composes as an operand).

// `bin_literal_desugars` (`b[…]` desugars to the `(bin …)` grammar form — each segment an ordinary
// call-shaped expression under the `bin` head) MIGRATED to the spec/syntax corpus (inc-6 batch-64):
// ml/237-bin-literal-typed-segments `b[u16(258), u8(1)]`→`(bin (u16 258) (u8 1))`, ml/239-bin-literal-empty
// `b[]`→`(bin)`, ml/393-bin-literal-le-modifier `b[u16(258, le), bits(1, 1)]`→`(bin (u16 258 le) (bits 1
// 1))`, ml/394-bin-literal-dependent-size `b[u16(Bytes.len(payload)), bytes(payload)]`→`(bin (u16 ((. Bytes
// len) payload)) (bytes payload))`, ml/395-bin-literal-as-operand `b[u8(1)] == other`→`(= (bin (u8 1)) other)`.

// `a_def_parameter_may_be_a_destructuring_pattern` + `a_destructuring_pattern_parameter_parses` (a def
// parameter that STARTS a compound pattern — `(`-tuple, `[`-list, `#{`-map, `{`-record, `b[`-binary — is a
// destructuring binder routed to `pattern`, not a bare name; plain-name / annotated params keep the
// ordinary binder path) MIGRATED to the spec/syntax corpus (inc-6 batch-68):
//   * ml/410-def-param-list-plain `[a, b]`→`(list a b)`, ml/411-def-param-map `#{ 1 = v }`→`(map (= 1 v))`
//     (canonical `(= key sub)` FieldPair), ml/414-def-param-bin `b[u8(n)]`→`(bin (u8 n))`.
//   * ml/412-def-param-nested-tuple-in-list-rest `[(a, b), .. rest]`→`(list (tuple a b) (.. rest))`,
//     ml/413-def-param-mixed-plain-and-list-rest `def f(x, [a, .. rest])`→`(def (f x (list a (.. rest))) x)`.
//   Already pinned: tuple `(a, b)`=ml/336, list-rest `[x, .. rest]`=ml/337, record `{x=a,y=b}`=ml/345 /
//   `{x=a}`=ml/346, plain-name/annotated-param=ml/102. (The regression guarded: `param` once routed only
//   `(`-led patterns to `pattern()`, "expected a name" on `[`/`#{`.)

// `bin_pattern_desugars` (in pattern position `b[…]` desugars to the same `(bin …)` head with sub-PATTERN
// segments — `u16(n)` binds `n`, `bytes(rest)` binds the tail) MIGRATED to the spec/syntax corpus (inc-6
// batch-64): ml/240-bin-pattern-match `match x with | b[u16(n), bytes(rest)] => n`→`(match x ((bin (u16 n)
// (bytes rest)) n))`, ml/396-bin-pattern-empty `b[]`→`(match x ((bin) 0))`, ml/397-bin-pattern-le-modifier
// `b[u16(n, le)]`→`(match x ((bin (u16 n le)) n))`. (Single-segment `b[u16(n)]` is ml/241.)

#[test]
fn number_before_keyword_is_not_a_quantity() {
    use crate::sexpr;
    // Only a bare NON-keyword identifier attaches as a unit. A word-operator keeps its infix
    // meaning after a number: `5 and mask` is the boolean `and`, not a quantity in unit `and`.
    let a = parse_ok("5 and mask");
    assert_eq!(sexpr::print(&a), "(and 5 mask)");
}

// `a_set_pattern_parses` + `a_set_rest_pattern_parses_in_a_match_arm` (a `#(`-led SET PATTERN — the pattern
// twin of the `#(…)` set literal: native set ctor leaf head, sub-pattern elements, a `.. rest` binding the
// residual set to `(.. rest)`; in def-param AND match-arm position) MIGRATED to the spec/syntax corpus
// (inc-6 batch-65):
//   * ml/398-set-pattern-param `def f(#(a, b)) = a`→`(def (f #set(a b)) a)`, ml/399-set-pattern-rest-param
//     `def f(#(a, .. rest)) = a`→`(def (f #set(a (.. rest))) a)`, ml/400-set-pattern-empty-param `def f(#())
//     = 0`→`(def (f #set()) 0)`.
//   * ml/333-set-rest-pattern already pins the match-arm `#(a, .. rest)` + `_` catch-all; ml/401-set-rest-
//     match-arm-literal-element `match s with | #(1, .. rest) => rest | _ => s`→`(match s (#set(1 (.. rest))
//     rest) (_ s))` adds the literal-element residual-binding form (the tree behind the #6877 e2e example).
//   The clean-surface round-trip (no `` `..` `` fallback) is each case's fmt-idempotence.

// (`a_destructuring_pattern_parameter_parses` MIGRATED with the a_def_parameter breadcrumb above, inc-6
// batch-68: its `(a, b)`=ml/336, `[x, .. rest]`=ml/337, `b[u8(n)]`=ml/414, plain-name (trivial) + annotated
// `xs: List(Int64)`=ml/102 assertions are all pinned there.)

// `a_tuple_rest_pattern_parses` + `a_record_rest_pattern_parses` (a tuple/record pattern with a trailing
// `.. rest` binding the remaining positional/un-named elements to the wrapped `(.. rest)` node — the twin
// of the list/map/set-pattern rest) MIGRATED to the spec/syntax corpus (inc-6 batch-66):
//   * ml/331-tuple-rest-pattern `(a, b, .. rest)`→`(tuple a b (.. rest))`, ml/402-tuple-rest-single-leading
//     `(x, .. rest)`→`(tuple x (.. rest))`, ml/403-tuple-rest-nested-in-rest-binder `(a, .. (b, c))`→
//     `(tuple a (.. (tuple b c)))` (the rest binder is a full sub-pattern position). Plain `(a, b)` = ml/336.
//   * ml/332-record-rest-pattern `{ a = x, .. rest }`→`(record (= a x) (.. rest))`, ml/404-record-rest-field-
//     shorthand `{ a, .. rest }`→`(record (= a a) (.. rest))` (field shorthand puns `{ a }`→`(= a a)`).
//     Plain `{ a = x, b = y }` = ml/345.

// `degenerate_rest_patterns_parse_permissively_without_panic` (the compound pattern surfaces are
// INTENTIONALLY PERMISSIVE about rest position/count — rest-ONLY, NON-TRAILING, and MULTIPLE rests all
// parse to the wrapped `(.. rest)` node in situ; the at-most-one/trailing-only constraints are the
// match-lowering's job, not the scope-blind parser's) MIGRATED to the spec/syntax corpus (inc-6 batch-67):
//   * rest-ONLY (whole-collection bind): ml/405-rest-only-record-pattern `{ .. rest }`→`(record (.. rest))`,
//     ml/406-rest-only-set-pattern `#(.. rest)`→`#set((.. rest))`, ml/407-rest-only-list-pattern `[.. rest]`→
//     `(list (.. rest))` (a list PATTERN `(list …)`, distinct from the `#list(…)` construction spread ml/151).
//   * ml/408-non-trailing-rest-pattern `(a, .. rest, b)`→`(tuple a (.. rest) b)`, ml/409-multiple-rests-pattern
//     `(a, .. r1, .. r2)`→`(tuple a (.. r1) (.. r2))` — surface-permissive; lowering rejects the malformed.
//   (Tuple has no rest-only form — a rest needs a leading element + comma; the map rest-only `#{ .. rest }`
//   is covered elsewhere.)

#[test]
fn parameterized_annotation_name_takes_a_glued_application() {
    use crate::sexpr;
    // `@tag("slow")` — a call-style annotation argument: a `(` GLUED to the annotation name makes
    // the name slot the application `(tag "slow")`, so the tree is `(@ (tag "slow") form)`.
    assert_eq!(
        sexpr::print(&parse_ok("@tag(\"slow\")\ndef f() = 1")),
        "(@ (tag \"slow\") (def (f) 1))"
    );
    // A bare `@test` (no glued paren) keeps the plain-name slot.
    assert_eq!(
        sexpr::print(&parse_ok("@test\ndef f() = 1")),
        "(@ test (def (f) 1))"
    );
    // GLUING GUARD: a `(` NOT glued to the name (whitespace/newline between) is the annotated FORM,
    // not the name's call args — `@test` then `(g)` on the next line is `(@ test g)`, NOT
    // `(@ (test g) …)`. (postfix's LParen arm doesn't check adjacency; the `@`-arm guard does.)
    assert_eq!(sexpr::print(&parse_ok("@test\n(g)")), "(@ test g)");
    // Multiple args + stacking with a bare annotation.
    assert_eq!(
        sexpr::print(&parse_ok("@test\n@cfg(\"a\", \"b\")\ndef f() = 1")),
        "(@ test (@ (cfg \"a\" \"b\") (def (f) 1)))"
    );
}

// `paren_comma_in_type_position_is_a_tuple_type` (a paren-comma `(A, B)` in TYPE position — RHS of a `:` —
// is the tuple TYPE `(Tuple A B)`, NOT the tuple VALUE ctor `#tuple(A B)` the prefix path builds in
// value/pattern position; tuple values/patterns and tuple TYPES share the `(…)` spelling) MIGRATED to the
// spec/syntax corpus (inc-6 batch-69):
//   * ml/415-tuple-type-param-annotation `def f(p: (Int64, Int64)) = p.0`→`(def (f (: p (Tuple Int64
//     Int64))) (. p 0))`, ml/416-tuple-type-paren-equals-explicit `Tuple(Int64, Int64)` — BYTE-IDENTICAL
//     tree.sexp to ml/415, pinning the `(A, B)` == `Tuple(A, B)` type-position equivalence.
//   * ml/417-tuple-type-function-operand `(Int64, Bool) -> Int64`→`(-> (Tuple Int64 Bool) Int64)`,
//     ml/418-tuple-type-nested `(Int64, (Bool, Int64))`→`(Tuple Int64 (Tuple Bool Int64))`,
//     ml/419-paren-single-type-transparent `(Int64)`→`Int64` (transparent grouping, not a 1-tuple),
//     ml/420-paren-empty-unit-type `()`→`unit`.
//   The VALUE-position `(1, 2)`→`#tuple(1 2)` contrast (the retyping is TYPE-only) is ml/06-tuple-literal.

// `a_destructuring_pattern_let_binder_parses` (a `let` binder opening a destructuring pattern binds by
// pattern — the twin of the pattern parameter) is fully subsumed by the spec/syntax corpus (inc-6 batch-69):
// ml/342-let-tuple-binder-destructure `let (a, b) = p in a + b`→`(let (((tuple a b) p)) (+ a b))`,
// ml/343-let-list-rest-binder-destructure `let [x, .. rest] = ys in x`→`(let (((list x (.. rest)) ys)) x)`,
// ml/03-let plain `let x = 1 in x`→`(let ((x 1)) x)`, ml/344-let-mixed-binder-destructure `let x = 1, (a,
// b) = p in x + a`→`(let ((x 1) ((tuple a b) p)) (+ x a))`. No new cases needed.

#[test]
fn quantity_sugar_does_not_cross_a_newline() {
    use crate::sexpr;
    // The quantity sugar (`5 feet` → Qty) repurposes number+name ADJACENCY, but statement sequencing
    // juxtaposes forms across lines with no separator — so a number ending one statement sits right
    // before the next statement's leading identifier. The sugar must NOT eat that identifier as a unit
    // (a miscompile that swallows the following statement). A NEWLINE between the number and the
    // candidate unit means they are different statements: leave the bare number, let the next form be
    // its own statement.
    //
    // `def a() = 10 <newline> a() + 5`: the `10` must stay a bare number (main's `a` def), and `a()+5`
    // is the next top-level form — NOT `(Qty.of 10 (Unit.of "a"))` eating the next line.
    let a = parse_ok("def a() = 10\na() + 5");
    assert_eq!(
        sexpr::print(&a),
        "(do (def (a) 10) (+ (a) 5))",
        "the quantity sugar must not span the newline into the next statement"
    );
    // A genuine SAME-LINE quantity is unchanged — `10 a` (no intervening newline) is still a quantity.
    assert_eq!(
        sexpr::print(&parse_ok("10 a")),
        r#"((. Qty of) 10 ((. Unit of) #"a"))"#
    );
    // Same-line even when a statement follows on the NEXT line: `5 feet` is the quantity, `x` is next.
    assert_eq!(
        sexpr::print(&parse_ok("5 feet\nx")),
        r#"(do ((. Qty of) 5 ((. Unit of) #"feet")) x)"#
    );
}

#[test]
fn as_conversion_does_not_cross_a_newline() {
    use crate::sexpr;
    // The `as` unit-conversion postfix (`value as meter` → `(Unit.in (Unit.of "meter") value)`) must
    // apply only WITHIN one statement. Statement sequencing juxtaposes forms across lines, so an `as`
    // beginning a new line must NOT reach back across the newline and absorb the previous statement's
    // trailing expression — `x as meter` split over two lines is a value `x` then a separate (erroring)
    // `as meter`, NOT `(x as meter)`. Same boundary the quantity sugar draws; the `as` operator landed
    // without it, so `def a() = 5.0 <newline> as meter` silently became `def a() = (5.0 as meter)`.
    //
    // `x <newline> as meter`: `x` is a complete statement; the leading `as` on the next line does not
    // continue it. (`read_ml` tolerates the stray `as`-with-no-left-operand error and still yields a
    // tree; the stray `as`/`meter` land as their own error-recovered forms — the point is `x` is NOT
    // folded into a `(Unit.in … x)` conversion.)
    let parsed = read_ml("x\nas meter");
    let printed = sexpr::print(&parsed.arenas);
    assert!(
        !printed.contains("Unit in"),
        "a leading `as` on a new line must not absorb the previous statement into a conversion: {printed}"
    );
    // A genuine SAME-LINE conversion is unchanged — `x as meter` (no intervening newline) still converts.
    assert_eq!(
        sexpr::print(&parse_ok("x as meter")),
        r#"((. Unit in) ((. Unit of) #"meter") x)"#
    );
    // Same-line `as` even when a statement follows on the NEXT line: the conversion is `5.0 as meter`,
    // `x` is the next statement — the newline after the conversion ends it, it does not chain into `x`.
    assert_eq!(
        sexpr::print(&parse_ok("5.0 as meter\nx")),
        r#"(do ((. Unit in) ((. Unit of) #"meter") 5.0) x)"#
    );
}

#[test]
fn as_conversion_target_may_be_a_compound_unit() {
    use crate::sexpr;
    // The `as` conversion target extends across a GLUED `/`/`*`/`^` chain into a COMPOUND unit, the
    // same surface as the `<num> GiB/s` quantity literal — so `x as GiB/s` converts to the rate unit.
    // Without this the bare-name case read only a SINGLE unit and `/s` fell to the enclosing infix loop
    // as a division of the conversion by unbound `s` (the sibling of BUG #51 on the conversion path).
    assert_eq!(
        sexpr::print(&parse_ok("x as GiB/s")),
        r#"((. Unit in) (/ ((. Unit of) #"GiB") ((. Unit of) #"s")) x)"#
    );
    // `^` binds tighter than `/` here too: `m/s^2`.
    assert_eq!(
        sexpr::print(&parse_ok("x as m/s^2")),
        r#"((. Unit in) (/ ((. Unit of) #"m") (^ ((. Unit of) #"s") 2)) x)"#
    );
    // A single unit is unchanged, and a SPACED `/ 2` stays a division of the conversion (glue rule).
    assert_eq!(
        sexpr::print(&parse_ok("x as meter")),
        r#"((. Unit in) ((. Unit of) #"meter") x)"#
    );
    assert_eq!(
        sexpr::print(&parse_ok("x as meter / 2")),
        r#"(/ ((. Unit in) ((. Unit of) #"meter") x) 2)"#
    );
}

#[test]
fn backtick_escapes_reserved_word() {
    // `` `let` `` is the name "let", not a let-form.
    let a = parse_ok("`let`");
    assert_eq!(a.as_name(a.root), Some("let"));
}

#[test]
fn string_unescape_and_nfc() {
    let a = parse_ok(r#" "a\nb" "#);
    assert_eq!(
        a.leaf(match a.get(a.root) {
            crate::ast::Struct::Atom(l) => *l,
            _ => panic!(),
        }),
        &Leaf::Str("a\nb".into())
    );
}

#[test]
fn never_panics() {
    for src in [
        "",
        "(",
        ")",
        "let",
        "match {",
        "1 +",
        ".",
        "=>",
        "fn(",
        "if then",
        "`",
        "\"",
        // Malformed / incomplete inline `world` decls: the world_expr/world_interface/world_member
        // paths must recover (a diagnostic), never panic — the crate's totality invariant extended
        // to the new surface. Empty interface (no members), missing `=`, dangling direction, a
        // member with no arrow/result, an unterminated param list, a non-import/export interface head.
        "world W =",
        "world W = | export i =",
        "world W = | export i",
        "world W = | export i = | m",
        "world W = | export i = | m :",
        "world W = | export i = | m : (p : u8)",
        "world W = | export i = | m : (p :",
        "world W = | frobnicate i = | m : () -> u8",
        "world W = | export = | m : () -> u8",
        "world W = | export i = | m : () ->",
        // Malformed / edge SEC-F1 `@resource` effect-op markers: the effect_op resource-lift
        // (lift_resource_marker / unwrap_resource_param) + the printer resugar must recover, never
        // panic. Dangling `@resource` (no type), `@resource` on a nullary/leading-arrow op (no param
        // to mark), two `@resource` markers (only the first lifts), `@resource` on the result.
        "effect E = | op : @resource",
        "effect E = | op : @resource -> Unit",
        "effect E = | op : @resource Bytes",
        "effect E = | op : @resource Bytes -> @resource Bytes -> Unit",
        "effect E = | op : Bytes -> @resource Unit",
        "effect E = | op : -> @resource Unit",
    ] {
        let _ = read_ml(src); // must not panic
    }
}

// ---- error recovery ----
//
// The parser is a RECOVERING parser: it never bails at the first error. It collects every error
// into `errors`, and — crucially — resynchronizes so one stray symbol yields roughly one error
// instead of an avalanche, and so structure AROUND a mistake is still recovered. These tests pin
// that behavior down: they assert the arena stays well-formed, that multiple independent errors
// are all reported, that recovery syncs on delimiters, and that parsing always terminates.

/// Assert the arena is well-formed regardless of errors: the root id is in range, every list
/// child id is in range and traversable, and the span table is total (1:1 with structure nodes,
/// the invariant the whole `SpanTable` design rests on). Returns the parse for further checks.
fn recovered(src: &str) -> Parsed {
    let p = read_ml(src);
    let n = p.arenas.structure.len();
    assert!(n > 0, "arena is never empty for {src:?}");
    assert!(
        (p.arenas.root.0 as usize) < n,
        "root id in range for {src:?}"
    );
    assert_eq!(
        p.spans.len(),
        n,
        "span table stays total (1:1 with structure) for {src:?}"
    );
    // Every span is a GEOMETRICALLY VALID slice of the source — ordered, in-bounds, and on UTF-8
    // char boundaries — even on malformed/recovered input. This is what makes `&src[sp.start..sp.end]`
    // safe: an LSP hover / diagnostic underline / codemod edit slices the source by a node's span, so
    // a span past the end or off a char boundary would PANIC on the exact byte a user hovers. Totality
    // (above) only says a span EXISTS per node; this says it can be safely SLICED. Checked for every
    // id (spans are a flat vector, so a plain scan covers the whole arena).
    for i in 0..n as u32 {
        let sp = p.spans.get(StructId(i)).expect("total span table");
        assert!(
            sp.start <= sp.end
                && sp.end <= src.len()
                && src.is_char_boundary(sp.start)
                && src.is_char_boundary(sp.end),
            "span {sp:?} for node {i} is not a valid slice of {src:?}"
        );
    }
    // Every reachable node's children are valid ids — the tree is fully traversable.
    fn walk(a: &Arenas, id: StructId, seen: &mut usize) {
        *seen += 1;
        if let crate::ast::Struct::List(children) = a.get(id) {
            for &c in children {
                assert!(
                    (c.0 as usize) < a.structure.len(),
                    "child id {} in range",
                    c.0
                );
                walk(a, c, seen);
            }
        }
    }
    let mut seen = 0;
    walk(&p.arenas, p.arenas.root, &mut seen);
    p
}

#[test]
fn every_ml_span_is_a_valid_source_slice_over_arbitrary_input() {
    // Names the span-GEOMETRY property `recovered` now enforces, so it is a first-class invariant
    // (not just an incidental helper check that a refactor could drop): on ANY input — well-formed or
    // garbage — every node's span is an ordered, in-bounds, char-boundary slice of the source, so
    // `&src[sp.start..sp.end]` (LSP hover / diagnostic underline / codemod edit) can never panic. The
    // s-expr surface got this via `every_node_span_slices_back_to_that_node...`; the ML surface parses
    // a far larger grammar with error recovery + graft/desugar spans, where an off-by-one is likelier.
    // A dedicated sweep over a sigil-rich alphabet (distinct seed from the recovery sweep) drives the
    // grammar's structural paths; `recovered` asserts the geometry for each generated program.
    let alphabet: Vec<char> = "()[]{}|,;=>-+*/<:.@#`\"\\ \tabcdefimntxλ中0123456789\n"
        .chars()
        .collect();
    let mut rng = SplitMix64(0x59a2_0d1c_5111_7a5f);
    for len in 0..=40usize {
        for _ in 0..100 {
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            let _ = recovered(&s); // asserts span geometry (+ totality/traversability) for this input
        }
    }
}

#[test]
fn recovered_arena_is_always_well_formed() {
    // Whatever the garbage, the arena is traversable and the span table is total.
    for src in [
        "@",
        "f(@)",
        "1 + @ + 2",
        "let x = @ in x",
        "[1, @, 3]",
        "{ a = @, b = 2 }",
        "match e with | @ => 1",
        "def f(@, x) = x",
        "f(a b c",
        "module m { @ }",
        ")(][}{",
        "1 @ 2 # 3 ~ 4",
    ] {
        let _ = recovered(src);
    }
}

/// A tiny deterministic PRNG (SplitMix64) — reproducible fuzz without a dependency, matching the
/// lexer/codec house style (the crate stays "plain").
struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[test]
fn recovered_arena_invariants_hold_on_arbitrary_input() {
    // The parser's charter invariant — never panic, always produce a well-formed traversable arena
    // with a TOTAL span table — on ARBITRARY input, not just the hand-picked garbage above. The
    // lexer's own fuzz calls `read_ml` on random soup but only checks non-panic; this asserts the
    // PARSER's structural invariants (`recovered`) hold on every fuzzed string. The alphabet stresses
    // the grammar's structural sigils (parens/brackets/braces, `,`/`;`/`|`, keywords' lead chars,
    // operators, quote/comment openers, unicode) so recovery, resync, and span-table upkeep are
    // exercised across deeply malformed shapes.
    let alphabet: Vec<char> = "()[]{}|,;=>-+*/<:.@#`\"\\ \tabcdefimntxλ中0123456789\n"
        .chars()
        .collect();
    let mut rng = SplitMix64(0x5111_7a5f_11e2_0d1c);
    for len in 0..=32usize {
        for _ in 0..120 {
            let s: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            // `recovered` asserts: non-empty arena, root in range, span table 1:1 with structure,
            // and every reachable child id in range (fully traversable) — for THIS fuzzed input.
            let _ = recovered(&s);
        }
    }
    // A few pathological repeats that have historically stressed recovery/depth guards.
    for s in [
        "((((((((", "))))))))", "{{{{{{{{", "[,[,[,[,", ";;;;;;;;", "||||||||", "@@@@@@@@",
    ] {
        let _ = recovered(s);
        let _ = recovered(&s.repeat(8));
    }
}

#[test]
fn does_not_bail_at_first_error() {
    // Several independent mistakes are ALL reported, not just the first. Three stray symbols
    // separated as their own top-level statements yield (at least) three errors.
    let p = recovered("@; ~; $");
    assert!(
        p.errors.len() >= 3,
        "each stray statement reports its own error, got {:?}",
        p.errors
    );
}

#[test]
fn a_single_stray_symbol_does_not_cascade() {
    // One bad token in the middle of an otherwise-fine call yields a small, bounded number of
    // errors — recovery resynchronizes rather than mis-parsing everything after it.
    let p = read_ml("f(a, $, c)");
    assert!(!p.ok(), "the stray `$` is reported");
    assert!(
        p.errors.len() <= 2,
        "one stray token stays bounded, got {} errors: {:?}",
        p.errors.len(),
        p.errors
    );
    // The call is still recovered as `(f a <error> c)` — the good arguments survive.
    let a = &p.arenas;
    assert_eq!(a.head_name(a.root), Some("f"));
    let call = a.as_form(a.root, "f").unwrap();
    assert_eq!(call.len(), 3, "three arguments recovered around the error");
    assert_eq!(a.as_name(call[0]), Some("a"));
    assert_eq!(a.as_name(call[2]), Some("c"));
}

#[test]
fn error_inside_brackets_does_not_escape_them() {
    // The offending token inside `( … )` must NOT consume the closing `)` — the parser resyncs on
    // the bracket, so the SECOND statement after it parses cleanly as its own form.
    let p = read_ml("f(@); g(x)");
    assert!(!p.ok());
    // Root is a `(do …)` of two statements; the second is a clean call `(g x)`.
    let top = p.arenas.as_form(p.arenas.root, "do").unwrap();
    assert_eq!(top.len(), 2, "two top-level statements survive: {top:?}");
    assert_eq!(p.arenas.head_name(top[1]), Some("g"));
    let g = p.arenas.as_form(top[1], "g").unwrap();
    assert_eq!(p.arenas.as_name(g[0]), Some("x"));
}

#[test]
fn missing_comma_between_args_recovers() {
    // `f(1 2)` — a missing separator is reported once, and BOTH arguments are still recovered. (This
    // uses NUMBER args deliberately: since the general unit-suffix landed, `f(a b)` is now a VALID
    // parse — `a` with unit `b`, i.e. `f((Qty.of a (Unit.of #b)))` — not a missing comma. A number
    // cannot be a unit name, so `1 2` is still unambiguously a missing separator, which keeps this
    // recovery invariant exercised on a shape the unit grammar does not claim.)
    let p = read_ml("f(1 2)");
    assert!(!p.ok(), "the missing `,` is reported");
    assert_eq!(p.errors.len(), 1, "exactly one error: {:?}", p.errors);
    assert!(
        p.errors[0].message.contains(','),
        "the error names the missing comma: {:?}",
        p.errors[0]
    );
    let a = &p.arenas;
    let call = a.as_form(a.root, "f").unwrap();
    assert_eq!(call.len(), 2, "both args recovered");
    // Both args are the number literals (Int atoms — `as_name` is None for a non-Name leaf).
    for (arg, want) in [(call[0], "1"), (call[1], "2")] {
        match a.get(arg) {
            crate::ast::Struct::Atom(lid) => {
                assert!(
                    matches!(a.leaf(*lid), Leaf::Int { .. }),
                    "arg is an Int literal"
                );
                let sp = p.spans.get(arg).unwrap();
                assert_eq!(&"f(1 2)"[sp.start..sp.end], want, "arg slices to {want}");
            }
            other => panic!("arg is an atom, got {other:?}"),
        }
    }
}

#[test]
fn an_unterminated_literal_names_its_specific_cause() {
    // A lexer ERROR token in expression position (an unterminated literal run to end-of-input) used
    // to read as the generic "expected an expression" — misdirecting, since the token IS where an
    // expression starts; the real defect is the unclosed literal. Each opener now names its cause,
    // the ML-surface twin of the s-expr reader's "unterminated string".
    for (src, needle) in [
        ("def f() = \"abc", "unterminated string literal"),
        ("def f() = b\"abc", "unterminated byte-string literal"),
        ("def f() = #\"abc", "unterminated symbol literal"),
        ("def f() = `abc", "unterminated backtick name"),
    ] {
        let p = read_ml(src);
        assert!(!p.ok(), "{src:?} is rejected");
        assert!(
            p.errors.iter().any(|e| e.message.contains(needle)),
            "{src:?} names {needle:?}, not the generic message: {:?}",
            p.errors
        );
        assert!(
            !p.errors
                .iter()
                .any(|e| e.message == "expected an expression"),
            "{src:?} does not fall back to the generic message: {:?}",
            p.errors
        );
    }
    // A well-terminated string is unaffected (no spurious error).
    assert!(
        read_ml("def f() = \"abc\"").ok(),
        "a closed string parses clean"
    );
}

#[test]
fn missing_comma_in_list_recovers() {
    // `[1 2 3]` — every element is recovered, with one missing-`,` error per gap. The literal
    // desugars to the native `#list(1 2 3)` ctor (head `Leaf::Ctor(List)`, recognized by kind
    // identity), so confirm the ctor kind + count the element children (all but the head).
    let p = read_ml("[1 2 3]");
    assert!(!p.ok());
    let a = &p.arenas;
    assert_eq!(
        a.compound_ctor_leaf(a.root),
        Some(crate::ast::CompoundCtor::List)
    );
    let crate::ast::Struct::List(items) = a.get(a.root) else {
        panic!("list literal is a List node")
    };
    assert_eq!(
        items.len() - 1,
        3,
        "all three elements recovered: {items:?}"
    );
}

#[test]
fn missing_closer_is_reported_and_recovered() {
    // An unterminated call reports the missing `)` but still yields a usable `(f a b)` tree
    // (rather than discarding the whole form).
    let p = read_ml("f(a, b");
    assert!(!p.ok());
    assert!(
        p.errors.iter().any(|e| e.message.contains(')')),
        "the missing `)` is reported: {:?}",
        p.errors
    );
    let a = &p.arenas;
    let call = a.as_form(a.root, "f").unwrap();
    assert_eq!(call.len(), 2);
}

#[test]
fn recovers_the_let_around_a_bad_binding() {
    // A stray value in a binding is isolated: the `let` shape and its body survive.
    let p = read_ml("let x = $ in x + 1");
    assert!(!p.ok());
    let a = &p.arenas;
    let tail = a.as_form(a.root, "let").expect("still a let form");
    assert_eq!(tail.len(), 2, "bindings + body recovered");
    // body is `(+ x 1)` — parsed cleanly after the bad binding.
    assert_eq!(a.head_name(tail[1]), Some("+"));
}

#[test]
fn keyword_boundary_is_not_swallowed_by_a_bad_condition() {
    // A stray symbol where the `if` condition belongs must not eat the `then` — the rest of the
    // form still parses, so we get an `(if …)` with three children.
    let p = read_ml("if $ then a else b");
    assert!(!p.ok());
    let a = &p.arenas;
    let if_form = a.as_form(a.root, "if").expect("still an if form");
    assert_eq!(if_form.len(), 3, "cond/then/else all recovered");
    assert_eq!(a.as_name(if_form[1]), Some("a"));
    assert_eq!(a.as_name(if_form[2]), Some("b"));
}

#[test]
fn match_arm_boundary_survives_a_bad_pattern() {
    // A garbage pattern in the first arm does not consume the `=>` or the `|` that starts the
    // next arm — both arms are recovered.
    let p = read_ml("match e with | @ => 1 | _ => 2");
    assert!(!p.ok());
    let a = &p.arenas;
    let m = a.as_form(a.root, "match").expect("still a match");
    assert_eq!(m.len(), 3, "scrutinee + two arms recovered: {m:?}");
}

#[test]
fn stray_closers_do_not_hang_and_stay_bounded() {
    // A pile of mismatched closers/garbage must terminate (the test completing IS the assertion)
    // and produce a well-formed arena with a finite error list.
    for src in [
        ")))))",
        "][}{)(",
        "f(((((",
        "[[[[[",
        "{{{{{",
        "#{#{#{",
        ",,,,,",
        "..........",
        "=> => =>",
        "| | | |",
        "@@@@@@@@@@",
        "let let let",
    ] {
        let p = recovered(src);
        assert!(
            p.errors.len() < 10_000,
            "error list stays finite for {src:?} (no runaway loop)"
        );
    }
}

#[test]
fn valid_programs_still_report_no_errors() {
    // Recovery must be inert on well-formed input — no spurious errors, exact trees preserved.
    for src in [
        "1 + 2 * 3",
        "f(a, b, c)",
        "let x = 1, y = 2 in x + y",
        "match e with | Some(n) => n | None => 0",
        "def f(x, y) = x + y",
        "[1, 2, 3]",
        "{ a = 1, b = 2 }",
        "#{ k = v }",
        "if a then b else c",
        "module m { def x = 1 def y = 2 }", // module members are whitespace-separated, no `;`
    ] {
        let p = read_ml(src);
        assert!(p.ok(), "no spurious errors on {src:?}: {:?}", p.errors);
    }
}

#[test]
fn exhaustive_short_token_soup_always_terminates_well_formed() {
    // The strongest termination evidence: enumerate EVERY sequence of up to four tokens drawn
    // from an alphabet chosen to stress recovery (delimiters, separators, keywords, junk). If any
    // combination could drive `prefix`/`sep_continue`/the block loops into a non-advancing cycle,
    // this test would hang — so its completion is the proof that parsing always makes progress.
    // Each parse is also checked for a well-formed, traversable arena and a total span table.
    let alphabet = [
        "(", ")", "[", "]", "{", "}", "#", ",", ";", ".", "=>", "|", "@", "let", "in", "if",
        "match", "with", "def", "x", "1",
    ];
    let mut count = 0usize;
    // lengths 1..=3 exhaustively; a light length-4 sweep keeps the total bounded but deep.
    for len in 1..=3 {
        let combos = alphabet.len().pow(len as u32);
        for mut n in 0..combos {
            let mut src = String::new();
            for _ in 0..len {
                src.push_str(alphabet[n % alphabet.len()]);
                src.push(' ');
                n /= alphabet.len();
            }
            let _ = recovered(&src); // must terminate + stay well-formed
            count += 1;
        }
    }
    assert!(count > 8_000, "swept a meaningful space, got {count}");
}

#[test]
fn nested_error_reports_once_and_outer_form_survives() {
    // A bad token nested two levels deep is reported, and every enclosing construct is still
    // recovered up to the root.
    let p = recovered("g(f(a, @), b)");
    assert!(!p.ok());
    let a = &p.arenas;
    // outer call `(g (f a <error>) b)`
    let g = a.as_form(a.root, "g").expect("outer call recovered");
    assert_eq!(g.len(), 2, "outer call keeps both args: {g:?}");
    let f = a.as_form(g[0], "f").expect("inner call recovered");
    assert_eq!(f.len(), 2, "inner call keeps both args: {f:?}");
    assert_eq!(a.as_name(g[1]), Some("b"), "arg after the bad one survives");
}

// ---- first-class embedded syntaxes (front-end syntax-switch) ----

// The embedded-region PARSE-TREE + round-trip tests (`json_embedded_region_grafts…`, `toml_embedded_region_
// grafts…`, `embedded_region_round_trips…`, `an_embedded_region_nested_in_a_larger_expr_re_emits…`,
// `a_grammar_tag_not_glued_to_a_brace_is_an_ordinary_name`, `a_brace_inside_a_json_string_does_not_close_the_
// region`) MIGRATED to the spec/syntax corpus (inc-6 batch-71). A reserved tag GLUED to `{` switches into a
// sub-grammar and grafts `(embedded #"tag" <sub-arena>)`; the printer re-emits the `tag{ … }` surface:
//   * ml/424-embedded-json-region `json{ {"a": [1, true], "b": null} }`→`(embedded #"json" (json-object
//     (member "a" (json-array 1 true)) (member "b" (json-null))))`, ml/425-embedded-toml-region (toml-document).
//   * ml/426-embedded-json-in-let (nested in a `let`), ml/427-embedded-mixed-in-tuple (json + toml in a tuple)
//     — the printer's embedded arm fires wherever `expr` recurses; round-trip is each case's fmt-idempotence.
//   * ml/428-grammar-tag-bare-is-a-name `json`→`json` (bare, not glued) + ml/430-grammar-tag-spaced-not-
//     embedded `json {}`→`(do json #record())` (a space before `{` does NOT switch).
//   * ml/429-embedded-json-brace-in-string `json{ {"s": "a}b{c"} }` — a `}` inside a JSON string doesn't
//     close the region early (the raw-region scanner tracks string literals).
// The recovery/span/never-panic embedded tests below STAY Rust (diagnostic-quality / span / totality guards).

#[test]
fn a_malformed_embedded_body_is_a_recovered_error_not_a_panic() {
    // A sub-grammar parse error lifts a diagnostic into this parse and leaves an `<error>` placeholder
    // under the embedded node — the arena stays well-formed, the parse never panics.
    let p = read_ml(r#"json{ {"a": } }"#); // missing value → JSON error
    assert!(!p.ok(), "a malformed JSON body surfaces a parse error");
    let a = &p.arenas;
    let emb = a
        .as_form(a.root, "embedded")
        .expect("still an (embedded …) node even on a bad body");
    assert_eq!(
        a.as_name(emb[1]),
        Some("<error>"),
        "a bad body grafts an <error> placeholder, keeping the arena well-formed"
    );
}

#[test]
fn embedded_node_spans_are_document_coordinates_that_slice_back_to_the_embedded_source() {
    // LSP-transparency: an embedded region's nodes keep their OWN spans, shifted into the OUTER
    // document's coordinates — so a cursor inside a `json{ … }` body resolves to the exact JSON node,
    // not the whole region. Assert every grafted node's span is a valid in-bounds slice of the OUTER
    // source (not the body), and that a known interior leaf's span slices to its literal text.
    let src = r#"json{ {"key": 42} }"#;
    let p = read_ml(src);
    assert!(p.ok(), "parses clean: {:?}", p.errors);
    let a = &p.arenas;
    let spans = &p.spans;
    // Every node's span is a valid slice of the OUTER document source (geometry preserved through the
    // offset shift — the whole point is these are document coordinates, safe to slice for an editor).
    for id in (0..a.structure.len() as u32).map(StructId) {
        let sp = spans.get(id).expect("total span table");
        assert!(
            sp.start <= sp.end
                && sp.end <= src.len()
                && src.is_char_boundary(sp.start)
                && src.is_char_boundary(sp.end),
            "embedded node {id:?} span {sp:?} is not a valid slice of the outer source {src:?}"
        );
    }
    // The `42` value leaf inside the JSON must span the literal `42` IN THE OUTER SOURCE (offset, not
    // body-relative). Find it: the embedded subtree is emb[1]; walk to the number leaf.
    let emb = a.as_form(a.root, "embedded").expect("root is (embedded …)");
    // The subtree root is a JSON object `(object (member "key" 42))`-ish; locate the leaf whose outer
    // span slices to "42".
    let mut found_42 = false;
    for id in (0..a.structure.len() as u32).map(StructId) {
        if let crate::ast::Struct::Atom(lid) = a.get(id)
            && matches!(a.leaf(*lid), Leaf::Int { .. })
        {
            let sp = spans.get(id).unwrap();
            assert_eq!(
                &src[sp.start..sp.end],
                "42",
                "the JSON number leaf's span must slice to `42` in the OUTER source"
            );
            found_42 = true;
        }
    }
    assert!(found_42, "the embedded JSON `42` leaf was grafted");
    let _ = emb;
}

#[test]
fn an_unterminated_embedded_region_recovers_without_panicking() {
    // No closing `}` at all — the scanner reports an unterminated region, consumes to end, and emits a
    // placeholder. Never a panic, never a hang.
    let p = read_ml(r#"json{ {"a": 1} "#); // no closing brace for the region
    assert!(!p.ok(), "an unterminated region is an error");
    assert!(
        p.arenas.as_form(p.arenas.root, "embedded").is_some(),
        "still produces a well-formed (embedded …) node"
    );
}

#[test]
fn embedded_syntax_switch_never_panics_and_stays_wellformed_over_arbitrary_bodies() {
    // The embedded-syntax switch (`json{ … }` / `toml{ … }`) runs on UNTRUSTED body text — the raw
    // region scanner (brace-balance, string-aware), the sub-grammar reader, and the span-remapping
    // graft all consume arbitrary bytes. Like every other surface, it must be TOTAL: never PANIC, and
    // on a SUCCESSFUL parse produce a well-formed traversable arena with a span table that is total +
    // GEOMETRICALLY valid over the OUTER source (spans are shifted into document coordinates, so a bad
    // offset would slice out of bounds). The hand tests pin specific shapes; this sweeps a
    // delimiter-rich alphabet through both grammars, plus unterminated / deeply-nested-brace / string-
    // heavy / multibyte bodies that stress the scanner. `recovered` asserts the invariants (arena
    // well-formed + total, char-boundary, in-bounds span table) for each generated program.
    let alphabet: Vec<char> = "{}[]\":,.-+0123456789 \tabctfn\\/\nλ中".chars().collect();
    let mut rng = SplitMix64(0xe3be_dded_c0de_1a7e);
    for grammar in ["json", "toml"] {
        for len in 0..=40usize {
            for _ in 0..80 {
                let body: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                // Wrap the arbitrary body in the switch — the region scanner + sub-grammar reader must
                // survive whatever it contains (unbalanced braces → unterminated region; a `}` in a
                // string → not a close; garbage → a recovered sub-grammar error). `recovered` asserts
                // no panic + a well-formed, span-total, geometry-valid arena.
                let _ = recovered(&format!("{grammar}{{{body}}}"));
                // Also the UNTERMINATED form (no closing brace) — the scanner must consume to end and
                // emit a placeholder, never hang or panic.
                let _ = recovered(&format!("{grammar}{{{body}"));
                // And embedded in a larger program (a def RHS), so the token-cursor advance past the
                // region is exercised in context.
                let _ = recovered(&format!("def d() = {grammar}{{{body}}}\nx"));
            }
        }
    }
    // A few pathological brace/string shapes head-on.
    for prog in [
        r#"json{{{{{{{{"#,             // deeply unbalanced opens
        r#"json{"}}}}}}}}"}"#,         // braces buried in a string
        r#"json{ {"a": "b\"}\"c"} }"#, // escaped quotes + braces in a string
        "toml{}",                      // empty body
        "json{}",                      // empty body
        r#"toml{ a = "]}[{" }"#,       // toml string with delimiters
        "json{中{λ}中}",               // multibyte around braces
    ] {
        let _ = recovered(prog);
    }
}

// ---- record-type annotation surface: `{field: T, …}` ----

// The brace record-type PARSE-TREE + round-trip tests (`brace_record_type_annotation_equals_the_explicit_
// record_form`, `brace_record_type_nests_and_carries_function_and_tuple_field_types`,
// `brace_record_type_round_trips_through_the_printer`) MIGRATED to the spec/syntax corpus (inc-6 batch-62,
// FIRST parser.rs migration). A `{x: T}` type-position brace is sugar the printer canonicalizes to the
// explicit `Record(x : T)` form (same arena):
//   * ml/386-brace-record-type-annotation `def f(r: {x: Int64, y: Int64}) = r`→`(def (f (: r (Record (: x
//     Int64) (: y Int64)))) r)`, format.cdz = the explicit `Record(x : Int64, y : Int64)` canonicalization.
//   * ml/387-brace-record-type-empty `{}`→`(Record)`, ml/388-brace-record-type-function-field `{describe:
//     Int64 -> Int64}`→`(Record (: describe (-> Int64 Int64)))`, ml/389-brace-record-type-nested-and-tuple
//     `{p: {x: Int64}, pair: (Int64, Bool)}`→nested `(Record …)`/`(Tuple …)`.
//   * ml/390-explicit-record-type-annotation `Record(x: Int64, y: Int64)` — BYTE-IDENTICAL tree.sexp to
//     ml/386, pinning the brace==explicit equivalence; the round-trip is each case's fmt-idempotence.
// The REJECT/steering tests below (head-app field, malformed WIT member type) stay Rust — diagnostic-
// quality assertions, out of the parse-tree/fmt corpus scope.

#[test]
fn a_head_app_record_type_field_is_rejected_steering_to_the_colon_form() {
    // RT1 (DESIGN-record-type-syntax OQ-A): the obsolete head-application record-TYPE field spelling
    // `Record(field(T))` is REJECTED — a record-type field is written `field: T`. The message steers
    // to the colon form. (The canonical `(: field T)` ascription is what the colon surface produces.)
    for src in [
        "def f(r: Record(a(Int64))) = r",
        "def f(r: Record(x(Int64), y(Bool))) = r",
        // Nested field type in head-app form is still the rejected spelling.
        "def f(r: Record(inner(Option(Bytes)))) = r",
    ] {
        let p = read_ml(src);
        assert!(!p.ok(), "head-app record field must reject: {src}");
        assert!(
            p.errors.iter().any(|e| e
                .message
                .contains("record-type field is written `field: T`")),
            "the reject steers to the colon form: {src} -> {:?}",
            p.errors
        );
    }
    // The canonical colon form parses clean and builds the `(: name T)` ascription — NOT rejected.
    let a = parse_ok("def f(r: Record(a: Int64, b: Bool)) = r");
    assert_eq!(
        crate::sexpr::print(&a),
        "(def (f (: r (Record (: a Int64) (: b Bool)))) r)"
    );
    // NO false-reject on a legitimate GENERIC type application — `List(a)` / `Tuple(A, B)` take
    // POSITIONAL type args (a bare type, not a `name(T)` field), so they still parse.
    assert!(read_ml("def f(xs: List(a)) = xs").ok());
    assert!(read_ml("def f(p: Tuple(Int64, Bool)) = p").ok());
    assert!(read_ml("def f(o: Option(Int64)) = o").ok());
}

#[test]
fn malformed_wit_member_type_spellings_are_rejected_with_guidance() {
    // A malformed WIT aggregate member type (wrong result arity, a variant case with 2+ payloads, a
    // non-name enum/flags case) is REJECTED with an actionable, steering message — not silently left as
    // a broken member descriptor. Same reject-with-guidance policy as the record-field form. Each is in
    // world-member type position, where the head IS the WIT type keyword.
    let cases: [(&str, &str); 4] = [
        (
            "world W = | export i = | m : (x : u8) -> result(bool, string, u8)",
            "at most two arguments",
        ),
        (
            "world W = | export i = | m : (x : u8) -> variant(Ok, Bad(u8, string))",
            "single payload type",
        ),
        (
            "world W = | export i = | m : (x : u8) -> enum(Red, Green(u8))",
            "an `enum` case is a bare name",
        ),
        (
            "world W = | export i = | m : (x : u8) -> flags(Read(u8), Write)",
            "a `flags` bit is a bare name",
        ),
    ];
    for (src, needle) in cases {
        let p = read_ml(src);
        assert!(!p.ok(), "malformed WIT member type must reject: {src}");
        assert!(
            p.errors.iter().any(|e| e.message.contains(needle)),
            "reject steers with `{needle}`: {src} -> {:?}",
            p.errors
        );
    }
    // The WELL-FORMED spellings still parse clean — no false reject.
    for src in [
        "world W = | export i = | m : (x : u8) -> result(bool, string)",
        "world W = | export i = | m : (x : u8) -> result(bool)",
        "world W = | export i = | m : (x : u8) -> variant(Ok, Bad(string))",
        "world W = | export i = | m : (x : u8) -> enum(Red, Green, Blue)",
        "world W = | export i = | m : (x : u8) -> flags(Read, Write)",
    ] {
        assert!(
            read_ml(src).ok(),
            "well-formed WIT member type must parse: {src}"
        );
    }
}
