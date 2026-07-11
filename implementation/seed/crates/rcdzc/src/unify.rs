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
use crate::ty::{IntTy, Scheme, Sign, Ty, Width};
use std::collections::HashMap;
use tracing::trace;

/// A substitution: what each type, width, and SIGN variable has been solved to. Applied to a type,
/// it replaces solved variables with their solutions (transitively).
#[derive(Clone, Debug, Default)]
pub struct Subst {
    tys: HashMap<u32, Ty>,
    widths: HashMap<u32, Width>,
    signs: HashMap<u32, Sign>,
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
            Ty::Int(it) => Ty::Int(IntTy {
                sign: self.apply_sign(it.sign),
                width: self.apply_width(it.width),
            }),
            Ty::Fn(p, r) => Ty::Fn(Box::new(self.apply(p)), Box::new(self.apply(r))),
            Ty::Record(fields) => Ty::Record(
                fields
                    .iter()
                    .map(|(k, t)| (k.clone(), self.apply(t)))
                    .collect(),
            ),
            Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| self.apply(t)).collect()),
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

    /// Apply the substitution to a sign — resolve a solved sign variable.
    fn apply_sign(&self, s: Sign) -> Sign {
        match s {
            Sign::Var(v) => match self.signs.get(&v) {
                Some(&sol) => self.apply_sign(sol),
                None => Sign::Var(v),
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
                trace!(target: "rcdzc::unify", var = *v, ty = %t.render_name(), "occurs-check failed (infinite type)");
                return Err(Reject::coded(
                    Code::TypeMismatch,
                    "a type would contain itself (infinite type)",
                ));
            }
            trace!(target: "rcdzc::unify", var = *v, solved = %t.render_name(), "bind type variable");
            subst.tys.insert(*v, t.clone());
            Ok(())
        }
        (Ty::Bool, Ty::Bool) | (Ty::Unit, Ty::Unit) | (Ty::Type, Ty::Type) => Ok(()),
        // Integers unify on BOTH axes — sign and width. A variable/deferred axis resolves to the
        // other's fixed value; two DIFFERENT fixed values conflict (no implicit promotion — neither a
        // width nor a signedness silently changes). Unifying the sign first lets a `mismatch` name the
        // conflict; a deferred literal (`Deferred` on both axes) grounds to whatever it meets.
        (Ty::Int(ia), Ty::Int(ib)) => {
            unify_sign(subst, ia.sign, ib.sign, &a, &b)?;
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
        // Tuples unify at EQUAL ARITY, position by position — a different arity is an irreconcilable
        // type (no structural subtyping), so it is the conflicting-use error.
        (Ty::Tuple(ea), Ty::Tuple(eb)) => {
            if ea.len() != eb.len() {
                return Err(mismatch(&a, &b));
            }
            for (ta, tb) in ea.iter().zip(eb) {
                unify(subst, ta, tb)?;
            }
            Ok(())
        }
        _ => Err(mismatch(&a, &b)),
    }
}

/// Unify two signednesses — a variable takes the other; a deferred sign is compatible with anything
/// (it grounds later); two DIFFERENT fixed signs conflict (a signed and an unsigned integer are
/// distinct types — no silent promotion). `a`/`b` are the enclosing integer types, for the error.
fn unify_sign(subst: &mut Subst, sa: Sign, sb: Sign, a: &Ty, b: &Ty) -> Result<(), Reject> {
    let sa = resolve_sign(subst, sa);
    let sb = resolve_sign(subst, sb);
    match (sa, sb) {
        (Sign::Var(v), Sign::Var(w)) if v == w => Ok(()),
        (Sign::Var(v), other) | (other, Sign::Var(v)) => {
            trace!(target: "rcdzc::unify", var = v, solved = ?other, "bind sign variable");
            subst.signs.insert(v, other);
            Ok(())
        }
        // A deferred (literal) sign takes whatever it meets.
        (Sign::Deferred, _) | (_, Sign::Deferred) => Ok(()),
        (Sign::Fixed(x), Sign::Fixed(y)) if x == y => Ok(()),
        (Sign::Fixed(x), Sign::Fixed(y)) => {
            trace!(target: "rcdzc::unify", lhs = x, rhs = y, "sign conflict (CDZ0301, no silent promotion)");
            Err(mismatch(a, b))
        }
    }
}

fn resolve_sign(subst: &Subst, s: Sign) -> Sign {
    match s {
        Sign::Var(v) => match subst.signs.get(&v) {
            Some(&sol) => resolve_sign(subst, sol),
            None => Sign::Var(v),
        },
        other => other,
    }
}

