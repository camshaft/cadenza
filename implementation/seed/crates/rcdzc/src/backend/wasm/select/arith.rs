use super::*;

/// The wasm machine slot (i32 for a ≤32-bit width, i64 otherwise) for an integer op of type `ot` —
/// the same choice [`Machine::slot`] makes, computed straight from the `IntTy` so `operand_src` need
/// not build a `Machine`.
pub(super) fn m_slot(ot: IntTy) -> ValType {
    if ot.ground_width() <= 32 {
        ValType::I32
    } else {
        ValType::I64
    }
}

/// Where a checked op leaves its result:
///  - `Stack` — the usual case: the result is left on the operand stack (via `local.get $r`), for the
///    enclosing expression to consume.
///  - `Slot(d)` — the caller wants the result in local `d` and NOT on the stack. Used when this op is
///    an OPERAND of an enclosing checked op: the enclosing op would otherwise `emit_operand(this) ;
///    local.set d` — computing this result into its own `$r` then COPYING it to `d`. Passing `Slot(d)`
///    makes THIS op use `d` as its `$r` directly, so its final `local.set` IS the store and the copy
///    (`local.get $r_inner ; local.tee d`) vanishes, along with the separate `$r_inner` scratch slot.
#[derive(Clone, Copy)]
pub(super) enum ResultDest {
    Stack,
    Slot(u32),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_checked_arith(
    db: &mut Db,
    id: StructId,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    emit_checked_arith_to(
        db,
        id,
        op,
        m,
        lhs,
        rhs,
        slots,
        base,
        high,
        scratch_ty,
        layout,
        out,
        ResultDest::Stack,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_checked_arith_to(
    db: &mut Db,
    id: StructId,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    dest: ResultDest,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // GUARD-ELIDED FAST PATH: when interval arithmetic proves the result stays in the type, NO overflow
    // guard and NO range-check follow — so each operand is used EXACTLY ONCE (only the machine op reads
    // it). There is then no reason to stash a non-reusable operand in a scratch slot for the guards to
    // re-read: emit both operands straight onto the wasm operand stack, run the machine op, and place the
    // result per `dest`. This skips both operand scratch slots AND the `$r` slot for the common
    // masked/refined-arith idiom (`(+ (& x 7) (& y 7))`, a loop counter step under a proving refinement).
    // `emit_operand` grounds a bare-literal operand to the op width `ot` (an out-of-range literal is still
    // rejected CDZ0302), exactly as the guarded path's `operand_src`/`emit_operand_into` do. B's transient
    // scratch (a nested computation) floats above `base` and never aliases A's already-pushed stack value.
    // Uses the SAME `provably_no_overflow` decision the guarded path below checks after the op — moved
    // up so the slot machinery is skipped entirely rather than claimed-then-unused. `provably_no_overflow`
    // = range analysis OR a discharged proof (keyed by the arith node `id`), so a verification-licensed
    // node elides here too once v-verification's b3 fills the oracle (behavior-neutral today: the stub
    // returns false, so this ≡ `arith_provably_in_range` alone — the identical wasm bytes as before).
    let result_ty = IntTy::fixed(m.signed, m.width);
    if crate::lower::provably_no_overflow(db, op, lhs, rhs, result_ty, id) {
        emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
        // B emits its own transient scratch above the running high-water — A is already on the stack, so B
        // never needs a slot A used; a fresh floor keeps B's width-disjoint scratch from re-typing a slot.
        let b_base = base.max(*high);
        emit_operand(db, rhs, ot, slots, b_base, high, scratch_ty, layout, out)?;
        out.push(match op {
            Prim::Add => m.add(),
            Prim::Sub => m.sub(),
            Prim::Mul => m.mul(),
            _ => return Err(Reject::decline("not a checked arithmetic op")),
        });
        match dest {
            ResultDest::Stack => {}
            ResultDest::Slot(d) => out.push(Lir::LocalSet(d)),
        }
        return Ok(());
    }
    // Each operand's SOURCE at every use site (the machine op + the guard's re-reads): a reusable
    // operand — a matching local, or a compile-time constant — is pushed directly (`local.get` / an
    // inline `const`) and needs NO scratch slot; only a nested computation is stashed in a fresh
    // scratch slot (source = that slot). `$r` (the result) always needs its own scratch. Scratch slots
    // are claimed from `base`; the operand recursion floats ABOVE whatever scratch is actually used, so
    // an operand that needs no copy also frees the slot it would have occupied.
    // WIDTH-PARTITIONED CLAIM (finding #21 — arm-inline computed-perform-key slot-width alias): every scratch
    // slot this checked-arith claims is stored at `m.slot()` (the op's machine width). SKIP any slot
    // `scratch_ty` already records at a DIFFERENT width — a live let-binder / handle temp holds it (e.g. an
    // i32 `Map.lookup`-match result `m2` bound in the enclosing arm, still live across the re-materialized
    // `(+ n 1)` perform key in the resume value). Reusing it for this i64 guard would declare one wasm local
    // at two widths → `local.tee` i32 into an i64-declared slot → validator reject (`expected i64, found i32`,
    // breaker finding #21 mmlminT func[10] @0x343). Same-width reuse preserved (no local-count growth); only a
    // genuine width conflict advances past the occupied slot. The claim records the slot's type itself.
    let want = m.slot();
    let mut next_scratch = base;
    let mut claim = |high: &mut u32, scratch_ty: &mut HashMap<u32, ValType>| {
        while matches!(scratch_ty.get(&next_scratch), Some(&w) if w != want) {
            next_scratch += 1;
        }
        let s = next_scratch;
        next_scratch += 1;
        *high = (*high).max(s + 1);
        scratch_ty.insert(s, want);
        s
    };
    // A reusable source is settled now; a non-reusable operand claims a scratch slot to be stored into.
    let sa_src = operand_src(db, lhs, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high, scratch_ty);
            OperandSrc::Slot(s)
        }
    };
    // COMMON-SUBEXPRESSION ELIMINATION: if B is a non-reusable computation STRUCTURALLY IDENTICAL to A,
    // it produces the same value — so compute it ONCE (as A, into `$a`) and read `$a` for B too, rather
    // than emitting the whole computation (and its guards) a second time. `(+ (* a b) (* a b))` becomes
    // one `*` + one guard, read twice. Safe because `core_eq` only matches PURE deterministic scalar
    // computations (see its doc) — no effects, and a trapping operand traps identically. Only fires when
    // A itself was stashed in a slot (`sa` is a Slot): a reusable A (a bare local/const) is already free
    // to re-push, so B just shares that same source with no CSE needed.
    let sb_src = operand_src(db, rhs, ot, slots)?;
    // `sb_shares_a` records that B is the SAME computation as A and reuses A's slot (CSE) — so B is NOT
    // emitted separately below (it would recompute into A's slot). Distinct from `sb_src.is_some()` (a
    // reusable source that also skips the emit but for a different reason).
    let mut sb_shares_a = false;
    let sb = match sb_src {
        Some(src) => src,
        None if matches!(sa, OperandSrc::Slot(_)) && core_eq(db, lhs, rhs) => {
            trace!(target: "rcdzc::select", lhs = lhs.0, rhs = rhs.0, "CSE: identical operands share one computation");
            sb_shares_a = true;
            sa
        }
        None => {
            let s = claim(high, scratch_ty);
            OperandSrc::Slot(s)
        }
    };
    // `$r` (the result slot): the caller-requested destination when this op is an operand of an
    // enclosing op (`Slot(d)`), else a fresh scratch slot. Using `d` directly means this op's final
    // `local.set` IS the store the enclosing op wanted — no copy. `d` is one of the enclosing op's
    // operand slots, claimed BELOW this op's `base`, so this op's own operand scratch (claimed from
    // `base` up) never collides with it.
    let sr = match dest {
        ResultDest::Slot(d) => d,
        ResultDest::Stack => claim(high, scratch_ty),
    };
    // Operands that DO need a copy recurse above the scratch slots claimed so far; A is stored before
    // B runs, so B may reuse A's operand scratch (the liveness the high-water mark captures).
    let operand_base = next_scratch;
    // <A> compute A into $a — only when A is a stashed (non-reusable) operand; a reusable source is
    // pushed in place at each use. `emit_operand_into` writes the result straight into `$a`: a nested
    // checked op targets `$a` as its own result slot (no copy), any other operand is `emit_operand`ed
    // then `local.set $a`. A bare-literal operand is grounded to the OP's width `ot` by `emit_operand`.
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            lhs,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // <B> compute B into $b — only for a stashed operand that is NOT shared with A (CSE). When
    // `sb_shares_a`, B's value already sits in A's slot (computed once), so re-emitting it would both
    // recompute and clobber — skip it.
    //
    // B emits ABOVE A's high-water (`b_base = max(operand_base, *high)`), not at the shared
    // `operand_base`. A's transient scratch is dead once A is stored in `$a`, so REUSING it would be
    // sound by liveness — BUT a slot A typed one way (an inlined heap-match materializes its scrutinee
    // as an i32 handle) and B reuses at another width (an i64 arith guard) re-types one wasm local to
    // two types → an invalid module (`expected i64, found i32`). Floating B above A's high-water hands B
    // fresh, never-typed slots — the same disjoint-slot discipline `emit_loop_iteration`/`emit_call_args`
    // apply to sibling arguments (a slot's TYPE is fixed for the whole function, so width-disjoint temps
    // must not alias even when their lifetimes don't overlap).
    let b_base = operand_base.max(*high);
    if sb_src.is_none()
        && !sb_shares_a
        && let OperandSrc::Slot(sb_slot) = sb
    {
        emit_operand_into(
            db, rhs, ot, sb_slot, slots, b_base, high, scratch_ty, layout, out,
        )?;
    }
    // push$a push$b <machine-op> — the result is left on the operand stack.
    sa.push(out);
    sb.push(out);
    out.push(match op {
        Prim::Add => m.add(),
        Prim::Sub => m.sub(),
        Prim::Mul => m.mul(),
        _ => return Err(Reject::decline("not a checked arithmetic op")),
    });
    // GUARD ELISION was already checked at the top of this fn (the `arith_provably_in_range` fast path):
    // when the result provably stays in the type, BOTH the machine overflow guard AND the narrow
    // range-check are dead, and — since no guard then re-reads the operands or the result — that path
    // emits the operands inline with NO scratch slots at all and returns before the slot machinery here.
    // So reaching THIS point means a guard follows and reads `$r` — store the machine result there first.
    out.push(Lir::LocalSet(sr));
    // Step 1: the machine-slot overflow guard (only where the machine op can overflow its slot). This is
    // the DEFINED outcome of the trapping default — an overflowing `+`/`-`/`*` traps rather than yielding
    // an undefined value; the guard is emitted (or provably elided) at EVERY reachable overflow, so no
    // integer op with undefined overflow behavior is ever emitted. This is the general partial-operation
    // discipline for arithmetic: an operation with no in-type result for its inputs (an overflowing add,
    // a `MIN/-1` divide) raises a trap of a defined kind here rather than producing an unspecified value —
    // the total-or-trap alternative to the fallible ops that instead return an `Option` (e.g. `List.at`):
    //= spec/capabilities/core-semantics.md#partial-operations-have-a-defined-outcome
    //# An operation that has no result for some inputs MUST, on those inputs, either evaluate to a value the executable semantics defines or raise a trap of a defined kind.
    //= spec/capabilities/core-semantics.md#partial-operations-have-a-defined-outcome
    //# An operation that has no result for some inputs MUST NOT produce an unspecified value.
    //= spec/capabilities/numeric-model.md#overflow-is-defined
    //# An integer operation that overflows its type MUST have a defined, deterministic outcome fixed by the numeric model, whether that outcome is a value or a trap.
    //= spec/capabilities/numeric-model.md#overflow-is-defined
    //# The compiler MUST NOT emit an integer operation whose overflow behavior is undefined.
    //= constitution.md#iii-the-compiler-introduces-no-undeclared-nondeterminism
    //# The compiler MUST emit each numeric operation with a fully specified result so that the operation does not vary between conforming runtimes.
    emit_machine_overflow_guard(op, m, sa, sb, sr, out);
    // Step 2: the narrow-width range-check on the exact result in `$r`. For a narrow signed `± const`
    // the exact result moves in ONE direction from an in-range operand, so only that bound is reachable
    // — drop the dead check. `(+ a C)` C>0 (or `(- a C)` C<0) moves UP → only `r > max`; the reverse
    // moves DOWN → only `r < min`. (`C==0` is elided in `lower`; a two-const op folds there too.) The
    // general/two-runtime case, and `*`, keep BOTH bounds (a product can leave either side).
    // A const `+`/`-` moves the exact result in ONE direction from an in-range operand, so a narrow
    // range-check needs only that bound (cycle 38). This does NOT hold for `*`: a narrow `(* a C)`
    // product can leave EITHER type bound (positive `a` overflows up, negative `a` down), so a const
    // multiplier keeps BOTH range bounds. Restricted to `Add`/`Sub` explicitly (`const_operand_split`
    // also matches `Mul` now — for the mul-guard fast path below — so the op check is load-bearing).
    let reach = match const_operand_split(op, sa, sb) {
        Some((_, c)) if c != 0 && matches!(op, Prim::Add | Prim::Sub) => {
            let moves_up = (matches!(op, Prim::Add) && c > 0) || (matches!(op, Prim::Sub) && c < 0);
            if moves_up {
                ReachableBounds::UpperOnly
            } else {
                ReachableBounds::LowerOnly
            }
        }
        _ => ReachableBounds::Both,
    };
    emit_range_check(m, sr, reach, out);
    // The result. `Stack` leaves it on the operand stack (`local.get $r`) for the enclosing expression;
    // `Slot(d)` means `$r` IS `d` and the caller wants the value only in the slot, so nothing is pushed
    // (the `local.set $r` above already stored it) — this is where the copy-into-the-operand-slot goes
    // away.
    if matches!(dest, ResultDest::Stack) {
        out.push(Lir::LocalGet(sr));
    }
    Ok(())
}

/// Emit operand `id` (at op width `ot`) so its value ends up in local `slot`. When `id` is itself a
/// nested checked `+`/`-`/`*`, it is emitted with `ResultDest::Slot(slot)` so its own result store
/// writes `slot` directly — no `emit_operand` + separate `local.set`, hence no `local.get $r_inner ;
/// local.tee slot` copy and no extra `$r_inner` scratch. Any other operand (a projection, a call, a
/// conversion, a shift/bitwise, a literal, …) is `emit_operand`ed onto the stack then `local.set slot`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_operand_into(
    db: &mut Db,
    id: StructId,
    ot: IntTy,
    slot: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // A node MATERIALIZED into a slot (CSE / LICM / a match-scrutinee) reads back as a `local.get`, not a
    // recomputation — honor it BEFORE the nested-arith re-emit below (which would rebuild the checked op,
    // defeating the sharing). Read the slot, store into the destination. (The top-level `emit` has the same
    // fast path, but this operand-into-slot path bypasses `emit` for a nested checked op, so it needs its
    // own check.)
    if let Some(&src) = slots.get(&id) {
        out.push(Lir::LocalGet(src));
        out.push(Lir::LocalSet(slot));
        return Ok(());
    }
    if let Core::Arith { op, lhs, rhs } = core_of(db, id)
        && matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
    {
        // WIDTH from the CONSUMING op when this nested arith has NO width anchor of its own. A nested
        // `+`/`-`/`*` whose operands are all deferred-width (bare literals, or `if`/`match` branches of
        // bare literals) types as `Int(Deferred)` — which `int_ty_of` would ground to the i64 DEFAULT,
        // storing an i64 result into the i32 slot the enclosing narrow op declared → INVALID WASM
        // (`(+ (+ (if c 1 2) (if d 3 4)) 5) : Int8`). It also computed the inner op at the WRONG width, so
        // its overflow range-check checked i64 not the narrow type. Emit it at the consuming width `ot`
        // instead: the inner op then computes AND range-checks at the right width, and a bare-literal
        // branch is grounded (and `fits_width`-checked) to `ot`, so an out-of-range branch literal is
        // REJECTED rather than silently truncated. SOUND: a genuine FIXED inner width differing from `ot`
        // is a CDZ0301 fault that aborts before emit, so a deferred-width inner arith reaching here has no
        // anchor and correctly takes its context's width. A fixed inner width (a real Int64 sub-result) is
        // kept as-is.
        let own = int_ty_of(db, id);
        let m = if own.width_is_fixed() {
            Machine::of(own)
        } else {
            Machine::of(ot)
        };
        // STRENGTH REDUCTION reaches the NESTED-operand path too: a `(* v 2^k)` that is an OPERAND of an
        // enclosing op (`(* (* x 2) 4)`) strength-reduces to `v << k` exactly as a top-level `* 2^k` does
        // (the `Core::Arith` emit arm). Without this, a nested constant-pow2 multiply fell straight to
        // `emit_checked_arith_to`, emitting the full `mul` + `div_s` round-trip guard the top-level path
        // avoids. The shift leaves its result on the stack; store it into the operand slot (mirrors the
        // fallback `emit_operand ; LocalSet` below).
        if matches!(op, Prim::Mul)
            && let Some((val, k)) = mul_pow2_shift(db, lhs, rhs, m)
        {
            // Write the shift result DIRECTLY into the operand slot (its own `$r == slot`) — no separate
            // `emit + local.set slot` copy, mirroring the nested checked `+`/`-`/`*` path below.
            emit_mul_pow2_as_shift(
                db,
                m,
                val,
                k,
                slots,
                base,
                high,
                scratch_ty,
                layout,
                out,
                ResultDest::Slot(slot),
            )?;
            return Ok(());
        }
        return emit_checked_arith_to(
            db,
            id,
            op,
            m,
            lhs,
            rhs,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            ResultDest::Slot(slot),
        );
    }
    emit_operand(db, id, ot, slots, base, high, scratch_ty, layout, out)?;
    out.push(Lir::LocalSet(slot));
    Ok(())
}

