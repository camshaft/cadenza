//! `infer : Hir → typed-Hir` — the Hindley-Milner inference pass, module-wide.
//!
//! Real HM (per `inference-plan-learn-from-seed-coarse-kind-mistakes` / ask-75). Each function gets a
//! signature of fresh type variables (params + return) BEFORE any body is inferred, so a call — a
//! self-call (recursion) or a call to a function defined later — unifies its arguments against the
//! callee's signature vars regardless of order. Inference is a SEPARATE pass before lowering; the
//! produced typed tree carries every node's SOLVED type, and lowering only reads it. A yet-unknown
//! operand or branch is a fresh var that unifies with its concrete sibling ORDER-INDEPENDENTLY (the
//! whole ask-14/…/77 order-dependence family cannot arise).
//!
//! Phase 2a is monomorphic (no let-generalization / polymorphic recursion yet), but it is genuine
//! unification across the whole call graph — the mechanism Phase 2b's generics/records extend by
//! adding `Ty` variants + unify arms, never by re-deriving at emit.

use crate::diag::Code;
use crate::ir::{Hir, HirFunc, HirModule, Reject, Typed, TypedFunc, TypedModule, TypedNode};
use crate::ty::{instantiate, unify, Subst, TVarSupply, Ty};
use std::collections::HashMap;

/// A function's signature during inference: param type vars + return type var.
#[derive(Clone)]
struct Sig {
    params: Vec<Ty>,
    ret: Ty,
}

/// Infer types for a resolved module, producing a fully-typed module. Signatures are assigned fresh
/// vars up front (so recursion / forward references unify), then each body is inferred against them,
/// then everything is finalized against the completed substitution.
pub fn infer_module(module: HirModule) -> Result<TypedModule, Reject> {
    let mut supply = TVarSupply::new();
    let mut subst = Subst::new();

    // Assign each function a signature of fresh type variables.
    let sigs: Vec<Sig> = module
        .funcs
        .iter()
        .map(|f| Sig {
            params: (0..f.arity).map(|_| supply.fresh()).collect(),
            ret: supply.fresh(),
        })
        .collect();

    // Pre-pass: bind the return-type var of every MODULE-RECORD function (a nullary function whose
    // body is a bare `Hir::Record`) to its record type BEFORE the main loop, so a `(. m f)` projection
    // in another function sees a solved `Ty::Record` operand regardless of inference order. A module
    // record's field types come from FuncRef/value signatures (known signature-first), so this needs
    // only the signatures. Without it, a projection of a module inferred before the record's own body
    // sees an unsolved var and declines.
    for (f, sig) in module.funcs.iter().zip(&sigs) {
        // A MODULE-RECORD function is NULLARY and its record body references no local — its fields are
        // FuncRefs and nullary calls, typeable from signatures alone. A user data-record function like
        // `(def (f n) (record (a n)))` is NOT this (it has a param / a free local) and is left to the
        // main loop, where its locals are bound.
        if f.arity == 0 {
            if let Hir::Record(_) = &f.body {
                if !hir_uses_local(&f.body) {
                    let mut ctx = Infer { supply: &mut supply, subst: &mut subst, sigs: &sigs, locals: HashMap::new() };
                    let typed = ctx.expr(&f.body)?;
                    unify_at(&mut subst, &typed.ty, &sig.ret, "module record type")?;
                }
            }
        }
    }

    // Infer each body against its signature, threading the shared substitution across the whole
    // call graph (so a constraint discovered in one function propagates to another).
    let mut bodies: Vec<Typed> = Vec::with_capacity(module.funcs.len());
    for (f, sig) in module.funcs.iter().zip(&sigs) {
        let mut ctx = Infer {
            supply: &mut supply,
            subst: &mut subst,
            sigs: &sigs,
            locals: HashMap::new(),
        };
        // Parameters bind locals 0..arity to their signature vars.
        for (i, pty) in sig.params.iter().enumerate() {
            ctx.locals.insert(i as u32, pty.clone());
        }
        let typed = ctx.expr(&f.body)?;
        // The body's type must equal the function's declared return var.
        unify_at(
            &mut subst,
            &typed.ty,
            &sig.ret,
            "function body type must match its return type",
        )?;
        bodies.push(typed);
    }

    // Finalize: resolve every function's signature + body against the completed substitution. A
    // residual type variable (an undetermined type) is a compiler bug surfaced as a decline.
    let mut funcs = Vec::with_capacity(module.funcs.len());
    for ((f, sig), body) in module.funcs.iter().zip(&sigs).zip(bodies) {
        let params = sig
            .params
            .iter()
            .map(|t| ground(&subst, t))
            .collect::<Result<Vec<_>, _>>()?;
        let ret = ground(&subst, &sig.ret)?;
        let body = finalize(&subst, body)?;
        funcs.push(TypedFunc {
            name: f.name.clone(),
            params,
            ret,
            body,
        });
    }
    Ok(TypedModule {
        funcs,
        exports: module.exports,
    })
}

/// Phase-0/1 single-body entry retained for the unit tests + the `compile_program` degenerate path:
/// infer a lone body as `main`'s (a nullary entry). Delegates to `infer_module` on a one-function
/// module.
#[allow(dead_code)]
pub fn infer(hir: Hir) -> Result<Typed, Reject> {
    let module = HirModule {
        funcs: vec![HirFunc {
            name: "main".to_string(),
            arity: 0,
            body: hir,
        }],
        exports: vec![crate::ir::Export {
            name: "main".to_string(),
            func: 0,
        }],
    };
    let typed = infer_module(module)?;
    Ok(typed.funcs.into_iter().next().unwrap().body)
}

struct Infer<'a> {
    supply: &'a mut TVarSupply,
    subst: &'a mut Subst,
    sigs: &'a [Sig],
    /// Local id → its type (a signature var for a param, or the value's type for a `let`).
    locals: HashMap<u32, Ty>,
}

