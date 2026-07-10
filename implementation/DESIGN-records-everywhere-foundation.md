# DESIGN — The Prelude-Is-The-Only-Lookup Foundation

**Status:** implementation spec, ready to build. Follow it in order; do not re-decide.
**Crate:** `implementation/seed/crates/rcdzc` (the native `rcdzc` compiler). All `file:line`
citations are relative to `implementation/seed/crates/rcdzc/src/` unless a full path is given.
**Tasks folded:** #155 (move eager well-known names into the prelude), #159 (sum names resolve to a
ctor record — this doc moves their meta channel into stored fields), #162 (`Meta.k` → namespaced
`Symbol`), #157 (delete `parse_type_expr`). Unblocks #152 (int widths) and #156 (macros).

**The structural idea in one sentence:** a built-in type is *just a `Hir::Record`* — it carries its
ops plus a small **meta channel** (`Meta.t`, `Meta.apply`) as ordinary fields, the record flows
through `resolve` untouched, and the "what does this name MEAN as a type" question is answered by a
**meta projection done lazily in `infer`, at the use site** — never by a special-case in `resolve`.

**The five load-bearing mechanisms (all folded in; read them before §1):**

1. **`Hir::Record` is a `BTreeMap<Symbol, Hir>`.** Field order stops mattering; lookup is `O(log n)`
   by key; the "ctors first, meta appended" invariant becomes a *structural* consequence of key
   ordering (`ns: None` sorts before `ns: Some("meta")`) instead of an insertion-order convention.
2. **`Meta` is an ordinary prelude record**, `{ apply, t, capabilities, entry, … }`, each field a
   stored `Hir::Symbol`. There is NO `meta` grammar keyword. `Meta.apply` reads (via the reader's
   existing dotted-name sugar) as `(. Meta apply)` — plain member projection that yields the symbol
   value `Hir::Symbol(Symbol::meta("apply"))`. The string `"meta"` appears *nowhere* in `resolve`.
3. **One projection node: `Hir::Proj { operand, key }`**, where `key` is *just another Hir node*
   (`Hir::Int` for positional, `Hir::Symbol` for named/meta). It replaces `Hir::TupleProj` and
   `Hir::RecordProj`; `TypedNode` and lowering collapse to one arm each. `Mir::Proj` is already the
   single target — the lower end was always unified; this unifies the front end to match.
4. **`fold_proj` — a single generic reducer run during Hir construction.** `member()` builds nothing
   special: it computes `operand` and `key`, then calls `fold_proj(operand, key)`, which reduces a
   projection of a *literal* record/tuple (every prelude type/sum/module/`Meta` record) to the field
   value immediately, and otherwise returns `Hir::Proj { operand, key }` to survive to infer/lower
   (user data records, runtime tuples). This is the "folding logic in Hir construction" that keeps
   `member()` free of per-shape special-casing.
5. **Resolution carries an explicit evaluation `Mode`.** `enum Mode { Value, Key, Pattern, Quote }`.
   `Value` is the default (three-tier lookup, form dispatch). `Key` turns a bare `Name` into a
   `Hir::Symbol` *without a scope lookup* (a member key is a label, not a value) and delegates any
   non-name to `Value`. `Pattern` unifies the pattern rule (a bare `Name` binds unless it is a
   ctor). `Quote` is where `quote`/`quasiquote` fold in (a later pass; the enum leaves the slot).
   `quote`/`quasiquote` were *always* evaluation-mode switches — `Key`/`Pattern` join them as
   first-class, replacing today's ad-hoc shape-inspection (`name_of` on a key node, `prelude.get`
   peeking inside `collect_binders`).

---

## 1. Rationale — the operator rule is the north star

There is exactly ONE thing a built-in name means, and it is carried by a **value in one map**, not
by a name-string test scattered through `resolve`. The rule this refactor enforces:

> **`resolve` never branches on a _source name string_ or a _record field-name string_ to decide
> what a name or a form MEANS.** It performs exactly two generic operations, parameterized by the
> current `Mode`, plus it recognizes a small, FIXED, finite set of grammar keywords.

The two generic operations are:

1. **Name resolution** (value position, `Node::Name`, `Mode::Value`): a single three-tier lookup —
   `scope.lookup(n)` → `self.index.get(n)` → `self.prelude.get(n)` — returning the stored `Hir`
   value **verbatim** (a bare type name returns its record; the record is reduced to its type-value
   *later*, in `infer`, §6). No `match name.as_str()` follows it. In `Mode::Key` the same
   `Node::Name` becomes `Hir::Symbol(Symbol::name(n))` with NO lookup; in `Mode::Pattern` it binds
   (unless it is a ctor).
2. **Member projection** (`member()`, `(. obj key)`): resolve `obj` in `Value` and `key` in `Key`
   (so `key` is an `Hir::Int` or `Hir::Symbol` — "just another Hir node"), then `fold_proj(obj, key)`
   performs the ONE generic reduction: literal record/tuple → the field value; otherwise `Hir::Proj`.

### The invariant, stated honestly

The invariant is **not** "grep for `"meta"` / `"Int64"` returns zero hits." The honest, enforceable
invariant is:

> `resolve` hard-codes a **fixed grammar-name set** and nothing else. That set is exactly `.` and the
> true-syntax keywords `if let fn match do : const and or not def type module export quote quasiquote`.
> **`meta` is NOT in this set** — it is a prelude record field, reached by ordinary projection.
> Every name outside the grammar set resolves through the ONE generic three-tier lookup (in `Value`)
> or becomes a `Symbol`/binder (in `Key`/`Pattern`), and every field/variant/op is projected by the
> ONE generic `fold_proj`. No built-in _value_ (operator, constructor, type record, sum ctor, `unit`,
> `Meta`) is named anywhere in `resolve` except as a `prelude` map _entry_.

The number of value-dispatch arms in `resolve` drops from ~38 to zero, `member()` loses all key-shape
special-casing, and every future named feature is a map entry. The acceptance grep is in §13.

---

## 2. Violation inventory to eliminate

Every literal-name / field-string dispatch on a _named value_, and every per-shape key/pattern
special-case, that must be deleted. Line numbers verified against the working tree
(2026-07-10); where the design's earlier draft mis-cited, the **actual** line is given.

| #   | Site                  | What it is                                                                                                  | Disposition                                                                     |
| --- | --------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| V1  | `resolve.rs:718-729`  | The 9-arm `match name.as_str()` in the prelude branch (`"Int64"`→`TypeVal(Int)`, …, `"Tuple"`→`Intrinsic`)  | DELETE → return the prelude record verbatim; `infer` reduces it via `Meta.t`    |
| V2  | `resolve.rs:774-783`  | Operator arms `+ - *` → `Hir::Arith`                                                                         | DELETE → prelude `Intrinsic(Arith)` + generic head apply                        |
| V3  | `resolve.rs:784-795`  | Operator arms `& \| ^ / %` → `Hir::Bit`                                                                      | DELETE → prelude `Intrinsic(Bit)`                                               |
| V4  | `resolve.rs:796-805`  | Operator arms `<< >>` → `Hir::Shift`                                                                         | DELETE → prelude `Intrinsic(Shift)`                                             |
| V5  | `resolve.rs:806-817`  | Operator arms `< > <= >= =` → `Hir::Cmp`                                                                     | DELETE → prelude `Intrinsic(Cmp)`                                               |
| V6  | `resolve.rs:840-880`  | The 5 scope-guarded constructor forms `tuple list record map set` (`record`@856 delegates to `fn record`@1326; `map` inline @862-873) | DELETE → prelude `Builtin(BuiltinForm)` + generic head `build_form`             |
| V7  | `resolve.rs:974-1005` | The `(meta …)` inline dispatch (`match meta_key { "apply" \| "t" }`) inside `member()`                       | DELETE → `Meta` prelude record + `Key`-mode key + `fold_proj` by `Symbol`       |
| V8  | `resolve.rs:184-216`  | `parse_type_expr`'s parallel name matcher (`"Int64"`→`Ty::Int`, …, `"Tuple"`/`"List"` heads)                | DELETE → resolve payloads as ordinary values                                    |
| V9  | `prelude.rs:70-73`    | The DEAD `List/Map/Set/Tuple → Intrinsic(Type*)` inserts (overwritten by the module records at 147/164/177) | DELETE (once V1 stops re-deriving them and the builder moves into `Meta.apply`) |
| V10 | `resolve.rs:966-1047` | `member()` inspecting the key node's SHAPE (positional int @966; `name_of` on the key @1047; the meta-list fork) | REPLACE → `Key`-mode key + `fold_proj` (member never inspects key shape itself) |
| V11 | `resolve.rs:1098-1180`| Pattern resolution peeking `self.prelude.get(n)` for the ctor test in `collect_binders`/`check_linear`/`check_irrefutable` | UNIFY under `Mode::Pattern` — the binder-vs-ctor rule stated once (§8.1)         |

