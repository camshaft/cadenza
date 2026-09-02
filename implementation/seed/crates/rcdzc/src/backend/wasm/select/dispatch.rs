use super::*;

/// The probe chain over a match's arms, dispatching on a scrutinee already resolved to `src` (pushed
/// once per probe — a local read or an inline constant, never a recomputation). See
/// [`emit_match_arms_tailable`], which resolves `src` and (for a computed scrutinee) evaluates it once.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_probe_chain(
    db: &mut Db,
    scrutinee: StructId,
    src: OperandSrc,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    // BR_TABLE DECISION TREE for a DENSE integer match: ≥3 `Int` probes over a small contiguous-ish
    // range dispatch in O(1) via a jump table instead of the linear `if (== k)` cascade below. Only
    // fires for an unguarded integer match with a wildcard default (see `try_emit_scalar_br_table`),
    // and only when the value range is dense enough to not waste table slots. `None` → fall through to
    // the linear chain (a sparse range, guards, too few arms, a non-int probe).
    if let Some(()) = try_emit_scalar_br_table(
        db, src, arms, it, result_it, block_ty, slots, base, high, scratch_ty, layout, out, tail,
    )? {
        return Ok(());
    }
    // BRANCHLESS TERMINAL PAIR: when the chain has narrowed to exactly TWO arms — a literal-probe arm
    // then its unconditional cover (a wildcard, or the redundant last probe of an exhaustive wildcard-less
    // match) — this is `(if (scrutinee == probe0) body0 body1)`, the same shape the standalone 2-arm match
    // selects (see `emit_match_dispatch`). When both are unguarded with cheap trap-free `is_select_arm`
    // bodies and the result is a scalar, emit `body0 ; body1 ; (scrutinee == probe0) ; select` instead of
    // an `if`/`else` block — so the TAIL of an N-arm sparse chain (`(match x (0 a) (5 b) (_ c))` → the
    // inner `(5 b)/(_ c)` pair) is branchless too, not only a standalone 2-arm match. `body1` covers every
    // non-`probe0` value, so the select is total. TAIL position is fine: a trap-free body is never a call,
    // so no arm is a tail call to preserve (matching the standalone case). Falls through to the linear
    // chain for a guarded arm, a heavier/possibly-trapping body, or a non-Int/Bool probe.
    if arms.len() == 2
        && arms.iter().all(|a| a.guard.is_none())
        && matches!(
            arms[0].probe,
            crate::core::Probe::Int(_) | crate::core::Probe::Bool(_)
        )
        && is_select_arm(db, arms[0].body)
        && is_select_arm(db, arms[1].body)
        && !matches!(block_ty, BlockType::Empty)
    {
        // A FLOAT result grounds a bare-`ConstFloat` arm to the block's float width (read off `block_ty`) —
        // else the literal defaults `Float64` and emits `f64.const` under an `f32` select (the all-literal
        // Float32 match's terminal pair). `result_it` is Int-only; mirror the standalone 2-arm select's fix.
        let res_ty = match result_it {
            Some(rit) => Ty::Int(rit),
            None => match block_ty {
                BlockType::Val(ValType::F32) => Ty::Float(crate::ty::FloatTy::fixed(32)),
                BlockType::Val(ValType::F64) => Ty::Float(crate::ty::FloatTy::fixed(64)),
                _ => Ty::Bool,
            },
        };
        emit_branch(
            db,
            arms[0].body,
            &res_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
        emit_branch(
            db,
            arms[1].body,
            &res_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
        )?;
        emit_probe_condition(&arms[0].probe, src, it, out);
        out.push(Lir::Select);
        return Ok(());
    }
    // An arm body is emitted via `emit_arm_body` (grounds a bare-`ConstInt` body to the match's result
    // width, threads the tail context). The chain dispatches per arm below.
    let Some((arm, rest)) = arms.split_first() else {
        // No arm matched and no wildcard — `lower` forbids this for a runtime match, so it is a
        // compiler bug if reached. Decline rather than emit an undefined fallthrough.
        return Err(Reject::decline(
            "match ran off the end with no wildcard arm",
        ));
    };
    // An UNGUARDED arm whose probe always matches — a wildcard, or the LAST arm of an exhaustive
    // wildcard-less match (its probe redundant since every earlier probe failed) — is the unconditional
    // tail: emit its body at THIS nesting, no `if`. A GUARDED arm is NEVER an unconditional tail (its
    // guard may fail), so it always emits a test; `lower`'s exhaustiveness guarantees a later UNGUARDED
    // cover, so the chain still terminates.
    let probe_redundant = matches!(arm.probe, crate::core::Probe::Wild) || rest.is_empty();
    if arm.guard.is_none() && probe_redundant {
        // A literal-probe arm reached as the unconditional tail (the last arm of an exhaustive
        // wildcard-less match) STILL knows `scrutinee == literal` — refine its body so a `(- n 1)` there
        // sheds its guard. (A `Wild` tail arm binds no constant → the frame is unchanged.)
        let frame =
            refined_frame_for_match_arm(db, scrutinee, &arm.probe, db.current_refinements());
        db.push_range_refinements(frame);
        let r = emit_arm_body(
            db, arm.body, result_it, block_ty, slots, base, high, scratch_ty, layout, out, tail,
        );
        db.pop_range_refinements();
        return r;
    }
    // The matched body AND the `else` recursion are both INSIDE this `if` block — so a self-loop `br`
    // from either must jump one MORE level out to reach the loop top (depth + 1).
    let inner = deeper_tail(tail);
    // The arm's TEST: `probe` (scrutinee == literal), AND its `guard` when present. A `Wild` probe has no
    // literal test — the guard alone gates it. To preserve short-circuit trap semantics (the guard is
    // evaluated only when the probe matched — a guard MAY contain a trapping op), a literal-probe-plus-
    // guard nests the guard inside the probe's `if`; the two else-arms both fall through to `rest`.
    let has_literal_probe = !matches!(arm.probe, crate::core::Probe::Wild);
    if has_literal_probe {
        // `if (scrutinee == literal) <guard-gated body> else <rest>`.
        src.push(out);
        match &arm.probe {
            crate::core::Probe::Int(v) => {
                let m = Machine::of(it);
                // PROBE-AGAINST-ZERO → `eqz`. A `0` literal arm (the shape of every recursion base case
                // `(match n (0 …) …)`) is `scrutinee == 0` — exactly `i32.eqz`/`i64.eqz` (one
                // instruction), not a pushed `0` constant + `eq` (two). Same instruction-selection the
                // comparison path applies to `(= n 0)`; mirrored here for the match probe. A nonzero
                // literal keeps the `const ; eq`.
                if v.to_i64_bits() == 0 {
                    out.push(if m.slot32 { Lir::I32Eqz } else { Lir::I64Eqz });
                } else {
                    out.push(m.konst(v.to_i64_bits()));
                    out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
                }
            }
            crate::core::Probe::Bool(b) => {
                out.push(Lir::ConstI32(if *b { 1 } else { 0 }));
                out.push(Lir::I32Eq);
            }
            crate::core::Probe::Str(_) | crate::core::Probe::Bytes(_) => {
                unreachable!(
                    "a string/byte-literal probe folds; a runtime String/Bytes match desugars to a \
                     value-eq if-chain, never reaching the scalar probe emit"
                )
            }
            crate::core::Probe::Char(c) => {
                // A runtime char-literal probe (Char-rep 3/N): compare the i32 code-point scrutinee to THIS
                // literal's code point with `const ; i32.eq` (the nonzero-Int shape; `it` = int_ty_of(Char)
                // = signed-32, so `m.slot32` → i32.eq). `is_scalar` (2/N) now routes a runtime char
                // scrutinee to this probe chain — this is the multi-arm dispatch (distinct from the 2-arm
                // `select` in `emit_probe_condition`, patched too).
                let m = Machine::of(it);
                out.push(m.konst(*c as u32 as i64));
                out.push(if m.slot32 { Lir::I32Eq } else { Lir::I64Eq });
            }
            crate::core::Probe::ListLen { .. } => {
                unreachable!(
                    "a list-length probe folds; a runtime list match declines at build_lit_test"
                )
            }
            crate::core::Probe::MapHasKeys { .. } => {
                unreachable!(
                    "a map-key probe folds; a runtime map match declines at build_lit_test"
                )
            }
            crate::core::Probe::Wild => unreachable!("has_literal_probe"),
        }
        out.push(Lir::If(block_ty));
        emit_arm_guarded_body(
            db, scrutinee, arm, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty,
            layout, out, inner,
        )?;
        // The probe's ELSE (the fall-through probe chain) starts scratch ABOVE the high-water the THEN
        // (a guarded body) reached, NOT at `base`. A guard in the THEN may stash an i32 HEAP HANDLE (a
        // runtime `value-eq`/`MatchSum`) in a low slot, typing it i32 for the whole function; the ELSE's
        // fall-through i64 iteration arithmetic reusing that slot number would force one wasm local to
        // two types (invalid module). The two `if` branches are mutually exclusive at RUN time but share
        // ONE function-global local declaration, so a slot used at two widths across them is illegal. A
        // scalar guard/body leaves `*high` unchanged, so this is byte-identical for the common case. (The
        // `src` scrutinee slot is below `base`-relative scratch and stays live regardless.)
        let else_base = *high;
        out.push(Lir::Else);
        emit_probe_chain(
            db, scrutinee, src, rest, it, result_it, block_ty, slots, else_base, high, scratch_ty,
            layout, out, inner,
        )?;
        out.push(Lir::End);
        Ok(())
    } else {
        // A `Wild` probe with a guard: the guard alone gates the arm — `if guard body else rest`. There
        // is NO probe `if` here (a wildcard needs no literal test), so pass `tail`, NOT `inner`: the ONLY
        // block the guard's body/fall-through nest inside is the guard's own `if` (which
        // `emit_arm_guarded_body` accounts for with its own `deeper_tail`). Passing the probe-adjusted
        // `inner` here DOUBLE-COUNTED the nesting — a self-tail-call in the fall-through `br`'d one level
        // too far, PAST the loop, producing invalid wasm (`expected i64 but nothing on stack`). `inner`
        // is correct ONLY for the literal-probe path above, where a real probe `if` IS pushed.
        emit_arm_guarded_body(
            db, scrutinee, arm, src, rest, it, result_it, block_ty, slots, base, high, scratch_ty,
            layout, out, tail,
        )
    }
}

