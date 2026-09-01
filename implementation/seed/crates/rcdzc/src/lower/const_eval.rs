//! `lower::const_eval` — the general compile-time constant evaluator, split out of `lower.rs`. A
//! self-contained value interpreter over a closed `CVal` value tree (`DESIGN-general-const-eval.md`):
//! `const_eval`/`const_eval_inner` drive it, `core_to_cval`/`cval_to_core`/`cval_to_ast` bridge to the
//! `Core`/AST world, `const_eval_apply`/`apply_const_prim` evaluate calls + primitives, and
//! `const_pattern_matches`/`cval_eq`/`cval_seq_eq` decide match arms — so a nested recursion consuming
//! another recursion's constant result folds where the old unroll-and-refold could not. Behaviour-
//! preserving move: private items become `pub(super)` and reach the rest of the tree via `use super::*`.

use super::*;

/// Whether the node at `id` lowers to a compile-time CONSTANT value — a constant scalar/string/float/
/// unit, or a constant compound (`SumNew`/`Tuple`/`Record`/`ListNew`/`MapNew`) all of whose parts are
/// constant. Used to decide whether a `Map.insert` key can fold (a constant key merges into a constant
/// map; a runtime key keeps the persistent op). Mirrors the constant test `const_value_ast` performs.
/// Whether an already-lowered [`Core`] is a compile-time CONSTANT value — the [`is_const_value`] test on a
/// `Core` in hand (rather than a node to lower). Used by the recursive-const-fold unroll to accept only a
/// fully-folded constant result.
/// A self-contained compile-time constant VALUE for the general const-evaluator (`DESIGN-general-const-eval.md`,
/// Stage a: scalars + lists). Unlike `Core` — whose compounds reference AST nodes, so a nested const result
/// cannot flow as a value into an enclosing fold — a `CVal` is a closed value tree, so a nested const call
/// evaluates to a `CVal` the caller consumes directly. That native composition is exactly what the
/// unroll-and-refold could not do (a recursion consuming another recursion's const result; a let-bound
/// nested-recursion result carried through a filter). Stage a covers Int/Bool/Str/Bytes/Unit and homogeneous
/// lists; Ast/sum/record/map values are later stages (an unhandled form yields `None`, so the caller falls
/// through to the existing unroll/decline — a COMPLETENESS gain on the accept path, never a miscompile).
#[derive(Clone)]
pub(super) enum CVal {
    Int(crate::ast::IntValue),
    Bool(bool),
    /// A `Char` value (a Unicode scalar). Materializes to `Core::ConstChar`. Equality/ordering compare by
    /// scalar value; `Char.to-int` reads the scalar, `Char.from-int` builds one (fallibly, via `Option`).
    /// Carried so a Char threaded through a RECURSION const-folds — a non-recursive Char op already folds via
    /// `core_of`, but the recursive engine runs the value-interpreter, which had no Char value.
    Char(char),
    /// A `Float` value — the EXACT `Decimal` (matches `Core::ConstFloat`). Arithmetic (`+`/`-`/`*`/`/`) folds at
    /// the operation node's solved WIDTH exactly like `lower_float_arith` (a non-finite result declines);
    /// equality is by canonical bits, ordering by IEEE partial order (both at the operand width, in the prim
    /// block, which has the node). Only a FINITE float is ever a `CVal::Float` (a `nan` is `Core::ConstFloatNan`,
    /// declined by `core_to_cval`). Carried so a Float threaded through a RECURSION const-folds (ca03).
    Float(crate::ast::Decimal),
    // `Arc` (not `Rc`) to match `cadenza-ast`'s `Leaf::Str(Arc<str>)`/`Leaf::Bytes(Arc<[u8]>)` + `Core`'s
    // `ConstStr`/`ConstBytes`, so a value flows leaf↔Core↔CVal with a refcount bump, no re-allocation.
    Str(std::sync::Arc<str>),
    Bytes(std::sync::Arc<[u8]>),
    Unit,
    List(Vec<CVal>),
    /// A SUM / variant value (Stage b) — an `Ast` node, an `Option`/`Result`, or any user sum. `disc` is the
    /// variant discriminant (matched against a variant pattern) and `payloads` are the constructor's argument
    /// values (empty for a nullary variant like `Option.None`). Re-materializes to a `Core::SumNew { disc,
    /// payloads }` directly (no constructor head needed — payloads are synthesized const nodes), so a value
    /// SOURCED from `core_of` (a reflected `Ast.module` form, a quote) round-trips too, not just a
    /// syntactically-constructed one.
    Sum {
        disc: u32,
        payloads: Vec<CVal>,
    },
    /// A RECORD value (Stage b) — named fields, canonically ordered by the `Symbol` key. Built by a record
    /// literal; projected by member access. The substrate of the operator's `contract(m) -> Record(id, …)`
    /// API, where a caller reads `.id` off a const-folded descriptor.
    Record(std::collections::BTreeMap<crate::resolved::Symbol, CVal>),
    /// A TUPLE value (Stage b) — a fixed positional product; projected by index.
    Tuple(Vec<CVal>),
    /// A TAKEN `trap` in a const-fold — the const-evaluator executed a `trap("msg")` on the folded path (e.g. a
    /// self-reflection transform hit a genuinely-missing required pragma). Carries the trap's message. It
    /// PROPAGATES like an exception: any operation consuming it short-circuits to it (the `ce!` combinator), so
    /// the whole fold becomes this trap; `cval_to_core` materializes it to a `Core::Poison(ConstTrap, msg)` so
    /// the trap's MESSAGE surfaces as the compile error (CDZ0304) — not the generic "runtime AST value"
    /// decline. This makes a const-executed trap a fail-loud, actionable compile-time error.
    Trap(std::sync::Arc<str>),
    /// A CLOSURE value — a `fn`/lambda captured as a first-class const value so a HIGHER-ORDER fold works: a
    /// `const f: (T) -> U` parameter bound to a lambda argument, then APPLIED per element (a `List.map`/
    /// `filter`/`fold`, or a user recursive map that threads a closure). `node` is the lambda literal's
    /// occurrence (its params + body resolve via `lambda_of`); `env` is the binding environment CAPTURED at
    /// closure creation, so a closure over an outer `const` binding evaluates correctly. A closure exists
    /// ONLY transiently during folding — it never materializes to a runtime/emitted value: `cval_to_core`/
    /// `cval_to_ast` decline it, and `cval_eq` does not compare it (a function is not observable data).
    Closure {
        node: StructId,
        env: std::rc::Rc<CEnv>,
    },
    /// A MAP value — an association list `(key, value)` in INSERTION order (latest write per key wins;
    /// `insert` replaces in place or appends). Kept as a `Vec` (not a sorted/hashed map) because it is only
    /// ever QUERIED at compile time (`lookup`/`size`), NEVER MATERIALIZED to a `Core`: `cval_to_core`/
    /// `cval_to_ast` DECLINE a `CVal::Map`. That is the soundness guard — the runtime map is a CHAMP whose
    /// ITERATION ORDER the compiler must not presume, so a const map that would be emitted/encoded (order-
    /// exposing) is declined; only order-INDEPENDENT results (a looked-up value, a size) fold. Equality is
    /// order-independent (same key set → same per-key values).
    Map(Vec<(CVal, CVal)>),
    /// A SET value — the distinct members in INSERTION order. Like `CVal::Map`, only ever QUERIED (contains
    /// / size), NEVER MATERIALIZED (`cval_to_core`/`cval_to_ast` decline it): the runtime set is a CHAMP
    /// whose iteration order the compiler must not presume, so an order-exposing use declines and only
    /// order-independent results (a membership bool, a size) fold. Equality is order-independent.
    Set(Vec<CVal>),
}

/// The const-evaluator's binding environment: a CALL parameter's name occurrence → its argument value.
/// Only call params live here; a `let`-bound reference follows its `Ref` to the initializer (re-evaluated in
/// the current env), and a match pattern binder is a `SumPayload` projection of the (re-evaluated) scrutinee
/// — so nothing else needs an explicit binding.
pub(super) type CEnv = crate::fxhash::FxHashMap<StructId, CVal>;

/// Evaluate `node` to a compile-time constant `CVal` under `env`, or `None` if it is not a constant this
/// stage can evaluate. `budget` debits one per step so a non-terminating / explosively-growing recursion
/// declines rather than hangs (the same soundness guard the unroll relies on). Reuses `resolved_of` for the
/// resolved form; delegates nothing to `core_of` (it is a closed value interpreter), so it composes.
pub(super) fn const_eval(
    db: &mut Db,
    node: StructId,
    env: &CEnv,
    budget: &mut u64,
) -> Option<CVal> {
    // NATIVE-DEPTH GUARD (shared `db.descent_depth` — the compiler-wide recursion policy the `rcdzc-compile`
    // worker stack in `host.rs` is sized from). `const_eval` <-> `const_eval_apply` mutually recurse to fold
    // an application; the `budget` below bounds cumulative WORK but NOT native call DEPTH, so an EXPLOSIVE
    // self-application — `(v1 v1)` whose fold re-applies unboundedly (v-cdz-smith's `selfapp-typeinfer-
    // overflow` escape, seed 14281198340853570680) — drove this recursion PAST the native stack and
    // HARD-ABORTED the worker (SIGABRT, bypassing `catch_unwind` + the hang watchdog) BEFORE the budget ran
    // out. Bounding it here on the SAME counter as `core_of`/`type_of` caps the COMBINED lowering+fold depth
    // at the stack-sized limit: past it, DECLINE the fold (`None`) — the caller falls back to runtime
    // lowering / the resource-limit decline — instead of overflowing. Far above any real fold's depth.
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        trace!(target: "rcdzc::lower", node = node.0, "const-eval depth limit hit → decline fold (explosive self-application)");
        return None;
    }
    db.descent_depth += 1;
    let r = const_eval_inner(db, node, env, budget);
    db.descent_depth -= 1;
    r
}