Sites that are **correct and stay** (structural, not name-dispatch): the empty-list `()`→`Unit`
(matched on `items.is_empty()`); the three-tier bare-name precedence (`resolve.rs:693-700`); the
head-is-list Apply arm (`resolve.rs:763-770`); `select.rs` (already fully name-free — `emit_intrinsic`
dispatches only on the `Intrinsic` enum); the pattern binder pre-pass that allocates fresh locals +
checks linearity (its *rule* moves under `Mode::Pattern`, but a pre-pass is still needed so the body
sees the binders — §8.1).

**Note on the reader.** `Meta.apply` is expressible *today* with no reader change: the s-expression
reader (`cdz-compiler/src/ast.rs:334-345`) desugars any dotted token `a.b` into `(. a b)`, with `b`
an ordinary `Node::Name`. So `Meta.apply` → `(. Meta apply)`, `(. Option Meta.apply)` →
`(. Option (. Meta apply))`. Nothing in this refactor touches the reader.

---

## 3. The prelude's new entries (concrete `Hir`)

`Prelude = HashMap<String, Hir>` (prelude.rs:22) already holds every built-in as a value. We add the
operators, the constructors, and the `Meta` record, and give the scalars / type-records / sums their
**meta fields** so one plain `Hir::Record` serves both the bare-name role and the member role. Below,
`p` is the map under construction in `prelude.rs::build()`.

### 3.1 Operators (15 entries)

```rust
use crate::ir::{ArithOp, BitOp, ShiftOp, CmpOp, Intrinsic};

p.insert("+".into(),  Hir::Intrinsic(Intrinsic::Arith(ArithOp::Add)));
p.insert("-".into(),  Hir::Intrinsic(Intrinsic::Arith(ArithOp::Sub)));
p.insert("*".into(),  Hir::Intrinsic(Intrinsic::Arith(ArithOp::Mul)));
p.insert("&".into(),  Hir::Intrinsic(Intrinsic::Bit(BitOp::And)));
p.insert("|".into(),  Hir::Intrinsic(Intrinsic::Bit(BitOp::Or)));
p.insert("^".into(),  Hir::Intrinsic(Intrinsic::Bit(BitOp::Xor)));
p.insert("/".into(),  Hir::Intrinsic(Intrinsic::Bit(BitOp::Div)));
p.insert("%".into(),  Hir::Intrinsic(Intrinsic::Bit(BitOp::Rem)));
p.insert("<<".into(), Hir::Intrinsic(Intrinsic::Shift(ShiftOp::Left)));
p.insert(">>".into(), Hir::Intrinsic(Intrinsic::Shift(ShiftOp::Right)));
p.insert("<".into(),  Hir::Intrinsic(Intrinsic::Cmp(CmpOp::Lt)));
p.insert(">".into(),  Hir::Intrinsic(Intrinsic::Cmp(CmpOp::Gt)));
p.insert("<=".into(), Hir::Intrinsic(Intrinsic::Cmp(CmpOp::Le)));
p.insert(">=".into(), Hir::Intrinsic(Intrinsic::Cmp(CmpOp::Ge)));
p.insert("=".into(),  Hir::Intrinsic(Intrinsic::Cmp(CmpOp::Eq)));
```

`(+ a b)` resolves the head `+` generically → `Hir::Intrinsic(Arith(Add))` → builds
`Apply(Intrinsic(Arith(Add)), [a, b])`. Eager form (operators are strict).

### 3.2 Collection constructors (5 entries)

```rust
use crate::ir::BuiltinForm;
p.insert("tuple".into(),  Hir::Builtin(BuiltinForm::Tuple));
p.insert("list".into(),   Hir::Builtin(BuiltinForm::List));
p.insert("record".into(), Hir::Builtin(BuiltinForm::Record));
p.insert("map".into(),    Hir::Builtin(BuiltinForm::Map));
p.insert("set".into(),    Hir::Builtin(BuiltinForm::Set));
```

