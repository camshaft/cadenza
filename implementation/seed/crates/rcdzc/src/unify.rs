//! Unification — the pure Hindley-Milner core: a substitution, unification with occurs-check, and
//! scheme instantiation. This is the machinery the ONE generic application rule uses (`infer`), and
//! that the full function inference reuses unchanged.
//!
//! It is a deterministic, order-independent solve over type and width variables (`type-system.md`
//! §Inference Is Principal-Type Inference By Unification). A [`Subst`] maps a variable to the type (or
//! width) it has been solved to; `unify` extends it by equating two types, failing (a [`Reject`]) when
//! they cannot be made equal — the conflicting-use type error. `instantiate` freshens a [`Scheme`]'s
//! bound variables so each use is independent, which is what makes an operator generic over the
//! integer type. Nothing here reads a column or an AST node — it is pure over [`Ty`].

use crate::diag::{Code, Reject};
use crate::ty::{IntTy, Scheme, Ty, Width};
use std::collections::HashMap;

/// A substitution: what each type variable and width variable has been solved to. Applied to a type,
/// it replaces solved variables with their solutions (transitively).
#[derive(Clone, Debug, Default)]
pub struct Subst {
    tys: HashMap<u32, Ty>,
    widths: HashMap<u32, Width>,
}

impl Subst {
    pub fn new() -> Subst {
        Subst::default()
    }

    /// Apply the substitution to a type — replace every solved variable with its solution, following
    /// chains (a variable solved to another variable resolves through). Total and terminating: the
    /// occurs-check keeps the variable graph acyclic.
    pub fn apply(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Var(v) => match self.tys.get(v) {
                Some(t) => self.apply(t),
                None => Ty::Var(*v),
            },
            Ty::Int(it) => Ty::Int(IntTy { signed: it.signed, width: self.apply_width(it.width) }),
            Ty::Fn(p, r) => Ty::Fn(Box::new(self.apply(p)), Box::new(self.apply(r))),
            Ty::Record(fields) => Ty::Record(
                fields.iter().map(|(k, t)| (k.clone(), self.apply(t))).collect(),
            ),
            Ty::Bool | Ty::Unit | Ty::Type | Ty::Any => ty.clone(),
        }
    }

    /// Apply the substitution to a width — resolve a solved width variable.
    fn apply_width(&self, w: Width) -> Width {
        match w {
            Width::Var(v) => match self.widths.get(&v) {
                Some(&sol) => self.apply_width(sol),
                None => Width::Var(v),
            },
            other => other,
        }
    }
}

/// Unify two types, extending `subst` so both become equal, or return the conflicting-use type error.
/// Order-independent: `unify(a, b)` and `unify(b, a)` reach the same solution. `Ty::Any` (a poison's
/// type) unifies with anything so a "no" never induces a spurious conflict.
pub fn unify(subst: &mut Subst, a: &Ty, b: &Ty) -> Result<(), Reject> {
    let a = subst.apply(a);
    let b = subst.apply(b);
    match (&a, &b) {
        // A poison type never conflicts.
        (Ty::Any, _) | (_, Ty::Any) => Ok(()),
        // Two variables / a variable and a type: bind the variable (occurs-check first).
        (Ty::Var(v), Ty::Var(w)) if v == w => Ok(()),
        (Ty::Var(v), t) | (t, Ty::Var(v)) => {
            if occurs(subst, *v, t) {
                return Err(Reject::coded(
                    Code::TypeMismatch,
                    "a type would contain itself (infinite type)",
                ));
            }
            subst.tys.insert(*v, t.clone());
            Ok(())
        }
        (Ty::Bool, Ty::Bool) | (Ty::Unit, Ty::Unit) | (Ty::Type, Ty::Type) => Ok(()),
        // Integers unify only at equal signedness; their widths unify (a variable/deferred width
        // resolves to a fixed one; two different fixed widths conflict — no implicit promotion).
        (Ty::Int(ia), Ty::Int(ib)) => {
            if ia.signed != ib.signed {
                return Err(mismatch(&a, &b));
            }
            unify_width(subst, ia.width, ib.width)
        }
        (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) => {
            unify(subst, pa, pb)?;
            unify(subst, ra, rb)
        }
        (Ty::Record(fa), Ty::Record(fb)) => {
            if fa.len() != fb.len() {
                return Err(mismatch(&a, &b));
            }
            for (k, ta) in fa {
                match fb.get(k) {
                    Some(tb) => unify(subst, ta, tb)?,
                    None => return Err(mismatch(&a, &b)),
                }
            }
            Ok(())
        }
        _ => Err(mismatch(&a, &b)),
    }
}