pub(super) fn const_eval_inner(
    db: &mut Db,
    node: StructId,
    env: &CEnv,
    budget: &mut u64,
) -> Option<CVal> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    // Evaluate a sub-expression and SHORT-CIRCUIT a taken `trap`: a `CVal::Trap` propagates like an exception
    // up through every consuming operation (a trap in any operand/element/field/scrutinee traps the whole
    // fold), so the trap's message reaches the top. Sites that DIRECTLY return their sub-eval (a `Ref`, a
    // `let` body, a match arm body) propagate the trap naturally and need no `ce!`.
    macro_rules! ce {
        ($n:expr) => {{
            let v = const_eval(db, $n, env, budget)?;
            if let CVal::Trap(_) = v {
                return Some(v);
            }
            v
        }};
    }
    // `Ast.module` (whatever its resolved shape — a `(. Ast module)` member or the reflect op-record) folds
    // via `core_of` to the reflected module `Ast` (a `Core::SumNew` tree); convert that constant to a `CVal`
    // so the reflected forms flow as matchable values into a transform (the `Ast.module`-SOURCE depth gap).
    if crate::eval::meta_apply_of(db, node) == Some(crate::resolved::Prim::ReflectModule) {
        let c = core_of(db, node);
        return if core_is_const_value(db, &c) {
            core_to_cval(db, &c)
        } else {
            None
        };
    }
    // A BARE nullary `Map.empty` VALUE (`(. Map empty)`, `meta_apply_of == MapEmpty`): `core_of` folds it to
    // a constant empty `Core::MapNew`; bridge that to a `CVal::Map` so a `Map.insert`/`Map.lookup`/`Map.len`
    // chain starting from a bare empty map folds (the `apply_const_prim` arms fold the ops; this supplies
    // their empty-map base). Handled HERE, before the `match` — the `Member` arm below would otherwise try to
    // PROJECT `empty` off the `Map` MODULE operand (which does not const-evaluate to a record) and decline,
    // so a bare `Map.empty` under `(const …)`/a demand path never folded. The PARENTHESIZED application form
    // `(Map.empty)` already folds via `const_eval_apply`'s `Prim::MapEmpty` arm; this closes the bare-member
    // gap the same way `ReflectModule`/`variant_disc_of` close theirs. Kept tight to `MapEmpty` (a nullary
    // prim `core_of` folds to a constant) so it never shadows a genuine record projection.
    if crate::eval::meta_apply_of(db, node) == Some(crate::resolved::Prim::MapEmpty) {
        let c = core_of(db, node);
        return if core_is_const_value(db, &c) {
            core_to_cval(db, &c)
        } else {
            None
        };
    }
    // A NULLARY VARIANT used as a bare VALUE (`Option.None`, a `(. Sum V)` member form, a bare-name variant):
    // it carries a `(meta variant)` discriminant and DENOTES the empty-payload sum value, WHATEVER its
    // resolved shape. Handle it BEFORE the `match` — the `Member`/`Ref` arms would otherwise try to PROJECT
    // `None` off the `Option` operand (a record projection that declines) or follow the reference through to
    // the bare `(intrinsic sum-new)` constructor head (a non-value this interpreter declines). The taken
    // `_ => Option.None` arm of an Option-threaded AST-navigation helper is exactly this shape, so this is
    // what lets the operator's clean (no-sentinel) self-reflection transform const-fold. An APPLIED variant
    // (`Option.Some x`, `Ast.Name n`) is a `Resolved::Apply` whose node carries no variant meta, so it is
    // unaffected — its payloads fold through `const_eval_apply`'s variant arm.
    if let Some(disc) = crate::eval::variant_disc_of(db, node) {
        return Some(CVal::Sum {
            disc,
            payloads: Vec::new(),
        });
    }
    match resolved_of(db, node) {
        Resolved::Int(v) => Some(CVal::Int(v)),
        Resolved::Bool(b) => Some(CVal::Bool(b)),
        Resolved::Char(c) => Some(CVal::Char(c)),
        Resolved::Float(d) => Some(CVal::Float(d)),
        Resolved::Str(s) | Resolved::SymbolConst(s) => Some(CVal::Str(s.into())),
        Resolved::Bytes(b) => Some(CVal::Bytes(b.into())),
        Resolved::Unit => Some(CVal::Unit),
        // A call parameter reference — its value is bound in the env by the enclosing application.
        Resolved::Param { binder } => env.get(&binder).cloned(),
        // A plain reference (a `let`-bound name, a nullary-def reference) follows through to its
        // initializer / body, re-evaluated in the current env (const, so re-evaluation is sound).
        Resolved::Ref { value } => const_eval(db, value, env, budget),
        // A `let`'s value is its body's value; a body reference to a bound name follows via `Ref` above.
        Resolved::Let { body, .. } => const_eval(db, body, env, budget),
        Resolved::If { cond, then_, else_ } => match ce!(cond) {
            CVal::Bool(true) => const_eval(db, then_, env, budget),
            CVal::Bool(false) => const_eval(db, else_, env, budget),
            _ => None,
        },
        Resolved::And { lhs, rhs, is_and } => match (ce!(lhs), is_and) {
            (CVal::Bool(false), true) => Some(CVal::Bool(false)),
            (CVal::Bool(true), false) => Some(CVal::Bool(true)),
            (CVal::Bool(_), _) => match ce!(rhs) {
                CVal::Bool(b) => Some(CVal::Bool(b)),
                _ => None,
            },
            _ => None,
        },
        Resolved::Not { operand } => match ce!(operand) {
            CVal::Bool(b) => Some(CVal::Bool(!b)),
            _ => None,
        },
        Resolved::List { elems } => {
            let mut vs = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                vs.push(ce!(e));
            }
            Some(CVal::List(vs))
        }
        Resolved::Match { scrutinee, arms } => {
            let v = ce!(scrutinee);
            for (pat, body) in arms.iter() {
                // `None` = the match is undecidable at this stage (a pattern shape not yet interpreted):
                // decline the whole evaluation rather than guess an arm (guessing would miscompile).
                match const_pattern_matches(db, *pat, &v)? {
                    true => return const_eval(db, *body, env, budget),
                    false => continue,
                }
            }
            None
        }
        // A RECORD literal — evaluate each field's value; canonically ordered by the `Symbol` key.
        Resolved::Record { fields } => {
            let mut m = std::collections::BTreeMap::new();
            for (sym, &val) in fields.iter() {
                m.insert(sym.clone(), ce!(val));
            }
            Some(CVal::Record(m))
        }
        // Member access `(. operand key)` — project a field off a constant record.
        Resolved::Member { operand, key } => match ce!(operand) {
            CVal::Record(m) => m.get(&key).cloned(),
            _ => None,
        },
        // A TUPLE literal — evaluate each positional element.
        Resolved::Tuple { elems } => {
            let mut vs = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                vs.push(ce!(e));
            }
            Some(CVal::Tuple(vs))
        }
        // A tuple projection `(. operand index)` — read element `index` off a constant tuple.
        Resolved::Proj { operand, index } => match ce!(operand) {
            CVal::Tuple(vs) => vs.get(index).cloned(),
            _ => None,
        },
        // A match pattern binder — a projection of the scrutinee value along `steps`.
        Resolved::SumPayload {
            scrutinee, steps, ..
        } => {
            let mut v = ce!(scrutinee);
            for step in steps.iter() {
                v = match (step, &v) {
                    (crate::core::PathStep::Elem(i), CVal::List(xs)) => xs.get(*i)?.clone(),
                    // An `Elem(i)` on a TUPLE reads slot `i` — a tuple pattern binder `(tuple a b)` reads each
                    // binder out through an `Elem` step (and a MULTI-payload variant's `(Ctor a b)` reaches
                    // its payload tuple via `[Payload, Elem(i)]`). Without this a tuple destructure declined.
                    (crate::core::PathStep::Elem(i), CVal::Tuple(vs)) => vs.get(*i)?.clone(),
                    (crate::core::PathStep::RestFrom(k), CVal::List(xs)) => {
                        CVal::List(xs.get(*k..).map(<[CVal]>::to_vec).unwrap_or_default())
                    }
                    // A tuple-pattern REST binder over a CONSTANT tuple — the trailing sub-tuple (a NEW
                    // tuple of the elements from `k` onward). A tuple's arity is fixed, so this is a
                    // constant gather (the twin of the list `RestFrom` slice above).
                    (crate::core::PathStep::TupleRestFrom(k), CVal::Tuple(vs)) => {
                        CVal::Tuple(vs.get(*k..).map(<[CVal]>::to_vec).unwrap_or_default())
                    }
                    // Unwrap a sum variant's single payload — a variant pattern binder `(Ctor x)` reads its
                    // payload out through a `[Payload]` step. (A multi-payload variant's `[Payload, Elem(i)]`
                    // reaches the `Elem` on the payload TUPLE — a later stage; here the payload is one value.)
                    (crate::core::PathStep::Payload, CVal::Sum { payloads, .. }) => {
                        payloads.first()?.clone()
                    }
                    // Elem-on-tuple/record and multi-payload tuples are later stages.
                    _ => return None,
                };
            }
            Some(v)
        }
        // A type ascription `(: value T)` — its value is the inner expression's value (the type is a
        // compile-time-only annotation, erased at runtime). Notably the `([] : List T)` empty-list base arm.
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            const_eval(db, expr, env, budget)
        }
        Resolved::Apply { head, args } => const_eval_apply(db, node, head, &args, env, budget),
        // A `fn`/lambda LITERAL as a value — capture it as a `Closure` over the current env so it can be
        // passed to a `const` function parameter and APPLIED per element in a higher-order fold (map/filter/
        // fold). `node` (this lambda's occurrence) resolves its params + body via `lambda_of` when applied.
        Resolved::Lambda { .. } => Some(CVal::Closure {
            node,
            env: std::rc::Rc::new(env.clone()),
        }),
        // A `handle` — DELEGATE to the effect reducer, then const-eval its RESULT. `reduce_handle` threads the
        // continuations/resumes/state and returns a PURE AST (`(let ((s <init>)) <body-with-perform-answers>)`
        // — it keeps a growing COLLECTION state as a re-read `let` binding, which `core_of` alone leaves as a
        // `Core::Let` this stage cannot fold). Const-evaluating that reduced AST folds the query answers via
        // the scalar/List/Map/Set arms — so a `(const (handle …))` over a closed finite handle with const init
        // folds its answer (v-effects cm02/cms3/cms4). NOT a reducer reimplementation — it reuses the one
        // `reduce_handle`; `None` (an open/irreducible handle) declines cleanly. Budget-threaded.
        Resolved::Handle { init, arms, body } => {
            let reduced = crate::effects::reduce_handle(db, init, &arms, body, false)?;
            const_eval(db, reduced, env, budget)
        }
        // (A NULLARY variant used as a bare value — `Option.None`, a payload-less user variant — is folded
        // to its empty-payload sum by the `variant_disc_of` guard ABOVE the match, before the Member/Ref
        // arms can shadow it.)
        // A node this stage does not interpret STRUCTURALLY, but which `core_of` already folds to a constant
        // — notably `Ast.module` (the reflected module `Ast`, built as a `Core::SumNew` tree) and a `quote`.
        // Convert that constant `Core` into a `CVal` so it flows as a value into the surrounding recursion/
        // composition (the `Ast.module`-SOURCE depth gap: the reflected forms fold, but must propagate as
        // matchable/projectable values through a nested-recursion filter). `core_of` declines (yields a
        // non-constant) for a runtime value, so this is still const-only.
        _ => {
            let c = core_of(db, node);
            if core_is_const_value(db, &c) {
                core_to_cval(db, &c)
            } else {
                None
            }
        }
    }
}