impl<'a> Infer<'a> {
    fn expr(&mut self, hir: &Hir) -> Result<Typed, Reject> {
        match hir {
            Hir::Int(n) => Ok(Typed {
                node: TypedNode::Int(*n),
                ty: Ty::Int,
            }),
            Hir::Bool(b) => Ok(Typed {
                node: TypedNode::Bool(*b),
                ty: Ty::Bool,
            }),
            Hir::Str(s) => Ok(Typed { node: TypedNode::Str(s.clone()), ty: Ty::String }),
            Hir::Unit => Ok(Typed { node: TypedNode::Unit, ty: Ty::Unit }),
            // `_` in VALUE position is meaningless (it is only a pattern leaf, handled by
            // `infer_pattern`); reaching here means a `_` outside a pattern — a clean reject.
            Hir::Wildcard => Err(Reject::decline("`_` is only valid in a pattern")),
            Hir::Local(id) => {
                let ty = self
                    .locals
                    .get(id)
                    .cloned()
                    .ok_or_else(|| Reject::decline(format!("unresolved local {id}")))?;
                Ok(Typed {
                    node: TypedNode::Local(*id),
                    ty,
                })
            }
            Hir::Call { func, args } => {
                let sig = self
                    .sigs
                    .get(*func)
                    .ok_or_else(|| Reject::decline("call to unknown function"))?
                    .clone();
                // TOO MANY arguments is a genuine arity error (CDZ0201). FEWER arguments than parameters
                // is a PARTIAL APPLICATION — `(add 3)` where `add` takes two. With compile-time closures,
                // an under-arity `Call` types as a `Ty::Fn(remaining_params, ret)` (the type of the partial
                // application), after unifying the supplied args against the leading params. The FOLD then
                // decides emittability: a partial application completed to full arity at compile time
                // collapses to a `Call`; one that escapes survives as a `Ty::Fn` value and declines at
                // `select` (Increment B). So the accept/decline line moves from infer (eager, type-blind)
                // to the fold (reduction-aware) — exactly where const-fold is decided. Over-application
                // stays CDZ0201.
                if args.len() > sig.params.len() {
                    return Err(Reject::coded(
                        Code::TypeError,
                        format!("function expects {} argument(s), got {}", sig.params.len(), args.len()),
                    ));
                }
                let mut targs = Vec::with_capacity(args.len());
                for (a, pty) in args.iter().zip(&sig.params) {
                    let ta = self.expr(a)?;
                    self.unify_at(&ta.ty, pty, "argument type must match the parameter")?;
                    targs.push(ta);
                }
                // If under-arity, the result is a `Ty::Fn` of the remaining params to the return type.
                let ty = if args.len() < sig.params.len() {
                    let remaining: Vec<Ty> = sig.params[args.len()..].to_vec();
                    Ty::Fn(remaining, Box::new(sig.ret))
                } else {
                    sig.ret
                };
                Ok(Typed {
                    node: TypedNode::Call {
                        func: *func,
                        args: targs,
                    },
                    ty,
                })
            }
            Hir::FuncRef(func) => {
                // A function value: its type is the referenced function's signature `Fn(params, ret)`.
                let sig = self
                    .sigs
                    .get(*func)
                    .ok_or_else(|| Reject::decline("reference to unknown function"))?
                    .clone();
                let ty = Ty::Fn(sig.params, Box::new(sig.ret));
                Ok(Typed { node: TypedNode::FuncRef(*func), ty })
            }
            Hir::Intrinsic(op) => {
                // A built-in operation value: its type is `Fn(params, ret)` from the op's signature.
                // A PARAMETRIC op's signature uses `Ty::Param(i)` placeholders; instantiate them with a
                // fresh var per parameter (like a `Ctor` instantiates its sum's params) so `List.len`'s
                // element type is fresh per use. A monomorphic op (`param_count`=0) instantiates trivially.
                let (params, ret) = op.signature();
                let args: Vec<Ty> = (0..op.param_count()).map(|_| self.supply.fresh()).collect();
                let params = params.iter().map(|p| instantiate(&Some(p.clone()), &args)).collect();
                let ret = instantiate(&Some(ret), &args);
                Ok(Typed { node: TypedNode::Intrinsic(*op), ty: Ty::Fn(params, Box::new(ret)) })
            }
            Hir::Ctor { def, index } => {
                // A constructor VALUE — a single-arity function `Fn([payload], Sum{def, args})`.
                // Instantiate the sum's params with FRESH vars (parametric — one fresh var per param,
                // a fresh instance per use), then the variant's payload template under that
                // instantiation is the argument type (Unit for a nullary variant). So `Some : a → Option
                // a` with a fresh `a` each occurrence.
                let args: Vec<Ty> = def.params.iter().map(|_| self.supply.fresh()).collect();
                let is_nullary = def.variants()[*index].payload.is_none();
                let ret = Ty::Sum { def: def.clone(), args: args.clone() };
                // ⚡A BARE NULLARY constructor in VALUE position IS the nullary sum value (05-compound-
                // types.sexp "a bare nullary constructor is the nullary sum value"; core-semantics.md:
                // its argument type is Unit, so `NNil` ≡ `(NNil unit)`). So type it as the SUM directly,
                // not `Fn([Unit], Sum)` — a bare `NNil` and an applied `(Node.NLit n)` then share one
                // `Sum` type across an `if`'s branches. A UNARY ctor bare stays a function value
                // (`Fn([payload], Sum)`), applied via `Apply`. (When a nullary ctor IS the head of an
                // `Apply(NNil, [unit])`, `infer_apply` still sees a `Ctor` node — handled there.)
                if is_nullary {
                    Ok(Typed {
                        node: TypedNode::Ctor { def: def.clone(), index: *index },
                        ty: ret,
                    })
                } else {
                    let payload = instantiate(&def.variants()[*index].payload, &args);
                    Ok(Typed {
                        node: TypedNode::Ctor { def: def.clone(), index: *index },
                        ty: Ty::Fn(vec![payload], Box::new(ret)),
                    })
                }
            }
            Hir::Match { scrutinee, arms } => self.infer_match(scrutinee, arms),
            Hir::Trap(msg) => {
                // A runtime trap is Never — a fresh var unifies it with any sibling (an arm's result
                // type), so `(match o ((Some x) x) ((None _) (trap)))` unifies the trap arm with `x`.
                Ok(Typed { node: TypedNode::Trap(msg.clone()), ty: self.supply.fresh() })
            }
            Hir::Apply { func, args } => {
                // A NULLARY constructor applied to `unit` — `(None unit)` / `(NNil unit)` / `(Sign.Pos
                // unit)` — is the explicit spelling of the same nullary sum value the BARE ctor denotes
                // (its argument type is Unit). Since a bare nullary `Ctor` now types as the `Sum` (not a
                // `Fn`), the applied form must be handled here too: check the sole arg is unit, then the
                // result is the sum. (A UNARY ctor falls through to the ordinary function-application
                // path below, where its `Fn([payload], Sum)` type applies.)
                if let Hir::Ctor { def, index } = func.as_ref() {
                    if def.variants()[*index].payload.is_none() && args.len() == 1 {
                        let targ = self.expr(&args[0])?;
                        self.unify_at(&targ.ty, &Ty::Unit, "a nullary constructor takes unit")?;
                        let sum_args: Vec<Ty> = def.params.iter().map(|_| self.supply.fresh()).collect();
                        return Ok(Typed {
                            node: TypedNode::Ctor { def: def.clone(), index: *index },
                            ty: Ty::Sum { def: def.clone(), args: sum_args },
                        });
                    }
                }
                // Apply a function-VALUE. Infer the callee to a `Ty::Fn`, unify each arg with its
                // parameter type, result is the function's return type.
                let tfunc = self.expr(func)?;
                let targs: Vec<Typed> = args.iter().map(|a| self.expr(a)).collect::<Result<_, _>>()?;
                // NULLARY-as-value convention: a nullary function projected from a module is applied as
                // `((. m answer) unit)`. If the callee is a `Fn([], ret)` and the sole argument is
                // `unit`, the application yields `ret` — the unit is the calling convention for a
                // nullary function value, not a real parameter (core-semantics.md: construction/
                // application is `(f unit)`). This avoids a fake unit parameter whose type an unused
                // export would leave unsolved.
                let solved_func = self.subst.apply(&tfunc.ty);
                if let Ty::Fn(ps, r) = &solved_func {
                    if ps.is_empty() && targs.len() == 1 && matches!(self.subst.apply(&targs[0].ty), Ty::Unit) {
                        let ret = (**r).clone();
                        return Ok(Typed { node: TypedNode::Apply { func: Box::new(tfunc), args: targs }, ty: ret });
                    }
                }
                // Applying a value that is ALREADY the nullary sum VALUE to `unit` is identity — the
                // nullary-as-value convention through a binding: `(let ((c None)) (c unit))` binds `c` to
                // the `Sum` (a bare nullary ctor is its value), and `(c unit)` yields that same sum. So the
                // result is the sum; the `unit` is the calling convention, not a real argument. (The
                // direct `(None unit)` form is handled by the `Hir::Ctor` head case above; this covers the
                // through-a-local form the head-check cannot see.)
                if matches!(solved_func, Ty::Sum { .. })
                    && targs.len() == 1
                    && matches!(self.subst.apply(&targs[0].ty), Ty::Unit)
                {
                    return Ok(tfunc);
                }
                // PARTIAL APPLICATION of a function VALUE (a `let`-bound lambda / partial call): if the
                // callee solves to a `Fn(ps, r)` with MORE params than supplied args, this is an under-
                // arity application — unify each supplied arg against the leading params, and the result
                // is `Fn(remaining_ps, r)` (the type of the partial application). The FOLD then completes
                // it (spine-collapse + β-reduce) when the rest arrive, or it declines at `select` if it
                // escapes (Increment B). This mirrors the `Hir::Call` under-arity rule — a partial
                // application types as a `Ty::Fn`, never a CDZ0201 arity error. (Over-application, and a
                // callee that is not a `Fn` at all, still fall through to the strict unify below.)
                // (At least ONE arg must be supplied — a ZERO-arg application `(Some)` is NOT currying,
                // it is a malformed / degenerate application → falls to the strict unify → CDZ0201. A
                // ctor applied to zero args must not fabricate a unit payload.)
                if let Ty::Fn(ps, r) = &solved_func {
                    if !targs.is_empty() && targs.len() < ps.len() {
                        for (targ, pty) in targs.iter().zip(ps.iter()) {
                            self.unify_at(&targ.ty, pty, "argument type must match the parameter")?;
                        }
                        let remaining: Vec<Ty> = ps[targs.len()..].to_vec();
                        let ty = Ty::Fn(remaining, r.clone());
                        return Ok(Typed { node: TypedNode::Apply { func: Box::new(tfunc), args: targs }, ty });
                    }
                }
                let ret = self.supply.fresh();
                let expected = Ty::Fn(targs.iter().map(|t| t.ty.clone()).collect(), Box::new(ret.clone()));
                self.unify_at(&tfunc.ty, &expected, "applied value must be a function of the argument types")?;
                Ok(Typed { node: TypedNode::Apply { func: Box::new(tfunc), args: targs }, ty: ret })
            }
            Hir::Tuple(elems) => {
                let telems: Vec<Typed> = elems
                    .iter()
                    .map(|e| self.expr(e))
                    .collect::<Result<_, _>>()?;
                let ty = Ty::Tuple(telems.iter().map(|t| t.ty.clone()).collect());
                Ok(Typed {
                    node: TypedNode::Tuple(telems),
                    ty,
                })
            }
            Hir::Record(fields) => {
                // Infer each field; the record's type is its (name, ty) pairs SORTED by name — the
                // canonical form, so two records of the same shape have the same `Ty` regardless of
                // source order (and lowering's slot order matches).
                let mut tfields: Vec<(String, Typed)> = Vec::new();
                for (name, e) in fields {
                    tfields.push((name.clone(), self.expr(e)?));
                }
                let mut field_tys: Vec<(String, Ty)> =
                    tfields.iter().map(|(n, t)| (n.clone(), t.ty.clone())).collect();
                field_tys.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Typed { node: TypedNode::Record(tfields), ty: Ty::Record(field_tys) })
            }
            Hir::List(elems) => {
                // The list's element type is a SINGLE fresh var to which EVERY element unifies — the
                // generic-unification payoff: two differently-typed elements clash at this var, a type
                // error (CDZ0201, e.g. `(list 1 true)`). The result is the parametric `List elem`. An
                // empty `(list)` leaves `elem` a var — solved by a later use, else `ground` declines it.
                let elem = self.supply.fresh();
                let telems: Vec<Typed> = elems
                    .iter()
                    .map(|e| self.expr(e))
                    .collect::<Result<_, _>>()?;
                for te in &telems {
                    self.unify_at(&te.ty, &elem, "list elements must all have the same type")?;
                }
                Ok(Typed {
                    node: TypedNode::List(telems),
                    ty: Ty::List(Box::new(elem)),
                })
            }
            Hir::Map(entries) => {
                // A map's KEY type + VALUE type are each a single fresh var every entry unifies to (like
                // a list's element type) — the parametric `Map K V`. A key set is runtime data, not part
                // of the type. An empty `(map)` leaves K,V vars — solved by a later use, else grounded.
                let key = self.supply.fresh();
                let val = self.supply.fresh();
                let tentries: Vec<(Typed, Typed)> = entries
                    .iter()
                    .map(|(k, v)| Ok((self.expr(k)?, self.expr(v)?)))
                    .collect::<Result<_, Reject>>()?;
                for (tk, tv) in &tentries {
                    self.unify_at(&tk.ty, &key, "map keys must all have the same type")?;
                    self.unify_at(&tv.ty, &val, "map values must all have the same type")?;
                }
                Ok(Typed {
                    node: TypedNode::Map(tentries),
                    ty: Ty::Map(Box::new(key), Box::new(val)),
                })
            }
            Hir::Set(elems) => {
                // A set's ELEMENT type is a single fresh var every element unifies to — `Set E`.
                let elem = self.supply.fresh();
                let telems: Vec<Typed> = elems
                    .iter()
                    .map(|e| self.expr(e))
                    .collect::<Result<_, _>>()?;
                for te in &telems {
                    self.unify_at(&te.ty, &elem, "set elements must all have the same type")?;
                }
                Ok(Typed {
                    node: TypedNode::Set(telems),
                    ty: Ty::Set(Box::new(elem)),
                })
            }
            Hir::RecordProj(field, r) => {
                let tr = self.expr(r)?;
                // The operand must resolve to a record CARRYING `field`. A record without it is an
                // absent-field type error (CDZ0201, compile-time — the field set is part of the type);
                // a resolved non-record is CDZ0201; an unsolved var declines (one field cannot pin the
                // whole field set).
                let solved = self.subst.apply(&tr.ty);
                let field_ty = match &solved {
                    Ty::Record(fields) => match fields.iter().find(|(n, _)| n == field) {
                        Some((_, t)) => t.clone(),
                        None => {
                            return Err(Reject::coded(
                                Code::TypeError,
                                format!("record has no field `{field}`"),
                            ))
                        }
                    },
                    Ty::Var(_) => {
                        return Err(Reject::decline(
                            "record projection on a value of unknown record type",
                        ))
                    }
                    other => {
                        return Err(Reject::coded(
                            Code::TypeError,
                            format!("member access on a non-record ({other:?})"),
                        ))
                    }
                };
                Ok(Typed {
                    node: TypedNode::RecordProj(field.clone(), Box::new(tr)),
                    ty: field_ty,
                })
            }
            Hir::TupleProj(n, t) => {
                let tt = self.expr(t)?;
                // The operand must be a tuple whose arity covers index `n`. If its type is still a
                // var, constrain it to a tuple of `n+1` fresh vars (with the projected element a
                // fresh result). A resolved non-tuple, or an out-of-range index, is CDZ0201 — the
                // tuple's ARITY is part of its type (05-compound-types).
                let solved = self.subst.apply(&tt.ty);
                let elem_ty = match &solved {
                    Ty::Tuple(elems) => {
                        if *n >= elems.len() {
                            return Err(Reject::coded(
                                Code::TypeError,
                                format!("tuple index {n} out of arity {}", elems.len()),
                            ));
                        }
                        elems[*n].clone()
                    }
                    Ty::Var(_) => {
                        // TODO fix this!
                        // Unknown tuple: require arity >= n+1 by unifying with a fresh tuple. (Phase
                        // 3a: a projection off a parameter tuple of unknown arity — rare; supported.)
                        let mut fresh: Vec<Ty> = (0..=*n).map(|_| self.supply.fresh()).collect();
                        let elem = fresh[*n].clone();
                        // pad is fine; unify against exactly n+1 elements (minimum arity).
                        let tuple_ty = Ty::Tuple(std::mem::take(&mut fresh));
                        self.unify_at(
                            &tt.ty,
                            &tuple_ty,
                            "tuple projection requires a tuple operand",
                        )?;
                        elem
                    }
                    other => {
                        return Err(Reject::coded(
                            Code::TypeError,
                            format!("tuple projection of a non-tuple ({other:?})"),
                        ))
                    }
                };
                Ok(Typed {
                    node: TypedNode::TupleProj(*n, Box::new(tt)),
                    ty: elem_ty,
                })
            }
            Hir::Arith(op, a, b) => {
                let ta = self.expr(a)?;
                let tb = self.expr(b)?;
                self.unify_at(&ta.ty, &Ty::Int, "arithmetic operand must be an integer")?;
                self.unify_at(&tb.ty, &Ty::Int, "arithmetic operand must be an integer")?;
                Ok(Typed {
                    node: TypedNode::Arith(*op, Box::new(ta), Box::new(tb)),
                    ty: Ty::Int,
                })
            }
            Hir::Bit(op, a, b) => {
                let ta = self.expr(a)?;
                let tb = self.expr(b)?;
                self.unify_at(
                    &ta.ty,
                    &Ty::Int,
                    "bitwise/division operand must be an integer",
                )?;
                self.unify_at(
                    &tb.ty,
                    &Ty::Int,
                    "bitwise/division operand must be an integer",
                )?;
                Ok(Typed {
                    node: TypedNode::Bit(*op, Box::new(ta), Box::new(tb)),
                    ty: Ty::Int,
                })
            }
            Hir::Shift(op, a, b) => {
                let ta = self.expr(a)?;
                let tb = self.expr(b)?;
                self.unify_at(&ta.ty, &Ty::Int, "shift value must be an integer")?;
                self.unify_at(&tb.ty, &Ty::Int, "shift count must be an integer")?;
                Ok(Typed {
                    node: TypedNode::Shift(*op, Box::new(ta), Box::new(tb)),
                    ty: Ty::Int,
                })
            }
            Hir::Cmp(op, a, b) => {
                let ta = self.expr(a)?;
                let tb = self.expr(b)?;
                self.unify_at(
                    &ta.ty,
                    &tb.ty,
                    "comparison operands must have the same type",
                )?;
                Ok(Typed {
                    node: TypedNode::Cmp(*op, Box::new(ta), Box::new(tb)),
                    ty: Ty::Bool,
                })
            }
            Hir::If(c, t, e) => {
                let tc = self.expr(c)?;
                self.unify_at(&tc.ty, &Ty::Bool, "if condition must be a boolean")?;
                let tt = self.expr(t)?;
                let te = self.expr(e)?;
                let result = self.supply.fresh();
                self.unify_at(&result, &tt.ty, "if branches must have the same type")?;
                self.unify_at(&result, &te.ty, "if branches must have the same type")?;
                Ok(Typed {
                    node: TypedNode::If(Box::new(tc), Box::new(tt), Box::new(te)),
                    ty: result,
                })
            }
            Hir::Let { id, value, body } => {
                let tv = self.expr(value)?;
                self.locals.insert(*id, tv.ty.clone());
                let tb = self.expr(body)?;
                let ty = tb.ty.clone();
                Ok(Typed {
                    node: TypedNode::Let {
                        id: *id,
                        value: Box::new(tv),
                        body: Box::new(tb),
                    },
                    ty,
                })
            }
            Hir::TypeVal(ty) => {
                // A type-value (a bare `Int64`/`Bool`/… as a compile-time VALUE) — typed as `Ty::Type`.
                // Used in `(: e T)`: the `T` infers to `Type`, its represented `Ty` is extracted.
                Ok(Typed { node: TypedNode::TypeVal(ty.clone()), ty: Ty::Type })
            }
            Hir::TypeCtor(kind) => {
                // A parametric type constructor — `List`, `Map`, `Set`, `Tuple2`, `Option`, `Result`.
                // Layer 2: these are first-class compile-time values typed as functions from Type(s) to Type.
                // `List : Type → Type`, `Map : Type → Type → Type`, etc. When applied to TypeVal arguments,
                // they β-reduce to a TypeVal of the constructed type (the fold handles this).
                use crate::ir::TypeCtorKind;
                let params = match kind {
                    TypeCtorKind::List | TypeCtorKind::Set | TypeCtorKind::Option => vec![Ty::Type],
                    TypeCtorKind::Map | TypeCtorKind::Result | TypeCtorKind::Tuple2 => vec![Ty::Type, Ty::Type],
                };
                let ty = Ty::Fn(params, Box::new(Ty::Type));
                Ok(Typed { node: TypedNode::TypeCtor(*kind), ty })
            }
            Hir::Annot(e, t) => {
                // `(: e T)` — annotate `e` with type `T`. Infer `T` (must be `Ty::Type`-typed), extract
                // its represented `Ty`, unify with `e`'s type (mismatch → CDZ0203). The node's type is
                // `e`'s type (unified with `T`'s). Lowers to just `e` (transparent — the constraint
                // already happened).
                //
                // ⚡Layer 2 change: parametric type expressions — `(Option Int64)`, `(List Bool)`,
                // `(Tuple A B)` — now ALSO type-check: they're `Apply(TypeCtor, [TypeVal, …])` which the
                // fold β-reduces to a TypeVal. So infer `t`, check it's `Ty::Type`-typed, and after the
                // fold it will be a TypeVal node we can extract the Ty from. If not a TypeVal after fold,
                // decline (a malformed / not-a-type RHS).
                let tt = match self.expr(t) {
                    Ok(tt) if matches!(self.subst.apply(&tt.ty), Ty::Type) => tt,
                    _ => {
                        return Err(Reject::decline(
                            "annotation type must be a type-valued expression",
                        ))
                    }
                };
                // Extract the Ty from the TypeVal node. Layer 2: for parametric types like `(List Int64)`,
                // the node is `Apply(TypeCtor, [TypeVal...])`. We need to β-reduce it here during inference
                // (before the main fold pass) to extract the constructed type. Call a helper to do this.
                let target_ty = match extract_type_value(&tt.node) {
                    Some(ty) => ty,
                    None => return Err(Reject::decline("annotation type did not reduce to a type-value")),
                };
                let te = self.expr(e)?;
                // Unify `e`'s type with the annotation target, mapping failure to CDZ0203.
                if let Err(ue) = unify(&te.ty, &target_ty, self.subst) {
                    return Err(Reject::coded(
                        Code::AnnotMismatch,
                        format!(
                            "annotation contradiction: expression has type {:?}, annotated as {:?}",
                            ue.left, ue.right
                        ),
                    ));
                }
                let ty = self.subst.apply(&te.ty);
                Ok(Typed { node: TypedNode::Annot(Box::new(te), Box::new(tt)), ty })
            }
            Hir::Const(e) => {
                // `(const e)` — assert `e` fully compile-time-reduces. Infer types it; the fold +
                // erasure fence (post-fold) will reject if not a fully-ground const. The node's type is
                // `e`'s type. Lowers to `fold(e)`.
                let te = self.expr(e)?;
                let ty = te.ty.clone();
                Ok(Typed { node: TypedNode::Const(Box::new(te)), ty })
            }
            Hir::Lambda { params, body } => {
                // `(fn (p…) body)` — a lambda. Fresh var per param; insert each into `self.locals`; infer
                // the body under them; the lambda's type is `Ty::Fn(param_vars, body_ty)`. This is textbook
                // HM for λ; `Ty::Fn` + `unify`'s existing `Fn` arm do the rest. The existing `Hir::Apply`
                // arm already unifies a `Fn`-typed callee against `Fn(argtys, freshret)` — so applying a
                // lambda already type-checks with no new code. A lambda is a transient compile-time value:
                // the fold β-reduces `Apply(Lambda, args)` to substitute params into body; a survivor
                // declines in `select`.
                let param_tys: Vec<Ty> = params.iter().map(|_| self.supply.fresh()).collect();
                for (id, ty) in params.iter().zip(&param_tys) {
                    self.locals.insert(*id, ty.clone());
                }
                let tb = self.expr(body)?;
                let ty = Ty::Fn(param_tys.clone(), Box::new(tb.ty.clone()));
                Ok(Typed {
                    node: TypedNode::Lambda {
                        params: params.clone(),
                        body: Box::new(tb),
                    },
                    ty,
                })
            }
            Hir::Error(reject) => Err(reject.clone()),
        }
    }

    /// Infer a `(match scrutinee arms…)`. Each arm's pattern is checked against the scrutinee type
    /// (binding its `Bind` locals at their solved types), each arm body inferred, and all bodies unified
    /// to ONE result type. Then exhaustiveness (CDZ0210) is checked against the scrutinee's variant set.
    fn infer_match(&mut self, scrutinee: &Hir, arms: &[(Hir, Hir)]) -> Result<Typed, Reject> {
        let tscrut = self.expr(scrutinee)?;
        let result = self.supply.fresh();
        let mut targs: Vec<(Typed, Typed)> = Vec::with_capacity(arms.len());
        for (pat, body) in arms {
            // A pattern is an ordinary `Hir` (a `Ctor`-application / `Tuple` / literal / `Local` binder /
            // `Wildcard`). Inference does TWO things only: (1) every pattern has the scrutinee's type
            // (a binder leaf takes the type it faces), and (2) every arm body has one result type.
            // EXHAUSTIVENESS is NOT checked here — it is a lowering (MIR) concern, over the concrete
            // variant/disc set.
            let tpat = self.infer_pattern(pat, &tscrut.ty)?;
            let tbody = self.expr(body)?;
            self.unify_at(&result, &tbody.ty, "match arms must have the same type")?;
            targs.push((tpat, tbody));
        }
        // A match on a TUPLE scrutinee is single-arm DESTRUCTURING (`(match t ((tuple a b) …))` — the
        // shape a self-hosted decoder uses to unpack a `(tuple <Ast> <next-offset>)`). A tuple has ONE
        // "variant" (itself), so exactly one arm and no discriminant: bind the tuple pattern's elements
        // against the scrutinee handle and emit the body. Exhaustiveness is by construction (one arm
        // covering the sole shape); `select` binds it via the tuple-payload path (arr-get per element).
        // Requires the single arm's pattern to be a `Tuple` (or a catch-all binder/wildcard).
        if matches!(self.subst.apply(&tscrut.ty), Ty::Tuple(_)) {
            if targs.len() != 1 {
                return Err(Reject::decline(
                    "a tuple-scrutinee match with more than one arm is a later phase",
                ));
            }
            if !matches!(targs[0].0.node, TypedNode::Tuple(_) | TypedNode::Wildcard | TypedNode::Local(_)) {
                return Err(Reject::decline(
                    "a tuple-scrutinee match arm must be a tuple pattern or a binder",
                ));
            }
            return Ok(Typed {
                node: TypedNode::Match { scrutinee: Box::new(tscrut), arms: targs },
                ty: result,
            });
        }
        // Otherwise this slice lowers only a SUM match (a `sum-disc` cascade). A scalar/literal-pattern
        // match (on an Int/Bool scrutinee) is a later phase — DECLINE it (a clean todo) rather than
        // miscompile through the sum path.
        let def = match self.subst.apply(&tscrut.ty) {
            Ty::Sum { def, .. } => def,
            _ => return Err(Reject::decline("a match on a non-sum scrutinee is a later phase")),
        };
        // This slice's `emit_match` guards ONLY on the top constructor's discriminant + binds its
        // payload (a binder / wildcard / tuple of binders). It does NOT yet refine on an inner LITERAL
        // (`(Some 0)`) nor descend a NESTED constructor (`(Some (Some x))`) — both need a deeper guard /
        // recursive coverage. DECLINE a pattern beyond the simple shape (a clean todo) rather than
        // miscompile by ignoring the inner refinement. (Simple = ctor of binder/wildcard/tuple-of-simple.)
        for (p, _) in &targs {
            if !typed_pattern_simple(&p.node) {
                return Err(Reject::decline(
                    "a match pattern with an inner literal or nested constructor is a later phase",
                ));
            }
        }
        // EXHAUSTIVENESS (CDZ0210): the arms must cover every variant of the sum, OR carry a top-level
        // catch-all (a bare binder / wildcard). Flat coverage — each `(Ctor …)` arm covers its variant;
        // nested-pattern coverage is a later refinement. Type-driven (against `def.variants`), not
        // value-driven.
        let has_catch_all = targs
            .iter()
            .any(|(p, _)| matches!(p.node, TypedNode::Wildcard | TypedNode::Local(_)));
        if !has_catch_all {
            let mut covered = vec![false; def.variants().len()];
            for (p, _) in &targs {
                if let Some(i) = typed_pattern_variant(&p.node) {
                    if let Some(slot) = covered.get_mut(i) {
                        *slot = true;
                    }
                }
            }
            if !covered.iter().all(|&c| c) {
                return Err(Reject::coded(
                    Code::NonExhaustive,
                    format!("match does not cover every variant of `{}`", def.name),
                ));
            }
        }
        Ok(Typed {
            node: TypedNode::Match { scrutinee: Box::new(tscrut), arms: targs },
            ty: result,
        })
    }

    /// Infer a PATTERN (an `Hir` tree) against the type `expected` it faces, producing the typed pattern
    /// and binding each `Local` leaf. A `Wildcard`/`Local` binder takes `expected`; a `Ctor`-application
    /// unifies its result with `expected` so the payload binder types solve; a `Tuple` recurses; a
    /// literal unifies with its type. Patterns compose recursively (core-semantics.md §Patterns Compose).
    /// Mirrors the shapes `resolve` emits for a pattern — no separate `Pattern` type.
    fn infer_pattern(&mut self, pat: &Hir, expected: &Ty) -> Result<Typed, Reject> {
        match pat {
            Hir::Wildcard => Ok(Typed { node: TypedNode::Wildcard, ty: expected.clone() }),
            Hir::Local(id) => {
                // A binding occurrence — bind it at the type it faces.
                self.locals.insert(*id, expected.clone());
                Ok(Typed { node: TypedNode::Local(*id), ty: expected.clone() })
            }
            // A BARE constructor in pattern position (`None`, not `(None _)`) — the "bare nullary
            // constructor is the nullary sum value" case. Its uniform form is `(Ctor _)`; a bare ctor
            // pattern is a later phase, so DECLINE (a clean todo) rather than unify its `Fn` type against
            // the scrutinee and wrongly reject (decline-don't-miscompile).
            Hir::Ctor { .. } => Err(Reject::decline("a bare nullary constructor pattern is a later phase")),
            Hir::Int(_) | Hir::Bool(_) => {
                let t = self.expr(pat)?;
                self.unify_at(&t.ty, expected, "a literal pattern matches the scrutinee type")?;
                Ok(t)
            }
            // `(Ctor sub)` — a constructor pattern. A NULLARY ctor head (`(Ready _)`, `(None _)`) now
            // types as the `Sum` itself, not a `Fn`; its payload is Unit, so bind the sub against Unit
            // and the pattern's type is that sum. A UNARY ctor head types as `Fn([payload], Sum)`: bind
            // the sub against the payload type. Either way, unify the sum result with `expected` (so the
            // sum's args solve from the scrutinee), then infer the sub-pattern.
            Hir::Apply { func, args } if args.len() == 1 => {
                let tfunc = self.expr(func)?;
                // A NULLARY ctor head (`(Ready _)`, `(Node.NNil _)`) now types as the `Sum` itself (not a
                // `Fn`) — whether written bare (`Hir::Ctor`) or qualified (`(. Node NNil)` = a
                // `RecordProj` resolving to the nullary ctor). Its payload is Unit. A UNARY ctor head
                // types as `Fn([payload], Sum)`. Either way unify the sum result with `expected` (so the
                // sum's args solve from the scrutinee), then infer the sub against the payload type.
                let (payload_ty, ret_ty) = match self.subst.apply(&tfunc.ty) {
                    Ty::Fn(ps, r) if ps.len() == 1 => (ps[0].clone(), (*r).clone()),
                    sum @ Ty::Sum { .. } => (Ty::Unit, sum),
                    other => {
                        return Err(Reject::coded(
                            Code::TypeError,
                            format!("a constructor pattern head is not a single-arity constructor ({other:?})"),
                        ))
                    }
                };
                self.unify_at(&ret_ty, expected, "a constructor pattern matches its sum type")?;
                let tsub = self.infer_pattern(&args[0], &payload_ty)?;
                Ok(Typed {
                    node: TypedNode::Apply { func: Box::new(tfunc), args: vec![tsub] },
                    ty: ret_ty,
                })
            }
            Hir::Tuple(subs) => {
                let elem_tys: Vec<Ty> = subs.iter().map(|_| self.supply.fresh()).collect();
                self.unify_at(expected, &Ty::Tuple(elem_tys.clone()), "a tuple pattern matches a tuple")?;
                let tsubs = subs
                    .iter()
                    .zip(&elem_tys)
                    .map(|(s, t)| self.infer_pattern(s, t))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Typed { node: TypedNode::Tuple(tsubs), ty: Ty::Tuple(elem_tys) })
            }
            // Any other shape (e.g. a bare nullary `Ctor` with no payload — matched as `(Ctor _)` so
            // this is the ctor value itself): infer as an expression and unify with `expected`.
            other => {
                let t = self.expr(other)?;
                self.unify_at(&t.ty, expected, "a pattern matches the scrutinee type")?;
                Ok(t)
            }
        }
    }

    fn unify_at(&mut self, a: &Ty, b: &Ty, msg: &str) -> Result<(), Reject> {
        unify_at(self.subst, a, b, msg)
    }
}