/// Try to emit a DENSE integer `match` as a BR_TABLE decision tree (O(1) jump) instead of the linear
/// `if (== k)` cascade. Returns `Ok(Some(()))` when it emitted the table, `Ok(None)` to fall back.
///
/// Eligible when: the match is NOT in tail position (a tail match keeps the linear chain, which threads
/// the self-loop context — a br_table here would bypass the match-based tail-loop and break O(1) stack);
/// the arms are ≥3 UNGUARDED `Int` probes followed by ONE trailing UNGUARDED wildcard default (a scalar
/// int match is always wildcard-terminated — int is unbounded, so exhaustiveness requires it); every
/// literal fits an i64; and the value RANGE is DENSE — `span = max - min + 1` satisfies `span <= 2*count`
/// and `span <= 256` (so the jump table is not mostly default padding). Otherwise fall back to the chain.
///
/// The index is `scrutinee - min` (a 0-based table position). Values outside `[min, max]`, and gaps in
/// the range with no arm, route to the default via `br_table`'s own unsigned out-of-range check — EXCEPT
/// an i64 scrutinee, where the required `i32.wrap_i64` of the shifted index could alias a value
/// `>= min + 2^32` into `[0, span)`; for that case a `br_if` bounds guard (`(idx as u64) >= span →
/// default`) precedes the table. A ≤32-bit scrutinee needs no guard (its slot IS i32; the subtraction is
/// exact mod 2^32 and br_table's bounds check is correct). The block structure mirrors
/// `try_emit_disc_br_table` (one typed `$join`, empty label blocks, each arm `br`s its value to `$join`).
#[allow(clippy::too_many_arguments)]
pub(super) fn try_emit_scalar_br_table(
    db: &mut Db,
    src: OperandSrc,
    arms: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<Option<()>, Reject> {
    // A SELF-LOOP tail match must keep the linear chain (it threads the loop context so a self-tail-call
    // in an arm iterates the loop — a br_table's value-join structure can't carry a loop `br` out of an
    // arm). A NON-self-loop match — value position (`NonTail`) OR a plain tail position with no loop
    // (`Tail(None)`, e.g. an exported non-recursive body) — is eligible: its arm bodies are ordinary
    // values `br`'d to the join block, and the join's value is the function's result. Disqualify only
    // `Tail(Some(_))` with real loop members (an empty-members `TailLoop` — the #7942
    // `returncall_shell_drop` carrier — never loop-iterates, so it stays eligible; see `is_self_loop_tail`).
    if is_self_loop_tail(tail) {
        return Ok(None);
    }
    // Split off a trailing unguarded wildcard default; the rest must be unguarded `Int` probes.
    let (default, int_arms): (&crate::core::MatchArm, &[crate::core::MatchArm]) = match arms.last()
    {
        Some(a)
            if matches!(a.probe, crate::core::Probe::Wild)
                && a.guard.is_none()
                && arms.len() >= 4 =>
        {
            (a, &arms[..arms.len() - 1])
        }
        _ => return Ok(None),
    };
    // O(1) SIZE GATE before the O(arms) literal walk below. Eligibility requires a DENSE range
    // (`span <= 256`, checked below) and the literals are DISTINCT (a duplicate falls back), so
    // `count <= span <= 256`: a table can NEVER fire with more than 256 int-arms. Reject those here in
    // O(1) instead of building an O(arms) `lits` vector that the density check would discard. This is
    // what keeps a LARGE sparse/dense match O(arms) overall: `emit_probe_chain` re-attempts this table
    // on every recursive `rest`, so without the gate a 6400-arm match rebuilt a shrinking O(arms) vector
    // at each of ~6400 levels — O(arms²). (A dense SUFFIX of <=256 arms still becomes eligible and emits
    // its table exactly as before — byte-identical; only the always-doomed long-prefix attempts are cut.)
    if int_arms.len() > 256 {
        return Ok(None);
    }
    let mut lits: Vec<i64> = Vec::with_capacity(int_arms.len());
    for a in int_arms {
        match &a.probe {
            crate::core::Probe::Int(v) if a.guard.is_none() => match v.to_i64() {
                Some(x) => lits.push(x),
                None => return Ok(None), // a value that doesn't fit i64 — fall back.
            },
            _ => return Ok(None), // a guard, a bool probe, or a wildcard mid-list — fall back.
        }
    }
    // Density: ≥3 arms, contiguous-enough range, capped table size.
    let min = *lits.iter().min().unwrap();
    let max = *lits.iter().max().unwrap();
    let span: i128 = max as i128 - min as i128 + 1;
    let count = lits.len() as i128;
    if count < 3 || span > 2 * count || span > 256 {
        return Ok(None);
    }
    let span = span as u32;
    // The table: index `i` (a shifted value `min + i`) → the arm whose literal is `min + i`, or the
    // default. `arm_at[i] = Some(arm_index)` maps a covered slot to its position in `int_arms`.
    let mut arm_at: Vec<Option<usize>> = vec![None; span as usize];
    for (ai, &lit) in lits.iter().enumerate() {
        let slot = (lit - min) as usize;
        if arm_at[slot].is_some() {
            return Ok(None); // duplicate literal — fall back (the chain handles it, first-wins).
        }
        arm_at[slot] = Some(ai);
    }
    let m = Machine::of(it);

    // Open the ONE typed join block, the default label block, then one empty block per COVERED arm
    // (arm 0 innermost). The br_table's targets index into these by SHIFTED VALUE, remapped to the
    // covering arm's block depth (a gap slot → the default depth).
    out.push(Lir::Block(block_ty)); // $join (typed)
    out.push(Lir::Block(BlockType::Empty)); // $default
    let n_arms = int_arms.len() as u32;
    for _ in 0..n_arms {
        out.push(Lir::Block(BlockType::Empty)); // $a_{n-1} … $a_0 (innermost = arm 0)
    }
    // Compute the shifted index `scrutinee - min` in the scrutinee's slot width.
    // At the innermost point the enclosing blocks (inner→outer) are: a_0 … a_{n-1}, default, join.
    // `br d`: d in 0..n → $a_d ; d = n → $default ; d = n+1 → $join.
    let default_depth = n_arms; // exits $default
    src.push(out);
    // The shifted index is `scrutinee - min`; when the covered range STARTS AT 0 (the common `(match x
    // (0 …) (1 …) …)` shape) the shift is the identity `x - 0`, so skip the dead `const 0 ; sub` — the
    // scrutinee IS the table index. (`m.sub()` wraps, so `x - 0 == x` exactly at both slot widths.)
    if min != 0 {
        out.push(m.konst(min));
        out.push(m.sub());
    }
    // The arm bodies' scratch floor. On the i64 path it rises PAST the reserved i64 idx slot so an arm
    // body's transient scratch never reuses that slot at a DIFFERENT width — a wasm local's type is fixed
    // function-wide, so an i64 dispatch-index slot and an i32 arm temp (e.g. a `String.to-bytes` Bytes
    // handle inlined into every arm) must occupy DISJOINT slots even though the idx is dead before any arm
    // body runs. Reusing `base` for both declared one local at two widths → "expected i32, found i64"
    // (the width-disjoint-slot family; cf. the heap-match/checked-arith fix). A ≤32-bit scrutinee needs no
    // idx slot (its shifted value stays on the stack), so arm bodies keep `base`.
    let mut arm_base = base;
    if !m.slot32 {
        // i64 scrutinee: guard against the wrap-aliasing (idx as u64 >= span → default), then narrow.
        let idx_slot = base;
        arm_base = base + 1;
        *high = (*high).max(arm_base);
        scratch_ty.insert(idx_slot, ValType::I64);
        out.push(Lir::LocalTee(idx_slot)); // keep idx, leave a copy on the stack
        out.push(Lir::ConstI64(span as i64));
        out.push(Lir::I64GeU);
        out.push(Lir::BrIf(default_depth)); // out of range → default (br_if pops the bool)
        out.push(Lir::LocalGet(idx_slot));
        out.push(Lir::I32WrapI64);
    }
    // Targets: one entry per SHIFTED VALUE `0..span`, each the block depth of the covering arm, or the
    // default depth for a gap. Arm `ai` (position in `int_arms`) sits at block depth `ai` (a_0 innermost).
    let targets: Vec<u32> = (0..span as usize)
        .map(|i| match arm_at[i] {
            Some(ai) => ai as u32,
            None => default_depth,
        })
        .collect();
    out.push(Lir::BrTable(targets, default_depth));

    // Emit each covered arm's body after its label's `end`, innermost (arm 0) first, then `br` its value
    // to $join. After `End`ing $a_0..$a_k, the enclosing blocks (inner→outer) are a_{k+1}…a_{n-1} (that is
    // `n_arms - 1 - k` blocks), then $default (1 block), then $join — so $join sits at DEPTH
    // `(n_arms - 1 - k) + 1 = n_arms - k` (the count of blocks BEFORE it; $join is AT that depth, not one
    // past). This mirrors `try_emit_disc_br_table`'s `(m - 1 - k) + join_from_arm_extra` with the always-
    // present $default block (extra = 1). A bare `n_arms - k + 1` branched ONE BLOCK TOO FAR — past $join
    // to the FUNCTION-result label, so in NON-tail position the arm value escaped the whole function and the
    // consuming code (`+ 100`, a `bytes-concat`, a `let` body) never ran (a silent wrong value; the default
    // arm, which falls through to $join with no `br`, was unaffected — masking the bug in tail position
    // where the function result IS $join).
    for (k, arm) in int_arms.iter().enumerate() {
        out.push(Lir::End); // close $a_k → br_table target `k` lands here
        emit_arm_body(
            db,
            arm.body,
            result_it,
            block_ty,
            slots,
            arm_base,
            high,
            scratch_ty,
            layout,
            out,
            TailPos::NonTail,
        )?;
        out.push(Lir::Br(n_arms - k as u32)); // → $join, carrying the value
    }
    // Close $default; emit the default body (falls through to $join's end — no `br` needed).
    out.push(Lir::End); // close $default
    emit_arm_body(
        db,
        default.body,
        result_it,
        block_ty,
        slots,
        arm_base,
        high,
        scratch_ty,
        layout,
        out,
        TailPos::NonTail,
    )?;
    out.push(Lir::End); // close $join
    Ok(Some(()))
}

/// A [`TailPos`] one `if` block deeper — a self-loop `br` from inside a fresh `if` targets one level
/// further out. Shared by the probe chain and the guarded-body emit (each opens an `if`).
pub(super) fn deeper_tail(tail: TailPos) -> TailPos {
    match tail {
        TailPos::Tail(tl) => TailPos::Tail(tl.map(|t| TailLoop {
            depth: t.depth + 1,
            ..t
        })),
        TailPos::NonTail => TailPos::NonTail,
    }
}

/// Whether `tail` is a REAL self-loop tail — a `Tail(Some(tl))` with actual loop MEMBERS that a
/// self-tail-call `br`s back to. The self-loop-depth-sensitive dispatch optimizations below
/// (unguarded-rest fall-through, disc `br_table`, branchless `select`, multi-column flat `br_if` chain)
/// SKIP when this is true, because their block nesting cannot carry a loop `br` (they'd need
/// flat-specific depth math). A `NonTail`, a `Tail(None)`, OR a `Tail(Some(tl))` whose `tl.members` is
/// EMPTY are all safe: an empty-members `TailLoop` never loop-iterates (`member_which` matches nothing),
/// so no self-loop `br` occurs and the depth stays static. Such an empty carrier is the #7942 cross-fn
/// `returncall_shell_drop` slot-holder for a NON-looped fn — a bare `matches!(tail, Tail(Some(_)))` check
/// wrongly treated it as a self-loop and disabled these opts (the multi-column flatten regressed to a
/// 510-`if` exponential tree). `TailPos`/`TailLoop` are `Copy`, so this takes `tail` by value.
fn is_self_loop_tail(tail: TailPos) -> bool {
    matches!(tail, TailPos::Tail(Some(tl)) if !tl.members.is_empty())
}

/// Emit a match-arm BODY at [`TailPos`] `tp`. Every arm produces the match's RESULT type, so a bare
/// `ConstInt` body is grounded to the result's integer width (`result_it`) — else a default-Int64 literal
/// arm beside a narrow arm pushes a mismatched slot and wasm rejects the block. A tail body goes through
/// `emit_tail` (a `ConstInt` is never a tail call); `tp` carries the self-loop context.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_arm_body(
    db: &mut Db,
    body: StructId,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tp: TailPos,
) -> Result<(), Reject> {
    if let (Some(rit), Core::ConstInt(_)) = (result_it, core_of(db, body)) {
        return emit_operand(db, body, rit, slots, base, high, scratch_ty, layout, out);
    }
    // A bare `ConstFloat` arm body takes the match's RESULT float width, not its own default `Float64`:
    // `(: (match n (0 0.5) (_ (f (- n 1)))) Float32)` has the annotation on the `match`, so a literal arm
    // solves to `Float64` (inference leaves a bare float literal at its default), and `Core::ConstFloat`'s
    // emit — which reads the node's own solved width — pushes an `f64.const` while the arm's block type is
    // `f32` → an INVALID module (`expected f32, found f64`). Unlike a narrow INT (masked into the shared i32
    // slot), `f32`/`f64` are DISTINCT machine types, a hard validation error. Ground it to the result f32
    // here — the match-arm twin of the if-branch grounding (`emit_tail_branch`/`emit_branch`). A simple
    // all-literal match slipped past because it const-folds to one `f32.const`; a match with a runtime arm
    // (a self-call, an arith spine) does not fold, so the bare literal reaches emit ungrounded.
    if let (BlockType::Val(ValType::F32), Core::ConstFloat(d)) = (block_ty, core_of(db, body)) {
        out.push(Lir::F32ConstBits(
            (f64::from_bits(d.to_f64_bits()) as f32).to_bits(),
        ));
        return Ok(());
    }
    match tp {
        TailPos::Tail(tl) => emit_tail(db, body, slots, base, high, scratch_ty, layout, out, tl),
        TailPos::NonTail => emit(db, body, slots, base, high, scratch_ty, layout, out),
    }
}