/// Convert a CONSTANT `Core` value into a `CVal` — the bridge for a value `core_of` already folded (a
/// reflected `Ast.module`, a quote) so it can participate in the evaluator's matching / projection /
/// re-materialization. Recurses on the compound's element NODES (a `Core` compound references its elements as
/// AST occurrences). `None` for a non-constant or a shape this stage does not carry.
pub(super) fn core_to_cval(db: &mut Db, c: &Core) -> Option<CVal> {
    Some(match c {
        Core::ConstInt(v) => CVal::Int(v.clone()),
        Core::ConstBool(b) => CVal::Bool(*b),
        Core::ConstChar(c) => CVal::Char(*c),
        Core::ConstFloat(d) => CVal::Float(d.clone()),
        Core::ConstStr(s) => CVal::Str(s.clone()),
        Core::ConstBytes(b) => CVal::Bytes(b.clone()),
        Core::Unit => CVal::Unit,
        Core::BytesOf { elems } => {
            // A `BytesOf` of constant byte elements is a constant bytes value.
            let elems = elems.clone();
            let mut bytes = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                let ec = core_of(db, e);
                match core_to_cval(db, &ec)? {
                    CVal::Int(v) => bytes.push(v.to_i64()? as u8),
                    _ => return None,
                }
            }
            CVal::Bytes(bytes.into())
        }
        Core::ListNew { elems } => {
            let elems = elems.clone();
            let mut vs = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                let ec = core_of(db, e);
                vs.push(core_to_cval(db, &ec)?);
            }
            CVal::List(vs)
        }
        Core::SumNew { disc, payloads } => {
            let (disc, payloads) = (*disc, payloads.clone());
            let mut ps = Vec::with_capacity(payloads.len());
            for &p in payloads.iter() {
                let pc = core_of(db, p);
                ps.push(core_to_cval(db, &pc)?);
            }
            CVal::Sum { disc, payloads: ps }
        }
        Core::Tuple { elems } => {
            let elems = elems.clone();
            let mut vs = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                let ec = core_of(db, e);
                vs.push(core_to_cval(db, &ec)?);
            }
            CVal::Tuple(vs)
        }
        Core::Record { fields } => {
            let fields: Vec<(crate::resolved::Symbol, StructId)> =
                fields.iter().map(|(k, &v)| (k.clone(), v)).collect();
            let mut m = std::collections::BTreeMap::new();
            for (k, v) in fields {
                let vc = core_of(db, v);
                m.insert(k, core_to_cval(db, &vc)?);
            }
            CVal::Record(m)
        }
        // A constant `Core::MapNew` (a `Map.empty`, a map literal, or a folded map) flows in as a `CVal::Map`
        // assoc list, so a const map-query composes. Keys/values are the entry NODES (converted via
        // `core_of`). Only ever queried, never re-materialized (`cval_to_core` declines a `CVal::Map`).
        Core::MapNew { entries, .. } => {
            let entries = entries.clone();
            let mut m = Vec::with_capacity(entries.len());
            for (k, v) in entries.iter() {
                let kc = core_of(db, *k);
                let vc = core_of(db, *v);
                m.push((core_to_cval(db, &kc)?, core_to_cval(db, &vc)?));
            }
            CVal::Map(m)
        }
        // A constant `Core::SetOf` (a `Set.empty`, a set literal, or a folded set) flows in as a `CVal::Set`
        // (distinct members). Elements are the entry NODES; only queried, never re-materialized.
        Core::SetOf { elems, .. } => {
            let elems = elems.clone();
            let mut s = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                let ec = core_of(db, e);
                s.push(core_to_cval(db, &ec)?);
            }
            CVal::Set(s)
        }
        _ => return None,
    })
}

/// Resolve a call HEAD to a CLOSURE value bound in `env` — a `const` function parameter carrying a lambda
/// argument. Follows `Ref`/`Annot` wrappers to the `Param` binder via a BOUNDED walk (a resolution chain is
/// short; the 64-step cap makes it total regardless), then reads the env. It does NOT re-evaluate the head
/// (unlike `const_eval`, whose recursion the fold budget bounds only by STEPS, not native stack depth), so
/// it is safe to call on every application without risking a stack overflow. Returns the closure's lambda
/// occurrence + its captured env.
pub(super) fn env_closure(
    db: &mut Db,
    head: StructId,
    env: &CEnv,
) -> Option<(StructId, std::rc::Rc<CEnv>)> {
    let mut cur = head;
    for _ in 0..64 {
        match resolved_of(db, cur) {
            Resolved::Param { binder } => {
                return match env.get(&binder) {
                    Some(CVal::Closure { node, env: e }) => Some((*node, e.clone())),
                    _ => None,
                };
            }
            Resolved::Ref { value } => cur = value,
            Resolved::Annot { expr, .. } => cur = expr,
            _ => return None,
        }
    }
    None
}