/// Whether a typed PATTERN is one this slice's `emit_match` faithfully lowers: a binder (`Local`), a
/// `Wildcard`, a constructor `(Ctor inner)` whose inner is itself simple, or a `Tuple` of simple
/// sub-patterns. NOT simple: an inner LITERAL (needs a value guard) or a NESTED constructor under a
/// ctor payload (needs a recursive disc guard) — those DECLINE (a later phase) rather than miscompile.
fn typed_pattern_simple(node: &TypedNode) -> bool {
    match node {
        TypedNode::Wildcard | TypedNode::Local(_) => true,
        // `(Ctor inner)` — a single-arg application of a constructor; the inner must be simple, but NOT
        // itself a nested constructor (a `Ctor`-application inner is a nested-ctor refinement).
        TypedNode::Apply { func, args } if matches!(func.node, TypedNode::Ctor { .. }) && args.len() == 1 => {
            let inner = &args[0].node;
            // A nested constructor application inside the payload is NOT yet supported.
            let nested_ctor = matches!(inner,
                TypedNode::Apply { func, .. } if matches!(func.node, TypedNode::Ctor { .. }));
            !nested_ctor && typed_pattern_simple(inner)
        }
        TypedNode::Tuple(elems) => elems.iter().all(|e| typed_pattern_simple(&e.node)),
        _ => false, // an inner literal, or any other shape
    }
}