`(list 1 2 3)` resolves the head `list` generically → `Hir::Builtin(List)` → the generic head branch
sees a `Builtin` and calls `build_form(List, &items[1..])`, constructing the same `Hir::List([1,2,3])`
the old eager arm produced — so infer/lower/select for collections are untouched. Eager form
(variadic; a `Builtin` marker exists precisely because a variadic constructor does not fit
`Intrinsic::signature()`'s fixed arity — see §10).

### 3.3 The `Meta` record (1 entry) — replaces the `(meta …)` grammar

`Meta` is a plain prelude record whose fields are the namespaced symbols. `Meta.k` = `(. Meta k)`
projects the field `k` (in `Key` mode `k` is a `Symbol::name("k")` used to look up the field), and the
field *value* is the meta-namespaced symbol. So `Meta.apply` reduces (via `fold_proj`, because `Meta`
is a literal record) to `Hir::Symbol(Symbol::meta("apply"))`.

```rust
// The meta channel is data, not grammar: each known meta key is a field of `Meta` holding its symbol.
// A future meta key (e.g. Meta.eval-discipline for macros, #156) is one more field here — no resolve edit.
p.insert("Meta".into(), Hir::Record(BTreeMap::from([
    (Symbol::name("apply"),        Hir::Symbol(Symbol::meta("apply"))),
    (Symbol::name("t"),            Hir::Symbol(Symbol::meta("t"))),
    (Symbol::name("capabilities"), Hir::Symbol(Symbol::meta("capabilities"))),
    (Symbol::name("entry"),        Hir::Symbol(Symbol::meta("entry"))),
])));
```

`(. Option Meta.apply)` = `(. Option (. Meta apply))`: the inner `(. Meta apply)` folds to
`Hir::Symbol(meta:apply)`; the outer `fold_proj(Option-record, Symbol(meta:apply))` projects
`Option`'s stored `Meta.apply` field (§3.5). No arm of `member()` ever names `"meta"`, `"apply"`, or
`"t"` — they are all data in the one map, reached by the one projection.

### 3.4 Scalars and type-records as plain records with meta fields

Each type name is a **plain `Hir::Record`** whose fields are `Symbol`-keyed (§5). It carries its ops
(as today) **plus** a meta channel: `Meta.t` (the type-value this name denotes, when it is a ground
type) and/or `Meta.apply` (the type-constructor to apply, when it is parametric). The record is what
the bare name resolves to; `infer` reduces it to a type-value at the use site (§6).

Helpers (prelude.rs) — `Symbol` keys are ergonomic to build:

```rust
fn field(name: &str, v: Hir) -> (Symbol, Hir) { (Symbol::name(name), v) }   // ns: None
fn meta(name: &str, v: Hir) -> (Symbol, Hir) { (Symbol::meta(name), v) }     // ns: Some("meta")
// Symbol::name(n) = Symbol { ns: None,                 name: n.into() }
// Symbol::meta(n) = Symbol { ns: Some("meta".into()),  name: n.into() }
// Records are built from an iterator of these pairs: Hir::Record(BTreeMap::from_iter([...])).
```

```rust
// Ground scalars: bare `Int64` reduces (in infer) to TypeVal(Int) via its Meta.t field; (. Int64 max)
// projects an ordinary field.
p.insert("Int64".into(), Hir::Record(BTreeMap::from_iter([
    meta("t", Hir::TypeVal(Ty::Int)),
    field("max",          Hir::Int(i64::MAX)),
    field("min",          Hir::Int(i64::MIN)),
    field("wrapping-add", Hir::Intrinsic(Intrinsic::WrappingAdd)),
    field("wrapping-sub", Hir::Intrinsic(Intrinsic::WrappingSub)),
    field("wrapping-mul", Hir::Intrinsic(Intrinsic::WrappingMul)),
    field("to-byte",      Hir::Intrinsic(Intrinsic::IntToByte)),
])));
// Bool / Unit: ground scalar, only the Meta.t field (member access on other keys declines).
p.insert("Bool".into(), Hir::Record(BTreeMap::from_iter([meta("t", Hir::TypeVal(Ty::Bool))])));
p.insert("Unit".into(), Hir::Record(BTreeMap::from_iter([meta("t", Hir::TypeVal(Ty::Unit))])));

// String / Bytes: ground Meta.t + op fields (moved verbatim from prelude.rs:134-159).
p.insert("String".into(), Hir::Record(BTreeMap::from_iter([
    meta("t", Hir::TypeVal(Ty::String)),
    field("from-bytes", Hir::Intrinsic(Intrinsic::StrFromBytes)),
])));
p.insert("Bytes".into(), Hir::Record(BTreeMap::from_iter([
    meta("t", Hir::TypeVal(Ty::Bytes)),
    field("of", …), field("len", …), field("concat", …),
    field("at", …), field("slice", …), field("compact", …),
])));

// Parametric type builders: Meta.apply is the type-builder Intrinsic; fields are the ops. NO Meta.t —
// a bare `List` is not itself a ground type, it is a type→type builder (§6).
p.insert("List".into(), Hir::Record(BTreeMap::from_iter([
    meta("apply", Hir::Intrinsic(Intrinsic::TypeList)),
    field("len", …), field("push", …), field("concat", …), field("at", …),  // from prelude.rs:146-153
])));
p.insert("Map".into(), Hir::Record(BTreeMap::from_iter([
    meta("apply", Hir::Intrinsic(Intrinsic::TypeMap)),
    field("empty", Hir::Map(vec![])), field("insert", …), field("lookup", …),
    field("remove", …), field("size", …),                                     // from prelude.rs:164-173
])));
p.insert("Set".into(), Hir::Record(BTreeMap::from_iter([
    meta("apply", Hir::Intrinsic(Intrinsic::TypeSet)),
    field("empty", Hir::Set(vec![])), field("of", …), /* insert, contains, remove, size, len, union, … */
])));                                                                            // from prelude.rs:177-191
p.insert("Tuple".into(), Hir::Record(BTreeMap::from_iter([
    meta("apply", Hir::Intrinsic(Intrinsic::TypeTuple)),                        // no ops today
])));
```

### 3.5 Sums as records with meta fields

The prelude sum loop (prelude.rs:79-102) already builds each sum as `Hir::Record(ctor fields)`. The
change is (a) the record is now a `BTreeMap<Symbol, Hir>` and (b) it gains the two meta fields, so
`(. Option Meta.apply)` / `Meta.t` project stored fields instead of firing the deleted inline dispatch:

```rust
for def in [prelude_option(), prelude_result(), prelude_sign(), prelude_ordering(), prelude_ast()] {
    let ctor = |i| Hir::Ctor { def: def.clone(), index: i };
    let mut fields: BTreeMap<Symbol, Hir> = def.variants().iter().enumerate()
        .map(|(i, v)| (Symbol::name(&v.name), ctor(i)))
        .collect();
    // The meta channel — projected by (. Option Meta.apply) / (. Option Meta.t).
    fields.insert(Symbol::meta("apply"), ctor(0));           // the type-constructor (any ctor carries the def)
    fields.insert(Symbol::meta("t"),     Hir::TypeVal(Ty::Type));
    if !def.qualified {
        for (i, v) in def.variants().iter().enumerate() { p.insert(v.name.clone(), ctor(i)); }
    }
    p.insert(def.name.clone(), Hir::Record(fields));
}
```

Because `ns: None` sorts before `ns: Some("meta")`, the named-ctor fields come first and the meta
fields last **structurally** — so any consumer that reads the first entry still lands on a ctor
(though after this refactor no consumer relies on order; C3 uses an explicit `Meta.apply` projection).
Prelude and user sums (§7) share this one shape.

---

## 4. The reduced generic `resolve` (before / after)

### 4.0 The `Mode` parameter

`expr` (resolve.rs:680, 41 call sites) gains a `mode: Mode`. To keep the common case clean, the
public entry `self.expr(node, scope)` is a thin wrapper for `self.expr_in(node, scope, Mode::Value)`;
`expr_in` is the mode-parameterized resolver. Each construct hands its children a mode explicitly —
mode is NOT globally sticky; it is chosen per child:

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Value, Key, Pattern, Quote }
```

- **`Value`** (default): the three-tier bare-name lookup + form dispatch, as today. A value child of
  any Value-mode form is resolved in `Value`.
- **`Key`**: `member()` resolves the key node here. A bare `Node::Name(n)` → `Hir::Symbol(Symbol::name(n))`
  with NO scope/index/prelude lookup (a member key is a label). A `Node::Int` → `Hir::Int` (positional
  key). Anything else (a list, e.g. `(. Meta apply)`) → delegate to `Value` (it is an expression that
  must evaluate to a symbol). This is the whole of `Key` mode.
- **`Pattern`**: pattern nodes resolve here (§8.1). A bare `Node::Name` is a *binder* unless it names a
  ctor; a `(head sub…)` head is a ctor/`tuple`/qualified-ctor and the sub-patterns recurse in
  `Pattern`; a literal is a refutable match value. Replaces the `prelude.get`-peeking in
  `collect_binders`/`check_linear`/`check_irrefutable`.
- **`Quote`**: reserved. `quote`/`quasiquote` fold in here later (#156-adjacent); until then they keep
  their existing boundary handling. The enum variant documents the intended home.

### 4.1 Bare-name lookup — `resolve.rs:693-729` (in `Value`)

**Before** (V1): the three-tier lookup, then a 9-arm `match name.as_str()` that rewrote
`Int64`→`TypeVal(Int)`, `List`→`Intrinsic(TypeList)`, etc.

**After** — fully generic; the prelude value is returned **verbatim**, no per-name rewrite:

```rust
// Mode::Value, Node::Name(name):
if let Some(id) = scope.lookup(name) { Hir::Local(id) }
else if let Some(&func) = self.index.get(name.as_str()) { Hir::Call { func, args: Vec::new() } }
else if let Some(node) = self.prelude.get(name.as_str()) { node.clone() }   // ← the whole branch
else if looks_like_numeric_literal(name) { /* CDZ0201 */ }
else { /* CDZ0101 unbound */ }
```

A bare `Int64` now resolves to its `Hir::Record`. It is `infer`, not `resolve`, that turns that record
into `TypeVal(Int)` — the meta lookup deferred to the use site (§6).

In `Mode::Key`, the same `Node::Name(name)` short-circuits to `Hir::Symbol(Symbol::name(name))` before
any of the above. In `Mode::Pattern`, it binds or resolves-as-ctor (§8.1).

### 4.2 Form head dispatch — `resolve.rs:757-956` (in `Value`)

**Before:** the head-is-list Apply path (763-770); then a `match head` special-casing `.`, the 15
operators (V2-V5), the 5 scope-guarded constructors (V6), the true-syntax keywords, and the trailing
`Call`/`Apply` arms.

**After:** the head-is-list Apply path is unchanged; `match head` keeps ONLY the fixed grammar keywords
and `.`; everything else falls to ONE generic branch that resolves the head as a value and acts on its
KIND:

```rust
match head {
    Some(".") if items.len() == 3 => self.member(items, scope),           // §4.4
    Some("if") | Some("let") | Some("fn") | Some("match") | Some("do")
    | Some(":") | Some("const") | Some("and") | Some("or") | Some("not") =>
        /* … existing grammar arms, unchanged (match resolves its patterns in Mode::Pattern) … */,
    Some("quote") | Some("quasiquote") => /* … boundary handling, unchanged (future: Mode::Quote) … */,
    _ => {
        // GENERIC: resolve the head as a value (Mode::Value), then dispatch on its KIND (never its name).
        let head_val = self.expr(&items[0], scope);   // = expr_in(.., Mode::Value)
        match head_val {
            Hir::Builtin(b)        => self.build_form(b, &items[1..], scope),  // list/tuple/map/set/record
            Hir::Call { func, .. } => {                                        // module fn (index hit)
                let args = items[1..].iter().map(|a| self.expr(a, scope)).collect();
                Hir::Call { func, args }
            }
            f => {                                                            // ctor / intrinsic / local / record
                let args = items[1..].iter().map(|a| self.expr(a, scope)).collect();
                Hir::Apply { func: Box::new(f), args }
            }
        }
    }
}
```

A type-application `(List Int64)` lands in the last arm: `f` is the `List` **record**, and infer's
Apply path reads its `Meta.apply` to get the builder (§6). `def`/`export`/`module`/`type` are
collected before body resolution and are unchanged.

### 4.3 `build_form` — new helper

```rust
fn build_form(&mut self, b: BuiltinForm, args: &[Node], scope: &Scope) -> Hir {
    match b {
        BuiltinForm::Tuple  => Hir::Tuple(args.iter().map(|e| self.expr(e, scope)).collect()),
        BuiltinForm::List   => Hir::List(args.iter().map(|e| self.expr(e, scope)).collect()),
        BuiltinForm::Set    => Hir::Set(args.iter().map(|e| self.expr(e, scope)).collect()),
        BuiltinForm::Record => self.record(&[&[/*head placeholder*/], args].concat(), scope), // reuse fn record @1326
        BuiltinForm::Map    => self.map_from(args, scope),  // reuse the body of today's map arm (862-873)
    }
}
```

`fn record` (resolve.rs:1326) already builds a `Hir::Record` from `(k v)` pairs; `build_form` calls it
(the `record` form arm at 856 was already a one-line delegate to it). `record`'s duplicate-field check
stays: it must reject a repeated field name (today's BTreeSet `seen` check) BEFORE the `BTreeMap`
silently overwrites — keep the explicit check and emit CDZ0201 on a dup (§5). The `map` body
(862-873) moves into `map_from` verbatim. `record` produces a `Hir::Record` whose keys are
`Symbol { ns: None, … }`.

### 4.4 `member()` — key-mode + fold_proj — `resolve.rs:964-1048`

**Before:** positional int (966-971); the `(meta …)` inline `match meta_key` (974-1005, V7); the
named-field find + prelude-record inline projection (1006-1047). `member()` inspected the key node's
shape and knew the string `"meta"`.

**After** — `member()` names no keys and inspects no key shape; it delegates shape to `Key` mode and
reduction to `fold_proj`:

```rust
fn member(&mut self, items: &[Node], scope: &Scope) -> Hir {
    let operand = self.expr(&items[1], scope);                 // Mode::Value
    let key     = self.expr_in(&items[2], scope, Mode::Key);   // Node::Name→Symbol, Int→Int, list→Value
    self.fold_proj(operand, key)
}
```

`fold_proj` is the ONE generic reducer (it is the "folding logic in Hir construction"):

```rust
fn fold_proj(&mut self, operand: Hir, key: Hir) -> Hir {
    match (&operand, &key) {
        // Compile-time projection of a LITERAL record by a Symbol: return the stored field value.
        // Covers every prelude type/sum/module/`Meta` record. A miss DECLINES (a not-yet-realized
        // member, or an absent meta key — e.g. capabilities/entry that a later phase reads).
        (Hir::Record(fields), Hir::Symbol(s)) => match fields.get(s) {
            Some(v) => v.clone(),
            None => Hir::Error(Reject::decline("member not present on record")),
        },
        // Compile-time projection of a LITERAL tuple by an index: the element.
        (Hir::Tuple(elems), Hir::Int(n)) if (*n as usize) < elems.len() => elems[*n as usize].clone(),
        // Runtime projection: survives to infer/lower as the single Proj node.
        (_, Hir::Int(n)) if *n < 0 =>
            Hir::Error(Reject::coded(Code::TypeError, "negative tuple index")),
        (_, Hir::Int(_)) | (_, Hir::Symbol(Symbol { ns: None, .. })) =>
            Hir::Proj { operand: Box::new(operand), key: Box::new(key) },
        // A meta-namespaced symbol against a non-literal-record operand is meaningless at run time.
        (_, Hir::Symbol(Symbol { ns: Some(_), .. })) =>
            Hir::Error(Reject::decline("meta projection on a non-record value")),
        _ => Hir::Error(Reject::decline("member-access key is not a name, index, or symbol")),
    }
}
```

`Hir::Proj { operand, key }` (key = `Hir::Int` for positional, `Hir::Symbol{ns:None}` for a named field
of a user data record) is what reaches infer/lower (§10.2). Prelude/type/sum/module/`Meta` records —
being literal `Hir::Record`/`Hir::Tuple` in hand — fold away here and never reach runtime.

---

## 5. The `Symbol` type + `Hir::Proj` + `Hir::Record` as `BTreeMap`

New IR (ir.rs) — a single symbol type, one projection node, and a map-keyed record:

```rust
use std::collections::BTreeMap;