/// Evaluate a function application to a `CVal`. A primitive (or a prelude wrapper that routes to an
/// intrinsic in its body) applies its const semantics; a def/lambda binds its parameters to the evaluated
/// arguments in a fresh env and evaluates its body (recursion permitted — the `budget` bounds it).
pub(super) fn const_eval_apply(
    db: &mut Db,
    node: StructId,
    head: StructId,
    args: &[StructId],
    env: &CEnv,
    budget: &mut u64,
) -> Option<CVal> {
    // Short-circuit a taken `trap` in an evaluated sub-expression (see `const_eval`'s `ce!`).
    macro_rules! ce {
        ($n:expr) => {{
            let v = const_eval(db, $n, env, budget)?;
            if let CVal::Trap(_) = v {
                return Some(v);
            }
            v
        }};
    }
    // A TAKEN `trap("msg")` on the const-folded path — the fold executed a trap (e.g. a self-reflection
    // transform reached a genuinely-missing required pragma). Evaluate its message and yield a `CVal::Trap`,
    // which propagates up (via `ce!`) and materializes to a `Core::Poison(ConstTrap, msg)` — so the trap's
    // MESSAGE is the compile error, not the generic "runtime AST value" decline.
    if crate::eval::meta_apply_of(db, head).or_else(|| crate::eval::prim_of(db, head))
        == Some(Prim::Trap)
    {
        let msg = match args.first().map(|&a| const_eval(db, a, env, budget)) {
            Some(Some(CVal::Str(s))) => s,
            _ => std::sync::Arc::from("trap (const-executed)"),
        };
        return Some(CVal::Trap(msg));
    }
    // A HIGHER-ORDER application `f(x…)` where the head `f` is a `const` PARAMETER bound to a CLOSURE (a
    // lambda passed as an argument — the mapper/predicate/folder of a `List.map`/`filter`/`fold` or a user
    // recursive map that threads a closure): look the closure up in the env and APPLY it — bind its params
    // to the evaluated args in its CAPTURED (lexical) env, then evaluate its body. `f` has no static lambda
    // for `lambda_of` to reduce (it is a fold-time value living in the env), so without this a higher-order
    // fold declines. Arg count must match; a taken `trap` in an arg short-circuits via `ce!`.
    // A VARIANT CONSTRUCTOR applied to its payload(s) — an `Ast.Name`/`Ast.List`/`Option.Some`/user-sum
    // constructor. Its value is the sum with the constructor's discriminant and the evaluated payloads.
    if let Some(disc) = crate::eval::variant_disc_of(db, head) {
        let mut payloads = Vec::with_capacity(args.len());
        for &a in args {
            payloads.push(ce!(a));
        }
        return Some(CVal::Sum { disc, payloads });
    }
    if let Some(prim) =
        crate::eval::prim_of(db, head).or_else(|| crate::eval::meta_apply_of(db, head))
    {
        // A compound-VALUE constructor reached in its APPLIED form — a `(record …)`/`(tuple …)`/`(list …)`
        // literal that resolves to an applied `RecordNew`/`TupleNew`/`ListNew` rather than the symbol-headed
        // `Resolved::Record`/`Tuple`/`List` (a record literal always does). `reduce_ctor` rewrites it to that
        // symbol-headed compound, whose resolved form the dedicated arms above (`Resolved::Record` etc.) build
        // into a `CVal` — evaluating each field/element in the CURRENT env, so a compound built from const-
        // param values folds too. Without this a record literal declined (`apply_const_prim` carries no
        // `RecordNew` arm), so a record threaded through a fn — `(const (get-x (record …)))` — could not fold.
        if matches!(prim, Prim::RecordNew | Prim::TupleNew | Prim::ListNew)
            && let Ok(built) = crate::eval::reduce_ctor(db, prim, node, args)
        {
            return const_eval(db, built, env, budget);
        }
        let mut vs = Vec::with_capacity(args.len());
        for &a in args {
            vs.push(ce!(a));
        }
        // `List.at (list, index)` → an `Option`: `Option.Some elem` in range, else `Option.None`. The
        // Some/None discriminants are read off THIS node's result type (`option_discs`). `List.at` is the
        // destructor `child`/`at-or-name` build on. Handled HERE (not `apply_const_prim`) because it needs
        // the node + `db`.
        if prim == Prim::ListAt
            && let [CVal::List(xs), CVal::Int(i)] = &vs[..]
        {
            let (some_disc, none_disc) = option_discs(db, node)?;
            let idx = i.to_i64()?;
            return Some(if idx >= 0 && (idx as usize) < xs.len() {
                CVal::Sum {
                    disc: some_disc,
                    payloads: vec![xs[idx as usize].clone()],
                }
            } else {
                CVal::Sum {
                    disc: none_disc,
                    payloads: Vec::new(),
                }
            });
        }
        // `Char.from-int i` — the FALLIBLE `Int64 → (Option Char)`: `(Some #\c)` for a Unicode scalar value,
        // `(None)` for a surrogate / out-of-range integer (the recursive-engine twin of the `core_of`
        // `lower_char_from_int` fold; discs off THIS node's result type, like `List.at`). Never traps.
        if prim == Prim::CharFromInt
            && let [CVal::Int(i)] = &vs[..]
        {
            let (some_disc, none_disc) = option_discs(db, node)?;
            let scalar = i
                .to_i64()
                .and_then(|n| u32::try_from(n).ok())
                .and_then(char::from_u32);
            return Some(match scalar {
                Some(c) => CVal::Sum {
                    disc: some_disc,
                    payloads: vec![CVal::Char(c)],
                },
                None => CVal::Sum {
                    disc: none_disc,
                    payloads: Vec::new(),
                },
            });
        }
        // MAP ops (queried, never materialized — `CVal::Map`). `Map.empty` → an empty map; `Map.insert
        // (m, k, v)` → `m` with `k ↦ v` (replace-in-place if present, else append — latest write wins);
        // `Map.lookup (m, k)` → `Option v` (the Some/None discs off THIS node's result type, like
        // `List.at`); `Map.size m` → the entry count. A key comparison the stage cannot DECIDE (`cval_eq`
        // = None) DECLINES the whole op rather than risk a wrong verdict.
        if prim == Prim::MapEmpty {
            return Some(CVal::Map(Vec::new()));
        }
        if prim == Prim::MapInsert
            && let [CVal::Map(m), k, v] = &vs[..]
        {
            let mut out = m.clone();
            let mut replaced = false;
            for (ek, ev) in out.iter_mut() {
                match cval_eq(ek, k) {
                    Some(true) => {
                        *ev = v.clone();
                        replaced = true;
                        break;
                    }
                    Some(false) => {}
                    None => return None,
                }
            }
            if !replaced {
                out.push((k.clone(), v.clone()));
            }
            return Some(CVal::Map(out));
        }
        if prim == Prim::MapLookup
            && let [CVal::Map(m), k] = &vs[..]
        {
            let (some_disc, none_disc) = option_discs(db, node)?;
            let mut found: Option<CVal> = None;
            for (ek, ev) in m.iter() {
                match cval_eq(ek, k) {
                    Some(true) => {
                        found = Some(ev.clone());
                        break;
                    }
                    Some(false) => {}
                    None => return None,
                }
            }
            return Some(match found {
                Some(v) => CVal::Sum {
                    disc: some_disc,
                    payloads: vec![v],
                },
                None => CVal::Sum {
                    disc: none_disc,
                    payloads: Vec::new(),
                },
            });
        }
        if prim == Prim::MapSize
            && let [CVal::Map(m)] = &vs[..]
        {
            return Some(CVal::Int(crate::ast::IntValue::from_i64(m.len() as i64)));
        }
        // `Map.to-list m` → the entries as a list of `(key value)` tuples in canonical KEY order — the
        // const_eval twin of `lower_map_to_list`'s fold (the Map analogue of the `SetToList` arm above), so a
        // map built through the RECURSIVE engine or consumed by a const-param helper materializes too. Sort by
        // KEY via `cval_key_order` (the SAME order `const_key_order`/the runtime `map-to-list` op use); a key
        // the canonical order cannot rank (Char/Bytes/Float/nested — declines, matching the op) declines the
        // whole materialization. Each entry → `CVal::Tuple([key, value])`, the op's `(List (Tuple K V))` shape.
        if prim == Prim::MapToList
            && let [CVal::Map(m)] = &vs[..]
        {
            let mut sorted = m.clone();
            // Every KEY must be individually orderable (a 0/1-entry sort never calls the comparator).
            let mut orderable = sorted.iter().all(|(k, _)| cval_key_order(k, k).is_some());
            sorted.sort_by(|a, b| {
                cval_key_order(&a.0, &b.0).unwrap_or_else(|| {
                    orderable = false;
                    std::cmp::Ordering::Equal
                })
            });
            if !orderable {
                return None;
            }
            return Some(CVal::List(
                sorted
                    .into_iter()
                    .map(|(k, v)| CVal::Tuple(vec![k, v]))
                    .collect(),
            ));
        }
        // SET ops (queried, never materialized — `CVal::Set`, same soundness as `CVal::Map`). `Set.of` builds
        // a set from a LIST (deduped); `Set.insert (s, e)` adds `e` if absent; `Set.contains (s, e)` → Bool;
        // `Set.size s` → the count. A membership comparison the stage cannot decide (`cval_eq` = None)
        // declines the op rather than risk a wrong verdict. (`Set.empty` folds via `core_to_cval(SetOf)`.)
        if prim == Prim::SetOf
            && let [CVal::List(xs)] = &vs[..]
        {
            let mut out: Vec<CVal> = Vec::with_capacity(xs.len());
            for e in xs.iter() {
                let mut seen = false;
                for x in out.iter() {
                    match cval_eq(x, e) {
                        Some(true) => {
                            seen = true;
                            break;
                        }
                        Some(false) => {}
                        None => return None,
                    }
                }
                if !seen {
                    out.push(e.clone());
                }
            }
            return Some(CVal::Set(out));
        }
        if prim == Prim::SetInsert
            && let [CVal::Set(s), e] = &vs[..]
        {
            let mut out = s.clone();
            let mut present = false;
            for x in out.iter() {
                match cval_eq(x, e) {
                    Some(true) => {
                        present = true;
                        break;
                    }
                    Some(false) => {}
                    None => return None,
                }
            }
            if !present {
                out.push(e.clone());
            }
            return Some(CVal::Set(out));
        }
        if prim == Prim::SetContains
            && let [CVal::Set(s), e] = &vs[..]
        {
            let mut found = false;
            for x in s.iter() {
                match cval_eq(x, e) {
                    Some(true) => {
                        found = true;
                        break;
                    }
                    Some(false) => {}
                    None => return None,
                }
            }
            return Some(CVal::Bool(found));
        }
        if prim == Prim::SetLen
            && let [CVal::Set(s)] = &vs[..]
        {
            return Some(CVal::Int(crate::ast::IntValue::from_i64(s.len() as i64)));
        }
        // `Set.to-list s` → the set's elements as a list in CANONICAL VALUE ORDER — the const_eval twin of
        // `lower_set_to_list`'s fold (#3765), so a set built through the RECURSIVE engine (a `CVal::Set` this
        // stage never reduced to a `Core::SetOf`) or consumed by a const-param helper materializes too, not
        // just a syntactic `Set.of`. Byte-matches the runtime `set-to-list` op: the enumeration order is the
        // spec-pinned canonical value total order (`collections-and-text.md` §Set Iteration Is Deterministic),
        // which `cval_key_order` computes — the SAME order `const_key_order`/`value_cmp_shaped` use (v-runtime
        // contract). An element the canonical order cannot rank (Char/Bytes/Float/nested — `cval_key_order`
        // declines, EXACTLY the classes the runtime op declines too) declines the whole materialization. The
        // set is already dedup'd by value (`Set.of`/insert folds), so this only reorders.
        if prim == Prim::SetToList
            && let [CVal::Set(s)] = &vs[..]
        {
            let mut sorted = s.clone();
            // Every element must be individually orderable (a 0/1-element sort never calls the comparator).
            let mut orderable = sorted.iter().all(|e| cval_key_order(e, e).is_some());
            sorted.sort_by(|x, y| {
                cval_key_order(x, y).unwrap_or_else(|| {
                    orderable = false;
                    std::cmp::Ordering::Equal
                })
            });
            if !orderable {
                return None;
            }
            return Some(CVal::List(sorted));
        }
        // `Set.remove (s, e)` → `s` without `e` (a no-op if absent). Order-independent, never materialized.
        if prim == Prim::SetRemove
            && let [CVal::Set(s), e] = &vs[..]
        {
            let mut out = Vec::with_capacity(s.len());
            for x in s.iter() {
                match cval_eq(x, e) {
                    Some(true) => {}
                    Some(false) => out.push(x.clone()),
                    None => return None,
                }
            }
            return Some(CVal::Set(out));
        }
        // SET ALGEBRA (queried, never materialized — same soundness as the other `CVal::Set` ops). `∪` keeps
        // the left members and appends each right member not already present; `∩` keeps left members that are
        // also in the right; `∖` keeps left members not in the right. Membership is `cval_set_member`, which
        // declines (`None`) on an undecidable comparison, so the whole op declines rather than risk a wrong
        // member set. (Only order-INDEPENDENT results — a size, a membership — ever leave the fold; the set
        // itself never re-materializes, `cval_to_core` declines a `CVal::Set`.)
        if prim == Prim::SetUnion
            && let [CVal::Set(a), CVal::Set(b)] = &vs[..]
        {
            let mut out = a.clone();
            for e in b.iter() {
                if !cval_set_member(a, e)? {
                    out.push(e.clone());
                }
            }
            return Some(CVal::Set(out));
        }
        if prim == Prim::SetIntersection
            && let [CVal::Set(a), CVal::Set(b)] = &vs[..]
        {
            let mut out = Vec::new();
            for e in a.iter() {
                if cval_set_member(b, e)? {
                    out.push(e.clone());
                }
            }
            return Some(CVal::Set(out));
        }
        if prim == Prim::SetDifference
            && let [CVal::Set(a), CVal::Set(b)] = &vs[..]
        {
            let mut out = Vec::new();
            for e in a.iter() {
                if !cval_set_member(b, e)? {
                    out.push(e.clone());
                }
            }
            return Some(CVal::Set(out));
        }
        // `Map.remove (m, k)` → `m` without the entry keyed `k` (a no-op if absent). Key comparison via
        // `cval_eq`; an undecidable one declines. Order-independent, never materialized (query-only).
        if prim == Prim::MapRemove
            && let [CVal::Map(m), k] = &vs[..]
        {
            let mut out = Vec::with_capacity(m.len());
            for (ek, ev) in m.iter() {
                match cval_eq(ek, k) {
                    Some(true) => {}
                    Some(false) => out.push((ek.clone(), ev.clone())),
                    None => return None,
                }
            }
            return Some(CVal::Map(out));
        }
        // FLOAT arithmetic — `+`/`-`/`*`/`/` (the `Prim::Add`… identity; `core_of` remaps to `FAdd`… only at
        // emit, on a `float_operand` test) over two constant floats. Fold at the node's solved width EXACTLY
        // like `lower_float_arith`: round each operand + the result through the width (`Float32` via binary32),
        // then `Decimal::from_f64` — a NON-FINITE result (overflow → ±inf, `/0.0` → NaN) has no value form, so
        // it declines (`None`). Handled here (not `apply_const_prim`) because the width needs `node`'s type.
        if matches!(prim, Prim::Add | Prim::Sub | Prim::Mul | Prim::Div)
            && let [CVal::Float(a), CVal::Float(b)] = &vs[..]
        {
            let width = match crate::infer::type_of(db, node) {
                crate::ty::Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            };
            let at = |f: f64| if width == 32 { f as f32 as f64 } else { f };
            let (x, y) = (
                at(f64::from_bits(a.to_f64_bits())),
                at(f64::from_bits(b.to_f64_bits())),
            );
            let r = at(match prim {
                Prim::Add => x + y,
                Prim::Sub => x - y,
                Prim::Mul => x * y,
                _ => x / y,
            });
            return crate::ast::Decimal::from_f64(r).map(CVal::Float);
        }
        // FLOAT comparison — `=` by the CANONICAL BYTE FORM at the operand width (`-0.0 ≠ 0.0`), `< <= > >=`
        // by the IEEE PARTIAL order (an unordered/NaN operand → false), matching `lower_comparison`'s constant
        // float arms. Both operands are finite (a bare `nan` is `Core::ConstFloatNan`, which `core_to_cval`
        // declines, and float arithmetic declines a non-finite result — so no NaN reaches a `CVal::Float`).
        if matches!(prim, Prim::Eq | Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge)
            && let [CVal::Float(a), CVal::Float(b)] = &vs[..]
        {
            let (fa, fb) = (a.to_f64_bits(), b.to_f64_bits());
            let ba = const_float_bits_at_operand_width(db, args[0], fa);
            let bb = const_float_bits_at_operand_width(db, args[1], fb);
            return Some(CVal::Bool(if matches!(prim, Prim::Eq) {
                ba == bb
            } else {
                match f64::from_bits(ba).partial_cmp(&f64::from_bits(bb)) {
                    Some(ord) => compare_ord(prim, ord),
                    None => false,
                }
            }));
        }
        // Three-way `Ordering.of` (`Prim::Compare`) — the const_eval twin of `core_of`'s `lower_compare`.
        // `apply_const_prim` carries no Compare arm and the delegation below cannot rebuild this member-headed
        // op, so a three-way compare threaded through a RECURSION (a const comparator / sort / min-max) declined.
        // Build the nullary Ordering variant at the RESULT type's discs — `ordering_discs(node)` reads
        // Less/Equal/Greater by NAME (never a hardcoded 0/1/2). A BARE leaf orders via `cval_key_order`
        // directly (Int/Str/Bool/Char/Bytes; a Float leaf has no total order → `None` → declines, as it must).
        // A COMPOUND operand orders through the SAME canonical order ONLY when it is orderable at the runtime
        // vocabulary (`is_orderable_compound` — a Char/Float-in-compound leaf declines, matching `lower_compare`;
        // `cval_key_order` alone is more permissive since it blesses Char for the Set/Map to-list enumeration).
        if prim == Prim::Compare
            && let [a, b] = &vs[..]
        {
            let is_compound = matches!(
                a,
                CVal::Tuple(_) | CVal::Record(_) | CVal::List(_) | CVal::Sum { .. }
            );
            let ord = if is_compound {
                let ty = crate::infer::type_of(db, args[0]);
                if is_orderable_compound(db, &ty) {
                    cval_key_order(a, b)
                } else {
                    None
                }
            } else {
                cval_key_order(a, b)
            };
            if let Some(ord) = ord
                && let Some((lt, eq, gt)) = ordering_discs(db, node)
            {
                let disc = match ord {
                    std::cmp::Ordering::Less => lt,
                    std::cmp::Ordering::Equal => eq,
                    std::cmp::Ordering::Greater => gt,
                };
                trace!(target: "rcdzc::fold", node = node.0, ?ord, disc, "const_eval folds a three-way Ordering.of to its Ordering variant");
                return Some(CVal::Sum {
                    disc,
                    payloads: Vec::new(),
                });
            }
        }
        // OVERFLOW POLICY in const_eval (STAGE 2b, #5313/#5337): a `+`/`-`/`*` whose NODE resolves to Wrap
        // mode wraps its CVal result to the operand's solved width (two's-complement, mod 2^width) instead of
        // computing the exact bignum (which would later surface an overflow as CDZ0302/CDZ0304). This is the
        // CVal-interpreter twin of the `lower_arith` Wrap fast-path — same overflow_mode_of decision + same
        // `wrap_to`, so a const-FOLDED recursion (`(const (f 2))`) under `(pragma overflow … wrap)` wraps
        // per-op exactly like the runtime wrapping op + the direct fold (no drift). Handled HERE (not in
        // `apply_const_prim`, which carries no `db`/node) because the mode+width need `node`'s type — the same
        // reason the width-dependent ops above are handled at this caller. Trap mode falls through to the
        // exact `apply_const_prim` fold (an overflow then surfaces as today). Only `+`/`-`/`*` carry a policy.
        if matches!(prim, Prim::Add | Prim::Sub | Prim::Mul)
            && crate::infer::overflow_mode_of(db, node) == crate::db::OverflowMode::Wrap
            && let [CVal::Int(a), CVal::Int(b)] = &vs[..]
            && let crate::ty::Ty::Int(it) = peel_qty_inner_ty(crate::infer::type_of(db, node))
        {
            let exact = match prim {
                Prim::Add => a.add(b),
                Prim::Sub => a.sub(b),
                _ => a.mul(b), // Mul (the `matches!` guard admits only Add/Sub/Mul)
            };
            return Some(CVal::Int(
                exact.wrap_to(it.ground_signed(), it.ground_width()),
            ));
        }
        if let Some(r) = apply_const_prim(prim, &vs) {
            return Some(r);
        }
        // A prim the evaluator does not fold natively (Ast.encode, Blake3.of, Bytes.concat, …) — these ARE
        // compile-time folds `core_of` implements. Materialize the (constant) argument values to nodes,
        // rebuild the application over a fresh copy of the prim head, and fold it via `core_of`, then convert
        // the constant back to a `CVal`. So a descriptor field `Blake3.of(Ast.encode(…))` const-evaluates
        // even though `const_eval` has no hand-written rule for those prims.
        let head_copy = copy_ast_subtree(db, head);
        let mut items = Vec::with_capacity(vs.len() + 1);
        items.push(head_copy);
        for v in &vs {
            items.push(cval_to_ast(db, v)?);
        }
        let app = db.push_list(items);
        crate::resolve::resolve_subtree(db, app);
        let c = core_of(db, app);
        let isc = core_is_const_value(db, &c);
        return if isc { core_to_cval(db, &c) } else { None };
    }
    // A HIGHER-ORDER application `f(x…)` where the head `f` is a `const` FUNCTION PARAMETER bound to a
    // closure (the mapper/predicate/folder of a user recursive map that threads a closure). Resolve the head
    // through `Ref`/`Annot` wrappers to its env-bound `Closure` via a BOUNDED walk (`env_closure` — it does
    // NOT re-evaluate the head, so it cannot recurse unboundedly on the fold hot path), then apply: bind the
    // closure's params to the evaluated args in its CAPTURED (lexical) env and evaluate its body. Guarded on
    // `lambda_params_of(head).is_none()` so a STATIC lambda/def head still takes the direct path below.
    if crate::eval::lambda_params_of(db, head).is_none()
        && let Some((lam, cenv)) = env_closure(db, head, env)
    {
        let cparams = crate::eval::lambda_params_of(db, lam)?;
        if cparams.len() != args.len() {
            return None;
        }
        let body = crate::eval::lambda_body(db, lam)?;
        let mut child = (*cenv).clone();
        for (&p, &a) in cparams.iter().zip(args.iter()) {
            let av = if crate::eval::lambda_params_of(db, a).is_some() {
                CVal::Closure {
                    node: a,
                    env: std::rc::Rc::new(env.clone()),
                }
            } else {
                ce!(a)
            };
            child.insert(p, av);
        }
        return const_eval(db, body, &child, budget);
    }
    // A NULLARY def call `(mk)` — a nullary def resolves its NAME to a `Ref` straight at its body (no
    // `Lambda` wrapper), so `lambda_params_of` is `None` and the param-binding path below bails. Evaluate
    // its body directly in a fresh env (no params to bind). This lets a PROJECTION/consumer of a nullary
    // const fn that returns a compound fold — `descriptor().id` where `descriptor()` returns a record: the
    // field read reduces to that field's constant and the record (with its non-representable `Ast` siblings)
    // never materializes. GUARDED on `!is_recursive`: the step `budget` bounds TOTAL work but NOT native
    // stack depth, so an unproductive nullary self-recursion `(def (f) (f))` would recurse ~1M native frames
    // and overflow the stack before the budget stops it (the same hazard the closure fold avoids by not
    // re-evaluating the head). A recursive nullary is instead handled by `core_of`'s dedicated arm (which
    // reads `is_recursive` and tries the general evaluator with its own reduction guard), so declining here
    // is a hand-off, not a loss.
    if args.is_empty()
        && crate::eval::lambda_params_of(db, head).is_none()
        && let Some(body) = crate::eval::lambda_body_of_nullary(db, head)
        && !crate::eval::is_recursive(db, body)
    {
        return const_eval(db, body, &CEnv::default(), budget);
    }
    let params = crate::eval::lambda_params_of(db, head)?;
    if params.len() != args.len() {
        return None;
    }
    let body = crate::eval::lambda_body(db, head)?;
    let mut child = CEnv::default();
    for (&p, &a) in params.iter().zip(args.iter()) {
        // A FUNCTION-valued argument (a lambda literal / named-fn ref that `lambda_of` reduces) is captured
        // as a `Closure` over the CALLER's env, so a higher-order callee applies it per element (map/filter/
        // fold). Evaluating it as an ordinary value would decline (a lambda's `core_of` is not a constant),
        // aborting the whole fold. A re-passed closure PARAMETER is not a static lambda (it is a `Param`
        // bound in the env), so it falls to `ce!` and follows its env binding — already a `Closure`.
        let av = if crate::eval::lambda_params_of(db, a).is_some() {
            CVal::Closure {
                node: a,
                env: std::rc::Rc::new(env.clone()),
            }
        } else {
            ce!(a)
        };
        child.insert(p, av);
    }
    const_eval(db, body, &child, budget)
}