/// The variant index a typed PATTERN covers — a constructor pattern `(Ctor …)` is `Apply{ func:
/// Ctor{index}, .. }`, covering that variant; a bare `Ctor` value (a nullary pattern head, though the
/// uniform form is `(Ctor _)`) covers its own index. Anything else (a binder/wildcard/literal) covers
/// no specific variant. Drives the flat sum-exhaustiveness check in `infer_match`.
fn typed_pattern_variant(node: &TypedNode) -> Option<usize> {
    match node {
        TypedNode::Apply { func, .. } => match &func.node {
            TypedNode::Ctor { index, .. } => Some(*index),
            _ => None,
        },
        TypedNode::Ctor { index, .. } => Some(*index),
        _ => None,
    }
}

/// Unify two types, mapping a failure to CDZ0201 with the solved conflicting types named.
fn unify_at(subst: &mut Subst, a: &Ty, b: &Ty, msg: &str) -> Result<(), Reject> {
    unify(a, b, subst)
        .map_err(|e| Reject::coded(Code::TypeError, format!("{msg}: {:?} vs {:?}", e.left, e.right)))
}

/// Resolve a type against the substitution to a GROUND type; a residual `Var` — at the top OR nested
/// inside a compound (a tuple/record element) — is an undetermined type, surfaced as a decline, never
/// silently defaulted. EXCEPTION: an unsolved var appearing only as a SUM type ARGUMENT is a PHANTOM
/// type parameter the value does not depend on — e.g. `(None unit)` is `Option a` with `a` free, yet
/// the VALUE is fully determined (the None payload is unit). The corpus renders this `(Option Any)`;
/// the value renders `(None unit)`. So a phantom sum arg is defaulted to a placeholder (`Unit`) rather
/// than declined — it never affects the rendered value.
fn ground(subst: &Subst, ty: &Ty) -> Result<Ty, Reject> {
    let t = default_phantom_sum_args(&subst.apply(ty));
    if has_unsolved_var(&t) {
        return Err(Reject::decline(
            "type could not be determined (unsolved type variable)",
        ));
    }
    Ok(t)
}