/// Emit a runtime LIST match's arms as a length-dispatch `if`-chain, each ARM BODY at [`TailPos`] `tail`.
/// The list handle is already materialized (its slot is in `arm_slots` under the scrutinee) and `len_slot`
/// holds `vec-len`. Each non-final arm tests its length condition and, on match, emits its body; the final
/// (or `Any`) arm is the unconditional `else`. In TAIL position each arm body is emitted via `emit_arm_body`
/// (so a tail self-call in an arm becomes a `return_call` / loop iteration) — and since each preceding
/// non-final arm nests the remaining arms one `if` DEEPER, the threaded `TailLoop.depth` bumps +1 per level
/// (via `deeper_tail`) so a self-loop `br` targets the loop top correctly. `result_it` grounds a
/// bare-`ConstInt` arm body to the match's integer result width (as `emit_arm_body` does for scalar arms).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_list_arms_tailable(
    db: &mut Db,
    arms: &[crate::core::ListArm],
    len_slot: u32,
    block_ty: BlockType,
    result_it: Option<IntTy>,
    arm_slots: &HashMap<StructId, u32>,
    arm_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    let Some((first, rest)) = arms.split_first() else {
        out.push(Lir::Unreachable);
        return Ok(());
    };
    // An UNGUARDED `Any` (or the final) arm is the unconditional tail. A GUARDED arm — even an `Any`/rest
    // one — may FAIL its guard, so it is NOT unconditional: it still tests its guard and falls through.
    let is_tail_arm = first.guard.is_none()
        && (rest.is_empty() || matches!(first.cond, crate::core::ListArmCond::Any));
    if is_tail_arm {
        // The unconditional final arm — its body is in the SAME tail position as the whole match.
        return emit_arm_body(
            db, first.body, result_it, block_ty, arm_slots, arm_base, high, scratch_ty, layout,
            out, tail,
        );
    }
    // BRANCHLESS 2-ARM LIST SELECT: a list match of exactly TWO arms — a LENGTH-test arm then a single
    // unconditional cover (an `Any`/final rest arm) — is `(if (len ⋈ k) body0 body1)`, the list analogue of
    // the scalar/sum 2-arm select. When both are UNGUARDED with cheap trap-free `is_select_arm` bodies and
    // the result is a scalar, emit `body0 ; body1 ; (len ⋈ k) ; select` instead of an `if`/`else` block —
    // so `(match xs ((list) 0) ((list a .. r) 1))` (dispatch on `len == 0`) goes branchless. Only for
    // NON-self-loop position (`select` cannot carry a loop `br`; a trap-free body is never a tail call).
    // A body that reads an ELEMENT/REST binder does so via `SumPayload` — NOT trap-free — so `is_select_arm`
    // declines and the structured `if` survives (no speculative out-of-bounds element read on the wrong
    // arm), exactly as a payload-reading sum arm keeps its `if`.
    // `rest` is a single UNGUARDED arm: it is the last arm of an exhaustive match, so it is the
    // UNCONDITIONAL cover — the fall-through emits its body with NO cond re-test (the `is_tail_arm` rule),
    // so its own `cond` (whether `Any` or a now-redundant length like `LenGe(1)` complementing the first
    // arm's `LenEq(0)`) is irrelevant. Any single unguarded `rest` arm qualifies.
    if !is_self_loop_tail(tail)
        && matches!(block_ty, BlockType::Val(_))
        && first.guard.is_none()
        && !matches!(first.cond, crate::core::ListArmCond::Any)
        && let [cover] = rest
        && cover.guard.is_none()
        && is_select_arm(db, first.body)
        && is_select_arm(db, cover.body)
    {
        let res_ty = match result_it {
            Some(rit) => Ty::Int(rit),
            None => type_of(db, first.body),
        };
        emit_branch(
            db, first.body, &res_ty, arm_slots, arm_base, high, scratch_ty, layout, out,
        )?;
        emit_branch(
            db, cover.body, &res_ty, arm_slots, arm_base, high, scratch_ty, layout, out,
        )?;
        out.push(Lir::LocalGet(len_slot));
        match first.cond {
            crate::core::ListArmCond::LenEq(n) => {
                out.push(Lir::ConstI32(n as i32));
                out.push(Lir::I32Eq);
            }
            crate::core::ListArmCond::LenGe(k) => {
                out.push(Lir::ConstI32(k as i32));
                out.push(Lir::I32GeU);
            }
            crate::core::ListArmCond::Any => unreachable!("guarded by the matches! above"),
        }
        out.push(Lir::Select);
        return Ok(());
    }
    // Open the LENGTH test — except for an `Any` cond (a guarded catch-all/rest), whose length always holds
    // so its only gate is the guard. For a length-carrying cond, `if (len ⋈ k)` wraps the arm.
    let has_len_test = !matches!(first.cond, crate::core::ListArmCond::Any);
    if has_len_test {
        out.push(Lir::LocalGet(len_slot));
        match first.cond {
            crate::core::ListArmCond::LenEq(n) => {
                out.push(Lir::ConstI32(n as i32));
                out.push(Lir::I32Eq);
            }
            crate::core::ListArmCond::LenGe(k) => {
                out.push(Lir::ConstI32(k as i32));
                out.push(Lir::I32GeU);
            }
            crate::core::ListArmCond::Any => unreachable!(),
        }
        out.push(Lir::If(block_ty));
    }
    // Inside the length `if` (or unconditionally, for an `Any` guarded arm): emit the arm's body, gated on
    // its GUARD when present. A guarded arm becomes `if guard then body else <rest>` — a false guard FALLS
    // THROUGH to the remaining arms, exactly as a false length test does; the guard is a boolean the arm's
    // element/rest binders are in scope for (resolve Case 6lg), emitted as an operand before the `if`. The
    // body/rest sit one `if` deeper per opened `if`, so the tail depth bumps accordingly.
    let after_len_tail = if has_len_test {
        deeper_tail(tail)
    } else {
        tail
    };
    match first.guard {
        None => {
            emit_arm_body(
                db,
                first.body,
                result_it,
                block_ty,
                arm_slots,
                arm_base,
                high,
                scratch_ty,
                layout,
                out,
                after_len_tail,
            )?;
        }
        Some(g) => {
            // The guard reads the scrutinee handle (in `arm_slots`) via its binders' `SumPayload`; emit it
            // as an i32 boolean at `arm_base`. The body/rest start scratch ABOVE the guard's high-water (a
            // guard stashing a heap handle types a low slot i32; a body reusing that slot at i64 would fail
            // validation — the same discipline the scalar guard emit follows).
            emit(db, g, arm_slots, arm_base, high, scratch_ty, layout, out)?;
            let body_base = *high;
            out.push(Lir::If(block_ty));
            let deeper = deeper_tail(after_len_tail);
            emit_arm_body(
                db, first.body, result_it, block_ty, arm_slots, body_base, high, scratch_ty,
                layout, out, deeper,
            )?;
            out.push(Lir::Else);
            emit_list_arms_tailable(
                db, rest, len_slot, block_ty, result_it, arm_slots, body_base, high, scratch_ty,
                layout, out, deeper,
            )?;
            out.push(Lir::End);
        }
    }
    if has_len_test {
        out.push(Lir::Else);
        // The remaining arms are ALSO one `if` deeper — pass the bumped tail.
        emit_list_arms_tailable(
            db,
            rest,
            len_slot,
            block_ty,
            result_it,
            arm_slots,
            arm_base,
            high,
            scratch_ty,
            layout,
            out,
            deeper_tail(tail),
        )?;
        out.push(Lir::End);
    }
    Ok(())
}