// A namespaced compile-time symbol. It is the field-key type of a Hir::Record, the value a `Meta.k`
// projection yields, and the key of a Hir::Proj. A plain field / bare member key has ns: None; the
// `Meta` record's fields hold ns: Some("meta") symbols. The Option leaves room for future namespaces.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]   // Ord: ns:None sorts before ns:Some (ctors before meta)
pub struct Symbol { pub ns: Option<String>, pub name: String }
impl Symbol {
    pub fn name(n: &str) -> Self { Symbol { ns: None, name: n.into() } }
    pub fn meta(n: &str) -> Self { Symbol { ns: Some("meta".into()), name: n.into() } }
}

pub enum Hir {
    /* … */
    Record(BTreeMap<Symbol, Hir>),           // was Vec<(String, Hir)>
    Symbol(Symbol),                          // NEW: a compile-time symbol value (Meta.k, or a key)
    Proj { operand: Box<Hir>, key: Box<Hir> },  // NEW: replaces TupleProj + RecordProj
    /* Hir::TupleProj / Hir::RecordProj DELETED */
}
```

**Why `BTreeMap`.** The only field-order reliance today is `infer.rs:293-294` (`fields.first()` to grab
"the first ctor" for type-application) — which this refactor *already* replaces with an explicit
`Meta.apply` projection (C3, §6 use site 2). `BTreeMap` makes `.first()` stop compiling, forcing that
fix; discriminants live in `Hir::Ctor { index }`, not in position, so reordering is invisible; and key
ordering makes "ctors before meta" structural. Lookup is `fields.get(&sym)` — no linear `find`.

**Duplicate-field diagnostic is preserved.** `BTreeMap::insert` silently overwrites; `fn record`
(resolve.rs:1326) must keep its explicit dup check (the `seen` set) and emit CDZ0201
(`record names the field \`{name}\` more than once`) BEFORE inserting.

**Containment (unchanged, and now enforced by types).** `Symbol` keys and `BTreeMap` live ONLY in
`Hir::Record`. `Ty::Record` (ty.rs:257) and `TypedNode::Record` (ir.rs:629) keep `Vec<(String, …)>`
keys — they are sorted-by-name into positional slots at lowering (lower.rs:120-131), and a namespaced
key leaking there would corrupt slot order. Meta fields never reach that boundary: a type-record is
reduced to a `TypeVal` in infer *before* it would become a `TypedNode::Record` (§6), and a user data
record has only `ns: None` keys, which convert to `String` by `.name` when infer builds its
`TypedNode::Record`/`Ty::Record` (infer.rs:400-411). Infer MUST debug-assert that a data record it
types has no namespaced key.