/// Replace an unsolved `Var` that appears as a SUM type ARGUMENT with the placeholder `Ty::Unit` (the
/// `(Option Any)` phantom-parameter case — see [`ground`]). Recurses so a nested sum arg is covered.
/// A `Var` in a NON-sum-arg position (a tuple/list/record element, a bare result) is left unsolved so
/// it still declines — only a genuinely phantom sum parameter is defaulted.
fn default_phantom_sum_args(ty: &Ty) -> Ty {
    match ty {
        Ty::Sum { def, args } => Ty::Sum {
            def: def.clone(),
            args: args
                .iter()
                .map(|a| match a {
                    Ty::Var(_) => Ty::Unit, // a phantom parameter → a renderable placeholder
                    other => default_phantom_sum_args(other),
                })
                .collect(),
        },
        Ty::Tuple(es) => Ty::Tuple(es.iter().map(default_phantom_sum_args).collect()),
        Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), default_phantom_sum_args(t))).collect()),
        Ty::List(e) => Ty::List(Box::new(default_phantom_sum_args(e))),
        Ty::Map(k, v) => Ty::Map(Box::new(default_phantom_sum_args(k)), Box::new(default_phantom_sum_args(v))),
        Ty::Set(e) => Ty::Set(Box::new(default_phantom_sum_args(e))),
        // A `Fn` type is ALWAYS a compile-time-only value in rcdzc — a function / constructor / intrinsic
        // value folds away (an `Apply` reduces to a `Call`/`Mir::Sum`/`emit_intrinsic`; a bare survivor
        // declines in `select`, never crossing to run time as data). So ANY unsolved var inside a `Fn`
        // is PHANTOM — it is resolved at the APPLICATION site, not the value site, and an unapplied
        // occurrence (a prelude record's other fields, `Err`'s discarded payload) never affects a value.
        // Default every free var inside a `Fn` to `Unit` so such a value grounds. This subsumes the
        // constructor-payload and parametric-intrinsic (`List.len : List a → Int`) cases.
        Ty::Fn(ps, r) => Ty::Fn(
            ps.iter().map(default_all_vars).collect(),
            Box::new(default_all_vars(r)),
        ),
        // Type is a ground leaf — no vars to default.
        Ty::Type => Ty::Type,
        other => other.clone(),
    }
}