/// Unify two widths — a variable/deferred width takes the other; two different fixed widths conflict.
fn unify_width(subst: &mut Subst, a: Width, b: Width) -> Result<(), Reject> {
    let a = resolve_width(subst, a);
    let b = resolve_width(subst, b);
    match (a, b) {
        (Width::Var(v), Width::Var(w)) if v == w => Ok(()),
        (Width::Var(v), other) | (other, Width::Var(v)) => {
            subst.widths.insert(v, other);
            Ok(())
        }
        // A deferred (literal) width takes whatever it is unified with.
        (Width::Deferred, _) | (_, Width::Deferred) => Ok(()),
        (Width::Fixed(x), Width::Fixed(y)) if x == y => Ok(()),
        (Width::Fixed(x), Width::Fixed(y)) => Err(Reject::coded(
            Code::NumericMismatch,
            format!("integer widths differ: {x} vs {y}"),
        )),
    }
}

fn resolve_width(subst: &Subst, w: Width) -> Width {
    match w {
        Width::Var(v) => match subst.widths.get(&v) {
            Some(&sol) => resolve_width(subst, sol),
            None => Width::Var(v),
        },
        other => other,
    }
}

/// Whether variable `v` occurs in `t` (after substitution) — the occurs-check that keeps the variable
/// graph acyclic, so binding `v := t` cannot create an infinite type.
fn occurs(subst: &Subst, v: u32, t: &Ty) -> bool {
    match subst.apply(t) {
        Ty::Var(w) => v == w,
        Ty::Fn(p, r) => occurs(subst, v, &p) || occurs(subst, v, &r),
        Ty::Record(fields) => fields.values().any(|ft| occurs(subst, v, ft)),
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Type | Ty::Any => false,
    }
}

/// The conflicting-use type error for two irreconcilable types.
fn mismatch(a: &Ty, b: &Ty) -> Reject {
    Reject::coded(
        Code::NumericMismatch,
        format!("cannot unify {} with {}", a.render_name(), b.render_name()),
    )
}

/// A source of fresh variable numbers — one counter for a whole inference solve, so no two fresh
/// variables collide. Type and width variables share the counter's space (they are distinguished by
/// which map they land in), which is harmless and keeps one source of truth.
#[derive(Debug, Default)]
pub struct Fresh {
    next: u32,
}

impl Fresh {
    pub fn new() -> Fresh {
        Fresh { next: 0 }
    }

    /// A fresh variable number.
    pub fn var(&mut self) -> u32 {
        let n = self.next;
        self.next += 1;
        n
    }
}

/// Instantiate a scheme: replace each bound type/width variable with a FRESH one, so this use is
/// independent of every other. Returns the freshened type. This is what makes `+`'s `∀a. (Int a) →
/// (Int a) → (Int a)` apply at a fresh `a` each time — generic over the integer type.
pub fn instantiate(scheme: &Scheme, fresh: &mut Fresh) -> Ty {
    let mut ty_map: HashMap<u32, u32> = HashMap::new();
    for &v in &scheme.ty_vars {
        ty_map.insert(v, fresh.var());
    }
    let mut width_map: HashMap<u32, u32> = HashMap::new();
    for &v in &scheme.width_vars {
        width_map.insert(v, fresh.var());
    }
    rename(&scheme.ty, &ty_map, &width_map)
}