**`Hir::Symbol` typing.** A `Hir::Symbol` that survives to infer (a `Meta.k` used as a standalone
value, not in a projection) is typed opaque — a compile-time-only symbol, a later-phase concern for
macros (#156). Give it a `Ty::Type`-adjacent opaque type marked `is_comptime_only` so the erasure
fence (§6) forbids it at runtime. (In practice every `Meta.k` is consumed by `fold_proj` at resolve
time; a bare surviving `Hir::Symbol` is rare and its opaque typing exists for completeness.)

**Shape-based disambiguation is genuine grammar via `Mode`, not value dispatch.** `(. r meta)` in
`Key` mode → `Symbol { ns: None, name: "meta" }`, projecting a _user field_ literally named `meta`.
`(. r Meta.apply)` = `(. r (. Meta apply))` — the key is a list, so `Key` mode delegates to `Value`,
evaluating it to `Hir::Symbol(meta:apply)`. The fork is `Key`-mode's "name → symbol, list → value" —
a structural mode rule, not a name test. A user record with a field called `meta` is unaffected, and a
user record with a field called `Meta` shadows the prelude `Meta` only within its own scope (ordinary
three-tier precedence).

---

## 6. The meta lookup, done lazily in `infer`

A type-record carries its meaning in its meta fields; **`infer` performs the meta projection at the
use site**, so the record can flow untouched through resolve and only "becomes a type" when something
uses it as one.

Three use sites, all in `infer`:

1. **Bare type-record used as a value → its `Meta.t`.** In `infer`'s `Hir::Record` arm, before typing
   the record structurally, check for a `meta("t")` field:
   - Has `Meta.t → TypeVal(τ)`: emit `{ node: TypedNode::TypeVal(τ), ty: Ty::Type }`. So a bare
     `Int64` / `Bool` / `String` / a fully-applied sum is a type-value typed `Ty::Type`. `(let ((t
     Int64)) t)` gives `t : Type`.
   - Else has `Meta.apply → builder` (a parametric type name like `List`/`Option` used *bare*,
     unapplied): emit the builder value (`Intrinsic(TypeList)` / `Ctor{def,0}`), typed by its own
     signature (a type→type function). A bare `List` is a builder, not a ground `Ty::Type`.
   - Else (no meta fields): type structurally as `Ty::Record` (an ordinary user/data record), after
     the debug-assert that no key is namespaced. Keys convert `Symbol{ns:None}.name → String` here.

2. **Type-application `(List Int64)` / `(Option Int64)` → head's `Meta.apply`.** In `infer`'s
   `Hir::Apply` arm, when the callee is a record, project its `meta("apply")` field (replacing the
   `fields.first()` block at infer.rs:293-302) and use *that* as the applied function:
   ```rust
   let func_for_typeapp = match func.as_ref() {
       Hir::Record(fields) => fields.get(&Symbol::meta("apply")).unwrap_or(func.as_ref()),
       _ => func.as_ref(),
   };
   ```
   The downstream ctor/intrinsic type-application logic (infer.rs:303-333) and `extract_type_value`
   (infer.rs:1155-1188) are UNCHANGED — they already reduce `Apply(Ctor, [TypeVal…])` and
   `Apply(Intrinsic(TypeList), [TypeVal…])` to the compound `Ty`.

3. **Annotation `(: e T)` / payload types → `extract_type_value`.** Unchanged: because use site 1
   reduces a bare type-record to a `TypedNode::TypeVal(τ)`, and use site 2 reduces an applied one to
   `Apply(builder, …)`, `extract_type_value` (infer.rs:1155) sees exactly the shapes it already
   handles. The annotation gate (`matches!(self.subst.apply(&tt.ty), Ty::Type)`, infer.rs:639) still
   holds: a ground type-record is `Ty::Type` by rule 1.

**Erasure is preserved.** A reduced type-record is a `TypeVal` typed `Ty::Type`, which
`is_comptime_only` marks and the post-fold erasure fence (fold.rs:74, `check_erasure_fence`, wired at
pipeline.rs:55) already forbids at runtime. Storing a bare `Int64` in a `(list Int64)` fails the fence
exactly as today. No new runtime leakage.

---

## 7. Sum-record consumers — mechanical rekey only

Because sums stay `Hir::Record`, the sites that read a sum name keep matching `Hir::Record`; they
absorb the `Vec<(String,_)> → BTreeMap<Symbol,_>` rekey and, where they compared a key to a `&str`,
use a `Symbol` / `BTreeMap::get` instead:

| Site | File:line                                  | Current                                                       | Change                                                                     |
| ---- | ------------------------------------------ | ------------------------------------------------------------- | -------------------------------------------------------------------------- |
| C1   | `prelude.rs:36-44` `sum_ref`               | `fields.iter().find_map` over **values**, matching `Hir::Ctor`| `fields.values().find_map(...)` — matches on the value, not the key; NONE-logic change (the appended meta `apply` is a ctor) |
| C2   | `resolve.rs:1227-1232` `resolve_ctor_head` | `fields.iter().find(\|(n,_)\| n == variant_name)` (`n: &String`) | `fields.get(&Symbol::name(variant_name))` — the rekey                      |
| C3   | `infer.rs:293-302` `func_for_typeapp`      | `if let Some((_, Hir::Ctor{..})) = fields.first()`             | `fields.get(&Symbol::meta("apply"))` (§6 use site 2)                       |

**User sums must carry meta fields too.** `collect_user_types` (resolve.rs:44-92) builds each user sum
as `Hir::Record(fields)` and inserts it at resolve.rs:91. Rebuild those fields in the SAME shape as
prelude sums (§3.5) — `Symbol`-keyed ctor fields + the two meta fields:

```rust
// was: let fields: Vec<(String, Hir)> = segments … ;  prelude.insert(sref.name, Hir::Record(fields));
let mut fields: BTreeMap<Symbol, Hir> = segments.iter().enumerate()
    .map(|(i, (tag, _))| (Symbol::name(tag), Hir::Ctor { def: sref.clone(), index: i }))
    .collect();
fields.insert(Symbol::meta("apply"), Hir::Ctor { def: sref.clone(), index: 0 });
fields.insert(Symbol::meta("t"),     Hir::TypeVal(crate::ty::Ty::Type));
prelude.insert(sref.name.clone(), Hir::Record(fields));
```

Prelude and user sums now share ONE representation; `(. MyUserSum Meta.apply)` projects a stored field
just like `(. Option Meta.apply)`, with no inline dispatch. **The single most load-bearing rekey is
`prelude::sum_ref` (C1)** — the sum-name→`SumRef` recovery `parse_type_expr` calls; verify it still
returns the def after fields are `Symbol`-keyed (it matches on the ctor VALUE, so it does).

---

## 8. The shadow fix + pattern unification

### 8.0 Operator/constructor shadowing (the miscompile)

The known miscompile: a `let`- or `def`-bound `+` / `list` is silently ignored because the operator
arms (resolve.rs:774-817) and constructor guards (840-880) fire before / independently of the full
lookup. Three tiers of shadowing exist today (verified):

- **Tier 1 (bare names):** already correct (scope → index → prelude, resolve.rs:693-700).
- **Tier 2 (constructors `tuple list record map set`):** shadow only against _lexical locals_ (the
  `scope.lookup(name).is_none()` guard). A top-level `(def (list …) …)` does NOT shadow.
- **Tier 3 (operators):** do NOT shadow at all — no guard.

**The fix is deletion, not new guard code.** Once operators are prelude `Intrinsic` values (§3.1) and
constructors are prelude `Builtin` values (§3.2), the generic head branch (§4.2) resolves the head
through `self.expr` → the three-tier lookup:

- `(let ((+ f)) (+ 1 2))`: head `+` → generic branch → `self.expr(+)` → `scope.lookup("+")` HIT →
  `Hir::Local(id)` → `Apply(Local, [1, 2])`. **Tier 3 fixed.**
- `(def (list …) …)` then `(list 1 2)`: generic branch → `self.expr(list)` → `self.index` HIT →
  `Hir::Call { func }` → the `Hir::Call` arm builds `Call { func, args }`. **Tier 2 fixed.**
- Unshadowed common case preserved: `+` → `Intrinsic(Arith(Add))` → `Apply(Intrinsic,…)`; `(list 1 2)`
  → `Builtin(List)` → `build_form` → `Hir::List`.

Add two tests (tests.rs): an operator shadow `(let ((+ my-add)) (+ 1 2))` and a def-level constructor
shadow `(def (list …) …) … (list 3)`.

**Altitude note:** an unbound callee name now yields `Apply { func: Hir::Error, args }` instead of the
old `_ => decline`. `Hir::Error` must propagate through infer's `Apply` path as a diagnostic (it
already carries the `Reject`); confirm the unbound-name message still surfaces.