/// Unify two widths — a variable/deferred width takes the other; two different fixed widths conflict.
fn unify_width(subst: &mut Subst, a: Width, b: Width) -> Result<(), Reject> {
    let a = resolve_width(subst, a);
    let b = resolve_width(subst, b);
    match (a, b) {
        (Width::Var(v), Width::Var(w)) if v == w => Ok(()),
        (Width::Var(v), other) | (other, Width::Var(v)) => {
            trace!(target: "rcdzc::unify", var = v, solved = ?other, "bind width variable");
            subst.widths.insert(v, other);
            Ok(())
        }
        // A deferred (literal) width takes whatever it is unified with.
        (Width::Deferred, _) | (_, Width::Deferred) => Ok(()),
        (Width::Fixed(x), Width::Fixed(y)) if x == y => Ok(()),
        (Width::Fixed(x), Width::Fixed(y)) => {
            trace!(target: "rcdzc::unify", lhs = x, rhs = y, "width conflict (CDZ0301, no silent promotion)");
            Err(Reject::coded(
                Code::NumericMismatch,
                format!("integer widths differ: {x} vs {y}"),
            ))
        }
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
        Ty::Tuple(elems) => elems.iter().any(|ft| occurs(subst, v, ft)),
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Type | Ty::Any => false,
    }
}

/// The conflicting-use type error for two irreconcilable types. The CODE distinguishes the KIND of
/// conflict: two DIFFERENT NUMERIC types (a width or signedness mismatch — `Int32` vs `Int64`, signed
/// vs unsigned) is the numeric-model's no-silent-promotion rule (CDZ0301); ANY OTHER conflict (a
/// non-numeric type where another is required — `Bool` vs `Int64`) is the general type mismatch
/// (CDZ0203). This matches the corpus: CDZ0301 is reserved for two-different-numeric, everything else
/// is CDZ0203.
fn mismatch(a: &Ty, b: &Ty) -> Reject {
    trace!(target: "rcdzc::unify", lhs = %a.render_name(), rhs = %b.render_name(), "unify FAILED (conflicting use)");
    let both_numeric = matches!(a, Ty::Int(_)) && matches!(b, Ty::Int(_));
    let code = if both_numeric {
        Code::NumericMismatch
    } else {
        Code::TypeMismatch
    };
    Reject::coded(
        code,
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

/// Instantiate a scheme: replace each bound type/width/sign variable with a FRESH one, so this use is
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
    let mut sign_map: HashMap<u32, u32> = HashMap::new();
    for &v in &scheme.sign_vars {
        sign_map.insert(v, fresh.var());
    }
    rename(&scheme.ty, &ty_map, &width_map, &sign_map)
}

/// Rename the bound variables of a type per the fresh maps (a variable not in a map is free and kept).
fn rename(
    ty: &Ty,
    ty_map: &HashMap<u32, u32>,
    width_map: &HashMap<u32, u32>,
    sign_map: &HashMap<u32, u32>,
) -> Ty {
    match ty {
        Ty::Var(v) => Ty::Var(*ty_map.get(v).unwrap_or(v)),
        Ty::Int(it) => Ty::Int(IntTy {
            sign: rename_sign(it.sign, sign_map),
            width: rename_width(it.width, width_map),
        }),
        Ty::Fn(p, r) => Ty::Fn(
            Box::new(rename(p, ty_map, width_map, sign_map)),
            Box::new(rename(r, ty_map, width_map, sign_map)),
        ),
        Ty::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), rename(t, ty_map, width_map, sign_map)))
                .collect(),
        ),
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|t| rename(t, ty_map, width_map, sign_map))
                .collect(),
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

fn rename_sign(s: Sign, sign_map: &HashMap<u32, u32>) -> Sign {
    match s {
        Sign::Var(v) => Sign::Var(*sign_map.get(&v).unwrap_or(&v)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::{IntTy, Sign, Ty, Width};

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
        let wv = Ty::Int(IntTy {
            sign: Sign::Fixed(true),
            width: Width::Var(5),
        });
        let mut s = Subst::new();
        unify(&mut s, &wv, &Ty::int64()).unwrap();
        assert_eq!(s.apply(&wv), Ty::int64());
    }

    #[test]
    fn different_fixed_widths_conflict() {
        let i32t = Ty::Int(IntTy::fixed(true, 32));
        let i64t = Ty::int64();
        let mut s = Subst::new();
        assert!(unify(&mut s, &i32t, &i64t).is_err());
    }

    #[test]
    fn different_signs_conflict() {
        // Int8 and UInt8 differ only in sign — they must NOT unify (no silent promotion).
        let i8t = Ty::Int(IntTy::fixed(true, 8));
        let u8t = Ty::Int(IntTy::fixed(false, 8));
        let mut s = Subst::new();
        assert!(unify(&mut s, &i8t, &u8t).is_err());
    }

    #[test]
    fn a_deferred_sign_grounds_to_what_it_meets() {
        // A bare literal (deferred sign + width) unifies with UInt8 — the annotation grounds it. The
        // deferred axes don't conflict; the literal takes the unsigned-8 type.
        let lit = Ty::Int(IntTy::deferred());
        let u8t = Ty::Int(IntTy::fixed(false, 8));
        let mut s = Subst::new();
        assert!(unify(&mut s, &lit, &u8t).is_ok());
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
            sign_vars: vec![],
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