/// Emit a GUARDED arm's body gated on its guard (the caller has already established that the arm's
/// PROBE matched — for a literal probe, inside its `if`; for a `Wild` probe, unconditionally). Emits
/// `if guard body else <rest>` — a false guard falls through to the remaining arms, exactly as a
/// non-matching pattern does (`core-semantics.md` §Matching Is Exhaustive Or Rejected). An UNGUARDED arm
/// (reached only via a literal probe whose guard is `None`) emits its body directly. The guard is a
/// boolean value (an i32); it is emitted at `base` (a fresh scratch region, its result consumed by the
/// `if`).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_arm_guarded_body(
    db: &mut Db,
    scrutinee: StructId,
    arm: &crate::core::MatchArm,
    src: OperandSrc,
    rest: &[crate::core::MatchArm],
    it: IntTy,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    inner: TailPos,
) -> Result<(), Reject> {
    // This arm's PROBE matched to reach here — for a literal `Int` probe over a variable scrutinee, the
    // scrutinee EQUALS that literal, so refine its range to `[c, c]` for the BODY (a `(- n 1)` in the
    // `(5 …)` arm computes `4`, its guard dead). The GUARD is a boolean the arm gates on and is NOT
    // refined (a guard like `(> n 5)` reading the same variable must still be evaluated); only the body,
    // reached once the probe (and guard) held, sees the refinement. `Wild`/`Bool` probe → no refinement.
    let body_frame =
        refined_frame_for_match_arm(db, scrutinee, &arm.probe, db.current_refinements());
    match arm.guard {
        None => {
            db.push_range_refinements(body_frame);
            let r = emit_arm_body(
                db, arm.body, result_it, block_ty, slots, base, high, scratch_ty, layout, out,
                inner,
            );
            db.pop_range_refinements();
            r
        }
        Some(g) => {
            // `if guard body else <rest>`. The guard is a plain boolean value (never a tail call), so it
            // is emitted with `emit` at `base`; its result is the `if` condition.
            emit(db, g, slots, base, high, scratch_ty, layout, out)?;
            // The body and fallthrough start scratch ABOVE the high-water the GUARD reached, NOT at
            // `base` — the same discipline as the `Core::If` arms. A guard that stashes an i32 HEAP
            // HANDLE (a runtime `value-eq`/`MatchSum` on constructed sums, `(guard x (= (mk x) (mk 3)))`)
            // types a low slot i32 for the whole function; the fallthrough's loop-iteration i64 arith
            // (`(find (+ n 1))`) reusing that slot number at a different width fails validation. A scalar
            // guard leaves `*high == base`, so this is byte-identical for the common case.
            let body_base = *high;
            out.push(Lir::If(block_ty));
            // Both the body and the fallthrough are one `if` deeper than this arm's nesting.
            let deeper = deeper_tail(inner);
            // The BODY (probe matched AND guard held) sees the `[c,c]` refinement; the fall-through `rest`
            // does NOT (the probe failed there — its own arms refine themselves).
            db.push_range_refinements(body_frame);
            let body_res = emit_arm_body(
                db, arm.body, result_it, block_ty, slots, body_base, high, scratch_ty, layout, out,
                deeper,
            );
            db.pop_range_refinements();
            body_res?;
            out.push(Lir::Else);
            emit_probe_chain(
                db, scrutinee, src, rest, it, result_it, block_ty, slots, body_base, high,
                scratch_ty, layout, out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
    }
}

/// Try to emit a sum-discriminant switch as a BR_TABLE decision tree (O(1) jump) instead of the linear
/// `if (disc == k)` chain. Returns `Ok(Some(()))` when it emitted the table, `Ok(None)` to fall back to
/// the linear chain. Eligible when the arms are a set of ≥3 DISTINCT explicit discriminants (each
/// `disc: Some`), optionally followed by ONE trailing default (`disc: None`); a leading/mid default, or
/// fewer than 3 discs, falls back (the linear chain is fine and simpler there).
///
/// The value-producing structure — for discriminants `d_0..d_{m-1}` (each with a continuation) and a
/// default continuation, all yielding `block_ty`:
/// ```text
///   block $join (block_ty)          ; the ONE typed block; every arm br's its value here
///     block $default                ; empty control-flow labels …
///       block $a_{m-1} … block $a_0 ;   ($a_0 innermost)
///         <disc>                    ; sum-disc(scrutinee walked to `path`) → i32 on the stack
///         br_table [0,1,…,m-1] m    ;   index k → exits $a_k; out-of-range → exits $default
///       end                         ; $a_0 label → cont_0 runs here
///       <cont_0> ; br $join
///     end                           ; $a_1 label
///       <cont_1> ; br $join
///     … end $a_{m-1} <cont_{m-1}> ; br $join
///     end                           ; $default label
///     <default cont>                ; falls through to $join's end (no br needed — it is last)
///   end
/// ```
/// The inner blocks are EMPTY (jump labels only); only `$join` carries the result type, so the stack is
/// empty at each `br_table` target and each `end` is reached only via a `br` that already pushed the
/// value to `$join` — a well-typed structure wasm accepts. The `br_table` index maps arm position → its
/// label depth; a discriminant not in `0..m` (impossible for an exhaustive sum, but the ABI is total)
/// takes the default. NOTE: this handles the ROOT and any nested switch uniformly (the discriminant is
/// read at `path`); a continuation that is itself a nested switch still recurses through `emit_sum_cont`.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_emit_disc_br_table(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    arms: &[crate::core::SumArm],
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<Option<()>, Reject> {
    // Partition into explicit-disc arms (the table entries) and an optional trailing default.
    let (disc_arms, default): (&[crate::core::SumArm], Option<&crate::core::SumArm>) =
        match arms.last() {
            Some(a) if a.disc.is_none() => (&arms[..arms.len() - 1], Some(a)),
            _ => (arms, None),
        };
    // Every table arm must carry an explicit discriminant (a default anywhere but last → fall back).
    if disc_arms.len() < 3 || disc_arms.iter().any(|a| a.disc.is_none()) {
        return Ok(None);
    }
    // Distinct discriminants, and each in `0..disc_arms.len()` so a table position IS its discriminant
    // (sum discs are contiguous `0..k`; a match lists each variant once). If the discs are not exactly
    // the contiguous set `0..m` in arm order, fall back rather than build a sparse/misindexed table.
    let discs: Vec<u32> = disc_arms.iter().map(|a| a.disc.unwrap()).collect();
    let m = discs.len() as u32;
    let contiguous_in_order = discs.iter().enumerate().all(|(i, &d)| d == i as u32);
    if !contiguous_in_order {
        return Ok(None);
    }
    // EXHAUSTIVE-MATCH DEFAULT ELISION: with NO default arm the match lists every variant, and the discs
    // are exactly the contiguous `0..m` (checked above), so the discriminant is ALWAYS in `[0, m)` — the
    // `br_table`'s own out-of-range default is DEAD. Rather than a `$default` block wrapping a stack-
    // polymorphic `unreachable`, the LAST arm serves as the table default (`br_table 0 … m-2  default=m-1`):
    // one fewer block and no dead `unreachable`. When a real default arm IS present it keeps its own block
    // (the table default routes there for a disc the arms do not cover — though for a sum that cannot occur
    // either, the arm is still emitted since the shape allows a user wildcard).
    let has_default_block = default.is_some();
    // Open the ONE typed join block, then the label blocks: `m` arm labels, plus the `$default` label ONLY
    // when a default arm is present. Innermost = arm 0.
    // Block nesting at the innermost point (outermost→innermost): join, [default], a_{m-1}, …, a_0.
    // From there `br d` exits: d=0 → $a_0, …, d=m-1 → $a_{m-1}, then [d=m → $default,] d=(m+default) → $join.
    out.push(Lir::Block(block_ty)); // $join (typed)
    if has_default_block {
        out.push(Lir::Block(BlockType::Empty)); // $default
    }
    for _ in 0..m {
        out.push(Lir::Block(BlockType::Empty)); // $a_{m-1} … $a_0
    }
    // Push the discriminant at `path` — `sum-disc` for a boxed sum, the raw i32 / unboxed int for an
    // enum-disc value (see `push_discriminant`).
    push_discriminant(
        db, scrutinee, path, slots, base, high, scratch_ty, layout, out,
    )?;
    if has_default_block {
        // Target k (arm index) → depth k (exits $a_k); table default → depth m (exits $default).
        let targets: Vec<u32> = (0..m).collect();
        out.push(Lir::BrTable(targets, m));
    } else {
        // Exhaustive: disc ∈ [0, m). Index k (0..m-1) → $a_k; the table default IS the last arm $a_{m-1}
        // (depth m-1), the disc that necessarily remains — no separate default block.
        let targets: Vec<u32> = (0..m - 1).collect();
        out.push(Lir::BrTable(targets, m - 1));
    }
    // Now emit each arm body after its label's `end`, in innermost→outermost order (arm 0 first). After
    // closing block $a_k, control from `br_table` index k (or, for the last arm without a default block,
    // the table default) lands here; run the continuation and `br` its value to $join.
    // After `end`ing $a_0..$a_k the enclosing arm blocks (inner→outer) are a_{k+1}, …, a_{m-1}, then
    // [$default,] $join. So $join is `(m-1-k)` arm blocks out, plus 1 more if a $default block sits below.
    let join_from_arm_extra = if has_default_block { 1 } else { 0 };
    for (k, arm) in disc_arms.iter().enumerate() {
        out.push(Lir::End); // close $a_k → its br_table target lands here
        // The br_table path is only taken in NON-tail position (`emit_sum_match_arms` skips it when
        // looping — see there), so a continuation here is never a loop iteration.
        // RECORD this arm's entered-variant payload type (like the linear switch) so a nested switch /
        // literal-test in the continuation resolves a `Payload` step to the actual variant, not variant 0.
        let disc = arm
            .disc
            .expect("a table arm carries an explicit discriminant");
        let restore = record_entered_payload_ty(db, scrutinee, path, disc, out);
        emit_sum_cont(
            db,
            scrutinee,
            &arm.cont,
            result_it,
            block_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            TailPos::NonTail,
        )?;
        restore_entered_payload_ty(scrutinee, path, restore, out);
        // `br` the value to $join — EXCEPT the last arm of an EXHAUSTIVE match (no $default block), whose
        // `br` depth is 0: its body is the final code inside $join, so control falls THROUGH to $join's
        // `end` anyway. A `br 0` there jumps to exactly the next instruction (the `End` below) — a dead
        // branch. Skip it: the value stays on the stack and the block ends, identical behavior, one fewer
        // instruction. (A $default block, when present, sits between the last arm and $join, so the last
        // arm's depth is ≥1 and the `br` is real — the guard `!has_default_block` covers that.)
        let depth = (m - 1 - k as u32) + join_from_arm_extra;
        if depth != 0 {
            out.push(Lir::Br(depth)); // br to $join, carrying the value
        }
    }
    // Close $default and emit its continuation (falls through to $join's end — no `br` needed). Only when
    // a real default arm exists; an exhaustive match has no $default block (the last arm covered it).
    if let Some(d) = default {
        out.push(Lir::End); // close $default
        emit_sum_cont(
            db,
            scrutinee,
            &d.cont,
            result_it,
            block_ty,
            slots,
            base,
            high,
            scratch_ty,
            layout,
            out,
            TailPos::NonTail,
        )?;
    }
    out.push(Lir::End); // close $join
    Ok(Some(()))
}

/// Record the payload type of the variant `disc` (entered by a switch arm on the sub-value at `path`) into
/// `out.sum_path_types` at `path + [Payload]`, so a nested switch / literal-test / disc-walk in the arm's
/// continuation resolves a `Payload` step to the ACTUAL entered variant's payload (not variant 0's). Returns
/// the PRIOR value at that key for [`restore_entered_payload_ty`] to put back (scoped save/restore, so the
/// ELSE fall-through and sibling arms are unaffected). A no-op (`None` inserted-nothing marker via a bool)
/// when the sub-value is not a boxed sum with a resolvable payload — mirrors the Rust backend's
/// `sum_path_types` recording. The key is `path + [Payload]`; the returned `Option<Option<Ty>>` is
/// `Some(prior)` when a key was inserted (prior may be `None` = was absent), `None` when nothing was
/// inserted (a nullary/unresolvable variant — nothing to restore).
pub(super) fn record_entered_payload_ty(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    disc: u32,
    out: &mut Emit,
) -> Option<Option<Ty>> {
    record_entered_payload_ty_into(db, scrutinee, path, disc, &mut out.sum_path_types)
}

/// Undo [`record_entered_payload_ty`]: restore the prior value at `path + [Payload]` (or remove the key if
/// it was absent). A `None` `restore` (nothing was inserted) is a no-op.
pub(super) fn restore_entered_payload_ty(
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    restore: Option<Option<Ty>>,
    out: &mut Emit,
) {
    restore_entered_payload_ty_into(scrutinee, path, restore, &mut out.sum_path_types);
}

/// The map-level core of [`record_entered_payload_ty`] — records the entered variant's payload type into
/// `recorded` at `path + [Payload]`. Shared by the emit (over `Emit::sum_path_types`) and the ops collector
/// (over its own scratch map) so both resolve a `Payload` step to the same entered-variant type.
pub(super) fn record_entered_payload_ty_into(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    disc: u32,
    recorded: &mut HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
) -> Option<Option<Ty>> {
    let root = type_of(db, scrutinee);
    let sub = ty_at_path_recorded(db, scrutinee, &root, path, recorded);
    let payload = variant_payload_ty_at(db, &sub, disc)?;
    let mut path_key = path.to_vec();
    path_key.push(crate::core::PathStep::Payload);
    let prior = recorded.insert((scrutinee, path_key), payload);
    Some(prior)
}

/// The map-level core of [`restore_entered_payload_ty`]. `scrutinee` is the match ROOT — the key is scoped
/// by it so a nested match on a DIFFERENT scrutinee cannot collide on a shared relative path.
pub(super) fn restore_entered_payload_ty_into(
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    restore: Option<Option<Ty>>,
    recorded: &mut HashMap<(StructId, Vec<crate::core::PathStep>), Ty>,
) {
    let Some(prior) = restore else {
        return;
    };
    let mut path_key = path.to_vec();
    path_key.push(crate::core::PathStep::Payload);
    let key = (scrutinee, path_key);
    match prior {
        Some(t) => {
            recorded.insert(key, t);
        }
        None => {
            recorded.remove(&key);
        }
    }
}

/// Emit one SWITCH of the decision tree: for each variant arm, `sum-disc(<scrutinee walked to `path`>)
/// == disc`, then `if (block_ty) <continuation> else <rest>`; a default arm (`disc: None`) or the LAST
/// arm (its probe redundant — every earlier disc has been tested and this is the only one left) is the
/// unconditional tail. `path` reaches the sub-value THIS switch dispatches on — empty for the ROOT (the
/// scrutinee itself), a `[Payload…]` path for a NESTED switch. Each arm's CONTINUATION is a leaf body or
/// a deeper switch (`emit_sum_cont`), which is what makes the whole match a decision tree that shares the
/// outer probe. Mirrors `emit_match_arms_tailable` but probes the discriminant. (A dense set of ≥3 discs
/// takes the `try_emit_disc_br_table` fast path before this linear chain.)
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_sum_match_arms(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    arms: &[crate::core::SumArm],
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    // BR_TABLE DECISION TREE: a switch that tests ≥3 DISTINCT discriminants dispatches in O(1) via a
    // jump table instead of a linear `if (disc == k)` cascade (the arms below). Sum discriminants are
    // contiguous `0..variant_count`, so the table is dense with no wasted slots. `try_emit_disc_br_table`
    // returns `Some(())` when it emitted the table, `None` to fall through to the linear chain (too few
    // arms, or a shape it does not handle — a leading default, non-distinct discs).
    // SKIPPED ONLY FOR A SELF-LOOP (`Tail(Some(tl))`): the table wraps its arm continuations in nested
    // control-flow BLOCKS (`$join`/`$a_k`), a different block-nesting than the linear `if`-chain; a
    // self-tail-call compiled as a loop `br tl.depth` inside an arm would need a table-specific depth (not
    // the `deeper_tail` +1-per-`if` accounting the linear chain uses). The linear chain loops correctly and
    // covers the common recursive-sum shapes (2-variant Cons/Nil, Succ/Zero, Node/Leaf never hit the
    // ≥3-disc table anyway), so fall back to it when a self-loop is in play. A `NonTail` match (a sum match
    // used as an operand) OR a `Tail(None)` one (a non-self-recursive function body — EVERY body is emitted
    // via `emit_tail`, so this is the common case) keeps the O(1) table: the table's continuations are
    // emitted `NonTail` (a `return_call` `br`s to `$join` fine — it's frame-replacing, not depth-relative;
    // and a self-loop `br` never occurs here since there is no loop), so it is byte-identical to the
    // pre-tail behavior for both.
    if !is_self_loop_tail(tail)
        && let Some(()) = try_emit_disc_br_table(
            db, scrutinee, path, arms, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out,
        )?
    {
        return Ok(());
    }
    match arms.split_first() {
        None => Err(Reject::decline(
            "sum match ran off the end with no covering arm",
        )),
        // A default arm, or the last arm of an exhaustive switch — its probe is redundant, so emit its
        // continuation unconditionally (in the SAME tail position as the whole switch — no `if` opened).
        Some((arm, [])) => emit_sum_cont(
            db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, tail,
        ),
        Some((arm, _)) if arm.disc.is_none() => emit_sum_cont(
            db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, tail,
        ),
        Some((arm, rest)) => {
            let disc = arm.disc.expect("non-None handled above");
            // BRANCHLESS 2-ARM SUM SELECT: a switch of exactly TWO arms — this disc-arm then a single
            // unconditional cover (a `disc: None` default, or the last arm of an exhaustive switch) — is
            // `(if (disc == d) then else)`, the sum-discriminant twin of the scalar 2-arm select
            // (`emit_match_dispatch`). When both arms are `Leaf` bodies that are cheap trap-free
            // `is_select_arm`s and the result is a scalar (a value `block_ty`), emit `then ; else ;
            // (disc == d) ; select` instead of an `if`/`else` block — so a 2-variant enum match
            // `(match f (On 1) (Off 0))` goes branchless (`disc eqz ; select`) exactly as the equivalent
            // `if` would. Only for NON-self-loop position (`select` cannot carry a loop `br`; a `Leaf`
            // select-arm is never a tail call anyway) and when both continuations are plain leaves — a
            // guarded / nested-switch / lit-test continuation keeps the structured `if` below.
            if !is_self_loop_tail(tail)
                && matches!(block_ty, BlockType::Val(_))
                && let [cover] = rest
                && let crate::core::SumCont::Leaf(then_body) = &arm.cont
                && let crate::core::SumCont::Leaf(else_body) = &cover.cont
                && is_select_arm(db, *then_body)
                && is_select_arm(db, *else_body)
            {
                let (then_body, else_body) = (*then_body, *else_body);
                let res_ty = match result_it {
                    Some(rit) => Ty::Int(rit),
                    None => type_of(db, then_body),
                };
                emit_branch(
                    db, then_body, &res_ty, slots, base, high, scratch_ty, layout, out,
                )?;
                emit_branch(
                    db, else_body, &res_ty, slots, base, high, scratch_ty, layout, out,
                )?;
                push_discriminant(
                    db, scrutinee, path, slots, base, high, scratch_ty, layout, out,
                )?;
                push_disc_eq(disc, out);
                out.push(Lir::Select);
                return Ok(());
            }
            // discriminant(<scrutinee walked down `path`>) == disc — `sum-disc` for a boxed sum, the raw
            // i32 / unboxed int for an enum-disc value (see `push_discriminant`).
            push_discriminant(
                db, scrutinee, path, slots, base, high, scratch_ty, layout, out,
            )?;
            push_disc_eq(disc, out);
            out.push(Lir::If(block_ty));
            // The matched arm's continuation and the fall-through switch both sit one `if` deeper — bump
            // the tail depth so a self-loop `br` inside either targets the loop top (mirrors the scalar
            // `emit_probe_chain` / list `emit_list_arms_tailable` disc-nesting).
            let deeper = deeper_tail(tail);
            // RECORD this entered variant's payload type at `path + [Payload]` so a NESTED switch / literal-
            // test / disc-walk in the arm's continuation resolves a `Payload` step to the ACTUAL entered
            // variant's payload — not variant 0. Scoped save/restore fences it to this arm (the ELSE
            // fall-through and sibling arms must not see it). Only for a boxed sum with a real payload.
            let restore = record_entered_payload_ty(db, scrutinee, path, disc, out);
            emit_sum_cont(
                db, scrutinee, &arm.cont, result_it, block_ty, slots, base, high, scratch_ty,
                layout, out, deeper,
            )?;
            restore_entered_payload_ty(scrutinee, path, restore, out);
            // The fall-through switch (the disc-test's ELSE) starts scratch ABOVE the high-water the
            // matched arm's continuation (the THEN) reached, NOT at `base` — the same discipline as the
            // `Core::If` / guard sites. The THEN's continuation may contain a guard that stashes an i32
            // HEAP HANDLE (`value-eq`/`MatchSum`) in a low slot, typing it i32 for the whole function; the
            // ELSE's fall-through loop-iteration i64 arithmetic reusing that slot fails validation (the two
            // `if` branches share one function-global local declaration). A THEN that touches no heap
            // handle leaves `*high` where it was, so this is byte-identical for the common case.
            let else_base = *high;
            out.push(Lir::Else);
            emit_sum_match_arms(
                db, scrutinee, path, rest, result_it, block_ty, slots, else_base, high, scratch_ty,
                layout, out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
    }
}

/// Emit a sum-match LITERAL-TEST probe: push the scrutinee handle, walk `path` to the leaf sub-value
/// (`sum-payload`/`arr-get`/`vec-get`, tracking the sub-value TYPE so an erased newtype `Payload` is a
/// no-op and a list `Elem` reads `vec-get` not `arr-get`), read the leaf scalar (`get-int`/`get-bool` when
/// it is a boxed handle, raw when an erased scalar newtype), and compare against the literal `probe` —
/// leaving `[bool]` on the stack (1 = matched). The caller decides the control flow: the nested `if` form
/// (`SumCont::LitTest`'s `then_`/`els`) OR the flat multi-column `br_if` chain reuse the SAME probe emit per
/// column, so the walk+compare lives here once rather than copied. Declines a runtime Char/Map-key probe
/// (no runtime rep — a constant folds instead; never a miscompile). Extracted verbatim from the
/// `SumCont::LitTest` arm; the width/erased-newtype/rope-canonicalization comments are preserved inline.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_littest_probe(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    probe: &crate::core::Probe,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    // Push the scrutinee handle and walk to the leaf's boxed handle — tracking the sub-value TYPE as
    // the walk descends (mirrors `Core::SumPayload`), so an ERASED newtype `Payload` is a no-op (the
    // box is elided) and a `List` sub-value's `Elem` reads with `vec-get`, not `arr-get`. Without
    // this, a `(Bx (list …))` newtype's `ListLen` test called `sum-payload` on the raw list (garbage
    // length), and a boxed-list `Elem` used `arr-get` on a vec handle (garbage element).
    emit(db, scrutinee, slots, base, high, scratch_ty, layout, out)?; // [handle]
    let mut cur = type_of(db, scrutinee);
    // Whether the value now on the stack is a HEAP HANDLE (needs `get-int`/`get-bool` to read the
    // scalar leaf out of the box) or a RAW SCALAR already (read directly). It starts a handle unless
    // the scrutinee is itself an unboxed scalar — an ERASED single-variant newtype over a scalar,
    // `(type W (Wrap Int64))`, whose value IS a bare i64 (no box). Each heap-child accessor below
    // (`sum-payload`/`arr-get`/`vec-get`) produces a child HANDLE (→ true); an erased `Payload`
    // no-op leaves the representation unchanged. WITHOUT this, a literal-payload test on an erased
    // scalar newtype (`(match (W.Wrap n) ((W.Wrap 0) …) ((W.Wrap x) …))`) emitted `get-int` on the
    // raw i64 — an i32-handle unbox over an i64 value → an INVALID component (`func failed to
    // validate: expected i32, found i64`), a decline-don't-miscompile violation. The binding arm
    // reads the same payload raw (bare `local.get`), so this aligns the literal arm with it.
    let mut holds_handle = !matches!(cur.strip_nominal(), Ty::Int(_) | Ty::Bool | Ty::Float(_));
    let mut lit_prefix: Vec<crate::core::PathStep> = Vec::with_capacity(path.len());
    for step in path {
        lit_prefix.push(*step);
        match step {
            crate::core::PathStep::Payload => {
                match cur.strip_nominal() {
                    // A boxed sum's payload is unwrapped with `sum-payload`; its type is the ENTERED
                    // variant's payload (from `sum_path_types`, else variant 0) — a following `Elem`
                    // needs it to pick vec-get vs arr-get, and a non-variant-0 list payload matched
                    // by a nested element pattern reads the wrong accessor without it.
                    Ty::Sum { .. } => {
                        out.push(Lir::CallImport(OP_SUM_PAYLOAD));
                        holds_handle = true; // sum-payload yields the child HANDLE
                        cur = payload_step_ty_of(
                            db,
                            scrutinee,
                            Some(scrutinee),
                            &cur,
                            &lit_prefix,
                            &out.sum_path_types,
                        );
                    }
                    // An ERASED nominal newtype: the box is gone, so the `Payload` step is a static
                    // unwrap — NO `sum-payload` op, `cur` becomes the inner type. The stack value is
                    // UNCHANGED (still whatever the scrutinee was — a raw scalar for a scalar
                    // newtype), so `holds_handle` is left as-is.
                    inner => cur = inner.clone(),
                }
            }
            crate::core::PathStep::Elem(i) => {
                out.push(Lir::ConstI32(*i as i32));
                holds_handle = true; // arr-get/vec-get yield the child HANDLE
                if matches!(cur.strip_nominal(), Ty::List(_)) {
                    out.push(Lir::CallImport(OP_VEC_GET));
                    cur = match cur.strip_nominal() {
                        Ty::List(e) => (**e).clone(),
                        _ => Ty::Any,
                    };
                } else {
                    out.push(Lir::CallImport(OP_ARR_GET));
                    cur = match cur.strip_nominal() {
                        Ty::Tuple(elems) => elems.get(*i).cloned().unwrap_or(Ty::Any),
                        // A record erases to a tuple in sorted-field order, so field-slot `i` is
                        // `fields.values().nth(i)` — same index space as `Core::Record`/`Core::Proj`. Tracking
                        // it (not falling to `Ty::Any`) grounds a narrow int/float record field's width.
                        Ty::Record(fields) => fields.values().nth(*i).cloned().unwrap_or(Ty::Any),
                        _ => Ty::Any,
                    };
                }
            }
            crate::core::PathStep::RestFrom(_) => {} // never on a sum-lit-test path
            crate::core::PathStep::TupleRestFrom(_) => {} // never on a sum-lit-test path
        }
    }
    // Read the leaf scalar and compare against the literal. A `0` literal (a `(Some 0)`/`(Ok 0)`
    // payload pattern) is `payload == 0` — `i64.eqz` (one instruction), not `const 0 ; eq` (two);
    // the sum-payload twin of the scalar-probe eqz special case.
    match probe {
        crate::core::Probe::Int(v) => {
            // A BIGINT payload leaf is a HEAP handle, NOT a boxed fixnum — `get-int` (which reads a boxed
            // i64 fixnum) is wrong on it: it never equals the literal (silent wrong value), and in a
            // recursive fn the raw i32 handle vs `i64.const` is a type mismatch (invalid module, breaker
            // FINDING #22 / corpus adv-nonzero-bigint-literal-probe). Compare the BigInt PROPERLY: the
            // payload handle is on the stack (a `sum-payload` leaf), MATERIALIZE the literal as a fresh
            // owned BigInt leaf, `bigint-cmp` (three-way, BORROWS both) → 0 iff equal (`i64.eqz`), then
            // reclaim the owned literal leaf (the payload stays borrowed from the scrutinee's shell) — the
            // sum-payload literal-probe twin of `Core::BigIntCmp`'s borrow-and-reclaim. `bigint-cmp` borrows
            // (does not consume), so TEE the literal handle before the call so a copy survives to drop.
            // Applies for any literal value (a BigInt `0` is still a heap handle → must NOT take the i64.eqz
            // fast path below); this branch precedes it.
            if holds_handle && matches!(cur.strip_nominal(), Ty::BigInt) {
                let lit_slot = *high;
                *high += 1;
                scratch_ty.insert(lit_slot, ValType::I32);
                emit_const_bigint_leaf(v, out); // [payload, lit:i32] — fresh owned literal leaf
                out.push(Lir::LocalTee(lit_slot)); // stash a copy of the literal handle to drop after cmp
                out.push(Lir::CallImport(OP_BIGINT_CMP)); // borrows payload+lit → [cmp:i64] (0 = equal)
                out.push(Lir::LocalGet(lit_slot));
                out.push(Lir::CallImport(OP_DROP)); // reclaim the owned literal leaf → [cmp:i64]
                out.push(Lir::I64Eqz); // [bool]
                return Ok(());
            }
            // Read the scalar out of the box (`get-int` → NORMALIZED i64) when the leaf is a heap
            // handle, and compare at i64. An ERASED scalar newtype instead left the RAW payload on
            // the stack at its NATIVE machine width — i64 for `Int64`, but i32 for a NARROW newtype
            // (`(Wrap UInt8)`/`Int8`/`Int16`/`Int32`, whose raw rep is an i32 slot). So the compare
            // op must match the payload's actual width: `i64.eqz`/`i64.eq` over a boxed-or-i64 leaf,
            // `i32.eqz`/`i32.eq` over a narrow raw leaf. Reading the raw scalar but comparing it at
            // the hard-coded i64 emitted `i64.eqz` over an i32 → an INVALID component (the narrow
            // twin of the Int64 invalid-component this branch first fixed; `holds_handle`=false but
            // the width was still assumed i64). Boxed path stays i64 (`get-int` normalizes).
            let slot32 = if holds_handle {
                out.push(Lir::CallImport(OP_GET_INT)); // [i64] — normalized
                false
            } else {
                // The erased payload's native slot: i32 for a narrow width (`≤ 32`), else i64. `cur`
                // is the payload type after the path walk (an `Int` for a scalar newtype).
                match cur.strip_nominal() {
                    Ty::Int(it) => Machine::of(*it).slot32,
                    _ => false, // non-narrow / unknown → i64 (Int64 and the prior behavior)
                }
            };
            if v.to_i64_bits() == 0 {
                out.push(if slot32 { Lir::I32Eqz } else { Lir::I64Eqz }); // [bool]
            } else if slot32 {
                out.push(Lir::ConstI32(v.to_i64_bits() as i32));
                out.push(Lir::I32Eq); // [bool]
            } else {
                out.push(Lir::ConstI64(v.to_i64_bits()));
                out.push(Lir::I64Eq); // [bool]
            }
        }
        crate::core::Probe::Bool(b) => {
            // Same erased-newtype gate: a boxed Bool payload unboxes with `get-bool`, an erased
            // Bool newtype is already a raw i32 0/1 on the stack.
            if holds_handle {
                out.push(Lir::CallImport(OP_GET_BOOL)); // [i32]
            }
            out.push(Lir::ConstI32(if *b { 1 } else { 0 }));
            out.push(Lir::I32Eq); // [bool]
        }
        crate::core::Probe::Str(s) => {
            // A string-literal payload over a RUNTIME value (`(Ast.Name "+")` matched on a runtime
            // Ast, a `(k "lit")` map-value pattern): compare the leaf String handle against the
            // literal by CONTENT — the same `value-eq` (`champ_eq`) physical-byte compare
            // `Core::ValueEq` uses on two strings. The path walk above left the leaf String HANDLE on
            // the stack — a BORROWED payload (`sum-payload`/`arr-get`/`vec-get` all borrow).
            // Canonicalize it with `bytes-compact` (rope→flat, refcount-NEUTRAL: flattens in place,
            // returns the SAME handle, so the borrow is neither consumed nor a fresh mint) so a rope
            // payload and its flat twin compare equal — exactly as the `Core::ValueEq` emit does for a
            // borrowed String operand. Save the compacted leaf handle in a slot, build the literal as a
            // fresh OWNED `ConstStr` byte-leaf (canonical UTF-8, NFC by the reader — the same build the
            // `Core::ConstStr` emit lays down, so `value-eq` compares two canonical leaves), `value-eq`
            // (borrows + pops both → bool), then DROP the owned literal (the borrowed leaf is left to
            // its owner — no drop, matching the `Core::ValueEq` borrowed-operand rule).
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [leaf'] — canonical flat leaf, same handle
            let leaf_slot = *high;
            let lit_slot = *high + 1;
            *high += 2;
            scratch_ty.insert(leaf_slot, ValType::I32);
            scratch_ty.insert(lit_slot, ValType::I32);
            out.push(Lir::LocalSet(leaf_slot)); // stash the borrowed leaf handle
            // Build the literal string as a fresh flat UTF-8 byte-leaf (mirrors `Core::ConstStr`).
            let bytes = s.as_bytes();
            out.push(Lir::ConstI32(bytes.len() as i32));
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [buf]
            for (i, &byte) in bytes.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::ConstI32(byte as i32));
                out.push(Lir::CallImport(OP_BYTES_SET)); // [buf]
            }
            out.push(Lir::LocalTee(lit_slot)); // [lit] — keep the owned literal handle for the drop
            out.push(Lir::LocalGet(leaf_slot)); // [lit, leaf]
            out.push(Lir::CallImport(OP_VALUE_EQ)); // pops both (borrowed) → [bool]
            // DROP the owned literal (a fresh leaf we minted); the leaf handle is a borrowed payload,
            // left to its owner. The bool result stays on the stack for the `if` below.
            out.push(Lir::LocalGet(lit_slot));
            out.push(Lir::CallImport(OP_DROP));
            // `value-eq` left [bool] then we pushed/dropped the literal — the drop consumed its own
            // arg, so the stack is back to [bool]. Fall through to the shared `if`.
        }
        crate::core::Probe::Bytes(p) => {
            // A byte-string-literal payload over a RUNTIME Bytes value (`(Some b"AB")` matched on a runtime
            // `Some`) — the Bytes twin of the `Str` arm above, byte-for-byte the same shape. A Bytes is a
            // flat byte leaf, so: `bytes-compact` the borrowed payload handle (rope→flat, refcount-neutral),
            // stash it, build the literal as a fresh OWNED byte-leaf (`bytes-alloc`+`bytes-set` — exactly
            // what `Core::ConstBytes` emits, so `value-eq` compares two canonical leaves), `value-eq`
            // (borrows + pops both → bool), then DROP the owned literal (the payload leaf is left to its
            // owner). The ONLY difference from `Str` is the literal's bytes are `p` verbatim (arbitrary,
            // not necessarily UTF-8), not `s.as_bytes()`.
            out.push(Lir::CallImport(OP_BYTES_COMPACT)); // [leaf'] — canonical flat leaf, same handle
            let leaf_slot = *high;
            let lit_slot = *high + 1;
            *high += 2;
            scratch_ty.insert(leaf_slot, ValType::I32);
            scratch_ty.insert(lit_slot, ValType::I32);
            out.push(Lir::LocalSet(leaf_slot)); // stash the borrowed leaf handle
            out.push(Lir::ConstI32(p.len() as i32));
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // [buf]
            for (i, &byte) in p.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::ConstI32(byte as i32));
                out.push(Lir::CallImport(OP_BYTES_SET)); // [buf]
            }
            out.push(Lir::LocalTee(lit_slot)); // [lit] — keep the owned literal handle for the drop
            out.push(Lir::LocalGet(leaf_slot)); // [lit, leaf]
            out.push(Lir::CallImport(OP_VALUE_EQ)); // pops both (borrowed) → [bool]
            out.push(Lir::LocalGet(lit_slot));
            out.push(Lir::CallImport(OP_DROP));
            // Stack is back to [bool]. Fall through to the shared `if`.
        }
        crate::core::Probe::ListLen { len, at_least } => {
            // A list-pattern payload over a RUNTIME list: the path walked to the sub-value's LIST
            // HANDLE (an i32); its `vec-len` is the length to test. A FIXED-arity `(list p0…p_{n-1})`
            // matches length EXACTLY `n` (`vec-len == n`); a rest `(list p… .. rest)` matches AT
            // LEAST `n` (`vec-len >= n`, the tail binds the surplus). The leading element binders +
            // the rest binder read the list on their own via `SumPayload{Elem}/{RestFrom}` (resolve
            // Case 6l/6r), so this arm only emits the LENGTH gate. On a mismatch, control falls
            // through to `els` exactly as an Int/Bool literal test does.
            out.push(Lir::CallImport(OP_VEC_LEN)); // [len:i32]
            out.push(Lir::ConstI32(*len as i32));
            out.push(if *at_least { Lir::I32GeU } else { Lir::I32Eq }); // [bool]
        }
        crate::core::Probe::Char(c) => {
            // A char-literal payload over a RUNTIME char (Char-rep 4/N): a `Char` is an i32 code-point slot
            // (Char-rep 1/N), boxed into the i64 heap cell like a narrow int (`box_op_ty(Char) = box-int`).
            // The path walk left either the BOXED leaf handle (`holds_handle`) or a RAW i32 char (an erased
            // char newtype). Read the code point out of the box with `get-int` (→ i64) when boxed, else the
            // raw value is already the i32 slot; compare to THIS literal's code point (`i32.eq`/`i64.eq` to
            // match the read width) — the payload twin of the scalar char-scrutinee match (Char-rep 3/N).
            let cp = *c as u32 as i64; // Unicode scalar, always non-negative
            if holds_handle {
                out.push(Lir::CallImport(OP_GET_INT)); // [i64]
                out.push(Lir::ConstI64(cp));
                out.push(Lir::I64Eq); // [bool]
            } else {
                out.push(Lir::ConstI32(cp as i32));
                out.push(Lir::I32Eq); // [bool]
            }
        }
        crate::core::Probe::MapHasKeys { .. } => {
            // A map-pattern payload over a RUNTIME map: the key-presence gate would need a runtime
            // `map-lookup` per key (and the value binders a runtime keyed read), not yet wired — a
            // CONSTANT map folds the `MapHasKeys` test instead (`build_tree`), never reaching here.
            // Decline (like the runtime string-payload probe), never a miscompile.
            return Err(Reject::declined(
                crate::diag::DeclineId::WasmMapPatternRuntimeMap,
                "matching a map-pattern payload against a runtime map needs the per-binder runtime \
                 keyed-read (a constant map folds the key test instead)",
            ));
        }
        crate::core::Probe::Wild => {
            return Err(Reject::decline("a wildcard literal test is a compiler bug"));
        }
    }
    Ok(())
}