### 8.1 Pattern resolution under `Mode::Pattern`

Today the binder-vs-ctor rule is stated three times, each peeking `self.prelude.get(n)`:
`collect_binders` (resolve.rs:1098), `check_linear_rec` (1132), `check_irrefutable` (1161). `Mode::Pattern`
states the rule once: when resolving a pattern node, a bare `Node::Name(n)` is a **binder** unless `n`
resolves (via the ONE prelude lookup) to a `Hir::Ctor`, in which case it is that nullary ctor pattern;
`_` is the wildcard; a `(head sub…)` head resolves in `Value` (a ctor / `tuple` / qualified ctor
`(. T V)`) and the sub-patterns recurse in `Pattern`; a literal is a refutable value.

**What `Mode::Pattern` unifies:** the *resolution* of a pattern to its `Hir` (the arm's pattern half,
resolve.rs:1082/1089) now runs through `expr_in(pat, scope, Mode::Pattern)` — the head/ctor/qualified
recursion that `resolve_arm` open-codes becomes the ordinary `Value`/`Pattern` recursion, and the
binder-vs-ctor test is the single prelude lookup keyed by `Mode::Pattern`.

**What stays:** the binder **pre-pass** is still required — the body must see the binders in scope, so
`resolve_arm` (resolve.rs:1074) still (a) checks linearity (CDZ0102) and (b) allocates a fresh local
per binder and extends the scope BEFORE resolving the pattern and body. `Mode::Pattern` removes the
duplicated `prelude.get` peeking and the "resolve-pattern-as-expression-with-a-ctor-exception" open
coding, not the pre-pass. Honest scope: this is a real de-duplication (three ad-hoc ctor tests → one
mode rule), not the elimination of pattern machinery.

Keep the existing CDZ codes and their tests: linearity CDZ0102, refutable-in-binding CDZ0210 /
non-exhaustive CDZ-code, shape-incompatible CDZ0201. `Mode::Pattern` must not change which code fires
for which input — verify against the pattern corpus.

---

## 9. What stays true syntax

Kept as name-keyed arms in `form()` because they **bind names** or **control evaluation** — they
cannot be values whose args are eagerly resolved. These are grammar; matching their names does NOT
violate the operator rule:

- `if` — lazy branches.
- `and` / `or` / `not` — short-circuit; desugar to `Hir::If`. (May become prelude macros under #156.)
- `let`, `fn` — bind names / params, introduce scope.
- `match` — binds pattern binders (resolves them in `Mode::Pattern`, §8.1), dispatches.
- `do` — sequences + scopes declarations to following forms.
- `:` annotation, `const` — operate on UNEVALUATED type/expr structure at compile time.
- `def` / `export` / `module` / `type` — top-level declaration grammar (collected pre-body).
- `quote` / `quasiquote` — evaluation boundaries (the future home of `Mode::Quote`).
- `.` member access — resolves its operand in `Value` and its key in `Mode::Key`.

**`meta` is NOT in this list.** It was never true syntax in a released sense (it appears only in design
docs, never in `.cdz`/spec/corpus/tests); it is now the prelude record `Meta`, reached by projection.
Everything that is a VALUE (operators, constructors, type records, sum ctors, `unit`, `Meta`) leaves
this match and lives in the prelude.

---

## 10. New IR surface (ir.rs), counted

### 10.0 Added / deleted

**Added:**

1. `Intrinsic::Arith(ArithOp)`, `Intrinsic::Bit(BitOp)`, `Intrinsic::Shift(ShiftOp)`,
   `Intrinsic::Cmp(CmpOp)` — 4 variants wrapping the EXISTING op enums (`ArithOp`/`BitOp`/`ShiftOp`/`CmpOp`
   live at ir.rs:64-101; the `Intrinsic` enum itself is at ir.rs:112-179 and today has NO operator
   variants). No new op enums.
2. `Hir::Builtin(BuiltinForm)`, `enum BuiltinForm { List, Tuple, Set, Map, Record }` — 1 kind + a
   5-variant marker enum.
3. `struct Symbol { ns: Option<String>, name: String }` (derives `Ord`) + `Hir::Symbol(Symbol)`.
4. `Hir::Proj { operand: Box<Hir>, key: Box<Hir> }` — the unified projection; key is `Hir::Int` or
   `Hir::Symbol`.
5. `Hir::Record` rekeyed `Vec<(String, Hir)>` → `BTreeMap<Symbol, Hir>`.
6. `enum Mode { Value, Key, Pattern, Quote }` (resolve.rs, not ir.rs — it is a resolver-internal knob).

**Deleted:**

- `Hir::Arith / Bit / Shift / Cmp` (ir.rs:554-557) — `resolve` no longer produces them; infer's four
  operator arms (infer.rs:543-593) and lower's four Hir arms go.
- `Hir::TupleProj` (ir.rs:551) and `Hir::RecordProj` (ir.rs:553) — replaced by `Hir::Proj`.
- `TypedNode::TupleProj` (ir.rs:638) and `TypedNode::RecordProj` (ir.rs:641) — replaced by
  `TypedNode::Proj` (§10.2).

**The LOWERED operator representation stays:** `TypedNode::Arith/Bit/Shift/Cmp` (ir.rs:642-645) and
`Mir::Arith/Bit/Shift` (ir.rs:743-745) plus the struct-form `Mir::Cmp { op, operand_ty, a, b }`
(ir.rs:750 — NOT a tuple variant, and NOT at 743-745) are the targets operators route INTO (§10.1).
`Mir::Proj { slot, elem_ty, operand }` (ir.rs:742) is already the single projection target — the lower
end was always unified.

### 10.1 Operator intrinsics: signature, param_count, fold_const, and lowering

`Intrinsic::Arith/Bit/Shift/Cmp` need arms in the exhaustive `impl Intrinsic` methods and in `select`:

- **`signature()` (ir.rs:285):** `Arith`/`Bit`/`Shift` → `(vec![Ty::Int, Ty::Int], Ty::Int)`. `Cmp` →
  `(vec![Ty::Param(0), Ty::Param(0)], Ty::Bool)` with `param_count`=1 (its operands share one ordered
  type). infer's generic `Hir::Intrinsic` arm (infer.rs:222-232) already instantiates `param_count`
  fresh vars, so `(< a b)` unifies both operands to one fresh type — reproducing the old `Hir::Cmp`
  arm's `unify_at(&ta.ty, &tb.ty, …)` exactly.
- **`param_count()` (ir.rs:210):** `Arith`/`Bit`/`Shift` → 0; `Cmp` → 1. **This match is exhaustive
  with no wildcard — add the arms or the crate does not compile.**
- **`fold_const()` (ir.rs:371): MUST use CHECKED semantics.** `Arith` folds via
  `checked_add/sub/mul`, returning `None` on overflow (matching `Mir::Arith`/`fold_arith`). `Bit::Div/Rem`
  use `checked_div` and the `y == -1 → 0` rem rule (mirror `fold_bit`). If any stray
  `Apply(Intrinsic(Arith))` reaches fold un-lowered, wrapping here would silently diverge from the
  trapping `Mir::Arith`; CHECKED keeps parity. `Cmp` folds two int/bool constants to a `Mir::Bool`.
- **lower (lower.rs, the `Apply` dispatch, matching on `func.node`):** add arms turning applied
  operator intrinsics into the EXISTING `Mir` operator nodes so `select`'s checked paths are reused:
  ```rust
  TypedNode::Intrinsic(Intrinsic::Arith(op)) => Mir::Arith(op, box lower(a), box lower(b)),
  TypedNode::Intrinsic(Intrinsic::Bit(op))   => Mir::Bit(op,   box lower(a), box lower(b)),
  TypedNode::Intrinsic(Intrinsic::Shift(op)) => Mir::Shift(op, box lower(a), box lower(b)),
  TypedNode::Intrinsic(Intrinsic::Cmp(op))   => Mir::Cmp {
      op, operand_ty: args[0].ty.clone(),   // EXTRACT operand_ty from args[0].ty (both operands unified)
      a: box lower(a), b: box lower(b),
  },
  ```
  `Mir::Cmp` carries `operand_ty` (select needs it to pick i64-signed vs i32-unsigned and to route
  compound equality); Arith/Bit/Shift do not.
- **`select.rs:701` `emit_intrinsic`:** exhaustive `match op` with NO top-level wildcard. Add the 4
  variants as declines (only a bare/partially-applied operator value that survived fold reaches here,
  and it cannot emit):
  ```rust
  Intrinsic::Arith(_) | Intrinsic::Bit(_) | Intrinsic::Shift(_) | Intrinsic::Cmp(_) =>
      Err("a bare operator value cannot cross to run time (apply it)".to_string()),
  ```

### 10.2 The unified `Hir::Proj` through infer and lower

**`TypedNode::Proj { operand: Box<Typed>, selector: ProjSelector }`** replaces `TypedNode::TupleProj`
and `TypedNode::RecordProj`, where `enum ProjSelector { Index(usize), Field(String) }` — the compile-
time-resolved selector (a `Field` is a `ns:None` name; a meta selector never reaches infer because it
folded at resolve time). infer's single `Hir::Proj` arm subsumes today's two arms (infer.rs:465-541):

```rust
Hir::Proj { operand, key } => {
    let tr = self.expr(operand)?;
    match key.as_ref() {
        Hir::Int(n) => { /* tuple-projection typing: today's Hir::TupleProj arm (499-541), producing
                            TypedNode::Proj { operand, selector: Index(n) } */ }
        Hir::Symbol(Symbol { ns: None, name }) => { /* record-projection typing: today's Hir::RecordProj
                            arm (465-497), producing TypedNode::Proj { operand, selector: Field(name) } */ }
        _ => Err(Reject::decline("projection key is not an index or field name")),
    }
}
```

lower's single `TypedNode::Proj` arm subsumes today's two (lower.rs:163-185):

```rust
TypedNode::Proj { operand, selector } => {
    let slot = match selector {
        ProjSelector::Index(i) => i,                          // positional: index IS the slot
        ProjSelector::Field(f) => match &operand.ty {          // name → slot via sorted record type
            Ty::Record(fields) => fields.iter().position(|(n, _)| *n == f).expect("field present (infer)"),
            _ => 0,                                            // unreachable (infer proved record-ness)
        },
    };
    Mir::Proj { slot, elem_ty: typed.ty.clone(), operand: Box::new(lower(*operand)) }
}
```

`finalize` (infer.rs:1103-1104) collapses its two Proj arms to one; `hir_uses_local` (infer.rs:1000-1001)
collapses to one `Hir::Proj { operand, .. } => hir_uses_local(operand)`. `render.rs` operates on
`Ty`/`Mir`, not `Hir`/`TypedNode`, so it is untouched.

---

## 11. Sequenced migration increments

Each increment lands independently and MUST stay gate-green. **Baseline:** commit at the start of this
work (currently 342/378 realized corpus cases; do NOT let the FAIL set grow).

**Gate command (run from `implementation/seed`):**

```
cargo test -p rcdzc \
  && CADENZA_COMPILER=v2 CADENZA_RUNTIME=<fresh cdz_runtime.wasm> \
       cargo run -p cadenza-seed -- behavior-gate ../../spec/semantics
```

`CADENZA_RUNTIME` MUST be a FRESH `cargo component build` runtime wasm (a stale one → false "runtime
missing X" fails; a plain `cargo build` core module → all heap cases fail). **Bar for every increment:
`cargo test -p rcdzc` exit=0 AND the behavior-gate FAIL SET does not grow vs the pre-increment
baseline** (diff the FAIL set, not the PASS count — P/todo/skip drift is noise).

- **INC 0 — dead wiring (no behavior change).** Add `Intrinsic::Arith/Bit/Shift/Cmp` variants; add
  their `signature()`/`param_count()`/`fold_const()` arms (§10.1, CHECKED fold); add the lower
  `Apply(Intrinsic(op))→Mir::op` routing arms; add the `emit_intrinsic` decline arms. Nothing produces
  the new variants yet. _Gate:_ `cargo test -p rcdzc` compiles and passes; behavior gate unchanged.

- **INC 1 — operators → prelude (fixes tier-3 shadow).** Insert the 15 operator entries (§3.1). DELETE
  V2-V5 (resolve.rs:774-817). DELETE `Hir::Arith/Bit/Shift/Cmp` nodes + infer's four operator arms
  (infer.rs:543-593). Add the operator-shadow test. _Gate:_ operator + comparison corpus green; new
  shadow test green; FAIL set not grown.

- **INC 2 — constructors → prelude (fixes tier-2 shadow).** Add `Hir::Builtin(BuiltinForm)`; insert the
  5 entries (§3.2); add the generic head `Builtin` branch + `build_form` (§4.2/§4.3, reusing `fn record`
  and moving the `map` body). DELETE V6 (resolve.rs:840-880). Add the def-level constructor shadow test.
  _Gate:_ collection corpus green; shadow test green; FAIL set not grown.

- **INC 3 — the `Mode` enum + unified `Hir::Proj` + `Symbol`/`BTreeMap` rekey + type-records carry
  through (deepest edit; land alone).** Add `enum Mode` and the `expr_in`/`expr` split (§4.0). Rekey
  `Hir::Record` `Vec<(String,_)> → BTreeMap<Symbol,_>` (§5); update every construction/lookup
  (mechanical; §12). Replace `Hir::TupleProj`/`RecordProj` with `Hir::Proj` and `TypedNode::*Proj` with
  `TypedNode::Proj`/`ProjSelector`; collapse the infer + lower + finalize + hir_uses_local arms (§10.2).
  Rebuild `Int64/Bool/Unit/String/Bytes/List/Map/Set/Tuple` as records with meta fields (§3.4). DELETE
  V1 (resolve.rs:718-729) → bare name returns the record verbatim (§4.1). DELETE V9 (prelude.rs:70-73).
  Rewrite `member()` to `Key`-mode key + `fold_proj` (§4.4); DELETE V10 (the key-shape inspection).
  Teach infer to reduce a bare type-record via `Meta.t`/`Meta.apply` (§6, use sites 1+2). Sums are NOT
  yet meta-bearing here, so C2 (`resolve_ctor_head`) needs only the rekey; C1 (`sum_ref`) is unchanged.
  _Gate:_ first-class-type + module-access + type-application + tuple/record-projection corpus green;
  FAIL set not grown.

- **INC 4 — the `Meta` record + sum meta fields (task #162).** Insert the `Meta` prelude record (§3.3).
  Give prelude sums (§3.5) AND user sums (§7) their `Meta.apply` / `Meta.t` fields. DELETE V7
  (resolve.rs:974-1005) — the `(meta …)` inline dispatch — now that `Meta.k` folds through `fold_proj`.
  Verify C1/C2/C3 hold with meta fields present. _Gate:_ `Meta.apply`/`Meta.t` corpus green on BOTH
  prelude and user sums; capabilities/entry meta still declines (absent field → decline); FAIL set not
  grown.

- **INC 5 — `Mode::Pattern` (unify pattern resolution).** Route the arm's pattern half through
  `expr_in(pat, scope, Mode::Pattern)` (§8.1); state the binder-vs-ctor rule once; DELETE the
  `prelude.get` peeking in `collect_binders`/`check_linear_rec`/`check_irrefutable` (keep the linearity
  + fresh-local pre-pass). Keep every CDZ code firing on the same input. _Gate:_ full pattern +
  exhaustiveness corpus green; CDZ0102/0210/0201 tests unchanged; FAIL set not grown.

- **INC 6 — delete `parse_type_expr` (task #157).** Sum-decl payloads resolve through the uniform value
  resolver + `extract_type_value` — a payload naming `Int64` reduces via its `Meta.t`; a payload naming
  a sum reduces via its `Meta.apply`. DELETE V8 (resolve.rs:184-216). _Gate:_ payload-type corpus green;
  FAIL set not grown.

---

## 12. Full blast-radius edit list

- **ir.rs** — ADD `Intrinsic::Arith/Bit/Shift/Cmp` (4 variants); `Hir::Builtin` + `BuiltinForm`;
  `struct Symbol` (derive `Ord`) + `Hir::Symbol`; `Hir::Proj`; `TypedNode::Proj` + `enum ProjSelector`.
  REKEY `Hir::Record` to `BTreeMap<Symbol, Hir>`. DELETE `Hir::Arith/Bit/Shift/Cmp` (554-557),
  `Hir::TupleProj`/`RecordProj` (551/553), `TypedNode::TupleProj`/`RecordProj` (638/641). EXTEND
  `signature`/`param_count`/`fold_const` for operator intrinsics (§10.1).
- **prelude.rs** — DELETE dead inserts (70-73). ADD 15 operator + 5 constructor + 1 `Meta` entry.
  REBUILD 9 type entries as `BTreeMap<Symbol,_>` records with meta fields (§3.4). ADD meta fields to the
  sum loop (79-102). REKEY `sum_ref` (36-44) to `.values()` (C1; no logic change).
- **resolve.rs** — ADD `enum Mode` + `expr_in`/`expr` split (§4.0). DELETE V1 (718-729), V2-V5
  (774-817), V6 (840-880), V7 (974-1005), V8 (184-216), V10 (member key-shape 966-1047). SIMPLIFY the
  bare-name branch to `node.clone()` (§4.1). ADD the generic head branch + `build_form` (§4.2/§4.3),
  `member()` = `Key`-key + `fold_proj` (§4.4). REKEY `resolve_ctor_head` (1229, C2) and
  `collect_user_types` fields (69-91, §7) to `BTreeMap<Symbol,_>`. Route pattern resolution through
  `Mode::Pattern` and DELETE the `prelude.get` peeking in `collect_binders`/`check_linear_rec`/
  `check_irrefutable` (§8.1, keep the pre-pass). KEEP `fn record`'s dup-field check (§5).
- **infer.rs** — DELETE the four operator arms (543-593); operators type via the generic
  `Hir::Intrinsic` arm (222-232). REPLACE the two Proj arms (465-541) with one `Hir::Proj` arm →
  `TypedNode::Proj`/`ProjSelector` (§10.2); collapse `finalize` (1103-1104) and `hir_uses_local`
  (1000-1001) to one Proj arm each. ADD type-record reduction in the `Hir::Record` arm (§6 use site 1)
  and `Meta.apply` type-application in the `Hir::Apply` arm (§6 use site 2, replacing `func_for_typeapp`
  293-302). CONVERT `ns:None` `Symbol` keys → `String` when building `TypedNode::Record`/`Ty::Record`
  (400-411) + debug-assert no namespaced key reaches there. ADD `Hir::Symbol` opaque typing (§5).
- **lower.rs** — ADD the four `Apply(Intrinsic(op))→Mir::op` routing arms, extracting `operand_ty` for
  Cmp (§10.1). REPLACE the two `TypedNode::TupleProj`/`RecordProj` arms (163-185) with one
  `TypedNode::Proj` arm (§10.2). Record lowering (120-131) is UNCHANGED — it consumes `String`-keyed
  `TypedNode::Record`.
- **fold.rs** — no structural change: `Apply(Intrinsic(op))` folds via `fold_const`; the `Mir::Proj`
  literal-product fold (606-625) is UNCHANGED (it already reduces module records that lowered to
  `Mir::Tuple`). VERIFY a folded `(+ 1 2)` stays a constant `Mir::Int(3)`. Erasure fence (74) unchanged.
- **select.rs** — ADD the four `Intrinsic::Arith/Bit/Shift/Cmp` decline arms in `emit_intrinsic` (701).
  Its `Mir::Arith/Bit/Shift/Cmp` and `Mir::Proj` emit arms are UNCHANGED.
- **tests.rs** — ADD an operator-shadow test and a def-level constructor-shadow test (§8.0). No existing
  `TupleProj`/`RecordProj`/`Hir::Arith` literals to migrate (grep: none in tests.rs).

---

## 13. THE DISCIPLINE

This spec exists **because agents keep hard-coding** built-in meanings into `resolve` instead of adding
a map entry. The point is to make the ONLY way to add a named thing be a prelude entry. Every
increment's bar therefore includes a mechanical check:

1. **No new literal-name value dispatch.** After the increment, in `resolve.rs`:
   `grep -nE 'name\.as_str\(\)\s*==|match name\.as_str\(\)'` shows the head remains only the three-tier
   lookup and the FIXED grammar-keyword `match head` — no new arm returning a _value_ from a source name.
2. **No new field-string match on a built-in.** `grep -nE '"\w[\w-]*"\s*=>' resolve.rs` shows no NEW arm
   keying a built-in operator/constructor/type/op/meta-key by its string. The only string literals
   matched are the FIXED grammar set (§1: `.` and the true-syntax keywords). **`"meta"`, `"apply"`,
   `"t"` must NOT appear in `resolve.rs` at all** (they are `Meta`-record data / `Symbol::meta` calls in
   `prelude.rs`).
3. **The only named-thing resolution is the generic prelude lookup + the generic projection.** Every
   built-in `+`, `list`, `Int64`, `Option`, `Some`, `Meta.apply` resolves by cloning a `prelude` value
   and (for members) `fold_proj` of a `Symbol` produced in `Mode::Key` — never by a name test that
   yields a value, and never by `member()` inspecting the key node's shape.

**Honest acceptance:** the set of source-name strings `resolve` matches to produce meaning is exactly
the fixed grammar set of §1 (which no longer includes `meta`), and that set does not grow when a new
built-in VALUE (operator, constructor, type record, sum, meta key, width, macro) is added — a new value
is a `prelude.insert`, a new meta key is a `Meta`-record field. If adding a feature requires a new arm
in a `resolve` `match` on a name or field string, the discipline is violated and the change is wrong.

---

## 14. What it unblocks

- **Int widths (#152).** A width is ONE prelude entry — no `resolve` edit:
  ```rust
  p.insert("Int8".into(), Hir::Record(BTreeMap::from_iter([
      meta("t",   Hir::Intrinsic(Intrinsic::TypeInt(8))),  // or TypeVal(Ty::Int8) once the Ty exists
      field("max", Hir::Int(127)),
      field("min", Hir::Int(-128)),
      field("of",  Hir::Intrinsic(Intrinsic::IntOf(8))),
  ])));
  ```
  Bare `Int8` reduces via `Meta.t`; `Int8.max` / `Int8.of` project generically; a parametric `(Int n)`
  type-applies via the head's `Meta.apply`. Before this refactor each width needed a new arm in the
  resolve bare-name match, possibly a `parse_type_expr` arm, and a member special-case.
- **Macros (#156).** A macro drops in as a prelude value carrying a `Meta.eval-discipline` field — one
  more `Meta`-record field, one more prelude entry. Because `Meta.k` is a generic namespaced-`Symbol`
  projection, `Mode::Quote` has a reserved home, and the head of a form is resolved generically before
  dispatch, a macro-tagged prelude value is recognized by KIND in the same `form()` fallthrough that
  builds `Apply` — no new syntax, no name hard-code.

Every future named feature lands as data in the ONE map. A built-in type is a record, its meaning is a
meta field, and the meaning is read when the value is used. Resolution — one generic name lookup, one
generic projection, one `fold_proj`, and a small `Mode` — never grows a value special-case again.