/// Rename the bound variables of a type per the fresh maps (a variable not in a map is free and kept).
fn rename(ty: &Ty, ty_map: &HashMap<u32, u32>, width_map: &HashMap<u32, u32>) -> Ty {
    match ty {
        Ty::Var(v) => Ty::Var(*ty_map.get(v).unwrap_or(v)),
        Ty::Int(it) => Ty::Int(IntTy { signed: it.signed, width: rename_width(it.width, width_map) }),
        Ty::Fn(p, r) => Ty::Fn(
            Box::new(rename(p, ty_map, width_map)),
            Box::new(rename(r, ty_map, width_map)),
        ),
        Ty::Record(fields) => Ty::Record(
            fields.iter().map(|(k, t)| (k.clone(), rename(t, ty_map, width_map))).collect(),
        ),
        Ty::Bool | Ty::Unit | Ty::Type | Ty::Any => ty.clone(),
    }
}

fn rename_width(w: Width, width_map: &HashMap<u32, u32>) -> Width {
    match w {
        Width::Var(v) => Width::Var(*width_map.get(&v).unwrap_or(&v)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::{IntTy, Ty, Width};

    #[test]
    fn unifies_a_var_with_a_concrete_type() {
        let mut s = Subst::new();
        unify(&mut s, &Ty::Var(0), &Ty::Bool).unwrap();
        assert_eq!(s.apply(&Ty::Var(0)), Ty::Bool);
    }

    #[test]
    fn unify_is_order_independent() {
        let mut a = Subst::new();
        unify(&mut a, &Ty::Var(0), &Ty::int64()).unwrap();
        let mut b = Subst::new();
        unify(&mut b, &Ty::int64(), &Ty::Var(0)).unwrap();
        assert_eq!(a.apply(&Ty::Var(0)), b.apply(&Ty::Var(0)));
    }

    #[test]
    fn width_var_unifies_then_fixes() {
        // (Int w) unified with Int64 fixes w := 64.
        let wv = Ty::Int(IntTy { signed: true, width: Width::Var(5) });
        let mut s = Subst::new();
        unify(&mut s, &wv, &Ty::int64()).unwrap();
        assert_eq!(s.apply(&wv), Ty::int64());
    }

    #[test]
    fn different_fixed_widths_conflict() {
        let i32t = Ty::Int(IntTy { signed: true, width: Width::Fixed(32) });
        let i64t = Ty::int64();
        let mut s = Subst::new();
        assert!(unify(&mut s, &i32t, &i64t).is_err());
    }

    #[test]
    fn occurs_check_rejects_infinite_type() {
        // v = (-> v v) would be infinite.
        let mut s = Subst::new();
        let f = Ty::Fn(Box::new(Ty::Var(0)), Box::new(Ty::Var(0)));
        assert!(unify(&mut s, &Ty::Var(0), &f).is_err());
    }

    #[test]
    fn instantiate_freshens_bound_vars() {
        // ∀a. a -> a  instantiates to  ?n -> ?n  for a fresh n (not var 0).
        let scheme = Scheme {
            ty_vars: vec![0],
            width_vars: vec![],
            ty: Ty::Fn(Box::new(Ty::Var(0)), Box::new(Ty::Var(0))),
        };
        let mut fresh = Fresh::new();
        let a = instantiate(&scheme, &mut fresh);
        let b = instantiate(&scheme, &mut fresh);
        // Two instantiations use DIFFERENT fresh variables (independent uses).
        assert_ne!(a, b);
        // Each is a self-consistent `?n -> ?n`.
        if let Ty::Fn(p, r) = &a {
            assert_eq!(p, r);
        } else {
            panic!("expected a function type");
        }
    }
}