/// The decomposition of a flattenable multi-column literal-test arm ([`flattenable_multicol_arm`]): the
/// ordered per-column `(path, probe)` tests, the arm `body`, and the shared next-arm fall-through tail `S`.
pub(super) type FlatMulticolArm<'a> = (
    Vec<(&'a [crate::core::PathStep], &'a crate::core::Probe)>,
    StructId,
    &'a std::rc::Rc<crate::core::SumCont>,
);

/// Recognize a FLATTENABLE multi-column literal-test arm — a `LitTest` chain of ≥2 columns whose `then_`
/// spine terminates in a `Leaf` body and whose every `els` is the SAME `Rc<SumCont>` (the shared next-arm
/// fall-through, by `Rc::ptr_eq`). This is the shape a two-/multi-column tuple arm `(tuple i i a)` lowers to
/// (`build_tree`'s shared fall-through, verified linear-DAG): an OUTER `LitTest(col0){then_=inner, els=S}`
/// and an INNER `LitTest(col1){then_=Leaf(body), els=S}` with the SAME `S`. Returns the ordered per-column
/// `(path, probe)` tests, the arm `body`, and the shared tail `S` — so `emit_sum_cont` can emit a FLAT
/// `br_if` guard chain (each column `br_if`s to the arm-fail label, the shared tail emitted ONCE) instead of
/// the nested `if`/`else` that re-emits `S` in BOTH branches at every column (the O(2^cols) emit blowup).
///
/// The `Rc::ptr_eq` requirement is what makes flattening SOUND: the flat chain sends EVERY column's failure
/// to the ONE arm-fail label, so it is equivalent to the nested form ONLY when every column's `else` is the
/// SAME continuation. A chain whose `els`es differ (a refining probe that does not share its tail, a nested
/// pattern) fails the check → `None` → the caller keeps the byte-identical nested emit. Requires ≥2 columns
/// (a single-column `LitTest` is already optimal as a plain `if`/`else`; flattening it only adds block
/// overhead). A `Guarded`/`Switch` in the `then_` spine → `None` (not a flat multi-column arm).
pub(super) fn flattenable_multicol_arm(cont: &crate::core::SumCont) -> Option<FlatMulticolArm<'_>> {
    use crate::core::SumCont;
    let SumCont::LitTest {
        path,
        probe,
        then_,
        els,
    } = cont
    else {
        return None;
    };
    let shared = els;
    let mut cols: Vec<(&[crate::core::PathStep], &crate::core::Probe)> = vec![(path, probe)];
    let mut cur = then_;
    loop {
        match cur.as_ref() {
            // The spine ends at the arm body — flattenable iff we walked ≥2 columns (all sharing `S`).
            SumCont::Leaf(body) => {
                return (cols.len() >= 2).then_some((cols, *body, shared));
            }
            // A further column of the SAME arm: its `els` must be the SAME shared tail (else the two
            // failures go to different continuations and the flat single-target chain would be wrong).
            SumCont::LitTest {
                path,
                probe,
                then_,
                els,
            } => {
                if !std::rc::Rc::ptr_eq(els, shared) {
                    return None;
                }
                cols.push((path, probe));
                cur = then_;
            }
            // A guard or nested switch in the spine is not a flat multi-column arm — keep the nested emit.
            _ => return None,
        }
    }
}