/// Default EVERY unsolved `Var` in `ty` to `Ty::Unit` (recursing all compounds). Applied only INSIDE a
/// `Fn` type by `default_phantom_sum_args` — where a free var is always a phantom parameter (see there).
fn default_all_vars(ty: &Ty) -> Ty {
    match ty {
        Ty::Var(_) => Ty::Unit,
        Ty::Tuple(es) => Ty::Tuple(es.iter().map(default_all_vars).collect()),
        Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), default_all_vars(t))).collect()),
        Ty::List(e) => Ty::List(Box::new(default_all_vars(e))),
        Ty::Map(k, v) => Ty::Map(Box::new(default_all_vars(k)), Box::new(default_all_vars(v))),
        Ty::Set(e) => Ty::Set(Box::new(default_all_vars(e))),
        Ty::Sum { def, args } => Ty::Sum { def: def.clone(), args: args.iter().map(default_all_vars).collect() },
        Ty::Fn(ps, r) => Ty::Fn(ps.iter().map(default_all_vars).collect(), Box::new(default_all_vars(r))),
        // Type is a ground leaf — no vars to default.
        Ty::Type => Ty::Type,
        other => other.clone(),
    }
}

/// Whether an `Hir` tree references any `Local` — used to tell a module-record body (no locals; its
/// fields are FuncRefs/nullary calls) from a data-record body that reads a parameter (`(record (a n))`).
fn hir_uses_local(h: &Hir) -> bool {
    match h {
        Hir::Local(_) => true,
        Hir::Int(_) | Hir::Bool(_) | Hir::Str(_) | Hir::Unit | Hir::FuncRef(_) | Hir::Intrinsic(_) | Hir::Error(_) | Hir::TypeVal(_) | Hir::TypeCtor(_) => false,
        // A constructor value, a wildcard, and a trap reference no local; a match may in its scrutinee
        // or arm bodies.
        Hir::Ctor { .. } | Hir::Wildcard | Hir::Trap(_) => false,
        Hir::Match { scrutinee, arms } => {
            hir_uses_local(scrutinee) || arms.iter().any(|(_, b)| hir_uses_local(b))
        }
        Hir::Call { args, .. } => args.iter().any(hir_uses_local),
        Hir::Apply { func, args } => hir_uses_local(func) || args.iter().any(hir_uses_local),
        Hir::Record(fields) => fields.iter().any(|(_, e)| hir_uses_local(e)),
        Hir::Tuple(elems) => elems.iter().any(hir_uses_local),
        Hir::List(elems) => elems.iter().any(hir_uses_local),
        Hir::Map(entries) => entries.iter().any(|(k, v)| hir_uses_local(k) || hir_uses_local(v)),
        Hir::Set(elems) => elems.iter().any(hir_uses_local),
        Hir::TupleProj(_, t) => hir_uses_local(t),
        Hir::RecordProj(_, t) => hir_uses_local(t),
        Hir::Arith(_, a, b) | Hir::Bit(_, a, b) | Hir::Shift(_, a, b) | Hir::Cmp(_, a, b) => {
            hir_uses_local(a) || hir_uses_local(b)
        }
        Hir::If(c, t, e) => hir_uses_local(c) || hir_uses_local(t) || hir_uses_local(e),
        Hir::Let { value, body, .. } => hir_uses_local(value) || hir_uses_local(body),
        // A lambda uses a local iff its body does (its own params are locals it binds, but for the
        // module-record pre-pass heuristic, treat "body uses a local" conservatively — a lambda body
        // referencing a captured outer local counts).
        Hir::Lambda { body, .. } => hir_uses_local(body),
        // TypeVal/Annot/Const recurse into their sub-expressions.
        Hir::Annot(e, t) => hir_uses_local(e) || hir_uses_local(t),
        Hir::Const(e) => hir_uses_local(e),
    }
}