/// For the constant-operand overflow fast path: return `(runtime_operand, C)` when `(op sa sb)` has a
/// compile-time constant operand and the OTHER is a runtime value that the specialized `r </ₛ> a` guard
/// tests against. For `Add` (commutative) EITHER side may be the constant — the other is `a`. For `Sub`
/// (`a - C`) ONLY the RIGHT operand `sb` may be the constant: a constant LEFT (`C - b`) is not the
/// `a ± C` shape the sign reasoning covers (it would need `-b`'s own overflow analysis), so it declines
/// to the general guard. `None` when neither operand (of the eligible side) is a constant.
pub(super) fn const_operand_split(
    op: Prim,
    sa: OperandSrc,
    sb: OperandSrc,
) -> Option<(OperandSrc, i64)> {
    match op {
        // `+` and `*` are commutative — EITHER operand may be the constant; the other is the runtime `a`.
        Prim::Add | Prim::Mul => {
            if let Some(c) = sb.const_value() {
                Some((sa, c))
            } else {
                sa.const_value().map(|c| (sb, c))
            }
        }
        Prim::Sub => sb.const_value().map(|c| (sa, c)),
        _ => None,
    }
}

/// The machine-slot overflow guard for `(op a b)` with result in `$r` — traps (`if (empty) unreachable
/// end`) when the true result does not fit the MACHINE slot. For a NARROW `+`/`-` the machine add/sub
/// cannot overflow its slot (operands are far from the slot extremes), so the guard is skipped and the
/// range-check alone bounds the result; `*` always runs the `r/a≠b` guard (a narrow product can still
/// exceed the slot — e.g. two 48-bit values multiply past 2^64). See `emit_checked_arith`.
pub(super) fn emit_machine_overflow_guard(
    op: Prim,
    m: Machine,
    sa: OperandSrc,
    sb: OperandSrc,
    sr: u32,
    out: &mut Emit,
) {
    // `+`/`-` overflow the slot only at a FULL width; a narrow add/sub stays within the slot.
    let addsub_can_overflow = !m.narrow();
    // CONSTANT-OPERAND FAST PATH (full-width signed `+`/`-`): when one operand is a compile-time
    // constant `C != 0`, the general two-`xor` sign test collapses to a SINGLE signed compare of the
    // result `r` against the RUNTIME operand `a`. A signed add/sub overflows iff the true result leaves
    // the type, and with a known-sign constant that shows up as `r` landing on the wrong side of `a`:
    //   (+ a C): C>0 overflows only upward → wrap makes `r <ₛ a`;  C<0 only downward → `r >ₛ a`.
    //   (- a C): C>0 subtracts, overflows only downward → `r >ₛ a`; C<0 → `r <ₛ a`.
    // (`C==0` never overflows and is already elided by the `lower` identity fold, so it never reaches
    // here; a two-constant op folds entirely in `lower`. `a` is the OTHER, runtime operand.) Reads `$r`
    // first so the preceding `local.set $r` fuses to `local.tee $r` via the peephole. ~5 fewer ops than
    // the general guard, on the hot path (loop counters `(- n 1)`, accumulators `(+ acc 1)`).
    if addsub_can_overflow
        && m.signed
        && matches!(op, Prim::Add | Prim::Sub)
        && let Some((a_src, c)) = const_operand_split(op, sa, sb)
        && c != 0
    {
        // `r < a` traps for: add with C>0, sub with C<0. `r > a` traps for: add with C<0, sub with C>0.
        let trap_when_r_lt_a =
            (matches!(op, Prim::Add) && c > 0) || (matches!(op, Prim::Sub) && c < 0);
        out.push(Lir::LocalGet(sr));
        a_src.push(out);
        out.push(if trap_when_r_lt_a { m.lt_s() } else { m.gt_s() });
        out.push(Lir::IfIntegerOverflowEnd);
        return;
    }
    // NEGATION FAST PATH (full-width signed `(- 0 a)`): the constant is on the LEFT (`0 - a`), which
    // `const_operand_split` does not cover (a left constant is not the `a ± C` sign shape). But negation
    // has exactly ONE overflow: `-a` leaves the type iff `a == MIN` (since `-MIN` is not representable).
    // So the guard is a single equality `a == MIN → trap` — 4 ops (`get a ; const MIN ; eq ; if`) vs the
    // general two-`xor` sub guard's 8, and it tests the OPERAND `a` directly (no dependence on `$r`).
    // Full-width only: a narrow `(- 0 a)` cannot overflow the SLOT (the machine guard is skipped,
    // `addsub_can_overflow` is false), and its type-bound escape (`0 - MIN_N = -MIN_N > MAX_N`) is caught
    // by the range-check, exactly as for any other narrow sub.
    if addsub_can_overflow && m.signed && matches!(op, Prim::Sub) && sa.const_value() == Some(0) {
        let min = if m.slot32 { i32::MIN as i64 } else { i64::MIN };
        sb.push(out); // the operand `a`
        out.push(m.konst(min));
        out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
        out.push(Lir::IfIntegerOverflowEnd);
        return;
    }
    // IDENTICAL-OPERAND FAST PATH (full-width signed `(+ a a)` — doubling): the general add guard is
    // `((r^a) & (r^b)) < 0`, but with `b == a` (the SAME operand source — CSE fuses `(+ a a)` to one
    // slot) that is `((r^a) & (r^a)) < 0` = `(r^a) < 0`. So one `xor` and one `and` drop: the guard is
    // `get $r ; push a ; xor ; const 0 ; lt_s` (`(r^a)<0`). Sound — `x & x = x` is an identity, verified
    // value-exact vs the general guard at every boundary. Constant operands never reach here (a two-const
    // add folds in `lower`), so equal sources are the same slot/param.
    if addsub_can_overflow && m.signed && matches!(op, Prim::Add) && sa == sb {
        out.push(Lir::LocalGet(sr));
        sa.push(out);
        out.push(m.xor());
        out.push(m.konst(0));
        out.push(m.lt_s());
        out.push(Lir::IfIntegerOverflowEnd);
        return;
    }
    match op {
        Prim::Add if addsub_can_overflow && m.signed => {
            // signed add: `((r^a) & (r^b)) < 0` → trap.
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.xor());
            out.push(Lir::LocalGet(sr));
            sb.push(out);
            out.push(m.xor());
            out.push(m.and());
            out.push(m.konst(0));
            out.push(m.lt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Add if addsub_can_overflow => {
            // unsigned add: `r <ᵤ a` → trap (the sum carried out of the slot).
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.lt_u());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Sub if addsub_can_overflow && m.signed => {
            // signed sub: `((r^a) & (a^b)) < 0` → trap. Mathematically `((a^b) & (a^r)) < 0`, but `^`
            // and `&` are commutative, so we compute `(r^a)` FIRST — reading `$r` immediately after the
            // `local.set $r` that produced the result, so the peephole fuses that `set ; get` into a
            // `local.tee $r` (one fewer instruction). `(r^a)` ≡ `(a^r)`, `(r^a)&(a^b)` ≡ `(a^b)&(a^r)`.
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.xor());
            sa.push(out);
            sb.push(out);
            out.push(m.xor());
            out.push(m.and());
            out.push(m.konst(0));
            out.push(m.lt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Sub if addsub_can_overflow => {
            // unsigned sub: `a <ᵤ b` → trap (an unsigned value cannot go below 0). For a NARROW unsigned
            // width the machine sub CAN go negative in the slot (below 0), which the range-check then
            // catches — but the unsigned-underflow meaning is clearer as this direct test, and it holds
            // at full width where the range-check is a no-op. (A narrow signed/unsigned sub also relies on
            // the range-check for the upper edge, which never trips for sub.)
            sa.push(out);
            sb.push(out);
            out.push(m.lt_u());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        Prim::Mul => {
            // NARROW-PRODUCT-FITS-SLOT FAST PATH: when `2 * width <= slot bits`, the machine multiply in
            // the slot CANNOT overflow the slot — the largest magnitude product of two N-bit values needs
            // at most `2N` bits (`|a*b| < 2^(2N) <= 2^(slot bits)`). So the `div_s`/`div_u` round-trip
            // machine-overflow guard is entirely DEAD; the exact product sits in `$r` and the narrow
            // range-check (emitted after this guard) alone bounds it to `[min_N, max_N]`. Covers Int8/UInt8
            // (16 <= 32) and Int16/UInt16 (32 <= 32) in the i32 slot — a hardware DIVISION removed from
            // every such multiply. Int32×Int32 (64 > 32) and full-width still need the div check below.
            // (This is the mul analogue of the narrow `+`/`-` machine-guard skip: a narrow operand pair is
            // too small to overflow the slot; the range-check catches leaving the TYPE.)
            if m.narrow() && m.width * 2 <= m.slot_bits() {
                return;
            }
            // CONSTANT-MULTIPLIER FAST PATH (full-width signed `(* a C)`, `C` a compile-time constant).
            // The general guard runs a `div_s` (the slowest integer op) on EVERY multiply; but for a
            // known `C` the product `a*C` overflows iff `a` leaves the interval of `a`-values whose
            // product fits — a compile-time-constant interval, tested with TWO compares. `MAX/C` and
            // `MIN/C` truncate toward zero (Rust `/`), which is exactly the interval endpoints
            // (brute-verified at every boundary, both signs of C):
            //   C > 0: `aC` grows with `a` → fits iff `MIN/C <= a <= MAX/C`; trap iff `a > MAX/C || a < MIN/C`.
            //   C < 0: `aC` shrinks with `a` → fits iff `MAX/C <= a <= MIN/C`; trap iff `a < MAX/C || a > MIN/C`.
            // Eligible when `|C| >= 2` (`C ∈ {-1,0,1}` excluded: 0/1 fold in `lower`, and `C == -1` is the
            // negation whose `MIN/-1 = 2^63` bound is NOT i64-representable — `i64::MIN / -1` even panics —
            // so `-1` keeps the `div_s` guard) AND `C` is not a POSITIVE power of two (already
            // strength-reduced to a shift; a NEGATIVE power like `-2`/`-4` is not, so it IS eligible here).
            // Full-width only (the machine slot extremes ARE the type bounds); unsigned and narrow keep the
            // `div_s` round-trip below.
            if !m.narrow()
                && m.signed
                && let Some((a_src, c)) = const_operand_split(Prim::Mul, sa, sb)
                && c.unsigned_abs() >= 2
                && !(c > 0 && (c & (c - 1)) == 0)
            {
                let (slot_min, slot_max) = if m.slot32 {
                    (i32::MIN as i64, i32::MAX as i64)
                } else {
                    (i64::MIN, i64::MAX)
                };
                // The interval endpoints (both trunc-toward-zero); `a*C` fits iff `lo <= a <= hi`.
                // C>0: `aC` grows with `a` → [MIN/C, MAX/C]. C<0: `aC` shrinks → [MAX/C, MIN/C].
                let (lo, hi) = if c > 0 {
                    (slot_min / c, slot_max / c)
                } else {
                    (slot_max / c, slot_min / c)
                };
                // SINGLE unsigned range check (the classic `lo <= a <= hi` ⟺ `(a - lo) <=ᵤ (hi - lo)`
                // fold): shift the interval to start at 0 by subtracting `lo`, then ONE unsigned compare
                // decides both sides — `a < lo` wraps `a - lo` around to a huge unsigned value (> hi-lo),
                // `a > hi` overshoots `hi - lo` directly. So `trap ⟺ (a -ʷ lo) >ᵤ (hi - lo)`. This replaces
                // the two signed compares + two trap blocks (each re-reading `a`) with one subtract, one
                // unsigned compare, and ONE trap block. `hi - lo` fits the slot (the interval width is at
                // most the full slot span, and `c == +2`'s full-span case is excluded as a power of two),
                // and `a - lo` is a wrapping slot subtract (the wasm `i*.sub` is modular), so the unsigned
                // reading is exact. Brute-verified value-equal to the two-compare guard at every boundary,
                // both signs of C. (Reads `a` ONCE — no `local.tee`/CSE needed for the second read.)
                a_src.push(out);
                out.push(m.konst(lo));
                out.push(m.sub());
                out.push(m.konst(hi.wrapping_sub(lo)));
                out.push(m.gt_u());
                out.push(Lir::IfIntegerOverflowEnd);
                return;
            }
            // mul: `if a≠0 { if r/a ≠ b { unreachable } }` — guards div against a=0 (a=0 can't overflow);
            // the machine `div_s` traps on MIN/-1 itself (the sole case `r/a` can't detect at full width),
            // `div_u` is total for a≠0. Runs at every width — a narrow product can exceed the slot too.
            sa.push(out);
            out.push(m.konst(0));
            out.push(m.ne());
            out.push(Lir::If(BlockType::Empty)); // if a != 0 {
            out.push(Lir::LocalGet(sr));
            sa.push(out);
            out.push(m.div());
            sb.push(out);
            out.push(m.ne());
            out.push(Lir::IfIntegerOverflowEnd); //   if (r/a) != b { unreachable }
            out.push(Lir::End); // }
        }
        _ => {}
    }
}

/// Which SIGNED narrow-range bounds a result can actually leave — a range-analysis hint that lets the
/// range-check drop a provably-unreachable side. `Both` is the safe default (a general op can land
/// anywhere). `UpperOnly`/`LowerOnly` are asserted only where the caller has PROVEN the result cannot
/// leave the other side (a narrow signed `± const`: the exact result moves in ONE direction from an
/// in-range operand, so it can exceed only that bound). Ignored for an unsigned width (already one test).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ReachableBounds {
    Both,
    UpperOnly,
    LowerOnly,
}

/// The narrow-width range-check on an exact result in `$r`: trap unless `min_N <= r <= max_N`. A no-op
/// at a FULL width (`N == slot bits`, where the slot extremes ARE the bounds).
///
/// SIGNED width → two SIGNED guards: `r <ₛ min_N → trap` and `r >ₛ max_N → trap` (the bound and value
/// are signed slot values; the result sits sign-extended, so a value outside `[min_N, max_N]` is caught
/// on one side or the other). `reach` may PROVE only one side is possible (a narrow signed `± const`),
/// dropping the dead check — 4 instructions (`local.get`, `const`, compare, `if unreachable`).
///
/// UNSIGNED width → ONE UNSIGNED guard: `r >ᵤ max_N → trap`, i.e. `r >=ᵤ 2^N`. An unsigned narrow
/// result is `0 <= true value < 2^(slot bits)` and sits zero-extended, so the ONLY way it can leave the
/// type is by exceeding `2^N-1` — a single unsigned upper-bound test covers it. This is correct at EVERY
/// width, including one just below the slot size (a `UInt31` sum of `2^32-2` reads as a NEGATIVE signed
/// slot value, which the old signed `r <ₛ 0` guard caught and a signed `r >ₛ max` would MISS — the
/// unsigned compare catches it directly). (`reach` does not apply to unsigned — already one test.)
pub(super) fn emit_range_check(m: Machine, sr: u32, reach: ReachableBounds, out: &mut Emit) {
    if !m.narrow() {
        return;
    }
    let (min_n, max_n) = m.bounds();
    if m.signed {
        // r <ₛ min_N → trap. Skipped when the result provably cannot fall below min (UpperOnly).
        if reach != ReachableBounds::UpperOnly {
            out.push(Lir::LocalGet(sr));
            out.push(m.konst(min_n));
            out.push(m.lt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
        // r >ₛ max_N → trap. Skipped when the result provably cannot exceed max (LowerOnly).
        if reach != ReachableBounds::LowerOnly {
            out.push(Lir::LocalGet(sr));
            out.push(m.konst(max_n));
            out.push(m.gt_s());
            out.push(Lir::IfIntegerOverflowEnd);
        }
    } else {
        // r >=ᵤ 2^N → trap (the single unsigned upper-bound test; `2^N = max_N + 1`).
        out.push(Lir::LocalGet(sr));
        out.push(m.konst(max_n.wrapping_add(1)));
        out.push(m.ge_u());
        out.push(Lir::IfIntegerOverflowEnd);
    }
}

/// Emit a runtime `/`/`%`. The machine `div`/`rem` traps natively on ÷0 (all widths) and, for a FULL
/// signed width, on `MIN/-1` — exactly two of the defined traps. Two extra guards make it correct at any
/// width: a NARROW signed `/` whose `min_N / -1` overflows the type is NOT trapped by the machine op (the
/// quotient `2^(N-1)` fits the wider slot), so it is caught by a range-check on the result; `%` never
/// overflows (its result is bounded by the divisor), so it needs no range-check. Over scratch locals
/// `$a`,`$b`,`$r` when a range-check is required; otherwise a bare `operands; op` suffices.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_div_rem(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // STRENGTH REDUCTION: an UNSIGNED `/`/`%` by a constant POWER OF TWO becomes a shift/mask — far
    // cheaper than the hardware `div_u`/`rem_u`. `(/ a 2^k)` = `a >>ᵤ k`; `(% a 2^k)` = `a & (2^k - 1)`.
    // Only UNSIGNED: a signed `div_s`/`rem_s` rounds toward ZERO, which differs from an arithmetic shift
    // for negatives (`-1 / 2 = 0` but `-1 >>ₛ 1 = -1`), so a signed divide is left as-is. The constant
    // divisor is a nonzero power of two, so the ÷0 trap the machine op carries is provably not needed
    // (and `2^k - 1` for `%` is likewise exact). Applies at every width (the operand is already
    // range-valid; a shift/mask keeps it in range — an unsigned quotient/remainder only shrinks). `k=0`
    // (divisor 1) is excluded: `/1` is identity and `%1` is 0, both folded in `lower` before here.
    if !m.signed
        && let Core::ConstInt(v) = core_of(db, rhs)
        && let Some(d) = v.to_i64()
        && d > 1
        && (d & (d - 1)) == 0
    {
        let k = d.trailing_zeros() as i64;
        emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
        if matches!(op, Prim::Div) {
            out.push(m.konst(k));
            out.push(m.shr()); // unsigned width → `shr_u`
        } else {
            out.push(m.konst(d - 1));
            out.push(m.and());
        }
        return Ok(());
    }
    // STRENGTH REDUCTION: a SIGNED `/`/`%` by a constant POWER OF TWO `2^k` also becomes shifts, but a
    // plain arithmetic shift rounds toward −∞ while `div_s`/`rem_s` truncate toward ZERO — they disagree
    // for negatives. The textbook fix (Hacker's Delight) BIASES the dividend by `2^k − 1` when it is
    // negative, so the truncation matches, and it is BRANCHLESS:
    //
    //   bias = (x >>ₛ (W−1)) >>ᵤ (W−k)      ; W = slot bits. `x >>ₛ (W−1)` = all-ones iff x<0, else 0;
    //                                        ; `>>ᵤ (W−k)` turns that into `2^k − 1` iff x<0, else 0.
    //   q    = (x + bias) >>ₛ k             ; arithmetic shift — now truncates toward zero, = `x / 2^k`.
    //   r    = x − (q << k)                 ; `% 2^k` from the reduced quotient (`= x − q·2^k`).
    //
    // The divisor is a positive power of two, so ÷0 never applies and `MIN/−1` (the only `div_s` overflow)
    // cannot arise — no trap, no range-check even when narrow (`|q| ≤ |x|` stays in the slot; a narrow
    // dividend is already sign-extended, so `>>ₛ (W−1)` reads its true sign). Verified value-exact vs
    // `div_s`/`rem_s` for every `k ∈ 1..=W−2` and all sign/boundary inputs (`k = W−1` would need divisor
    // `2^(W−1)`, unrepresentable as a positive slot constant, so it never reaches here). The value operand
    // is read three times, so it is stashed in a scratch local `$a` once.
    if m.signed
        && let Core::ConstInt(v) = core_of(db, rhs)
        && let Some(d) = v.to_i64()
        && d > 1
        && (d & (d - 1)) == 0
    {
        let k = d.trailing_zeros() as i64;
        // NON-NEGATIVE DIVIDEND fast path: the bias sequence exists ONLY to make an arithmetic shift
        // truncate toward zero for NEGATIVE dividends (`-1 / 2 = 0` but `-1 >>ₛ 1 = -1`). When the dividend
        // is provably `≥ 0` (a mask `(& x 255)`, an unsigned-typed value, or a flow-refined `x` under
        // `(> x 0)`), truncation toward zero equals floor equals a plain shift/mask — the whole bias is
        // DEAD. Emit `x >>ₛ k` (div) / `x & (2^k−1)` (rem), exactly the unsigned case, 1 op instead of 6.
        // Verified: for `x ≥ 0`, `x / 2^k == x >> k` and `x % 2^k == x & (2^k−1)` (toward-zero = floor).
        if crate::lower::value_provably_nonneg(db, lhs) {
            emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
            if matches!(op, Prim::Div) {
                out.push(m.konst(k));
                out.push(m.shr_s_forced()); // x ≥ 0 → arithmetic shift = floor = toward-zero quotient
            } else {
                out.push(m.konst(d - 1));
                out.push(m.and());
            }
            return Ok(());
        }
        let w = m.slot_bits() as i64;
        // The dividend scratch `$a` must be a slot of THIS op's machine width (i64 for Int64, i32 for a
        // narrow int). Reserve it ABOVE the running high-water, NOT at `base`: when this `%`/`/` is emitted
        // as a SUB-EXPRESSION whose enclosing context already typed `base` at a DIFFERENT width — e.g. the
        // bool element `(= (% s 2) 0)` of a compound-`=` tuple, where the synthesized compare-fn allocates
        // `base` as the i32 Bool slot — writing the i64 dividend into `base` re-types one wasm local to two
        // widths → `type mismatch: expected i32, found i64`, an invalid module (the tuple-`=` const-divisor
        // miscompile). A slot at `*high` is guaranteed never pre-typed. Mirrors the `ValueEq`/`SumExpect`
        // "float above `*high`" discipline for exactly this hazard.
        let sa = *high;
        *high = sa + 1;
        scratch_ty.insert(sa, m.slot());
        // `$a = x` (emit the dividend once; later reads are cheap `local.get`s). Its own transient scratch
        // floats above the reserved `sa`.
        emit_operand(db, lhs, ot, slots, *high, high, scratch_ty, layout, out)?;
        out.push(Lir::LocalSet(sa));
        // `q = (x + bias) >>ₛ k`, bias = `(x >>ₛ (W−1)) >>ᵤ (W−k)`.
        let emit_quotient = |out: &mut Emit| {
            out.push(Lir::LocalGet(sa)); // x
            out.push(Lir::LocalGet(sa)); // x  (for the sign replicate)
            out.push(m.konst(w - 1));
            out.push(m.shr_s_forced());
            out.push(m.konst(w - k));
            out.push(m.shr_u_forced());
            out.push(m.add()); // x + bias
            out.push(m.konst(k));
            out.push(m.shr_s_forced()); // >>ₛ k
        };
        if matches!(op, Prim::Div) {
            emit_quotient(out);
        } else {
            // `r = x − (q << k)`.
            out.push(Lir::LocalGet(sa)); // x
            emit_quotient(out);
            out.push(m.konst(k));
            out.push(m.shl()); // q << k
            out.push(m.sub()); // x − q·2^k
        }
        return Ok(());
    }
    // A narrow signed division needs a range-check on the quotient (its `min_N / -1` overflows the type
    // but not the slot). Every other case — `%` (bounded by the divisor), unsigned `/` (magnitude only
    // shrinks), full-width signed `/` (the machine `div_s` traps MIN/-1 itself) — is exact after the
    // native trap, so no scratch is needed. And the range-check is DEAD in two cases, since the sole
    // overflowing quotient is `MIN_N / -1`:
    //   • the DIVISOR provably is NOT `-1` — a constant `≠ -1`, or a range excluding -1 (`(/ x:Int8 3)`,
    //     `(/ x (& y 7))`); or
    //   • the DIVIDEND is provably NON-NEGATIVE — `MIN_N` is negative, so a nonneg dividend can never be
    //     it. For `a ≥ 0` and any `d ≠ 0`, `|a/d| ≤ a ≤ MAX_N`, so the quotient always fits the type
    //     (`(/ (& x 7) d)`, a loop counter, an unsigned-sourced value). (÷0 still native-traps via `div_s`.)
    let needs_range_check = matches!(op, Prim::Div)
        && m.signed
        && m.narrow()
        && crate::lower::divisor_can_be_neg_one(db, rhs)
        && !crate::lower::value_provably_nonneg(db, lhs);
    if !needs_range_check {
        emit_operand(db, lhs, ot, slots, base, high, scratch_ty, layout, out)?;
        emit_operand(db, rhs, ot, slots, base, high, scratch_ty, layout, out)?;
        out.push(if matches!(op, Prim::Div) {
            m.div()
        } else {
            m.rem()
        });
        return Ok(());
    }
    // Narrow signed `/`: compute into `$r`, then range-check. Reserve `$r` ABOVE `*high` (not at `base`):
    // as a compound-`=` element (or any sub-expression whose enclosing context typed `base` differently)
    // a `base`-anchored slot would re-type one wasm local to two widths → invalid module (see the signed
    // pow2 branch above — the tuple-`=` const-divisor hazard).
    let sr = *high;
    *high = sr + 1;
    scratch_ty.insert(sr, m.slot());
    let operand_base = *high;
    emit_operand(
        db,
        lhs,
        ot,
        slots,
        operand_base,
        high,
        scratch_ty,
        layout,
        out,
    )?;
    emit_operand(
        db,
        rhs,
        ot,
        slots,
        operand_base,
        high,
        scratch_ty,
        layout,
        out,
    )?;
    out.push(m.div()); // traps on ÷0 natively; the machine op does not overflow at a narrow width
    out.push(Lir::LocalSet(sr));
    // A narrow signed quotient can overflow the type ONLY upward: the sole out-of-type case is
    // `MIN_N / -1 = 2^(N-1) = MAX_N + 1` (above the max). It can never fall below `min`: `|q| = |a|/|b| <=
    // |a| <= 2^(N-1)`, so the most-negative reachable quotient is `-2^(N-1) = MIN_N` itself (in range,
    // e.g. `MIN_N / 1 = MIN_N`). So the `r < min` half of the range-check is provably dead — only the
    // upper bound is reachable (`UpperOnly`), dropping 4 instructions (get/const/lt_s/if).
    emit_range_check(m, sr, ReachableBounds::UpperOnly, out);
    out.push(Lir::LocalGet(sr));
    Ok(())
}

/// Emit a runtime `<<`/`>>` that GUARDS the shift count and (for `<<`) tests overflow, over scratch
/// locals `$a=base` (value), `$b=base+1` (count), `$r=base+2` (result). A wasm shift MASKS the count mod
/// the slot width and never traps, so a naive lowering would leak C-style undefined-shift behavior. The
/// numeric model forbids this: a count outside `[0, N)` has no defined value (trap), and a left shift is
/// exact multiplication by `2^count`, so it traps on overflow like `*`. The sequence:
///
///   <A> set$a ; <B> set$b
///   ; count guard: `b >=ᵤ N` → trap           (a negative count read unsigned is huge, so ≥ N too)
///   ; get$a get$b <machine-shift> set$r
///   ; <<-only: <M-overflow round-trip> ; <range-check>
///   ; get$r
///
/// The count is guarded against the LANGUAGE width `N` (not the slot width). `>>` never overflows, so it
/// has only the count guard; it is arithmetic (`shr_s`) for a signed type, logical (`shr_u`) for an
/// unsigned one. `<<`'s overflow has two parts: the round-trip `(r >>[s/u] b) != a` catches bits shifted
/// out of the SLOT, and the range-check catches a result that fits the slot but not the narrower N-bit
/// type — together the exact `2^count`-overflow at any width.
#[allow(clippy::too_many_arguments)]
/// For a `Mul` node, if EXACTLY ONE operand is a compile-time constant power of two `2^k` with
/// `1 <= k < width`, return `(value_operand, k)` — the runtime factor and the shift amount that replaces
/// the multiply. `None` otherwise (neither operand a power of two, both constant — folded in `lower` —
/// or `k` out of the useful range: `2^0 = 1` is the `*1` identity `lower` already elides, and `k >=
/// width` can't be represented as a valid shift). The power-of-two test is on the constant's fit-in-i64
/// magnitude: `v > 0 && v & (v-1) == 0`, with `k = v.trailing_zeros()`. Commutative — checks both sides.
pub(super) fn mul_pow2_shift(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    m: Machine,
) -> Option<(StructId, u32)> {
    let pow2_k = |db: &mut Db, id: StructId| -> Option<u32> {
        match core_of(db, id) {
            Core::ConstInt(v) => {
                let n = v.to_i64()?;
                if n > 1 && (n as u64).is_power_of_two() {
                    let k = n.trailing_zeros();
                    // `k` must be a valid shift for this width (a `<< width` would trap as a bad count;
                    // such a multiplier only ever overflows anyway, but keep the shift well-formed).
                    if k < m.width {
                        return Some(k);
                    }
                }
                None
            }
            _ => None,
        }
    };
    // The OTHER operand must be the runtime value (not also a constant — that folds in `lower`).
    if let Some(k) = pow2_k(db, rhs)
        && !matches!(core_of(db, lhs), Core::ConstInt(_))
    {
        return Some((lhs, k));
    }
    if let Some(k) = pow2_k(db, lhs)
        && !matches!(core_of(db, rhs), Core::ConstInt(_))
    {
        return Some((rhs, k));
    }
    None
}

/// Emit `x * 2^k` as `x << k` with a COMPILE-TIME-CONSTANT count `k` — the strength-reduced multiply.
/// Same recipe as `emit_shift`'s `Shl` path (machine shift, then the overflow round-trip `(<r >> k) !=
/// x → trap`, then the narrow range-check) but the count is an inline constant, so there is NO count
/// operand and NO count guard (`k < width` by construction). The value operand `$a` is a reusable source
/// (a local/const pushed at each use) or stashed once in a scratch slot; `$r` holds the shift result.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_mul_pow2_as_shift(
    db: &mut Db,
    m: Machine,
    val: StructId,
    k: u32,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    dest: ResultDest,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // WIDTH-PARTITIONED CLAIM (finding #21 Mul sibling): the shift/round-trip scratch is stored at `m.slot()`;
    // skip any slot `scratch_ty` records at a different width (a live i32 let-binder / handle temp holds it)
    // so the guard never re-types a wasm local. Same discipline as `emit_checked_arith_to`.
    let want = m.slot();
    let mut next_scratch = base;
    let mut claim = |high: &mut u32, scratch_ty: &mut HashMap<u32, ValType>| {
        while matches!(scratch_ty.get(&next_scratch), Some(&w) if w != want) {
            next_scratch += 1;
        }
        let s = next_scratch;
        next_scratch += 1;
        *high = (*high).max(s + 1);
        scratch_ty.insert(s, want);
        s
    };
    // The value operand `$a` is read three times (the shift, the round-trip check's compare); a reusable
    // source (matching local / constant) is pushed at each use, else it is stashed once in scratch.
    let sa_src = operand_src(db, val, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high, scratch_ty);
            OperandSrc::Slot(s)
        }
    };
    // `$r` (result slot): the caller-requested destination when this shift is an OPERAND of an enclosing
    // op (`Slot(d)`) — so this shift's `local.set` IS the store the enclosing op wanted, no `local.get $r ;
    // local.set d` copy and no extra `$r` scratch — else a fresh scratch slot. `d` is one of the enclosing
    // op's operand slots, claimed BELOW this op's `base`, so this op's own operand scratch never collides
    // with it. The round-trip guard re-reads `$r`, which is fine at either a scratch slot or `d`.
    let sr = match dest {
        ResultDest::Slot(d) => d,
        ResultDest::Stack => claim(high, scratch_ty),
    };
    let operand_base = next_scratch;
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            val,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // `$a << k` → `$r` (count is the inline constant `k`, no guard: `k < width`).
    sa.push(out);
    out.push(m.konst(k as i64));
    out.push(m.shl());
    // GUARD ELISION: when interval analysis proves `val << k` (= `val * 2^k`) stays in the type, both the
    // round-trip overflow check AND the narrow range-check are dead — the machine `shl` already produced
    // the exact result. `(* (& x 15) 2)` = `(& x 15) << 1` ∈ [0,30] fits Int64. With NO guard reading `$r`,
    // the `local.set $r` exists only to place the result: for `Stack` leave it on the stack (emit nothing);
    // for `Slot(d)` the single `local.set d` IS the store — mirrors `emit_checked_arith_to`'s elision.
    if crate::lower::shl_provably_in_range(db, val, k) {
        match dest {
            ResultDest::Stack => {}
            ResultDest::Slot(d) => out.push(Lir::LocalSet(d)),
        }
        return Ok(());
    }
    // A guard follows and re-reads `$r` — store the machine result there first.
    out.push(Lir::LocalSet(sr));
    // Overflow round-trip: `($r >> k)` must recover `$a`, else the shift dropped bits out of the slot.
    // The inverse shift matches signedness so the round-trip is exact (arithmetic for signed).
    out.push(Lir::LocalGet(sr));
    out.push(m.konst(k as i64));
    out.push(m.shr());
    sa.push(out);
    out.push(m.ne());
    out.push(Lir::IfIntegerOverflowEnd);
    // Range-check: a narrow `<<` result may fit the slot but exceed the N-bit type.
    emit_range_check(m, sr, ReachableBounds::Both, out);
    // Leave the result where the caller wants it: on the stack (`Stack`) or already in `$r == d` (`Slot`).
    match dest {
        ResultDest::Stack => out.push(Lir::LocalGet(sr)),
        ResultDest::Slot(_) => {}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_shift(
    db: &mut Db,
    op: Prim,
    m: Machine,
    lhs: StructId,
    rhs: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let ot = IntTy::fixed(m.signed, m.width);
    // The count read once here to fold a compile-time-constant count (see the count-guard below).
    let const_count = match core_of(db, rhs) {
        Core::ConstInt(v) => v.to_i64(),
        _ => None,
    };
    // An OUT-OF-RANGE constant count (`k >= N`, or negative — which reads unsigned as `>= N`) makes the
    // shift ALWAYS trap: emit a bare `unreachable` (one instruction) and nothing else — no operand
    // evaluation, no shift. `unreachable` is stack-polymorphic, so it satisfies the function's result
    // type. (A constant OOR count is a defined runtime trap for the shift's count, not a compile-time
    // reject — so it stays a trap, just emitted directly instead of as a dead comparison + `if`.)
    if let Some(k) = const_count
        && (k < 0 || k >= m.width as i64)
    {
        // EVAL-ORDER (#4870 family): operands evaluate LEFT-TO-RIGHT, so a trapping LHS must surface its
        // trap BEFORE the shift's count trap. The bare-`unreachable` below skips operand evaluation, so a
        // trapping lhs (e.g. a `(let ((v (/ 2 0))) (<< v -1))` div-by-zero folded through a call) would be
        // wrongly elided — the count trap would mask lhs's. Guard it: when lhs is NOT trap-free, evaluate
        // lhs first for its side-effect trap; its value is left on the stack, which the stack-polymorphic
        // `unreachable` discards. The count trap itself stays `unreachable` — v-cdz-smith's oracle byte-probe
        // found the oracle AGREES with unreachable for a pure out-of-range shift count, so no specific-reason
        // flip here (that would introduce a divergence) until v-lean-oracle byte-confirms a specific kind.
        if !crate::lower::is_trap_free(db, lhs) {
            emit(db, lhs, slots, base, high, scratch_ty, layout, out)?;
        }
        out.push(Lir::Unreachable);
        return Ok(());
    }
    // The value `$a` and the count `$b` are read several times (count guard, the shift, the round-trip
    // check), so — like `emit_checked_arith` — a reusable operand (a matching local or a constant) is
    // pushed directly at each use (no scratch), and only a nested computation is stashed in a scratch
    // slot. `$r` (the result) always needs its own scratch. Both share the op's machine slot, so a
    // bare-literal value/count is grounded to that width (a mixed i32/i64 shift is invalid wasm).
    // WIDTH-PARTITIONED CLAIM (finding #21 shift sibling): scratch stored at `m.slot()`; skip any slot
    // `scratch_ty` records at a different width (a live i32 handle/let-binder) so the shift never re-types a
    // wasm local. Same discipline as `emit_checked_arith_to`.
    let want = m.slot();
    let mut next_scratch = base;
    let mut claim = |high: &mut u32, scratch_ty: &mut HashMap<u32, ValType>| {
        while matches!(scratch_ty.get(&next_scratch), Some(&w) if w != want) {
            next_scratch += 1;
        }
        let s = next_scratch;
        next_scratch += 1;
        *high = (*high).max(s + 1);
        scratch_ty.insert(s, want);
        s
    };
    let sa_src = operand_src(db, lhs, ot, slots)?;
    let sa = match sa_src {
        Some(src) => src,
        None => {
            let s = claim(high, scratch_ty);
            OperandSrc::Slot(s)
        }
    };
    let sb_src = operand_src(db, rhs, ot, slots)?;
    let sb = match sb_src {
        Some(src) => src,
        None => {
            let s = claim(high, scratch_ty);
            OperandSrc::Slot(s)
        }
    };
    // `$r` (the result scratch) is needed ONLY by `<<`, which reads it back for the overflow round-trip
    // + range-check. `>>` leaves its exact result on the stack, so it claims no `$r` slot (no dead local).
    let sr = if matches!(op, Prim::Shl) {
        claim(high, scratch_ty)
    } else {
        0 // unused for `>>` — the result stays on the stack.
    };
    let operand_base = next_scratch;
    // Stash a non-reusable value/count into its scratch slot (a nested op writes it directly).
    if sa_src.is_none()
        && let OperandSrc::Slot(sa_slot) = sa
    {
        emit_operand_into(
            db,
            lhs,
            ot,
            sa_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    if sb_src.is_none()
        && let OperandSrc::Slot(sb_slot) = sb
    {
        emit_operand_into(
            db,
            rhs,
            ot,
            sb_slot,
            slots,
            operand_base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
    }
    // Count guard: `b >=ᵤ N` → trap. A negative count read unsigned is huge (≥ N), so this one test
    // catches both a negative and a too-large count. Bound is the LANGUAGE width N, not the slot width.
    // ELIDED for a VALID constant count (`0 <= k < N`, established above): the guard's condition is a
    // compile-time `false`, so it is dead (mirrors `lower`'s const-`if` fold). Also elided for a RUNTIME
    // count the value-range lattice proves is already in `[0, N-1]` — the common masked-count idiom
    // `(<< x (& k 63))` / `(>> x (& k 7))`, where `(& k M)` with `M < N` bounds the count to `[0, M]`, so
    // the `>=ᵤ N` test can never fire. `value_range_within(rhs, 0, N-1)` confirms both bounds (the lower
    // bound also rules out a negative count reading huge unsigned). Only a count of genuinely unknown range
    // keeps the runtime test. (An OOR constant count already returned a bare `unreachable` at the top.)
    let count_in_range =
        const_count.is_some() || crate::lower::value_range_within(db, rhs, 0, m.width as i64 - 1);
    if !count_in_range {
        sb.push(out);
        out.push(m.konst(m.width as i64));
        out.push(m.ge_u());
        out.push(Lir::IfUnreachableEnd);
    }
    // push$a push$b <machine-shift>. `>>` (`shr`) is EXACT — its result only shrinks in magnitude, so it
    // needs NO overflow round-trip and NO range-check (a right-shift of an in-range value stays in
    // range). So `>>` leaves the result directly on the stack: no `$r` store, no `$r` local — the `set
    // $r ; get $r` round-trip the old code emitted for BOTH shifts was pure dead motion for `>>`. Only
    // `<<` needs `$r`: it is read back for the overflow round-trip check and the narrow range-check.
    sa.push(out);
    sb.push(out);
    out.push(match op {
        Prim::Shl => m.shl(),
        Prim::Shr => m.shr(),
        _ => return Err(Reject::decline("not a shift op")),
    });
    if matches!(op, Prim::Shl) {
        // GUARD ELISION: a `<<` whose result interval provably stays in the type needs neither the overflow
        // round-trip nor the range-check. For a CONSTANT count `(<< (& x 15) 2)` = [0,60] the fixed shift
        // amount drives `shl_provably_in_range`. For a RUNTIME count whose RANGE is known — the masked-count
        // idiom `(<< (& x 15) (& k 3))`, value [0,15] × count [0,7] → max 1920 — the dynamic variant bounds
        // the result by the count's max shift (`shl_provably_in_range_dynamic`).
        let elide = const_count.is_some_and(|k| {
            (0..m.width as i64).contains(&k)
                && crate::lower::shl_provably_in_range(db, lhs, k as u32)
        }) || (const_count.is_none()
            && crate::lower::shl_provably_in_range_dynamic(db, lhs, rhs));
        if elide {
            // The machine `shl` result is already on the stack (no round-trip needs `$r`) — nothing to do.
        } else {
            out.push(Lir::LocalSet(sr));
            // Round-trip: shifting `$r` back right by `$b` must recover `$a`; else the shift dropped bits
            // out of the SLOT (overflow). The inverse shift matches signedness so the round-trip is exact.
            out.push(Lir::LocalGet(sr));
            sb.push(out);
            out.push(m.shr());
            sa.push(out);
            out.push(m.ne());
            out.push(Lir::IfIntegerOverflowEnd);
            // Range-check: a narrow `<<` result may fit the slot but exceed the N-bit type.
            emit_range_check(m, sr, ReachableBounds::Both, out);
            out.push(Lir::LocalGet(sr));
        }
    }
    // `>>`: the result is already on the stack — nothing more to do.
    Ok(())
}

/// Emit a runtime `wrap` — TRUNCATE the operand (source machine `src`) to the target `dst`'s width and
/// signedness, keeping the low `dst.width` bits and reinterpreting them at the target sign. NEVER traps
/// (the whole point of `wrap`). Three composed steps, all width-generic:
///
///   1. emit the operand (it lands in the SOURCE slot, normalized to the source width);
///   2. MOVE it to the TARGET slot: `i32.wrap_i64` (i64→i32, drops the high half — which the mask would
///      drop anyway) or `i64.extend_i32_{s,u}` (i32→i64, extended by the SOURCE sign so the source value
///      is preserved before masking); same slot → nothing;
///   3. TRUNCATE to `dst.width` in the target slot when it is narrow (`dst.width < slot bits`): `and` the
///      low-`N`-bits mask, then — if the TARGET is signed — sign-extend from bit `N-1` via
///      `(x << (M-N)) >> (M-N)` (arithmetic shr). An unsigned target stops after the mask (zero-filled).
///
/// A full-width target (`dst.width == slot bits`) needs no truncation after the slot move — the slot IS
/// the width. The result is left normalized in the target slot, exactly as every other value.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_wrap(
    db: &mut Db,
    src: Machine,
    dst: Machine,
    operand: StructId,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // 1. The operand, in the source slot.
    emit(db, operand, slots, base, high, scratch_ty, layout, out)?;
    // 2. Move into the target slot (drop/extend the machine width). The extend is by the SOURCE sign so
    //    the source value's bits are preserved into the wider slot before the target mask.
    match (src.slot32, dst.slot32) {
        (false, true) => out.push(Lir::I32WrapI64), // i64 source → i32 target
        (true, false) => out.push(if src.signed {
            Lir::I64ExtendI32S
        } else {
            Lir::I64ExtendI32U
        }),
        _ => {} // same slot width — nothing to move
    }
    // 3. Truncate to the target width within the target slot, when narrower than the slot.
    //
    // REDUNDANT-TRUNCATION ELISION: the truncation is a no-op when the SOURCE value is already a valid,
    // identically-represented target value — i.e. the source width fits the target width AND they share
    // signedness. Then every source value lies in `[min_dst, max_dst]` and its normalized slot bits are
    // already the target's, so the mask (unsigned) or sign-extend (signed) changes nothing. This is the
    // `UInt8.wrap(UInt8)` identity and a same-sign widening like `UInt16.wrap(UInt8)`. A NARROWING
    // (`src.width > dst.width`) or a SIGN CHANGE (`Int8.wrap(UInt8)` — a `200` must become `-56` via
    // sign-extend) genuinely reshapes the value, so it keeps the truncation. (Signedness must match: even
    // at equal width, `Int8.wrap(UInt8)` reinterprets the top bit.)
    let truncation_is_identity = src.width <= dst.width && src.signed == dst.signed;
    // RANGE-BASED elision: even when the SOURCE TYPE is wider (or a different sign), the truncation is a
    // no-op if the operand's VALUE provably already lies in the target's `[min_N, max_N]` — then its low
    // N bits already encode it and the high slot bits are the correct sign extension, so masking/
    // sign-extending changes nothing. `UInt8.wrap(& x 255)` (operand ∈ [0,255], Int64-typed) and a wrap of
    // a flow-refined value shed the mask. Consults the same lattice as the guard-elision checks.
    //
    // `bounds()` is only defined for a NARROW width (`1u64 << 64` overflows), so it is consulted STRICTLY
    // behind the `dst.narrow()` guard — a full-width `wrap` (`UInt64.wrap`, `Int64.wrap`) never masks and
    // never queries the range.
    let operand_fits = dst.narrow() && {
        let (min_n, max_n) = dst.bounds();
        crate::lower::value_range_within(db, operand, min_n, max_n)
    };
    if dst.narrow() && !truncation_is_identity && !operand_fits {
        let slot_bits = dst.slot_bits();
        if dst.signed {
            // Sign-extend from bit N-1: `(x << (M-N)) >> (M-N)` with arithmetic (signed) shr. This both
            // masks (the << pushes the high bits out) and sign-fills. `dst.shr()` is arithmetic for a
            // signed dst.
            let shift = (slot_bits - dst.width) as i64;
            out.push(dst.konst(shift));
            out.push(dst.shl());
            out.push(dst.konst(shift));
            out.push(dst.shr());
        } else {
            // Zero-fill: mask to the low N bits.
            let mask = if dst.width >= 64 {
                -1i64 // all ones (unreachable for narrow, but total)
            } else {
                (1i64 << dst.width) - 1
            };
            out.push(dst.konst(mask));
            out.push(dst.and());
        }
    }
    Ok(())
}

/// For an equality comparison `(= a b)`, if EXACTLY ONE operand is a compile-time constant ZERO, return
/// the OTHER (non-zero) operand — the one to push before an `eqz`. `None` if neither operand is a
/// constant zero (a general equality → `eq`), or if BOTH are (a `0 == 0`, which `lower` already folds to
/// `true`, so it should not reach here — return `None` defensively so it takes the ordinary `eq` path
/// rather than a wrong single-operand `eqz`). The zero test is by VALUE (`IntValue::eq_value` against
/// zero), width-agnostic — a zero of any width is the additive identity the `eqz` recognizes.
/// If `id` is `(% x C)` with `C` a compile-time power of two `> 1`, return `(x, C-1)` — the dividend and
/// the low-bit mask, for the divisibility test `(= (% x 2^k) 0)` ⇔ `(= (x & (2^k−1)) 0)`. Sign-agnostic:
/// `x % 2^k == 0` iff `x`'s low `k` bits are all zero, whichever sign, so this fires for both signed and
/// unsigned `%`. `None` for any other operand (a non-power-of-two divisor, a constant dividend that
/// already folded, or a different op). `C == 1` never reaches here (`%1` folds to `0` in `lower`).
pub(super) fn rem_pow2_mask(db: &mut Db, id: StructId) -> Option<(StructId, i64)> {
    let Core::Arith {
        op: Prim::Rem,
        lhs,
        rhs,
    } = core_of(db, id)
    else {
        return None;
    };
    let Core::ConstInt(v) = core_of(db, rhs) else {
        return None;
    };
    let d = v.to_i64()?;
    (d > 1 && (d & (d - 1)) == 0).then_some((lhs, d - 1))
}

pub(super) fn eq_zero_operand(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<StructId> {
    let is_zero = |db: &mut Db, id: StructId| matches!(core_of(db, id), Core::ConstInt(v) if v.eq_value(&crate::ast::IntValue::zero()));
    let l0 = is_zero(db, lhs);
    let r0 = is_zero(db, rhs);
    match (l0, r0) {
        (true, false) => Some(rhs),
        (false, true) => Some(lhs),
        _ => None, // neither, or both (folded elsewhere) → ordinary `eq`.
    }
}

/// The flat wasm comparison op for a relational prim over an operand integer type — the width chooses
/// i32 (≤32-bit, or a boolean operand) vs i64, and the SIGNEDNESS chooses `_s` (a signed type orders by
/// two's-complement value) vs `_u` (an unsigned type orders by magnitude). Equality is sign-agnostic
/// (the same bits compare equal either way). A ≤32-bit value is properly sign-/zero-extended in its
/// slot, so the i32 `_s`/`_u` ops compare it correctly.
pub(super) fn compare_op(op: Prim, it: IntTy) -> Lir {
    let narrow = it.ground_width() <= 32;
    let signed = it.ground_signed();
    match (op, narrow, signed) {
        (Prim::Eq, false, _) => Lir::I64Eq,
        (Prim::Lt, false, true) => Lir::I64LtS,
        (Prim::Gt, false, true) => Lir::I64GtS,
        (Prim::Le, false, true) => Lir::I64LeS,
        (Prim::Ge, false, true) => Lir::I64GeS,
        (Prim::Lt, false, false) => Lir::I64LtU,
        (Prim::Gt, false, false) => Lir::I64GtU,
        (Prim::Le, false, false) => Lir::I64LeU,
        (Prim::Ge, false, false) => Lir::I64GeU,
        (Prim::Eq, true, _) => Lir::I32Eq,
        (Prim::Lt, true, true) => Lir::I32LtS,
        (Prim::Gt, true, true) => Lir::I32GtS,
        (Prim::Le, true, true) => Lir::I32LeS,
        (Prim::Ge, true, true) => Lir::I32GeS,
        (Prim::Lt, true, false) => Lir::I32LtU,
        (Prim::Gt, true, false) => Lir::I32GtU,
        (Prim::Le, true, false) => Lir::I32LeU,
        (Prim::Ge, true, false) => Lir::I32GeU,
        // Not a comparison — `Core::Compare` only ever carries a comparison prim, so unreachable.
        _ => Lir::I64Eq,
    }
}

/// The machine op for the LOGICAL NEGATION of a comparison — used to fold `(not (CMP a b))` into a single
/// inverted comparison instead of `compare ; i32.eqz`. Every comparison over a TOTAL order has an exact
/// complement: `= ↔ ≠`, `< ↔ ≥`, `> ↔ ≤`. Integer order (signed and unsigned) and `Bool` order (a bool
/// is a total 0/1) are total, and these are the only operand types a `Core::Compare` carries (a compound
/// takes `Core::ValueEq`), so the complement holds for every case `compare_op` handles.
pub(super) fn compare_op_negated(op: Prim, it: IntTy) -> Lir {
    let negated = match op {
        Prim::Eq => {
            return if it.ground_width() <= 32 {
                Lir::I32Ne
            } else {
                Lir::I64Ne
            };
        }
        Prim::Lt => Prim::Ge,
        Prim::Gt => Prim::Le,
        Prim::Le => Prim::Gt,
        Prim::Ge => Prim::Lt,
        // Not a comparison — unreachable, as in `compare_op`.
        _ => return Lir::I64Ne,
    };
    compare_op(negated, it)
}

/// The integer type governing a runtime comparison's operands — read off whichever operand solves to
/// an integer (they unify to one type). A boolean comparison has no integer operand, so it grounds to
/// the ≤32-bit path via the default `i64`… (a bool is compared as an i32 — see `Compare` selection,
/// which reads the operand's own `valtype`). Falls back to signed-64.
pub(super) fn operand_int_ty(db: &mut Db, lhs: StructId, rhs: StructId) -> IntTy {
    // A boolean operand is an i32; represent that as a signed ≤32-bit width so `compare_op` picks i32.
    let bool_as_i32 = IntTy::fixed(true, 32);
    let lt = type_of(db, lhs);
    let rt = type_of(db, rhs);
    // Both operands share ONE machine width. Prefer whichever carries a CONCRETELY-fixed integer width
    // (a narrow-typed variable `n : UInt8` pins the pair to i32) over a still-DEFERRED bare literal (whose
    // width defaults to i64). POSITION-INDEPENDENT: `(< 1 n)` and `(< n 1)` both pick `n`'s width. Reading
    // `lhs` first unconditionally emitted a deferred LEFT literal at its i64 default beside the i32
    // variable → a mismatched operand pair → INVALID WASM ("expected i64, found i32"). This is the emit-
    // side companion of the `unify_width`/`unify_sign` inference fix (which links an ARITH op's operands
    // through its shared result-width var); a COMPARISON's result is `Bool`, so its operand widths are not
    // carried on a result var and must be reconciled HERE from the operands' own types. A grounded literal
    // is then narrowed by `emit_operand` at the shared width, whichever side it is on.
    let concrete =
        |t: &Ty| matches!(t, Ty::Int(it) if matches!(it.width, crate::ty::Width::Fixed(_)));
    match (&lt, &rt) {
        (Ty::Int(it), _) if concrete(&lt) => *it,
        (_, Ty::Int(it)) if concrete(&rt) => *it,
        (Ty::Int(it), _) => *it,
        (_, Ty::Int(it)) => *it,
        (Ty::Bool, _) | (_, Ty::Bool) => bool_as_i32,
        // A CHAR operand is an i32 code-point slot (`valtype_of(Ty::Char) = I32`, Char-rep 1/N), holding a
        // Unicode scalar 0..=0x10FFFF — ALWAYS non-negative, so a Char comparison is a signed-≤32 i32 op
        // (the same width bool/enum-disc use) and the signed compare matches Unicode-scalar (code-point)
        // order. Lets a runtime `= < <= > >=` on a char emit `i32.eq`/`i32.lt_s`/… rather than falling to
        // the i64 default below (which pushed i64 ops beside the i32 char operands → an INVALID module).
        // `is_scalar` now routes a runtime Char here (Char-rep 2/N).
        (Ty::Char, _) | (_, Ty::Char) => bool_as_i32,
        // An ENUM-DISCRIMINANT operand is a bare discriminant i32 (like a bool), so its comparison is an
        // i32 op — the same signed-≤32 width bool uses. Lets `(= c C.Red)` emit `i32.eq` on the raw
        // discriminants rather than a `value-eq` heap walk (which would misread a discriminant as a
        // tagged handle). Reached only for an enum-disc `=` routed here by `lower`.
        _ if ty_is_enum_disc(db, &lt) || ty_is_enum_disc(db, &rt) => bool_as_i32,
        _ => IntTy::i64(),
    }
}

/// The integer type of the node at `id`, if its solved type is an integer — used to ground a literal's
/// width at selection. Defaults to the signed-64 instance when the node is not an integer (a
/// defensive fallback; a `ConstInt` node always types as an integer).
/// Peel a `Ty::Qty` to its inner type — a quantity erases to its inner numeric's machine slot (the unit is
/// a compile-time value), so a width/valtype reader that classifies the inner must see through the `Qty`
/// wrapper. Used by the const-width emit arms (`ConstFloat`/`ConstFloatNan`) alongside `int_ty_of`'s and
/// `is_narrow_int`'s own inline peels. A non-quantity type passes through unchanged.
pub(super) fn peel_qty_ty(ty: Ty) -> Ty {
    // STRIP_NOMINAL → PEEL `Ty::Qty` → STRIP_NOMINAL, mirroring `int_ty_of` EXACTLY (the strip_nominal
    // lockstep the integer side maintains). Two erasures compose: a NOMINAL newtype over a quantity —
    // `(type Len (Q (Qty Float32 u)))` — must reach the inner Float32, and a nominal INSIDE the quantity is
    // stripped too. WITHOUT the outer strip, a `Nominal(Len, Qty{Float32})` missed the `Ty::Qty` arm and
    // fell to the f64 default → an `f64.const` where `box-float32` wanted f32 → INVALID wasm when a
    // nominal-over-Qty-Float32 was boxed as a heap value (v-rust-backend flagged the wasm twin of their
    // rust `float_width_of` strip→peel→strip fix). Returns an owned `Ty` (clones the final stripped inner).
    match ty.strip_nominal() {
        Ty::Qty { inner, .. } => inner.strip_nominal().clone(),
        other => other.clone(),
    }
}

pub(super) fn int_ty_of(db: &mut Db, id: StructId) -> IntTy {
    // `strip_nominal`: an ERASED single-variant newtype over an int — `(type W (Wrap UInt8))` — has the SAME
    // machine int width as its inner int, so a literal `(W.Wrap 5)` must ground to the INNER width (u8 → an
    // i32 slot), NOT the `_ => i64` default. WITHOUT the strip, a `Nominal(W, Int(u8))` literal fell to the
    // i64 default → `ConstI64` — while `is_narrow_int` (which DOES strip_nominal) prepended `i64.extend_i32_u`
    // before `box-int`, so the widen expected an i32 but got the i64 const → an INVALID component (`expected
    // i32, found i64`) when the erased narrow newtype was boxed into a tuple/sum/list element. The two width
    // decisions MUST agree on the same stripped type. (Mirrors the `is_narrow_int` strip_nominal fix.)
    //
    // PEEL `Ty::Qty`: a quantity over a narrow int — `(Qty Int8 u)` — erases to its inner narrow int's
    // machine width (an i32 slot), so a literal `(Qty.of (Int8.of 100) u)` magnitude must ground to the
    // INNER width, NOT the i64 default. WITHOUT the peel, `int_ty_of` returned i64 → `ConstI64` for the
    // magnitude, while `is_narrow_int` (which now peels Qty) prepended the i32→i64 extend / i64→i32 narrow —
    // the SAME i32-vs-i64 disagreement as the newtype case, surfacing when a `(Qty narrow-Int u)` constant
    // was boxed into a heap slot (e.g. a `Map.insert` value later read by `Map.lookup`). The two width
    // decisions MUST agree on the SAME peeled+stripped type. (Mirrors the `is_narrow_int` Qty peel.)
    let solved = type_of(db, id);
    let inner = match solved.strip_nominal() {
        Ty::Qty { inner, .. } => inner.strip_nominal(),
        other => other,
    };
    match inner {
        Ty::Int(it) => *it,
        // A CHAR value occupies an i32 code-point slot (Char-rep 1/N); a runtime char-literal `match`
        // grounds its scrutinee + probe constants to a signed ≤32-bit width so the per-probe compare is
        // `i32.eq` (code points are 0..=0x10FFFF, always non-negative). Without this the `_ => i64` default
        // emitted i64 ops on the i32 char slot → an invalid module (the same class as `operand_int_ty`'s
        // Char fix). Char-rep 3/N.
        Ty::Char => IntTy::fixed(true, 32),
        _ => IntTy::i64(),
    }
}

/// The wasm machine op for a runtime FLOAT arithmetic prim at a given width — the f64/f32 `add`/`sub`/
/// `mul`/`div`. `width` is the operands' solved float width (32 → f32, else f64). IEEE, never trapping.
/// The raw IEEE float-ordering machine op for `Prim::FLt/FLe/FGt/FGe` at the given width. IEEE partialOrd:
/// a NaN operand → 0 (false), `-0.0`/`+0.0` compare equal. (Not for `FEq` — equality uses the canonical-
/// byte bit compare, a different relation.)
//= spec/capabilities/numeric-model.md#a-floating-point-relational-operator-follows-the-ieee-partial-order
//# A floating-point relational operator (`<`, `<=`, `>`, `>=`) MUST follow the IEEE-754 partial order over the operand type, so that a relational operator with a not-a-number operand yields false because a not-a-number value is unordered with respect to every value including itself.
//= spec/capabilities/numeric-model.md#a-floating-point-relational-operator-follows-the-ieee-partial-order
//# A negative zero and a positive zero MUST compare as neither less than nor greater than one another under a floating-point relational operator, so that the two zeroes are ordered as equal even though they are distinct under equality.
pub(super) fn float_ordering_op(op: Prim, width: u32) -> Lir {
    let f32 = width == 32;
    match op {
        Prim::FLt if f32 => Lir::F32Lt,
        Prim::FLt => Lir::F64Lt,
        Prim::FLe if f32 => Lir::F32Le,
        Prim::FLe => Lir::F64Le,
        Prim::FGt if f32 => Lir::F32Gt,
        Prim::FGt => Lir::F64Gt,
        Prim::FGe if f32 => Lir::F32Ge,
        Prim::FGe => Lir::F64Ge,
        // Not a float-ordering prim — `Core::FloatCompare` only carries FEq (handled separately) or these.
        _ => unreachable!("float_ordering_op called with a non-ordering prim"),
    }
}

pub(super) fn float_arith_op(op: Prim, width: u32) -> Lir {
    let f32 = width == 32;
    match op {
        Prim::FAdd => {
            if f32 {
                Lir::F32Add
            } else {
                Lir::F64Add
            }
        }
        Prim::FSub => {
            if f32 {
                Lir::F32Sub
            } else {
                Lir::F64Sub
            }
        }
        Prim::FMul => {
            if f32 {
                Lir::F32Mul
            } else {
                Lir::F64Mul
            }
        }
        Prim::FDiv => {
            if f32 {
                Lir::F32Div
            } else {
                Lir::F64Div
            }
        }
        // A non-float-arith prim never reaches here (guarded by `op.is_float_arith()` at the call site).
        _ => Lir::F64Add,
    }
}