/// Emit a matched arm's CONTINUATION: a LEAF emits its body (a bare-`ConstInt` body grounded to the
/// match's result width `result_it`, as the scalar-match arms are); a nested SWITCH emits a fresh switch
/// chain on its deeper sub-value (`emit_sum_match_arms`), which is the decision tree recursing to share
/// the outer probe. The nested switch's `if`s reuse the SAME `block_ty` (both branches yield the match's
/// one result type at every depth). `tail` carries the [`TailPos`]: in a TAIL sum match each LEAF/GUARDED
/// body is a tail position (a self-tail-call there iterates the loop / becomes a `return_call`), and the
/// nested dispatch bumps the threaded loop `depth` +1 per enclosing `if` (via `deeper_tail`). `NonTail` is
/// byte-identical to the pre-tail behavior (bodies emit via `emit`).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_sum_cont(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    result_it: Option<IntTy>,
    block_ty: BlockType,
    slots: &HashMap<StructId, u32>,
    base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
    tail: TailPos,
) -> Result<(), Reject> {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            // SHARED SUM-PAYLOAD-PREFIX CSE: if this arm body reads ≥2 elements off the same payload tuple
            // (`(Node (tuple l r))`), compute the shared `sum-payload` prefix ONCE into a slot here and
            // register it so each element's `SumPayload` emit reads the slot + walks only its suffix. The
            // slots are fenced to THIS arm body (removed after), so a sibling arm never reads a prefix that
            // its own scrutinee value did not populate.
            let prefix_keys = materialize_payload_prefixes(
                db, *body, base, high, scratch_ty, slots, layout, out,
            )?;
            // A bare-`ConstInt` leaf grounds to the result width (never a tail call); otherwise the body is
            // emitted at the ambient `tail` position — in a tail match a self-tail-call in the body loops.
            // The arm body emits ABOVE the reserved prefix slots (the base advanced by `materialize_*`).
            let arm_base = (*high).max(base);
            let r = emit_arm_body(
                db, *body, result_it, block_ty, slots, arm_base, high, scratch_ty, layout, out,
                tail,
            );
            for key in prefix_keys {
                out.payload_prefix_slots.remove(&key);
            }
            r
        }
        // A GUARDED arm: `if cond then body else <els>`. The guard cond is a boolean (an i32); each of the
        // body and the fall-through `els` produces the match's result type (`block_ty`), grounding a
        // bare-literal body to the result width exactly as an `if` branch does. The `els` continuation
        // recurses — it is the rest of the sub-matrix (a later arm of the same variant, or the default).
        crate::core::SumCont::Guarded { cond, body, els } => {
            emit(db, *cond, slots, base, high, scratch_ty, layout, out)?;
            // The body and fall-through start scratch ABOVE the high-water the GUARD reached, NOT at
            // `base` — the same discipline as the `Core::If` / scalar-match-guard / probe-else sites. A
            // guard that stashes an i32 HEAP HANDLE (a runtime `value-eq`/`MatchSum` — `(guard (N.I x)
            // (= (mk x) (mk 3)))`) types a low slot i32 for the whole function; the fall-through's
            // loop-iteration i64 arithmetic reusing that slot fails validation. A scalar guard leaves
            // `*high == base`, so this is byte-identical for the common case.
            let body_base = *high;
            out.push(Lir::If(block_ty));
            // Both the body and the fall-through `els` sit one `if` deeper — bump the tail depth so a
            // self-loop `br` from either targets the loop top (mirrors `emit_arm_guarded_body`).
            let deeper = deeper_tail(tail);
            emit_arm_body(
                db, *body, result_it, block_ty, slots, body_base, high, scratch_ty, layout, out,
                deeper,
            )?;
            out.push(Lir::Else);
            emit_sum_cont(
                db, scrutinee, els, result_it, block_ty, slots, body_base, high, scratch_ty,
                layout, out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
        // A LITERAL TEST: `if (<sub-value at path> == literal) then <then_> else <els>`. Walk the `path`
        // from the scrutinee handle (`sum-payload`/`arr-get`, exactly as `Core::SumPayload` does), read the
        // leaf scalar (`get-int` → i64 / `get-bool` → i32), compare against the literal, and branch. Both
        // continuations recurse through `emit_sum_cont` and yield the match's result type (`block_ty`). The
        // `then_` typically ends in the arm body; `els` is the same-variant fall-through (the binding arm).
        // The read mirrors `SumPayload`'s walk + unbox; the compare mirrors `emit_probe_chain`'s Int/Bool
        // probe. A narrow-int payload's `get-int` yields the normalized i64, so an i64 compare against the
        // literal's i64 bits is exact (the pattern literal is in range or the arm is ill-typed and rejected
        // earlier).
        crate::core::SumCont::LitTest {
            path,
            probe,
            then_,
            els,
        } => {
            // FLAT MULTI-COLUMN `br_if` GUARD CHAIN (S2 emit-size): a `(tuple i i a)`-style arm lowers to a
            // `LitTest` chain of ≥2 columns whose every `els` is the SAME shared next-arm tail. The nested
            // `if`/`else` below re-emits that shared tail in BOTH the inner `then_` AND the outer `else` at
            // every column → the emit grows O(2^cols) even though the decision DAG is linear. Instead emit a
            // FLAT chain: open `$join` then `$arm_fail`, run each column's probe and `i32.eqz; br_if
            // $arm_fail` (any column mismatch jumps to the shared tail), then the body and `br $join`; close
            // `$arm_fail` and emit the shared tail ONCE (it falls through to `$join`'s end). Each column's
            // failure targets the ONE `$arm_fail` label, so this is equivalent to the nested form ONLY when
            // every `els` is the same continuation — which `flattenable_multicol_arm` checks by `Rc::ptr_eq`.
            //
            // NON-TAIL ONLY (`!Tail(Some(_))`), mirroring `try_emit_disc_br_table`'s self-loop skip: the flat
            // blocks are a different nesting than the `deeper_tail` +1-per-`if` accounting the nested chain
            // threads, so a self-loop `br` inside the body would need flat-specific depth math. A `NonTail`
            // (operand) or `Tail(None)` (non-self-recursive body) match keeps the shared-tail continuation
            // emitted `NonTail` inside `$arm_fail` — no loop `br` occurs, so the depth is static (0 to the
            // arm-fail body's own frame). The body sits inside `$arm_fail`→`$join` (2 blocks): its value
            // `br $join` is depth 1.
            if !is_self_loop_tail(tail)
                && let Some((cols, body, shared)) = flattenable_multicol_arm(cont)
            {
                out.push(Lir::Block(block_ty)); // $join (typed — carries the arm/tail result value)
                out.push(Lir::Block(BlockType::Empty)); // $arm_fail
                // Each column: probe → `[bool]`; a mismatch (`eqz`) `br_if`s out to $arm_fail (depth 0).
                for (cpath, cprobe) in &cols {
                    emit_littest_probe(
                        db, scrutinee, cpath, cprobe, slots, base, high, scratch_ty, layout, out,
                    )?;
                    out.push(Lir::I32Eqz); // matched? -> 0 ; mismatched -> 1
                    out.push(Lir::BrIf(0)); // mismatch: fall through to the shared tail ($arm_fail)
                }
                // All columns matched — emit the body and branch its value to $join (depth 1: out of
                // $arm_fail then to $join). The body is a leaf in NON-tail position; it starts scratch
                // ABOVE the high-water the column probes reached (`(*high).max(base)`, the same discipline as
                // the `Leaf` arm), so a probe's transient i32 slot never clashes with the body's temps.
                let body_base = (*high).max(base);
                emit_arm_body(
                    db,
                    body,
                    result_it,
                    block_ty,
                    slots,
                    body_base,
                    high,
                    scratch_ty,
                    layout,
                    out,
                    TailPos::NonTail,
                )?;
                out.push(Lir::Br(1)); // br $join
                out.push(Lir::End); // close $arm_fail
                // The shared next-arm tail, emitted ONCE — a column mismatch `br_if`ed here. It falls
                // through to $join's `end` (it produces the block's result), so no trailing `br` is needed.
                let tail_base = *high;
                emit_sum_cont(
                    db,
                    scrutinee,
                    shared,
                    result_it,
                    block_ty,
                    slots,
                    tail_base,
                    high,
                    scratch_ty,
                    layout,
                    out,
                    TailPos::NonTail,
                )?;
                out.push(Lir::End); // close $join
                return Ok(());
            }
            // Emit the scrutinee-walk + literal compare, leaving `[bool]` on the stack (extracted to
            // `emit_littest_probe` — the flat multi-column `br_if` chain above reuses the SAME probe emit
            // per column, so this is factored to one place rather than copied).
            emit_littest_probe(
                db, scrutinee, path, probe, slots, base, high, scratch_ty, layout, out,
            )?;
            out.push(Lir::If(block_ty));
            // Both continuations sit one `if` deeper — bump the tail depth (mirrors the guard/switch sites).
            let deeper = deeper_tail(tail);
            emit_sum_cont(
                db, scrutinee, then_, result_it, block_ty, slots, base, high, scratch_ty, layout,
                out, deeper,
            )?;
            // The `els` continuation starts scratch above the `then_`'s high-water — same discipline as
            // the disc-switch/guard sites: a `then_` that stashes an i32 heap handle must not have its
            // slot reused by `els`'s i64 loop arithmetic (byte-identical when `then_` touches no handle).
            let els_base = *high;
            out.push(Lir::Else);
            emit_sum_cont(
                db, scrutinee, els, result_it, block_ty, slots, els_base, high, scratch_ty, layout,
                out, deeper,
            )?;
            out.push(Lir::End);
            Ok(())
        }
        crate::core::SumCont::Switch { path, arms } => emit_sum_match_arms(
            db, scrutinee, path, arms, result_it, block_ty, slots, base, high, scratch_ty, layout,
            out, tail,
        ),
    }
}