/// Whether a resolved type still contains a type variable anywhere (top-level or nested in a
/// compound). `apply` has already been run, so a `Var` here is genuinely unsolved.
fn has_unsolved_var(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) => true,
        Ty::Tuple(elems) => elems.iter().any(has_unsolved_var),
        Ty::Record(fields) => fields.iter().any(|(_, t)| has_unsolved_var(t)),
        Ty::Fn(params, ret) => params.iter().any(has_unsolved_var) || has_unsolved_var(ret),
        Ty::List(elem) => has_unsolved_var(elem),
        Ty::Map(k, v) => has_unsolved_var(k) || has_unsolved_var(v),
        Ty::Set(elem) => has_unsolved_var(elem),
        Ty::Sum { args, .. } => args.iter().any(has_unsolved_var),
        // A `Param` placeholder never appears in an inferred type (always instantiated first); treat as
        // solved so a stray one never blocks grounding.
        Ty::Param(_) => false,
        Ty::Int | Ty::Bool | Ty::Unit | Ty::Bytes | Ty::String | Ty::Type => false,
    }
}

/// Resolve every node's type against the completed substitution (recursively).
fn finalize(subst: &Subst, typed: Typed) -> Result<Typed, Reject> {
    // A WILDCARD binds and emits nothing, so its type is irrelevant — never ground it (a `(Err _)` arm
    // leaves the discarded payload type free, which is fine; grounding it would false-decline).
    if matches!(typed.node, TypedNode::Wildcard) {
        return Ok(Typed { node: TypedNode::Wildcard, ty: Ty::Unit });
    }
    let ty = ground(subst, &typed.ty)?;
    let node = match typed.node {
        TypedNode::Int(n) => TypedNode::Int(n),
        TypedNode::Bool(b) => TypedNode::Bool(b),
        TypedNode::Str(s) => TypedNode::Str(s),
        TypedNode::Unit => TypedNode::Unit,
        TypedNode::Local(id) => TypedNode::Local(id),
        TypedNode::Call { func, args } => TypedNode::Call {
            func,
            args: args
                .into_iter()
                .map(|a| finalize(subst, a))
                .collect::<Result<_, _>>()?,
        },
        TypedNode::FuncRef(func) => TypedNode::FuncRef(func),
        TypedNode::Intrinsic(op) => TypedNode::Intrinsic(op),
        TypedNode::Ctor { def, index } => TypedNode::Ctor { def, index },
        TypedNode::Wildcard => TypedNode::Wildcard,
        TypedNode::Match { scrutinee, arms } => TypedNode::Match {
            scrutinee: Box::new(finalize(subst, *scrutinee)?),
            arms: arms
                .into_iter()
                .map(|(p, b)| Ok::<_, Reject>((finalize(subst, p)?, finalize(subst, b)?)))
                .collect::<Result<_, _>>()?,
        },
        TypedNode::Trap(msg) => TypedNode::Trap(msg),
        TypedNode::Apply { func, args } => TypedNode::Apply {
            func: Box::new(finalize(subst, *func)?),
            args: args.into_iter().map(|a| finalize(subst, a)).collect::<Result<_, _>>()?,
        },
        TypedNode::Tuple(elems) => TypedNode::Tuple(
            elems
                .into_iter()
                .map(|e| finalize(subst, e))
                .collect::<Result<_, _>>()?,
        ),
        TypedNode::Record(fields) => TypedNode::Record(
            fields
                .into_iter()
                .map(|(n, t)| Ok::<_, Reject>((n, finalize(subst, t)?)))
                .collect::<Result<_, _>>()?,
        ),
        TypedNode::List(elems) => TypedNode::List(
            elems
                .into_iter()
                .map(|e| finalize(subst, e))
                .collect::<Result<_, _>>()?,
        ),
        TypedNode::Map(entries) => TypedNode::Map(
            entries
                .into_iter()
                .map(|(k, v)| Ok::<_, Reject>((finalize(subst, k)?, finalize(subst, v)?)))
                .collect::<Result<_, _>>()?,
        ),
        TypedNode::Set(elems) => TypedNode::Set(
            elems
                .into_iter()
                .map(|e| finalize(subst, e))
                .collect::<Result<_, _>>()?,
        ),
        TypedNode::TupleProj(n, t) => TypedNode::TupleProj(n, Box::new(finalize(subst, *t)?)),
        TypedNode::RecordProj(f, t) => TypedNode::RecordProj(f, Box::new(finalize(subst, *t)?)),
        TypedNode::Arith(op, a, b) => TypedNode::Arith(
            op,
            Box::new(finalize(subst, *a)?),
            Box::new(finalize(subst, *b)?),
        ),
        TypedNode::Bit(op, a, b) => TypedNode::Bit(
            op,
            Box::new(finalize(subst, *a)?),
            Box::new(finalize(subst, *b)?),
        ),
        TypedNode::Shift(op, a, b) => TypedNode::Shift(
            op,
            Box::new(finalize(subst, *a)?),
            Box::new(finalize(subst, *b)?),
        ),
        TypedNode::Cmp(op, a, b) => TypedNode::Cmp(
            op,
            Box::new(finalize(subst, *a)?),
            Box::new(finalize(subst, *b)?),
        ),
        TypedNode::If(c, t, e) => TypedNode::If(
            Box::new(finalize(subst, *c)?),
            Box::new(finalize(subst, *t)?),
            Box::new(finalize(subst, *e)?),
        ),
        TypedNode::Let { id, value, body } => TypedNode::Let {
            id,
            value: Box::new(finalize(subst, *value)?),
            body: Box::new(finalize(subst, *body)?),
        },
        TypedNode::Lambda { params, body } => TypedNode::Lambda {
            params,
            body: Box::new(finalize(subst, *body)?),
        },
        TypedNode::TypeVal(ty) => TypedNode::TypeVal(ty),
        TypedNode::TypeCtor(kind) => TypedNode::TypeCtor(kind),
        TypedNode::Annot(e, t) => TypedNode::Annot(
            Box::new(finalize(subst, *e)?),
            Box::new(finalize(subst, *t)?),
        ),
        TypedNode::Const(e) => TypedNode::Const(Box::new(finalize(subst, *e)?)),
    };
    Ok(Typed { node, ty })
}