/// Apply a primitive to already-evaluated constant operands (Stage a: integer arithmetic + comparison,
/// equality, and the homogeneous list ops). An operand shape the prim does not define over yields `None`.
pub(super) fn apply_const_prim(prim: Prim, vs: &[CVal]) -> Option<CVal> {
    use std::cmp::Ordering;
    match (prim, vs) {
        (Prim::Add, [CVal::Int(a), CVal::Int(b)]) => Some(CVal::Int(a.add(b))),
        (Prim::Sub, [CVal::Int(a), CVal::Int(b)]) => Some(CVal::Int(a.sub(b))),
        (Prim::Mul, [CVal::Int(a), CVal::Int(b)]) => Some(CVal::Int(a.mul(b))),
        // Integer `/` and `%` — truncating quotient / remainder via the exact `divmod` (IntValue is a bignum,
        // so the division is exact; the sibling Add/Sub/Mul arms likewise fold the exact value). A ZERO divisor
        // makes `divmod` return `None`: fold to a fail-loud `CVal::Trap` (NOT `None`, which would DECLINE the
        // whole const fold and mask the fault behind a generic "cannot reduce" reject), so a `(const ...)` /
        // recursive fold that computes a divide-by-zero surfaces the same CDZ0304 "division by zero" the
        // `core_of` path and the runtime produce. The recursive-engine twin of `core_of`'s Int Div/Rem fold.
        (Prim::Div, [CVal::Int(a), CVal::Int(b)]) => Some(match a.divmod(b) {
            Some((q, _)) => CVal::Int(q),
            None => CVal::Trap(std::sync::Arc::from("division by zero")),
        }),
        (Prim::Rem, [CVal::Int(a), CVal::Int(b)]) => Some(match a.divmod(b) {
            Some((_, r)) => CVal::Int(r),
            None => CVal::Trap(std::sync::Arc::from("division by zero")),
        }),
        (Prim::Lt, [CVal::Int(a), CVal::Int(b)]) => Some(CVal::Bool(a.cmp(b) == Ordering::Less)),
        (Prim::Gt, [CVal::Int(a), CVal::Int(b)]) => Some(CVal::Bool(a.cmp(b) == Ordering::Greater)),
        (Prim::Le, [CVal::Int(a), CVal::Int(b)]) => Some(CVal::Bool(a.cmp(b) != Ordering::Greater)),
        (Prim::Ge, [CVal::Int(a), CVal::Int(b)]) => Some(CVal::Bool(a.cmp(b) != Ordering::Less)),
        (Prim::Eq, [a, b]) => cval_eq(a, b).map(CVal::Bool),
        // `Char.to-int` reads a Char's Unicode scalar as an `Int64`; ordering/`<`… compare by that scalar
        // (the documented Char semantics — `Resolved::Char`). This is the recursive-engine twin of the
        // `core_of` `CharToInt` fold; a Char threaded through a recursion needs it to fold.
        (Prim::CharToInt, [CVal::Char(c)]) => {
            Some(CVal::Int(crate::ast::IntValue::from_i64(*c as u32 as i64)))
        }
        (Prim::Lt, [CVal::Char(a), CVal::Char(b)]) => Some(CVal::Bool(a < b)),
        (Prim::Gt, [CVal::Char(a), CVal::Char(b)]) => Some(CVal::Bool(a > b)),
        (Prim::Le, [CVal::Char(a), CVal::Char(b)]) => Some(CVal::Bool(a <= b)),
        (Prim::Ge, [CVal::Char(a), CVal::Char(b)]) => Some(CVal::Bool(a >= b)),
        // STRING ordering — lexicographic by Unicode scalar sequence (`str: Ord` compares UTF-8 bytes, whose
        // order coincides with scalar-value order — the NFC form the reader normalized to). The recursive-engine
        // twin of `core_of`'s `ConstStr` comparison arm (`lower_comparison`); a String threaded through a
        // recursion (a const sort / linear-scan lookup by String key) needs it to fold instead of declining.
        (Prim::Lt, [CVal::Str(a), CVal::Str(b)]) => Some(CVal::Bool(a < b)),
        (Prim::Gt, [CVal::Str(a), CVal::Str(b)]) => Some(CVal::Bool(a > b)),
        (Prim::Le, [CVal::Str(a), CVal::Str(b)]) => Some(CVal::Bool(a <= b)),
        (Prim::Ge, [CVal::Str(a), CVal::Str(b)]) => Some(CVal::Bool(a >= b)),
        // BOOL ordering — `false < true` (Rust `bool: Ord`), matching `core_of`'s `ConstBool` comparison arm and
        // core-semantics §"the Bool type MUST offer a total order in which false is less than true".
        (Prim::Lt, [CVal::Bool(a), CVal::Bool(b)]) => Some(CVal::Bool(a < b)),
        (Prim::Gt, [CVal::Bool(a), CVal::Bool(b)]) => Some(CVal::Bool(a > b)),
        (Prim::Le, [CVal::Bool(a), CVal::Bool(b)]) => Some(CVal::Bool(a <= b)),
        (Prim::Ge, [CVal::Bool(a), CVal::Bool(b)]) => Some(CVal::Bool(a >= b)),
        (Prim::ListNew, elems) => Some(CVal::List(elems.to_vec())),
        (Prim::ListLen, [CVal::List(xs)]) => {
            Some(CVal::Int(crate::ast::IntValue::from_i64(xs.len() as i64)))
        }
        // `List.prepend (list, elem)` → `elem :: list` (the element becomes the new head).
        (Prim::ListPrepend, [CVal::List(xs), e]) => {
            let mut r = Vec::with_capacity(xs.len() + 1);
            r.push(e.clone());
            r.extend(xs.iter().cloned());
            Some(CVal::List(r))
        }
        // `List.push (list, elem)` → `list ++ [elem]` (the element becomes the new tail).
        (Prim::ListPush, [CVal::List(xs), e]) => {
            let mut r = xs.clone();
            r.push(e.clone());
            Some(CVal::List(r))
        }
        _ => None,
    }
}