/// Extract the `Ty` from a type-valued TypedNode, β-reducing TypeCtor applications inline. This is
/// Layer 2's mechanism for handling parametric type annotations: `(: e (List Int64))` infers the RHS
/// as `Apply(TypeCtor(List), [TypeVal(Int)])`, and we need to extract `Ty::List(Int)` here during
/// inference (before the main fold pass). A bare `TypeVal` is returned directly; an `Apply(TypeCtor, ...)`
/// is β-reduced to extract the constructed type; anything else returns `None` (not a type-value).
fn extract_type_value(node: &TypedNode) -> Option<Ty> {
    match node {
        // A bare TypeVal — return its Ty directly.
        TypedNode::TypeVal(ty) => Some(ty.clone()),
        // An Apply whose func is a TypeCtor — β-reduce it to extract the constructed type.
        TypedNode::Apply { func, args } => {
            if let TypedNode::TypeCtor(kind) = &func.node {
                // Extract the Ty from each TypeVal argument.
                let arg_tys: Vec<Ty> = args.iter().filter_map(|a| extract_type_value(&a.node)).collect();
                // If not all args are TypeVals, this is not a well-formed type constructor application.
                if arg_tys.len() != args.len() {
                    return None;
                }
                // Build the compound type based on the constructor kind and argument types.
                use crate::ir::TypeCtorKind;
                return match (*kind, arg_tys.as_slice()) {
                    (TypeCtorKind::List, [elem]) => Some(Ty::List(Box::new(elem.clone()))),
                    (TypeCtorKind::Set, [elem]) => Some(Ty::Set(Box::new(elem.clone()))),
                    (TypeCtorKind::Map, [k, v]) => Some(Ty::Map(Box::new(k.clone()), Box::new(v.clone()))),
                    (TypeCtorKind::Tuple2, [a, b]) => Some(Ty::Tuple(vec![a.clone(), b.clone()])),
                    (TypeCtorKind::Option, [a]) => Some(Ty::Sum {
                        def: crate::ty::prelude_option(),
                        args: vec![a.clone()],
                    }),
                    (TypeCtorKind::Result, [a, e]) => Some(Ty::Sum {
                        def: crate::ty::prelude_result(),
                        args: vec![a.clone(), e.clone()],
                    }),
                    // Arity mismatch or unsupported form.
                    _ => None,
                };
            }
            None
        }
        // Any other node is not a type-value.
        _ => None,
    }
}