/// Whether `e` is a member of the constant set / assoc-list `s` (by `cval_eq`). `None` if ANY comparison is
/// undecidable — the caller then declines the whole set/map op rather than risk a wrong verdict (the same
/// soundness guard the `Set.contains` / `Map.lookup` arms already apply inline).
pub(super) fn cval_set_member(s: &[CVal], e: &CVal) -> Option<bool> {
    for x in s.iter() {
        match cval_eq(x, e) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => return None,
        }
    }
    Some(false)
}

/// Structural equality of two constant values (Stage a's value domain). `None` for a pair this stage does
/// not compare (so the caller declines rather than guessing).
pub(super) fn cval_eq(a: &CVal, b: &CVal) -> Option<bool> {
    use std::cmp::Ordering;
    match (a, b) {
        (CVal::Int(x), CVal::Int(y)) => Some(x.cmp(y) == Ordering::Equal),
        (CVal::Bool(x), CVal::Bool(y)) => Some(x == y),
        (CVal::Char(x), CVal::Char(y)) => Some(x == y),
        (CVal::Str(x), CVal::Str(y)) => Some(x == y),
        (CVal::Bytes(x), CVal::Bytes(y)) => Some(x == y),
        (CVal::Unit, CVal::Unit) => Some(true),
        (CVal::List(x), CVal::List(y)) => cval_seq_eq(x, y),
        // A SUM value (an `Option`, a user variant, an `Ast` node) — equal iff same discriminant AND
        // element-wise-equal payloads. This is what lets `head-name(g) == Option.Some("type")`-style
        // navigation fold: equality on `Option`/variant results, not just scalars. Different disc ⇒ not
        // equal (no payload compare needed).
        (
            CVal::Sum {
                disc: da,
                payloads: pa,
            },
            CVal::Sum {
                disc: db,
                payloads: pb,
            },
        ) => {
            if da != db {
                return Some(false);
            }
            cval_seq_eq(pa, pb)
        }
        // A TUPLE — equal iff element-wise equal (a tuple's arity is fixed by its type, so a length
        // mismatch cannot occur between well-typed operands, but guard anyway).
        (CVal::Tuple(x), CVal::Tuple(y)) => cval_seq_eq(x, y),
        // A RECORD — equal iff the SAME keys map to equal values. Both maps are canonically ordered
        // (`BTreeMap`), so a key-set difference is a length or key mismatch.
        (CVal::Record(x), CVal::Record(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            for ((ka, va), (kb, vb)) in x.iter().zip(y.iter()) {
                if ka != kb || !cval_eq(va, vb)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        // Two MAPS are equal iff the SAME key set maps to equal values — ORDER-INDEPENDENT (insertion order
        // is not observable). For each entry of `x`, find a key-equal entry in `y` with an equal value; equal
        // lengths + every `x` key found ⇒ equal. A key/value comparison the stage cannot decide propagates
        // `None` (decline). (`x`/`y` have unique keys — `insert` replaces in place — so a per-entry match is
        // a bijection.)
        (CVal::Map(x), CVal::Map(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            for (kx, vx) in x.iter() {
                let mut matched = false;
                for (ky, vy) in y.iter() {
                    if cval_eq(kx, ky)? {
                        if !cval_eq(vx, vy)? {
                            return Some(false);
                        }
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return Some(false);
                }
            }
            Some(true)
        }
        // Two SETS are equal iff the SAME member set — ORDER-INDEPENDENT. Equal lengths + every `x` member
        // found in `y` ⇒ equal (members are distinct, so a per-member match is a bijection). An undecidable
        // comparison propagates `None`.
        (CVal::Set(x), CVal::Set(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            for mx in x.iter() {
                let mut matched = false;
                for my in y.iter() {
                    if cval_eq(mx, my)? {
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return Some(false);
                }
            }
            Some(true)
        }
        _ => None,
    }
}

/// Element-wise structural equality of two constant-value sequences (list elements, sum payloads, tuple
/// slots): `Some(false)` on a length mismatch, otherwise the conjunction of per-element `cval_eq`
/// (propagating a `None` from any element this stage cannot compare).
pub(super) fn cval_seq_eq(x: &[CVal], y: &[CVal]) -> Option<bool> {
    if x.len() != y.len() {
        return Some(false);
    }
    for (p, q) in x.iter().zip(y.iter()) {
        if !cval_eq(p, q)? {
            return Some(false);
        }
    }
    Some(true)
}

/// Whether the match pattern `pat` matches the constant value `v`: `Some(true)`/`Some(false)` for a decided
/// match, `None` when the pattern shape is not yet interpreted (the caller then DECLINES the whole
/// evaluation — never guesses an arm, since a wrong verdict would miscompile). Handles wildcard/binder, list
/// nil / leading+rest, scalar literals, and VARIANT patterns (a constructor, nullary or applied — matched by
/// discriminant, with payload sub-patterns matched recursively).
pub(super) fn const_pattern_matches(db: &mut Db, pat: StructId, v: &CVal) -> Option<bool> {
    if let Some(items) = db
        .ast
        .compound_form_of(pat, crate::ast::CompoundCtor::List)
        .map(<[StructId]>::to_vec)
    {
        let CVal::List(xs) = v else {
            return Some(false);
        };
        // A REST pattern `(list p0 … p_{k-1} .. rest)` matches a list with at least the leading count, each
        // leading element matching; a FIXED pattern `(list p0 … p_{n-1})` matches a list of exactly n, each
        // element matching.
        let rest_pos = db.ast.rest_marker(&items).map(|(i, _, _)| i);
        return match rest_pos {
            Some(rp) => {
                if xs.len() < rp {
                    return Some(false);
                }
                all_match(db, &items[..rp], xs)
            }
            None => {
                if xs.len() != items.len() {
                    return Some(false);
                }
                all_match(db, &items, xs)
            }
        };
    }
    // A TUPLE pattern `(tuple p0 … p_{n-1})` — destructures a `CVal::Tuple` positionally: it matches a tuple
    // of the SAME arity, each element matching its sub-pattern. (A tuple's arity is fixed by its type, so a
    // well-typed scrutinee always has the pattern's arity, but guard anyway.) Handled BEFORE the variant/ctor
    // dispatch because `tuple` is the tuple constructor, not a `(meta variant)`, so it would otherwise fall
    // through to the undecidable tail and decline — the gap that blocked a `(const (: t (Tuple …)))` recursion.
    // Recognizes the native `#tuple(…)` ctor-leaf head too (`compound_form_of`), not only the name-head alias
    // `(tuple …)` — the M2 native-compound migration nativized corpus tuple patterns, and reading only the
    // alias here silently declined the whole const-eval (a `(const …)` tuple-pattern recursion lost its trap).
    if let Some(items) = db
        .ast
        .compound_form_of(pat, crate::ast::CompoundCtor::Tuple)
        .map(<[StructId]>::to_vec)
    {
        let CVal::Tuple(vs) = v else {
            return Some(false);
        };
        if vs.len() != items.len() {
            return Some(false);
        }
        return all_match(db, &items, vs);
    }
    // A VARIANT pattern — a constructor with a `(meta variant)` discriminant. An APPLIED pattern
    // `(Ctor sp0 …)` is a list whose HEAD is the constructor (the discriminant is read off the HEAD, not the
    // whole application) and whose tail are the sub-patterns; a bare/nullary constructor (a name atom, or a
    // `(. Sum V)` member whose head is the `.` atom) is the constructor itself with no sub-patterns. Checked
    // BEFORE the bare-name binder case so a nullary constructor is matched by discriminant, not mistaken for a
    // catch-all binder.
    let (ctor_head, subpats): (StructId, Vec<StructId>) = match db.ast.get(pat) {
        crate::ast::Struct::List(xs2)
            if xs2.first().is_some_and(|&h| db.ast.as_name(h) != Some(".")) =>
        {
            (xs2[0], xs2.iter().skip(1).copied().collect())
        }
        _ => (pat, Vec::new()),
    };
    if let Some(pdisc) = crate::eval::variant_disc_of(db, ctor_head) {
        let CVal::Sum { disc, payloads, .. } = v else {
            return Some(false);
        };
        if *disc != pdisc {
            return Some(false);
        }
        if subpats.is_empty() {
            // A nullary-constructor pattern (no sub-patterns) matches a no-payload variant of this disc.
            return Some(payloads.is_empty());
        }
        // A single-payload constructor binds one value; a multi-payload constructor's tuple destructure is a
        // later stage (decline rather than mis-decide).
        let payloads = payloads.clone();
        if subpats.len() != payloads.len() {
            // A NULLARY variant (no payload) written with placeholder sub-pattern(s) — e.g. the corpus
            // convention `(Ordering.Less _)` / `(Ordering.Greater _)` for the payload-less Ordering variants
            // (core_of's matcher tolerates the `_`; `lower_compare` builds them with EMPTY payloads). A
            // wildcard/binder sub-pattern over an empty payload binds nothing and matches vacuously — the disc
            // already matched above. Any OTHER arity mismatch (a real payload-count disagreement) declines.
            if payloads.is_empty() && subpats.iter().all(|&sp| db.ast.as_name(sp).is_some()) {
                return Some(true);
            }
            return None;
        }
        return all_match(db, &subpats, &payloads);
    }
    if let Some(_n) = db.ast.as_name(pat) {
        // A binder or wildcard `_` matches any value (its binding is read via the arm body's `SumPayload`
        // projection, not bound here).
        return Some(true);
    }
    match (resolved_of(db, pat), v) {
        (Resolved::Int(lit), CVal::Int(x)) => Some(x.cmp(&lit) == std::cmp::Ordering::Equal),
        (Resolved::Bool(lit), CVal::Bool(x)) => Some(*x == lit),
        (Resolved::Char(lit), CVal::Char(x)) => Some(*x == lit),
        (Resolved::Str(lit), CVal::Str(x)) => Some(x.as_ref() == lit.as_str()),
        _ => None,
    }
}

/// All `pats[i]` match `vals[i]` — `Some(true)` only if every sub-pattern decides true, `Some(false)` if any
/// decides false, `None` if any is undecidable (so the caller declines).
pub(super) fn all_match(db: &mut Db, pats: &[StructId], vals: &[CVal]) -> Option<bool> {
    for (&p, x) in pats.iter().zip(vals.iter()) {
        if !const_pattern_matches(db, p, x)? {
            return Some(false);
        }
    }
    Some(true)
}

/// Materialize a constant value to a `Core` constant — the bridge from the evaluator's value domain back to
/// the lowering's. A list becomes a `Core::ListNew` whose elements are freshly-synthesized literal AST nodes
/// (so `core_is_const_value` / the backend see an ordinary constant list). `None` for a value with no
/// literal AST form this stage synthesizes (e.g. a `Unit` inside a list).
pub(super) fn cval_to_core(db: &mut Db, v: &CVal) -> Option<Core> {
    Some(match v {
        CVal::Int(x) => Core::ConstInt(x.clone()),
        CVal::Bool(b) => Core::ConstBool(*b),
        CVal::Char(c) => Core::ConstChar(*c),
        CVal::Float(d) => Core::ConstFloat(d.clone()),
        CVal::Str(s) => Core::ConstStr(s.clone()),
        CVal::Bytes(b) => Core::ConstBytes(b.clone()),
        CVal::Unit => Core::Unit,
        // A TAKEN const-fold trap surfaces its message as the compile error (CDZ0304 `ConstTrap`) — a
        // const-executed `trap` is a fail-loud, actionable authoring error (e.g. "missing required pragma:
        // input"), not a silent decline to a runtime value.
        CVal::Trap(msg) => Core::Poison(Reject::coded(Code::ConstTrap, msg.to_string())),
        // A CLOSURE is not materializable data — it exists only transiently as a higher-order fold applies it.
        // A fold that would YIELD a closure as its result is not a constant value; decline cleanly.
        CVal::Closure { .. } => return None,
        // A MAP is queried, never materialized (its runtime CHAMP iteration order must not be presumed) — decline.
        CVal::Map(_) => return None,
        CVal::Set(_) => return None,
        // A compound (list / sum / record / tuple) materializes to the corresponding `Core` DIRECTLY, its
        // element/payload nodes synthesized via `cval_to_ast` (each a `synth_core` node whose memoized core IS
        // the element's constant). No constructor head, no AST re-parse, no resolution — so a value with no
        // syntactic constructor (a reflected `Ast.module` form, a `List.at` Option) materializes exactly like
        // a syntactically-built one.
        CVal::List(xs) => {
            let mut elems = Vec::with_capacity(xs.len());
            for x in xs {
                elems.push(cval_to_ast(db, x)?);
            }
            Core::ListNew {
                elems: elems.into(),
            }
        }
        CVal::Sum { disc, payloads } => {
            let mut ps = Vec::with_capacity(payloads.len());
            for p in payloads {
                ps.push(cval_to_ast(db, p)?);
            }
            Core::SumNew {
                disc: *disc,
                payloads: ps.into(),
            }
        }
        CVal::Tuple(xs) => {
            let mut elems = Vec::with_capacity(xs.len());
            for x in xs {
                elems.push(cval_to_ast(db, x)?);
            }
            Core::Tuple {
                elems: elems.into(),
            }
        }
        CVal::Record(m) => {
            let mut fields = std::collections::BTreeMap::new();
            for (k, val) in m {
                fields.insert(k.clone(), cval_to_ast(db, val)?);
            }
            Core::Record {
                fields: std::rc::Rc::new(fields),
            }
        }
    })
}

/// Synthesize a NODE for a constant value — a scalar is a literal atom; a compound is a `synth_core` node
/// whose memoized core is the value's `Core` (so `core_of`/`core_is_const_value`/the backend see an ordinary
/// constant, no resolution needed). Used for the element/payload/field occurrences a materialized
/// `Core::ListNew`/`SumNew`/`Record`/`Tuple` references.
pub(super) fn cval_to_ast(db: &mut Db, v: &CVal) -> Option<StructId> {
    Some(match v {
        CVal::Int(x) => db.push_atom(crate::ast::Leaf::Int {
            value: x.clone(),
            radix: crate::ast::Radix::Dec,
        }),
        CVal::Bool(b) => db.push_atom(crate::ast::Leaf::Bool(*b)),
        CVal::Char(c) => db.push_atom(crate::ast::Leaf::Char(*c)),
        CVal::Float(d) => db.push_atom(crate::ast::Leaf::Float(d.clone())),
        CVal::Str(s) => db.push_atom(crate::ast::Leaf::Str(s.clone())),
        CVal::Bytes(b) => db.push_atom(crate::ast::Leaf::Bytes(b.clone())),
        CVal::Unit => synth_core(db, Core::Unit, crate::ty::Ty::Unit),
        CVal::List(_) | CVal::Sum { .. } | CVal::Record(_) | CVal::Tuple(_) => {
            let core = cval_to_core(db, v)?;
            // The type is not read on the const-fold / encode path (those read the memoized `Core`), so a
            // permissive `Any` is sufficient — the authoritative fact is the core.
            synth_core(db, core, crate::ty::Ty::Any)
        }
        // A trap is not a materializable element — it propagates (via `ce!`) before any compound is built, so
        // this is unreachable in practice; decline defensively rather than synthesize a bogus node.
        CVal::Trap(_) => return None,
        // A closure is not a materializable element (a function is not data); decline defensively.
        CVal::Closure { .. } => return None,
        // A MAP is queried, never materialized (its runtime CHAMP iteration order must not be presumed) — decline.
        CVal::Map(_) => return None,
        CVal::Set(_) => return None,
    })
}

/// Deep-copy an AST subtree into FRESH nodes (a prim head re-materialized for a delegated `core_of` fold), so
/// splicing it into a synthesized application never reparents a node the original program still holds.
pub(super) fn copy_ast_subtree(db: &mut Db, node: StructId) -> StructId {
    match db.ast.get(node) {
        crate::ast::Struct::Atom(lid) => {
            let leaf = db.ast.leaf(*lid).clone();
            db.push_atom(leaf)
        }
        crate::ast::Struct::List(children) => {
            let children = children.clone();
            let copies: Vec<StructId> = children.iter().map(|&c| copy_ast_subtree(db, c)).collect();
            db.push_list(copies)
        }
    }
}

pub(super) fn core_is_const_value(db: &mut Db, c: &Core) -> bool {
    match c {
        Core::ConstInt(_)
        | Core::ConstBool(_)
        | Core::ConstChar(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstFloat(_)
        | Core::Unit => true,
        Core::Tuple { elems } | Core::ListNew { elems } => {
            elems.iter().all(|&e| is_const_value(db, e))
        }
        Core::SumNew { payloads, .. } => payloads.iter().all(|&p| is_const_value(db, p)),
        Core::Record { fields } => fields.values().all(|&v| is_const_value(db, v)),
        Core::MapNew { entries, .. } => entries
            .iter()
            .all(|&(k, v)| is_const_value(db, k) && is_const_value(db, v)),
        Core::SetOf { elems, .. } => elems.iter().all(|&e| is_const_value(db, e)),
        // A `BytesOf` of constant elements is a constant bytes value (see `is_const_value`).
        Core::BytesOf { elems } => elems.iter().all(|&e| is_const_value(db, e)),
        _ => false,
    }
}
