use super::*;

pub(super) fn is_heap_type_for_retain(ty: &Ty) -> bool {
    is_heap_type(ty) || ty.has_free_var()
}

/// Whether the reference to the `let` binding `binder` ESCAPES the node at `id` — i.e. its reference
/// flows into a value that OUTLIVES the `let` (the returned result, an element of a constructed tuple,
/// or a call argument — all CONSUMING positions), as opposed to being used only to BORROW (a
/// `Core::Proj` operand: `arr-get` borrows and does not transfer ownership). An escaped binding must
/// NOT be dropped — its ownership transfers to the consumer (ownership-transfer-on-return). This is a
/// CONSERVATIVE analysis: any occurrence that is not provably a borrow is treated as an escape, so a
/// value is never wrongly reclaimed (a false "escapes" only leaks in a case we do not yet emit; a false
/// "does not escape" would be a use-after-free, which this avoids). `tail` marks whether `id` is in the
/// body's TAIL (result) position — a bare `LocalRef` in tail position is the return, an escape.
///
/// This is the aliasing discipline the compiler applies INTERNALLY: the escape/borrow classification is
/// computed here from the source, deciding where a `dup` retains and where a `drop` reclaims — the
/// program's author writes no use-count and no aliasing annotation to be memory-safe. Because the
/// analysis is conservative (only a provable borrow is treated as non-escaping), a live value is never
/// reclaimed under one reference while another still reads it, so the emitted component has no
/// unspecified aliasing behavior.
//= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
//# The aliasing discipline MUST be one the compiler applies internally to reclaim and reuse storage, rather than a use-counting obligation the program's author writes, so that a program's author states no aliasing annotation to be memory-safe.
//= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
//# A value MUST NOT be observably mutated through one reference while it is read through another in a way the executable semantics leaves unspecified.
/// What occurrence the escape walk keys on. A `let`/`Core::Param` binding is matched by its `StructId`
/// (`Binder`); a CLOSURE CAPTURE is referenced inside the closure body via `Core::Captured { index }` — a
/// slot index into the closure's capture list, NOT a binder — so it is matched by that INDEX (`Capture`).
/// One walk, two occurrence keys: the borrow-vs-escape classification (every non-base arm) is IDENTICAL,
/// only the base-case "is this THE occurrence" test differs. Used by the hcz capture-escape discriminator
/// ([`capture_escapes_via_body`]): a capture that escapes via the closure body's return needs a `dup` at
/// its `Core::Captured` read so the returned value owns an independent ref (else the monolithic cell-drop
/// double-frees it — the hcz1/hcz2 UAF). `Copy` so the 96 recursive forwards move nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EscapeTarget {
    /// A `let`/param binding, matched by its binder `StructId` (the existing binding-escape query).
    Binder(StructId),
    /// A closure capture, matched by its `Core::Captured { index }` slot index.
    Capture(usize),
    /// A specific EXTRACTION NODE (a `Core::SumPayload`/`Core::Proj` occurrence), matched by its own
    /// `StructId`. Unlike a binder/capture (referenced by later occurrences), an extraction node is used
    /// in-place; the query asks whether THAT node's value flows out (escapes) vs is borrowed — the
    /// SumPayload-escape twin of [`EscapeTarget::Capture`], driving [`collect_sumpayload_escape_dup_sites`]
    /// (the boundary-owned-scrutinee escape-retain, snowflake UAF).
    Node(StructId),
}

pub(super) fn binding_escapes(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    tail_borrowed: bool,
) -> bool {
    binding_escapes_dup_aware(db, id, EscapeTarget::Binder(binder), tail_borrowed, None)
}

/// Whether closure CAPTURE #`capture_index` ESCAPES the closure `body` via its return/a consuming use (vs is
/// only BORROWED) — the hcz capture-escape DISCRIMINATOR. Reuses [`binding_escapes_dup_aware`]'s exact
/// borrow-vs-escape walk, keyed on `Core::Captured { index }` occurrences ([`EscapeTarget::Capture`]) rather
/// than a binder. `true` = the capture flows out (the closure body returns it / passes it to a consuming
/// op), so its `Core::Captured` read MUST `dup` (retain +1) — the returned value then owns an independent
/// ref and the monolithic closure-cell drop frees only the cell's copy, so each ref frees exactly once (no
/// double-release, hcz1/hcz2). `false` = borrow-only (a `List.len`/field read of the capture, hcd1/hcd2) →
/// NO dup → the cell drop reclaims it once (v-memory-safety gate C: a borrow-only capture stays reclaimed,
/// by construction — the same borrow classification the binder query draws). No `dup_sites` (a fresh query).
// CONSUMED by `collect_captured_escape_dup_sites` (the hcz dup-on-escaping-captured-read gate).
pub(crate) fn capture_escapes_via_body(db: &mut Db, body: StructId, capture_index: usize) -> bool {
    binding_escapes_dup_aware(db, body, EscapeTarget::Capture(capture_index), false, None)
}

/// Whether the parameter `binder` ESCAPES (is consumed / flows out) the function `body`, vs is only
/// BORROWED. The exact escape query the reclaim discipline uses (via [`binding_escapes`], which handles a
/// `Core::Param` occurrence): `true` = the callee takes ownership (a consuming op / the result), so the
/// CALLER must NOT reclaim it; `false` = the callee only borrows it (a `byte-len` / content compare /
/// field read), so its OWNER reclaims it. The plain-export entry-param LIFT wrapper (`backend::wasm::mod`)
/// owns the value-heap value it lifts from `(ptr, len)`, so it must `drop` a lifted param the def only
/// borrows (`!param_escapes_body`) — the borrowed-owned-operand reclaim, at the boundary wrapper.
pub(crate) fn param_escapes_body(db: &mut Db, body: StructId, binder: StructId) -> bool {
    binding_escapes(db, body, binder, false)
}

/// The worker of [`binding_escapes`], with an optional `dup_sites` set. When `dup_sites` is `Some`, a
/// CONSUMING occurrence of `binder` that is a Perceus RETAIN site (in `dup_sites`) does NOT count as an
/// escape: the retain `dup`'d a fresh reference for the consuming op to take, leaving the binding's OWN
/// slot reference intact and owned — so the slot reference is DEAD after the body and MUST be reclaimed by
/// the enclosing `let` drop. Without this, a captured-then-inlined binding (`(mk-adder xs)` inlines to
/// `List.push xs …` beside `List.len xs`) `dup`'d its `xs` for the consuming push but the surviving slot
/// reference was never dropped (the consuming push read as an escape) — a value-correct LEAK. The rule the
/// drop sites apply: drop iff EVERY consuming occurrence is a dup_site (equivalently, `binding_escapes_dup_
/// aware(.., Some(dup_sites))` is false). SOUND across branches: only when every path's consume took a
/// `dup` does the slot reference survive on the taken path, and `mark_binder_dups` marks a consume a
/// dup_site iff `live_after` — so a LAST consume (no later use, `live_after=false`) is NOT a dup_site,
/// keeps escaping, and suppresses the drop → never a double-free. `None` reproduces the original
/// conservative "any non-borrow use escapes" analysis for the 79 other callers unchanged.
pub(super) fn binding_escapes_dup_aware(
    db: &mut Db,
    id: StructId,
    binder: EscapeTarget,
    tail_borrowed: bool,
    dup_sites: Option<&HashSet<StructId>>,
) -> bool {
    // FRONT-3 MEMO (v-memory-safety sign-off): the escape verdict is a pure function of
    // (id, binder, tail_borrowed) + the build-once-immutable Core graph — so memoize it, keyed by that FULL
    // context (tail_borrowed is the position-dependence, so it MUST be in the key; the full EscapeTarget too,
    // as Node/Binder/Capture are distinct verdicts). GATED on `dup_sites.is_none()` (the sumpayload/cont-
    // escape blowup path; the `Some` drop-site path neither reads nor writes the memo — no cross-
    // contamination). Linearizes the DAG-as-tree escape re-walk (db-query-diff / db-query-perfield front-3).
    // NO in_progress/tainted-withhold: the memo writes only AFTER the recursive call returns and the walk is
    // acyclic by spec (recursive defs refer to themselves via a static code ref, not a heap-value cycle; the
    // `Core::Call` arm recurses only into args, not the callee body), so no cycle-artifact can ever be cached
    // (see `Db::escape_verdict_memo`). The worker's recursion calls THIS wrapper, so nested queries memoize.
    if dup_sites.is_none()
        && let Some(&v) = db.escape_verdict_memo.get(&(id, binder, tail_borrowed))
    {
        return v;
    }
    let v = binding_escapes_dup_aware_inner(db, id, binder, tail_borrowed, dup_sites);
    if dup_sites.is_none() {
        db.escape_verdict_memo
            .insert((id, binder, tail_borrowed), v);
    }
    v
}

/// Whether the RESULT of the RestFrom extraction node `restfrom_node` (a `(.. r)` tail slice) ESCAPES —
/// reaches a CONSUMING position (a self-call arg / a persistent op / the result) — anywhere in `body`. The
/// 05:18721 PART-1 "rest-borrow-only" side-condition for the emit's RestFrom preservation-dup skip-gate
/// (emit.rs `Core::SumPayload` RestFrom arm, v-wasm-opt's site): the per-arm preservation dup may be skipped
/// (in a boundary-owned body, no sibling reads the handle after the vec-drop) ONLY when the rest is NOT
/// persistently consumed — i.e. this returns FALSE. If the rest IS consumed (`sum-l`'s tail threaded to the
/// self-call, `Option.expect`/ruf/AST-walker payload consumed), this returns TRUE and the dup is KEPT
/// (drift-safe by construction — never suppress a dup whose extracted value escapes). Reuses the `Node`-target
/// escape query (`EscapeTarget::Node`, the "this extraction node's value escapes" verdict).
// TEMP `allow`: consumed by v-wasm-opt's emit.rs RestFrom skip-gate (part-1 land removes the allow).
#[allow(dead_code)]
pub(super) fn restfrom_result_escapes(
    db: &mut Db,
    body: StructId,
    restfrom_node: StructId,
) -> bool {
    binding_escapes_dup_aware(db, body, EscapeTarget::Node(restfrom_node), false, None)
}

/// The unmemoized worker of [`binding_escapes_dup_aware`]. Its recursive `binding_escapes_dup_aware(...)`
/// calls resolve to the MEMOIZED wrapper above, so a shared subtree reached via many paths is computed once.
fn binding_escapes_dup_aware_inner(
    db: &mut Db,
    id: StructId,
    binder: EscapeTarget,
    tail_borrowed: bool,
    dup_sites: Option<&HashSet<StructId>>,
) -> bool {
    // NODE target: THIS extraction node's value escapes iff the walk reached it in a CONSUMING position
    // (`!tail_borrowed` — a parent `Proj`/borrow-op relaxes to `tail_borrowed`, a ctor-child/result/call-arg
    // keeps it consuming). Checked BEFORE the node-kind match so a `SumPayload`/`Proj` target is classified
    // by its OWN position (not descended into its scrutinee). Inert for `Binder`/`Capture` targets.
    if let EscapeTarget::Node(t) = binder
        && id == t
    {
        return !tail_borrowed && !dup_sites.is_some_and(|s| s.contains(&id));
    }
    // O(1) OCCURRENCE-ORACLE EARLY-PRUNE (same lever as `mark_binder_dups_inner`): a binder that PROVABLY
    // does not OCCUR in subtree `id` cannot ESCAPE from it (escape needs an occurrence), so short-circuit the
    // whole `core_of`-cloning descent. Sound + conservative — `binder_absent_in_subtree` returns true ONLY on
    // a DEFINITE all-zero occurrence bit (oracle live + node memoized); no-oracle / untracked / tainted-cyclic
    // → false → the normal walk runs (unchanged behaviour). This is the fix for the reclaim super-linear
    // blowup where `binding_escapes_dup_aware` re-descended binder-free subtrees per binder (db-query-diff
    // `cdz test` hang; profile: 27% self here after the `binder_occurs_rec` pre-pass fix). Binder-target only
    // (the oracle is per-binder; a `Capture`/`Node` target keeps the full walk).
    if let EscapeTarget::Binder(b) = binder
        && binder_absent_in_subtree(b, id)
    {
        return false;
    }
    match core_of(db, id) {
        // A reference to the binding: it escapes UNLESS this occurrence is a borrow (the operand of a
        // `Proj`, which `arr-get`-borrows). `tail_borrowed` is set by the `Proj` arm below for its
        // operand; every other occurrence (the result, a tuple element, a call arg) is consuming.
        // WARNING: A CONSUMING occurrence that is a Perceus retain (`dup_sites`) does NOT escape: the `dup`
        // gave the consuming op its own reference, so the binding's slot reference survives + must be
        // reclaimed by the `let` drop (see the fn doc). Only applies when `dup_sites` is `Some`.
        Core::LocalRef { binder: b } => {
            matches!(binder, EscapeTarget::Binder(t) if t == b)
                && !tail_borrowed
                && !dup_sites.is_some_and(|s| s.contains(&id))
        }
        // A reference to a PARAMETER, treated exactly like a `LocalRef` to a `let` binding: it escapes
        // unless this occurrence is a borrow (`tail_borrowed`) or a Perceus retain (`dup_sites`). Params are
        // referenced via `Core::Param` (not `LocalRef`), so without this arm a param-keyed escape query
        // (the owned-heap-param drop epilogue's `param_escapes_non_backedge`) would NEVER see the param's
        // own occurrences and wrongly report "never escapes" → drop a value that flows out → UAF. SAFE for
        // the ~80 let-binder callers: they pass a `let`-binding id, and a param occurrence's `b` is a
        // distinct param-binder id, so the `Binder(t) if t == b` guard is false there (this arm is inert).
        Core::Param { binder: b } => {
            matches!(binder, EscapeTarget::Binder(t) if t == b)
                && !tail_borrowed
                && !dup_sites.is_some_and(|s| s.contains(&id))
        }
        // A CLOSURE CAPTURE read (`Core::Captured { index }`, the `arr-get env slot` inside a closure body)
        // — the capture-escape twin of the two binder arms above, matched by slot INDEX rather than binder.
        // It escapes UNLESS this occurrence is a borrow (`tail_borrowed`, set by a `Proj`/`*.len`/etc. arm) —
        // the SAME borrow classification. Only fires for an `EscapeTarget::Capture(ci)` query and only when
        // this read is of THAT capture (`ci == index`); a `Binder` query never matches a `Captured` node, so
        // this arm is inert for every existing binder caller. Drives [`capture_escapes_via_body`] → the hcz
        // dup-on-escaping-captured-read gate. (No `dup_sites` interplay: the capture query passes `None`.)
        Core::Captured { index, .. } => {
            matches!(binder, EscapeTarget::Capture(ci) if ci == index)
                && !tail_borrowed
                && !dup_sites.is_some_and(|s| s.contains(&id))
        }
        // A projection of a SCALAR element BORROWS its operand — `arr-get` then `get-int`/`get-bool` COPIES
        // the value out, retaining nothing from the aggregate — so a `LocalRef` directly under such a `Proj`
        // does not escape through it (recurse with the borrow flag). But a projection of a NESTED-COMPOUND
        // element returns the CHILD HANDLE, a live reference INTO the aggregate that TRANSFERS OUT to the
        // consumer (a call arg, a constructor element, or the return); if the aggregate were then dropped,
        // that drop would cascade to free the extracted child — a use-after-free (the byte-decode `(let ((r
        // (one …))) (loop … (. r 0)))` threading a boxed-sum `(. r 0)` into a param returned garbage). So a
        // nested-compound projection ESCAPES its operand: the operand must NOT be reclaimed (its child left
        // through the projection). Conservative — the aggregate's array + its other children leak rather
        // than risk the UAF (the analysis's stated bias: a false "escapes" only leaks). `get_op(id)` is
        // `Some` for a scalar element (borrow), `None` for a nested-compound (escape). `List.len`/`Bytes.len`
        // (`vec-len`/`bytes-len`) read a scalar count — always a borrow. `String.scalar-len` likewise walks
        // its string buffer via borrowing `bytes-len`/`bytes-get` reads and returns a scalar count.
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        // `Blake3.of` BORROWS its Bytes operand (reads it, returns a FRESH hash — operand not retained).
        Core::Blake3Of { operand } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        // `Ast.print` (runtime) BORROWS its Ast operand (renders a fresh String — operand not retained).
        Core::AstPrint { operand, .. } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        // `Ast.encode` (runtime) BORROWS its Ast operand (serializes to a fresh Bytes — operand not retained).
        Core::AstEncode { operand, .. } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        // `Ast.decode` (runtime) BORROWS its Bytes operand (parses to a fresh Ast handle — operand not retained).
        Core::AstDecode { operand, .. } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        Core::Proj { operand, .. } => {
            // A compound projection is TRANSPARENT to borrowing: if THIS projection's own result is itself
            // borrowed by its parent (`tail_borrowed` — a deeper scalar `.field`/`.len` reads a scalar OUT
            // of this compound child), the extracted child handle is TRANSIENT and the operand is NOT
            // retained past it. So recurse borrowing when EITHER this is a scalar element OR the incoming
            // context already borrows (`scalar_element || tail_borrowed`). Without `|| tail_borrowed` a
            // SCALAR-BOTTOMED chain THROUGH compound intermediates (`(. (. (. a 1) 1) 1)`: a.1/a.1.1
            // compound, a.1.1.1 scalar) reset the borrow flag at each compound step → the operand `a` read
            // as ESCAPING. That mattered TWO ways for the dqe4-8 leak: (1) the let-epilogue drop was emitted
            // ONLY because the spurious `mark_binder_dups` dup put `a` in `dup_sites` (a fragile rescue that
            // the dup-suppression fix removes), and (2) the `never_escapes` gate on the dup-suppression
            // (`collect_dup_sites` below) uses THIS query with `dup_sites=None`, so without the fix a
            // borrow-only projected binder wrongly reads as escaping and the gate never fires. A GENUINE
            // child-handle escape (compound proj in a CONSUMING position, incoming `tail_borrowed` false) is
            // UNCHANGED (`false || false`), so it still escapes — no under-retain / UAF.
            let scalar_element = matches!(get_op(db, id), Ok(Some(_)));
            binding_escapes_dup_aware(
                db,
                operand,
                binder,
                scalar_element || tail_borrowed,
                dup_sites,
            )
        }
        // `List.at` BORROWS its list (`vec-len`/`vec-get` both borrow; the read element is DUP'd into the
        // `Some` payload rather than moved) — so a list bound here does not escape through `List.at`. The
        // index is a scalar. Recurse borrowing the list; the index cannot hold a heap reference.
        Core::ListAt { list, index, .. } => {
            binding_escapes_dup_aware(db, list, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, index, binder, false, dup_sites)
        }
        // `Bytes.at` BORROWS its bytes (`bytes-len`/`bytes-get` both borrow; the byte read is a raw i32
        // VALUE, not a heap handle, so nothing is retained from the sequence). The index is a scalar.
        Core::BytesAt { bytes, index, .. } => {
            binding_escapes_dup_aware(db, bytes, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, index, binder, false, dup_sites)
        }
        // `String.at` BORROWS its string — the `Some` branch `dup`s it before the `bytes-slice` consumes
        // the copy (so the returned slice owns an INDEPENDENT reference, not part of the source), and the
        // `None` branch takes no reference. So a binding used as the string operand does NOT escape through
        // `String.at` — the enclosing `let`/owner still reclaims it — exactly like `List.at`/`Bytes.at`.
        // The index is a scalar. (This borrow discipline is why `String.at` composes in a recursive char
        // scan that threads the same string through both `String.at` and the recursive call.)
        Core::StrAt { string, index, .. } => {
            binding_escapes_dup_aware(db, string, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, index, binder, false, dup_sites)
        }
        // `String.scalar-at` BORROWS its string operand (bytes-scalar-at reads the buffer, does not consume
        // it) — same discipline as `String.at`: the string binding does NOT escape, the index is a scalar.
        Core::StrScalarAt { operand, index, .. } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, index, binder, false, dup_sites)
        }
        // `String.slice` BORROWS its string operand (the Some branch `dup`s it before the consuming
        // `bytes-slice`, the None branch takes no reference — same discipline as `String.at`), so a binding
        // used as the string does NOT escape (its owner reclaims it). start/end are scalars.
        Core::StrSlice {
            string, start, end, ..
        } => {
            binding_escapes_dup_aware(db, string, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, start, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, end, binder, false, dup_sites)
        }
        // `Bytes.concat`/`slice`/`compact` all CONSUME their bytes operand(s) into the new sequence
        // (`bytes-concat`/`bytes-slice`/`bytes-compact` consume, per `value-heap-runtime.md §Constructors
        // Consume`). A binding used as an operand escapes into the result. `slice`'s start/len are scalars.
        Core::BytesConcat { lhs, rhs } => {
            binding_escapes_dup_aware(db, lhs, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, rhs, binder, false, dup_sites)
        }
        // The runtime BigInt ops BORROW their operand handles (`bigint-add`/…/`to-i64-checked` `unbox_
        // bigint`-read without consuming, then the `emit_bigint_borrow_*` helpers drop only an OWNED
        // temporary), so — like `value-eq` — a binding used DIRECTLY as an operand does NOT escape (the
        // enclosing `let` still drops it). A binding that flows into a CONSTRUCTED/owned operand (e.g.
        // `(+ (BigInt.of x) y)` where `x` feeds a `BigInt.of`) DOES escape into that owned temporary,
        // which the op then drops — the `tail_borrowed: true` borrow-in-tail computes exactly this (a
        // direct `LocalRef` borrows; a producer arm resets to consuming). `bigint-of-i64`'s operand is an
        // i64 scalar (no heap ref) — always consuming, `false`.
        Core::BigIntBinOp { lhs, rhs, .. } | Core::BigIntCmp { lhs, rhs, .. } => {
            binding_escapes_dup_aware(db, lhs, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, rhs, binder, true, dup_sites)
        }
        Core::BigIntOfI64 { value } => {
            binding_escapes_dup_aware(db, value, binder, false, dup_sites)
        }
        Core::BigIntToI64 { operand } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        // `Value.encode`/`decode` BORROW their operand: `value-encode` is an inspector (walks `v` to a fresh
        // owned doc); `value-decode` reads `bytes` to CONSTRUCT a fresh owned value. Neither retains a
        // reference to the operand in the result, so a binding used directly as the operand does NOT escape
        // through it (recurse `tail_borrowed: true`, like `BigIntToI64`/`ListLen`).
        Core::ValueEncode { value: operand, .. } | Core::ValueDecode { bytes: operand, .. } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        // `Char.to-int`'s operand is a `Char` (an i32 SCALAR code point, never a heap handle), so it cannot
        // retain a heap reference — a binding used as the operand does NOT escape (recurse `tail_borrowed:
        // true`, like the scalar-yielding `BigIntToI64`).
        Core::CharToInt { operand } | Core::IntToCharChecked { operand, .. } => {
            // The operand is a scalar (Char's i32 code point / Int64), never a heap handle, so it retains no
            // reference — recurse `tail_borrowed: true` (like the scalar-yielding `BigIntToI64`).
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        // The runtime Rational arithmetic/comparison ops BORROW their operand handles (`rational-add`/…/
        // `rational-cmp` unbox-read without consuming; the borrow helpers drop only an OWNED temporary), so
        // a binding used DIRECTLY as an operand does NOT escape (`tail_borrowed: true`, like the BigInt
        // arith). `RationalOfInts`'s num/den + `RationalOfIntWiden`'s value are i64 SCALARS (no heap ref) —
        // always consuming, `false`.
        Core::RationalBinOp { lhs, rhs, .. } | Core::RationalCmp { lhs, rhs, .. } => {
            binding_escapes_dup_aware(db, lhs, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, rhs, binder, true, dup_sites)
        }
        Core::RationalOfInts { num, den } => {
            binding_escapes_dup_aware(db, num, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, den, binder, false, dup_sites)
        }
        Core::RationalOfIntWiden { value } => {
            binding_escapes_dup_aware(db, value, binder, false, dup_sites)
        }
        // `rational-num`/`rational-den` BORROW the Rational operand (unbox-read without consuming),
        // returning a fresh BigInt handle — so a binding used directly as the operand does NOT escape.
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            binding_escapes_dup_aware(db, operand, binder, true, dup_sites)
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            binding_escapes_dup_aware(db, bytes, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, start, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, len, binder, false, dup_sites)
        }
        Core::BytesCompact { operand } => {
            binding_escapes_dup_aware(db, operand, binder, false, dup_sites)
        }
        // `String.from-bytes` CONSUMES its bytes operand (`str-from-bytes` transfers ownership out as the
        // String on success, drops it on failure), so a binding used as the operand escapes into the result.
        Core::StrFromBytes { bytes, .. } => {
            binding_escapes_dup_aware(db, bytes, binder, false, dup_sites)
        }
        // `String.to-bytes` CONSUMES its string operand (`bytes-compact` transfers the handle out as the
        // Bytes result), so a binding used as the operand escapes into the result.
        Core::StrToBytes { string } => {
            binding_escapes_dup_aware(db, string, binder, false, dup_sites)
        }
        // `str-nfc-normalize` CONSUMES its string operand (returns the same handle when already NFC, else a
        // fresh leaf with the original dropped), so a binding used as the operand escapes into the result.
        Core::NfcNormalize { string } => {
            binding_escapes_dup_aware(db, string, binder, false, dup_sites)
        }
        // A constructed tuple/list CONSUMES each element — a binding used as an element escapes into it.
        // `Bytes.of`'s elements are scalar bytes (Int64 0..=255), consumed into the sequence like a list's.
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => elems
            .iter()
            .any(|&e| binding_escapes_dup_aware(db, e, binder, false, dup_sites)),
        // A runtime `(bin …)` construction consumes each segment's scalar int value into the built bytes.
        Core::BinBuild { segs } => segs
            .iter()
            .any(|s| binding_escapes_dup_aware(db, s.value, binder, false, dup_sites)),
        // A runtime bit-field run consumes each field's scalar value (packed into the built bytes).
        Core::BinBitsBuild { fields } => fields
            .iter()
            .any(|f| binding_escapes_dup_aware(db, f.value, binder, false, dup_sites)),
        // A `BinIntRead` reads (borrows) its bytes operand to decode a segment: `bytes-get` COPIES each byte
        // out as a raw i32, retaining nothing from the sequence. A `BinRestRead` slices the tail but DUPs the
        // scrutinee first and slices the COPY (the original stays live). So a `LocalRef` used as the scrutinee
        // BORROWS through either — recurse with `tail_borrowed: true`, exactly like `Proj`/`ListLen`/`BytesAt`.
        // With `false` a bare `LocalRef` scrutinee was mis-marked ESCAPING, so the materializing `let`
        // (`lower_match_bin` wraps the runtime bin-match in `Core::Let{(scrutinee,scrutinee), if-chain}`)
        // skipped its closing drop → the owned `Bytes` scrutinee LEAKED one frame per match (value-correct —
        // a leak, not a miscompile; the fixed-width-only witness in
        // `dependent_size_bin_match_payload_read_leaves_no_live_objects`).
        // `off_plus` (§4a dynamic offset) is a scalar `BinIntRead` decode — it BORROWS its bytes and yields
        // an i64 count, carrying no heap reference out, so it cannot let the binding escape (recurse
        // `tail_borrowed: false`, like a scalar `len`).
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            binding_escapes_dup_aware(db, bytes, binder, true, dup_sites)
                || off_plus
                    .is_some_and(|op| binding_escapes_dup_aware(db, op, binder, false, dup_sites))
        }
        // A `BinSizedRead` borrows its bytes operand (DUP-then-`bytes-slice` the copy — the original survives,
        // like `BinRestRead`) and borrows its runtime length operand (a `BinIntRead` scalar read). The binding
        // escapes only if it flows into either as a NON-borrow, so recurse with `tail_borrowed: true` on the
        // bytes (borrowed) — the `len`/`off_plus` are scalar decodes that cannot carry a heap reference out.
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            binding_escapes_dup_aware(db, bytes, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, len, binder, false, dup_sites)
                || off_plus
                    .is_some_and(|op| binding_escapes_dup_aware(db, op, binder, false, dup_sites))
        }
        // `List.push`/`prepend`/`concat` CONSUME both operands (the persistent op takes ownership of the list
        // and the pushed/prepended/concatenated value into the result).
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            binding_escapes_dup_aware(db, list, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, elem, binder, false, dup_sites)
        }
        Core::ListConcat { lhs, rhs } => {
            binding_escapes_dup_aware(db, lhs, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, rhs, binder, false, dup_sites)
        }
        // `List.update` CONSUMES the list and the replacement element into the new list; the `index` is a
        // scalar (passed by value, never a heap handle) so it cannot escape into the result.
        Core::ListUpdate { list, elem, .. } => {
            binding_escapes_dup_aware(db, list, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, elem, binder, false, dup_sites)
        }
        // A map construction CONSUMES each entry's key AND value into the built map — a binding used as a
        // key or value escapes into it (like a tuple/list element).
        Core::MapNew { entries, .. } => entries.iter().any(|&(k, v)| {
            binding_escapes_dup_aware(db, k, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, v, binder, false, dup_sites)
        }),
        // `Map.insert` CONSUMES the map, the key, and the value into the new map (the persistent op takes
        // ownership of all three) — any of them used here escapes into the result.
        Core::MapInsert { map, key, val, .. } => {
            binding_escapes_dup_aware(db, map, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, key, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, val, binder, false, dup_sites)
        }
        // `Map.lookup` BORROWS the map (returns a fresh Option; the boxed key is an owned temporary the
        // emit drops), so a map bound here does NOT escape through the lookup. The key flows into an owned
        // temporary — consuming — so it escapes if used there.
        Core::MapLookup { map, key, .. } => {
            binding_escapes_dup_aware(db, map, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, key, binder, false, dup_sites)
        }
        // `Map.remove` CONSUMES the map into the new map (persistent op takes ownership); the key is boxed
        // into an owned temporary (consuming), dropped by the emit after the borrow-compare.
        Core::MapRemove { map, key, .. } => {
            binding_escapes_dup_aware(db, map, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, key, binder, false, dup_sites)
        }
        // `Map.size` BORROWS its map operand (`map-size` reads the root without consuming) — like `List.len`.
        Core::MapSize { map } => binding_escapes_dup_aware(db, map, binder, true, dup_sites),
        // A set construction CONSUMES each element into the built set — a binding used as an element
        // escapes into it (like a list element / a map key).
        Core::SetOf { elems, .. } => elems
            .iter()
            .any(|&e| binding_escapes_dup_aware(db, e, binder, false, dup_sites)),
        // `Set.insert` CONSUMES the set and the element into the new set (persistent op takes ownership) —
        // both escape if used here. `Set.remove` CONSUMES the set; its element is boxed into an owned
        // temporary (consuming), dropped by the emit after the borrow-compare.
        Core::SetInsert { set, elem, .. } | Core::SetRemove { set, elem, .. } => {
            binding_escapes_dup_aware(db, set, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, elem, binder, false, dup_sites)
        }
        // `Set.contains` BORROWS the set (returns a bool; the boxed element is an owned temporary the emit
        // drops), so a set bound here does NOT escape; the element flows into an owned temporary (consuming).
        Core::SetContains { set, elem, .. } => {
            binding_escapes_dup_aware(db, set, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, elem, binder, false, dup_sites)
        }
        // `Set.len` BORROWS its set operand (`set-size` reads the root without consuming) — like `Map.size`.
        Core::SetLen { set } => binding_escapes_dup_aware(db, set, binder, true, dup_sites),
        Core::SetToList { set, .. } => binding_escapes_dup_aware(db, set, binder, true, dup_sites),
        Core::MapToList { map, .. } => binding_escapes_dup_aware(db, map, binder, true, dup_sites),
        // A set-algebra op CONSUMES both operand sets into the result — either escapes if used here.
        Core::SetAlgebra { lhs, rhs, .. } => {
            binding_escapes_dup_aware(db, lhs, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, rhs, binder, false, dup_sites)
        }
        // A call CONSUMES its arguments; a host call OR a cross-component call likewise consumes its
        // arguments across the boundary.
        Core::Call { args, .. } | Core::HostCall { args, .. } => args
            .iter()
            .any(|&a| binding_escapes_dup_aware(db, a, binder, false, dup_sites)),
        // A sequencing block: the binding escapes if it escapes any statement or the tail.
        Core::Seq { stmts, tail } => {
            stmts
                .iter()
                .any(|&s| binding_escapes_dup_aware(db, s, binder, false, dup_sites))
                || binding_escapes_dup_aware(db, tail, binder, false, dup_sites)
        }
        // A boundary block / break — the binding escapes if it escapes the body / break value.
        Core::Block { body, .. } => binding_escapes_dup_aware(db, body, binder, false, dup_sites),
        Core::Break { value } => binding_escapes_dup_aware(db, value, binder, false, dup_sites),
        // Control flow: the binding escapes if it escapes any reachable sub-position.
        Core::If { cond, then_, else_ } => {
            binding_escapes_dup_aware(db, cond, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, then_, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, else_, binder, false, dup_sites)
        }
        Core::Match { scrutinee, arms } => {
            binding_escapes_dup_aware(db, scrutinee, binder, false, dup_sites)
                || arms.iter().any(|a| {
                    a.guard
                        .is_some_and(|g| binding_escapes_dup_aware(db, g, binder, false, dup_sites))
                        || binding_escapes_dup_aware(db, a.body, binder, false, dup_sites)
                })
        }
        Core::Let { bindings, body } => {
            bindings
                .iter()
                .any(|(_, v)| binding_escapes_dup_aware(db, *v, binder, false, dup_sites))
                || binding_escapes_dup_aware(db, body, binder, false, dup_sites)
        }
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => {
            binding_escapes_dup_aware(db, lhs, binder, false, dup_sites)
                || binding_escapes_dup_aware(db, rhs, binder, false, dup_sites)
        }
        // `value-eq` and `StrCmp` BORROW both operands (each drops only an OWNED temporary, never a
        // `LocalRef`), so a binding used DIRECTLY as an operand does NOT escape — the enclosing `let` still
        // drops it. A binding that flows into a CONSTRUCTED operand (`(= (Wrap x) …)`) DOES escape: it is
        // consumed into that owned temporary, which the op then drops (so the `let` must not double-drop).
        // The borrow-in-tail recursion (`tail_borrowed: true`) computes exactly this — a direct `LocalRef`
        // borrows, a constructor/call arm resets to consuming — mirroring the `Proj`/`ListLen` arm above.
        // StrCmp's operands are HEAP String/Symbol handles (a `let`-bound String CAN reach a StrCmp operand
        // as a direct `LocalRef`), so it MUST classify as borrow like `ValueEq`, NOT with the scalar
        // compares (whose operands are always scalars) — else a let-bound String operand is wrongly marked
        // escaping and the `let` skips its drop → a leak (the borrowing StrCmp left it to its owner).
        Core::ValueEq { lhs, rhs }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. } => {
            binding_escapes_dup_aware(db, lhs, binder, true, dup_sites)
                || binding_escapes_dup_aware(db, rhs, binder, true, dup_sites)
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            binding_escapes_dup_aware(db, operand, binder, false, dup_sites)
        }
        Core::Record { fields } => fields
            .values()
            .any(|&v| binding_escapes_dup_aware(db, v, binder, false, dup_sites)),
        // A sum construction CONSUMES each payload (it becomes part of the heap sum value).
        Core::SumNew { payloads, .. } => payloads
            .iter()
            .any(|&p| binding_escapes_dup_aware(db, p, binder, false, dup_sites)),
        // A sum match BORROWS its scrutinee — it reads the discriminant + payload (`sum-disc`/`sum-payload`)
        // WITHOUT consuming the shell, exactly like `SumPayload`/`SumExpect` (which recurse `tail_borrowed =
        // true`). The ONLY consume is the optional post-match shell reclaim, which fires solely for an OWNED
        // scrutinee (`sum_shell_reclaim_ok` → `heap_operand_ownership == Owned`); a let-bound binder
        // scrutinee is `Borrowed` (never reclaimed at the match), so a match of a let-bound sum used only in
        // borrow arms leaves the scrutinee for the enclosing `let` to drop. Recursing the scrutinee with
        // `false` (consume) WRONGLY marked such a binder as escaping and SUPPRESSED the `let`-drop — and the
        // match does not reclaim it either, so the whole sum graph leaked (`xop4`/`ruf*`: a fresh `Some`
        // matched twice was NEVER dropped). Borrow-classify the scrutinee so the `let`-drop reclaims it; a
        // payload that genuinely escapes an ARM is still caught by `cont_binding_escapes`.
        Core::MatchSum { scrutinee, root } => {
            binding_escapes_dup_aware(db, scrutinee, binder, true, dup_sites)
                || cont_binding_escapes(db, &root, binder, dup_sites)
        }
        // A list match: escapes if the binding escapes the scrutinee or any arm body. The SCRUTINEE position
        // is CONSUMING iff an arm REST-MINTS it (a `(.. r)` → `SumPayload` with a trailing `RestFrom` over the
        // scrutinee binder → a `vec-drop` that CONSUMES the spine), else it only BORROWS the scrutinee. Borrow-
        // classify (`true`) a bare-binding scrutinee NO arm rest-mints — a borrow-only list match (05:18721 `f`:
        // rest DEAD, arms return scalars) then does NOT mark its owned list escaping, so its owner (a caller
        // drop_after of a boundary-owned helper's arg, or the let-drop) reclaims it. A CONSUMING fold (`sum-l`:
        // `(.. t)` threaded into the tail-recursive call → `vec-drop` consumes the spine) keeps `false`
        // (consuming), so it is NOT mis-borrow-marked (which would double-free — the fold already reclaims the
        // spine). A payload/element that genuinely escapes an ARM is still caught by `arms.any(body)`.
        Core::MatchList { scrutinee, arms } => {
            let scrutinee_tail_borrowed = !matchlist_scrutinee_consumed(db, scrutinee, &arms);
            binding_escapes_dup_aware(db, scrutinee, binder, scrutinee_tail_borrowed, dup_sites)
                || arms
                    .iter()
                    .any(|a| binding_escapes_dup_aware(db, a.body, binder, false, dup_sites))
        }
        // A sum-payload read BORROWS the scrutinee (`sum-payload` reads without consuming), like a
        // projection operand — so a `LocalRef` reached through it does not escape.
        Core::SumPayload { scrutinee, .. } => {
            binding_escapes_dup_aware(db, scrutinee, binder, true, dup_sites)
        }
        // `expect` reads the scrutinee's payload (a borrow, like `SumPayload`) — a `LocalRef` reached
        // through it does not escape (the payload is unboxed/used in place, not moved out).
        Core::SumExpect { scrutinee, .. } => {
            binding_escapes_dup_aware(db, scrutinee, binder, true, dup_sites)
        }
        // A closure CONSUMES each captured value (it becomes part of the closure cell); a closure
        // application consumes both the closure value and its argument. (This increment's no-capture
        // closure has an empty `captures`, so it references no binding — but the arm is written for the
        // general case so a captured binding is correctly seen as escaping when captures land.)
        Core::Closure { captures, .. } => captures
            .iter()
            .any(|&c| binding_escapes_dup_aware(db, c, binder, false, dup_sites)),
        Core::CallClosure { closure, args } => {
            binding_escapes_dup_aware(db, closure, binder, false, dup_sites)
                || args
                    .iter()
                    .any(|&a| binding_escapes_dup_aware(db, a, binder, false, dup_sites))
        }
        // Leaves reference no binding. (`Core::Captured` is handled by its own arm ABOVE — it matches a
        // `Capture(index)` query and is inert for a `Binder` query, so it is NOT in this leaf group.) `trap`
        // diverges with no operand, so it holds no binding to escape.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
        | Core::Poison(_) => false,
    }
}

/// SOUND UNDER-APPROXIMATION of "the binder is CONSUMED on EVERY control-flow path through `id`" (must-escape),
/// used ONLY by the `Core::Proj` dup-suppression gate to distinguish an UNCONDITIONAL straight-line consume
/// (dqe7/8: `a` inserted into a `Map`/`Set` on every path — its consume has its OWN dup, so the compound-
/// projection keep-alive dup is SURPLUS → SUPPRESS) from a CONDITIONAL arm-escape (dqe17: `a` consumed only in
/// one `if`/match arm — the projection dup is NEEDED for the other path's reclaim → KEEP). Both are
/// `never_escapes == false` (they escape on SOME path), so the may-escape query alone cannot tell them apart;
/// this all-paths query is the discriminator.
///
/// Because a `true` verdict SUPPRESSES a dup (a WRONG `true` → under-retain → UAF), this MUST under-approximate.
/// It credits a consume ONLY on the STRAIGHT-LINE SPINE (constructors, collection ops, calls, and the
/// `let`/seq/block sequencing that is always evaluated); it does NOT descend ANY branch (`if`/match/list-match/
/// sum-match) — an arm-guarded consume is CONDITIONAL, so every branch node falls to `_ => false` (keep the
/// dup, the SAFE leak-not-UAF direction). The `tail_borrowed` flags for the arms handled here are copied
/// EXACTLY from [`binding_escapes_dup_aware`]'s arms; for a single-path (non-branch) node may-escape == must-
/// escape, so matching the trusted may classification is exact. Any arm not proven consuming-on-this-path
/// (borrow reads, numeric/compare ops, closures, and every branch) falls to `_ => false` — that only forgoes a
/// dup-suppression, it never suppresses a needed dup. Runs once per binder in `collect_dup_sites` (like the
/// `never_escapes` query); the O(1) occurrence-oracle prune bounds the walk.
fn binder_must_escape(db: &mut Db, id: StructId, binder: StructId, tail_borrowed: bool) -> bool {
    // A binder that PROVABLY does not occur in this subtree cannot be consumed in it — O(1) oracle prune
    // (the same lever as `binding_escapes_dup_aware_inner`; `false` = no oracle / not-memoized ⇒ full walk).
    if binder_absent_in_subtree(binder, id) {
        return false;
    }
    // This MIRRORS `binding_escapes_dup_aware_inner`'s per-node BORROW classification EXACTLY (same
    // `tail_borrowed` flags), so for a single-path (non-branch) node must-escape == may-escape and matching the
    // trusted may arms is precise. The ONLY divergence is at the FOUR branch nodes (`If`/`Match`/`MatchList`/
    // `MatchSum`): the may query ORs the arm/cont disjuncts (escapes on SOME path); the must query DROPS them
    // and credits ONLY the always-evaluated scrutinee/cond (an arm-guarded consume is conditional ⇒ not
    // must-escape). Dropping the arm disjuncts makes must STRICTLY ≤ may (the required soundness direction: a
    // wrong-small must-escape ⇒ keep the dup ⇒ leak-not-UAF). The `dup_sites`/`Node`-target machinery of the
    // may query is irrelevant here (must is always a `Binder` all-paths query), so it is omitted.
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => b == binder && !tail_borrowed,
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            binder_must_escape(db, operand, binder, true)
        }
        Core::Blake3Of { operand }
        | Core::AstPrint { operand, .. }
        | Core::AstEncode { operand, .. }
        | Core::AstDecode { operand, .. } => binder_must_escape(db, operand, binder, true),
        Core::Proj { operand, .. } => {
            let scalar_element = matches!(get_op(db, id), Ok(Some(_)));
            binder_must_escape(db, operand, binder, scalar_element || tail_borrowed)
        }
        Core::ListAt { list, index, .. } => {
            binder_must_escape(db, list, binder, true)
                || binder_must_escape(db, index, binder, false)
        }
        Core::BytesAt { bytes, index, .. } => {
            binder_must_escape(db, bytes, binder, true)
                || binder_must_escape(db, index, binder, false)
        }
        Core::StrAt { string, index, .. } => {
            binder_must_escape(db, string, binder, true)
                || binder_must_escape(db, index, binder, false)
        }
        Core::StrScalarAt { operand, index, .. } => {
            binder_must_escape(db, operand, binder, true)
                || binder_must_escape(db, index, binder, false)
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            binder_must_escape(db, string, binder, true)
                || binder_must_escape(db, start, binder, false)
                || binder_must_escape(db, end, binder, false)
        }
        Core::BytesConcat { lhs, rhs } => {
            binder_must_escape(db, lhs, binder, false) || binder_must_escape(db, rhs, binder, false)
        }
        Core::BigIntBinOp { lhs, rhs, .. } | Core::BigIntCmp { lhs, rhs, .. } => {
            binder_must_escape(db, lhs, binder, true) || binder_must_escape(db, rhs, binder, true)
        }
        Core::BigIntOfI64 { value } => binder_must_escape(db, value, binder, false),
        Core::BigIntToI64 { operand } => binder_must_escape(db, operand, binder, true),
        Core::ValueEncode { value: operand, .. } | Core::ValueDecode { bytes: operand, .. } => {
            binder_must_escape(db, operand, binder, true)
        }
        Core::CharToInt { operand } | Core::IntToCharChecked { operand, .. } => {
            binder_must_escape(db, operand, binder, true)
        }
        Core::RationalBinOp { lhs, rhs, .. } | Core::RationalCmp { lhs, rhs, .. } => {
            binder_must_escape(db, lhs, binder, true) || binder_must_escape(db, rhs, binder, true)
        }
        Core::RationalOfInts { num, den } => {
            binder_must_escape(db, num, binder, false) || binder_must_escape(db, den, binder, false)
        }
        Core::RationalOfIntWiden { value } => binder_must_escape(db, value, binder, false),
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            binder_must_escape(db, operand, binder, true)
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            binder_must_escape(db, bytes, binder, false)
                || binder_must_escape(db, start, binder, false)
                || binder_must_escape(db, len, binder, false)
        }
        Core::BytesCompact { operand } => binder_must_escape(db, operand, binder, false),
        Core::StrFromBytes { bytes, .. } => binder_must_escape(db, bytes, binder, false),
        Core::StrToBytes { string } => binder_must_escape(db, string, binder, false),
        Core::NfcNormalize { string } => binder_must_escape(db, string, binder, false),
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => elems
            .iter()
            .any(|&e| binder_must_escape(db, e, binder, false)),
        Core::BinBuild { segs } => segs
            .iter()
            .any(|s| binder_must_escape(db, s.value, binder, false)),
        Core::BinBitsBuild { fields } => fields
            .iter()
            .any(|f| binder_must_escape(db, f.value, binder, false)),
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            binder_must_escape(db, bytes, binder, true)
                || off_plus.is_some_and(|op| binder_must_escape(db, op, binder, false))
        }
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            binder_must_escape(db, bytes, binder, true)
                || binder_must_escape(db, len, binder, false)
                || off_plus.is_some_and(|op| binder_must_escape(db, op, binder, false))
        }
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            binder_must_escape(db, list, binder, false)
                || binder_must_escape(db, elem, binder, false)
        }
        Core::ListConcat { lhs, rhs } => {
            binder_must_escape(db, lhs, binder, false) || binder_must_escape(db, rhs, binder, false)
        }
        Core::ListUpdate { list, elem, .. } => {
            binder_must_escape(db, list, binder, false)
                || binder_must_escape(db, elem, binder, false)
        }
        Core::MapNew { entries, .. } => entries.iter().any(|&(k, v)| {
            binder_must_escape(db, k, binder, false) || binder_must_escape(db, v, binder, false)
        }),
        Core::MapInsert { map, key, val, .. } => {
            binder_must_escape(db, map, binder, false)
                || binder_must_escape(db, key, binder, false)
                || binder_must_escape(db, val, binder, false)
        }
        Core::MapLookup { map, key, .. } => {
            binder_must_escape(db, map, binder, true) || binder_must_escape(db, key, binder, false)
        }
        Core::MapRemove { map, key, .. } => {
            binder_must_escape(db, map, binder, false) || binder_must_escape(db, key, binder, false)
        }
        Core::MapSize { map } => binder_must_escape(db, map, binder, true),
        Core::SetOf { elems, .. } => elems
            .iter()
            .any(|&e| binder_must_escape(db, e, binder, false)),
        Core::SetInsert { set, elem, .. } | Core::SetRemove { set, elem, .. } => {
            binder_must_escape(db, set, binder, false)
                || binder_must_escape(db, elem, binder, false)
        }
        Core::SetContains { set, elem, .. } => {
            binder_must_escape(db, set, binder, true) || binder_must_escape(db, elem, binder, false)
        }
        Core::SetLen { set } => binder_must_escape(db, set, binder, true),
        Core::SetToList { set, .. } => binder_must_escape(db, set, binder, true),
        Core::MapToList { map, .. } => binder_must_escape(db, map, binder, true),
        Core::SetAlgebra { lhs, rhs, .. } => {
            binder_must_escape(db, lhs, binder, false) || binder_must_escape(db, rhs, binder, false)
        }
        Core::Call { args, .. } | Core::HostCall { args, .. } => args
            .iter()
            .any(|&a| binder_must_escape(db, a, binder, false)),
        Core::Seq { stmts, tail } => {
            stmts
                .iter()
                .any(|&s| binder_must_escape(db, s, binder, false))
                || binder_must_escape(db, tail, binder, false)
        }
        Core::Block { body, .. } => binder_must_escape(db, body, binder, false),
        Core::Break { value } => binder_must_escape(db, value, binder, false),
        // BRANCH: the cond is ALWAYS evaluated ⇒ credit it (may==must). The then/else arms are CONDITIONAL ⇒
        // DROP their disjuncts (an arm-only escape is not a must-escape) — this is the dqe17-preserving
        // under-approximation.
        Core::If { cond, .. } => binder_must_escape(db, cond, binder, false),
        // BRANCH: credit the always-evaluated scrutinee ONLY; the arm bodies + guards are conditional ⇒ dropped.
        Core::Match { scrutinee, .. } => binder_must_escape(db, scrutinee, binder, false),
        Core::Let { bindings, body } => {
            bindings
                .iter()
                .any(|(_, v)| binder_must_escape(db, *v, binder, false))
                || binder_must_escape(db, body, binder, false)
        }
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => {
            binder_must_escape(db, lhs, binder, false) || binder_must_escape(db, rhs, binder, false)
        }
        Core::ValueEq { lhs, rhs }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. } => {
            binder_must_escape(db, lhs, binder, true) || binder_must_escape(db, rhs, binder, true)
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            binder_must_escape(db, operand, binder, false)
        }
        Core::Record { fields } => fields
            .values()
            .any(|&v| binder_must_escape(db, v, binder, false)),
        Core::SumNew { payloads, .. } => payloads
            .iter()
            .any(|&p| binder_must_escape(db, p, binder, false)),
        // BRANCH: a sum match BORROWS its scrutinee (recurse `true`, like the may arm); the continuation arms
        // are conditional ⇒ dropped.
        Core::MatchSum { scrutinee, .. } => binder_must_escape(db, scrutinee, binder, true),
        // BRANCH: credit the scrutinee ONLY (same `false` flag as the may arm); the arm bodies are dropped.
        Core::MatchList { scrutinee, .. } => binder_must_escape(db, scrutinee, binder, false),
        Core::SumPayload { scrutinee, .. } | Core::SumExpect { scrutinee, .. } => {
            binder_must_escape(db, scrutinee, binder, true)
        }
        Core::Closure { captures, .. } => captures
            .iter()
            .any(|&c| binder_must_escape(db, c, binder, false)),
        Core::CallClosure { closure, args } => {
            binder_must_escape(db, closure, binder, false)
                || args
                    .iter()
                    .any(|&a| binder_must_escape(db, a, binder, false))
        }
        // `Core::Captured` reads a closure env slot by INDEX, never a `Binder` id ⇒ inert for this query.
        Core::Captured { .. }
        | Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
        | Core::Poison(_) => false,
    }
}

/// Whether a `Core::MatchList`'s scrutinee is CONSUMED (not merely borrowed) by its arms — true iff the
/// scrutinee is a bare binding an arm REST-MINTS (a `(.. r)` → a `SumPayload` whose path's last step is a
/// `RestFrom` over the scrutinee binder → a `vec-drop` that CONSUMES the scrutinee spine, returning the fresh
/// tail). A match with no such rest-mint only BORROWS its scrutinee (reads len/elements). A NON-ref scrutinee
/// (a producer) is conservatively CONSUMED (a fresh owned scrutinee is consumed/reclaimed by the match; the
/// borrow-relaxation only helps a param/let scrutinee). Used by the `MatchList` arm of
/// [`binding_escapes_dup_aware_inner`] so a borrow-only list match (05:18721 `f`) borrow-classifies its
/// scrutinee while a consuming fold (`sum-l`) keeps consuming.
fn matchlist_scrutinee_consumed(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
) -> bool {
    let sb = match core_of(db, scrutinee) {
        Core::Param { binder } | Core::LocalRef { binder } => binder,
        _ => return true, // non-ref scrutinee → keep the conservative consuming classification
    };
    let bodies: Vec<StructId> = arms.iter().map(|a| a.body).collect();
    bodies
        .into_iter()
        .any(|b| body_rest_mints_binder(db, b, sb, &mut HashSet::new()))
}

/// Whether subtree `id` REST-MINTS binder `sb` — contains a `SumPayload` whose path's last step is a
/// `RestFrom` and whose scrutinee is a direct ref to `sb` (the `(.. r)` tail-extraction that `vec-drop`-
/// CONSUMES `sb`'s spine). Recurses all children (cycle-guarded).
fn body_rest_mints_binder(
    db: &mut Db,
    id: StructId,
    sb: StructId,
    seen: &mut HashSet<StructId>,
) -> bool {
    if !seen.insert(id) {
        return false;
    }
    if let Core::SumPayload { scrutinee, path } = core_of(db, id)
        && matches!(path.last(), Some(crate::core::PathStep::RestFrom(_)))
        && matches!(
            core_of(db, scrutinee),
            Core::Param { binder } | Core::LocalRef { binder } if binder == sb
        )
    {
        return true;
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| body_rest_mints_binder(db, c, sb, seen))
}

/// Whether `binder` reaches `arm`'s RESULT exclusively as a MOVE-OUT (the arm's result IS the binder) or as
/// the IN-PLACE-REUSED BASE of a persistent builder (`List.push`/`prepend`/`update`, `Map.insert`/`remove`,
/// `Set.insert`/`remove`) — the shapes where the builder REUSES the base's storage IN PLACE at rc==1, so the
/// base's slot BECOMES the result (`keep`). This is the SOUNDNESS FENCE on the if-join-shared-child dup-skip
/// (v-memory-safety co-verify, the 980 ROPE UAF): the cross-arm dup may be skipped ONLY when EVERY consuming
/// arm reuses the binder in place, because then the binder's post-if scope drop IS exactly `keep`'s reclaim
/// (one balanced drop). For a FRESH-ALLOCATING consume — `String.concat`→`bytes-concat`, `List.concat`, a
/// tuple/list/map/set/sum/record ctor, a call — the binder becomes a distinct CHILD cell of `keep`, so its
/// own scope drop DOUBLE-FREES the cell `keep` still references (the 980 rope mode-2 `unreachable` trap; #5352
/// wrongly reached it). Conservative: any shape not PROVEN reuse-base/move-out returns `false` (keep the dup
/// → at worst the pre-#5352 benign known-leak, never a double-free). The builder arms also verify the binder
/// does NOT ALSO escape through a CHILD operand (elem/key/val), which would make it simultaneously a child.
pub(super) fn escapes_as_reuse_base_or_moveout(
    db: &mut Db,
    arm: StructId,
    binder: StructId,
) -> bool {
    match core_of(db, arm) {
        // MOVE-OUT: the arm's result IS the binder (it flows out unchanged as the join value).
        Core::LocalRef { binder: b } | Core::Param { binder: b } => b == binder,
        // IN-PLACE-REUSE builders: the binder must be the reused BASE (recurse — a nested builder chain on
        // the binder still reuses its slot) AND must NOT also escape through the CHILD operand (elem/key/val),
        // else it is simultaneously a fresh-alloc child → hazard → keep the dup.
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            escapes_as_reuse_base_or_moveout(db, list, binder)
                && !binding_escapes(db, elem, binder, false)
        }
        Core::ListUpdate { list, elem, .. } => {
            escapes_as_reuse_base_or_moveout(db, list, binder)
                && !binding_escapes(db, elem, binder, false)
        }
        Core::MapInsert { map, key, val, .. } => {
            escapes_as_reuse_base_or_moveout(db, map, binder)
                && !binding_escapes(db, key, binder, false)
                && !binding_escapes(db, val, binder, false)
        }
        Core::MapRemove { map, key, .. } => {
            escapes_as_reuse_base_or_moveout(db, map, binder)
                && !binding_escapes(db, key, binder, false)
        }
        Core::SetInsert { set, elem, .. } | Core::SetRemove { set, elem, .. } => {
            escapes_as_reuse_base_or_moveout(db, set, binder)
                && !binding_escapes(db, elem, binder, false)
        }
        // Let/Seq/Block/Break: the binder must reach the TAIL as reuse-base/move-out AND must NOT escape into
        // any bound value / earlier statement (that would consume it into a SIDE node — a child hazard).
        Core::Let { bindings, body } => {
            escapes_as_reuse_base_or_moveout(db, body, binder)
                && !bindings
                    .iter()
                    .any(|(_, v)| binding_escapes(db, *v, binder, false))
        }
        Core::Seq { stmts, tail } => {
            escapes_as_reuse_base_or_moveout(db, tail, binder)
                && !stmts.iter().any(|&s| binding_escapes(db, s, binder, false))
        }
        Core::Block { body, .. } => escapes_as_reuse_base_or_moveout(db, body, binder),
        Core::Break { value } => escapes_as_reuse_base_or_moveout(db, value, binder),
        // Everything else (concat, ctors, calls, if/match joins, borrowing reads, …) is NOT a proven in-place
        // reuse of the binder → conservative `false` (keep the dup).
        _ => false,
    }
}

/// Whether `binder`, reaching the RESULT of `init` on SOME control-flow path, does so as a MOVE-OUT or an
/// IN-PLACE-REUSED builder BASE ([`escapes_as_reuse_base_or_moveout`]) — descending `if`/`match` branch
/// splits (either arm counts) and Let/Seq/Block tails. The D1 DROP-ELIDE distinguisher: a binder consumed
/// into a later sibling initializer may be elided ONLY when it is a FRESH-ALLOC CHILD on EVERY path (its
/// slot is dead — the concat-child D1 shape); if on ANY path it is instead moved out or reused-in-place as
/// the base (so its slot BECOMES the later binding, e.g. LIST `l2 = push l1` where `l1`'s slot is the
/// FBIP-reused survivor, or the 980 then-arm `keep = if c r1 (concat r1 …)` move-out), its slot is LIVE and
/// the existing scope drop is its sole reclaim — do NOT elide (eliding leaks: LIST clean-0 → 2, 980 0 → 1).
pub(super) fn binder_reuses_or_moves_on_some_path(
    db: &mut Db,
    init: StructId,
    binder: StructId,
) -> bool {
    match core_of(db, init) {
        Core::If { then_, else_, .. } => {
            binder_reuses_or_moves_on_some_path(db, then_, binder)
                || binder_reuses_or_moves_on_some_path(db, else_, binder)
        }
        Core::Match { arms, .. } => arms
            .iter()
            .any(|a| binder_reuses_or_moves_on_some_path(db, a.body, binder)),
        Core::Let { body, .. } => binder_reuses_or_moves_on_some_path(db, body, binder),
        Core::Seq { tail, .. } => binder_reuses_or_moves_on_some_path(db, tail, binder),
        Core::Block { body, .. } => binder_reuses_or_moves_on_some_path(db, body, binder),
        // A non-branch result position: the binder reuses/moves iff it is the reuse-base/move-out here.
        _ => escapes_as_reuse_base_or_moveout(db, init, binder),
    }
}

/// Whether the loop-rebind VALUE `arg` PROVABLY produces a FRESH heap CELL (a distinct new allocation that is
/// not itself the old accumulator's cell) — the SOUND sufficient condition for the borrowed-accumulator drop
/// (`drop_old_borrowed` in `emit_loop_iteration`). TWO fresh-producing shapes:
///   • a runtime `rational-add`/`bigint-*` (RationalBinOp/BigIntBinOp) — computes a NEW Rational/BigInt (fresh
///     num/den or magnitude) from its operands' VALUES; never returns or embeds an operand's cell, so once it
///     has read the old accumulator that accumulator is genuinely dead (harmonic/Rational/BigInt folds → clean).
///   • a fresh COMPOUND-PRODUCT CTOR (`Core::Tuple`/`ListNew`/`Record`/`BytesOf`) — `arr-alloc`s a brand-new
///     cell. A RECURSIVE tuple/record/list-STATE handler rebinds its loop state slot to such a ctor each
///     iteration; the numeric-only gate saw the fresh ctor as non-fresh → left the OLD state shell undropped =
///     one leak per perform (v-effects confirmed via wasm dump on `/tmp/rectuple_tail`: a self-loop that
///     arr-allocs the new `#tuple` + rebinds the state local with NO free of the old shell — exactly this gap).
/// SOUNDNESS of the ctor arm rests ENTIRELY on the CALLER's conjunctive escape guard at the drop site
/// (`!binding_escapes(arg, binder)` for EVERY rebind arg). A ctor that would cascade-free a carried cell always
/// makes the old accumulator ESCAPE through it, so the guard blocks the drop BEFORE this fn is consulted:
///   • a ctor EMBEDDING a HEAP CHILD of the old accumulator (`v2max`→`Vec2.V2` reusing a Rational child; CSG
///     `fuse`→`Solid.Union` with the accumulator as a payload child) — the child extraction is a nested-compound
///     `Proj` (`get_op` None) in the ctor's CONSUMING element position → `binding_escapes` = true → drop BLOCKED;
///   • a ctor FBIP-REUSING the old cell consumes the accumulator → `binding_escapes` = true → drop BLOCKED;
///   • the `bb (Diff l _t) => bb l` child-carry is a bare payload rebind (not a product ctor) → still `false` here.
/// Only a ctor whose operands read the old accumulator via BORROWING scalar projections (`get_op` Some — the
/// scalar is COPIED, no heap child shared) passes BOTH the escape guard and this arm, so its old shell is truly
/// dead (14b-tuple: field 0 a fresh arith over unboxed scalars, field 1 an unboxed-scalar projection).
/// Following Let/Seq/Block tails; conservative `false` for everything else (keep the drop OFF → at worst the
/// pre-fix benign leak, never a cascade double-free). Sum ctors (`SumNew`) deferred to a separately-verified
/// follow-up (the excluded share-hazards above are sum ctors; the escape guard would cover them, but the
/// sum-state leak wants its own before/after pin).
pub(super) fn rebind_produces_fresh(db: &mut Db, arg: StructId) -> bool {
    match core_of(db, arg) {
        Core::RationalBinOp { .. } | Core::BigIntBinOp { .. } => true,
        // A fresh product-compound ctor — a distinct new cell (see the fn doc; the caller's escape guard
        // excludes any ctor that embeds/reuses a heap child of the old accumulator).
        Core::Tuple { .. } | Core::ListNew { .. } | Core::Record { .. } | Core::BytesOf { .. } => {
            true
        }
        Core::Let { body, .. } => rebind_produces_fresh(db, body),
        Core::Seq { tail, .. } => rebind_produces_fresh(db, tail),
        Core::Block { body, .. } => rebind_produces_fresh(db, body),
        _ => false,
    }
}

/// Whether `binder` escapes through a sum-match CONTINUATION — a leaf's body, or a nested switch's arms
/// (each recursed). The `Payload`/`Elem` path steps are heap reads that carry no binding, so only the arm
/// continuations matter (mirrors the `MatchSum` arm walk in `binding_escapes`).
pub(super) fn cont_binding_escapes(
    db: &mut Db,
    cont: &crate::core::SumCont,
    binder: EscapeTarget,
    dup_sites: Option<&HashSet<StructId>>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            binding_escapes_dup_aware(db, *body, binder, false, dup_sites)
        }
        // A guarded arm's binder can escape through either the guarded body or the fall-through
        // continuation (the guard cond only reads, never escapes a binding).
        crate::core::SumCont::Guarded { body, els, .. } => {
            binding_escapes_dup_aware(db, *body, binder, false, dup_sites)
                || cont_binding_escapes(db, els, binder, dup_sites)
        }
        // A literal test's binder can escape through either continuation (the `path` walk only reads).
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_binding_escapes(db, then_, binder, dup_sites)
                || cont_binding_escapes(db, els, binder, dup_sites)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| cont_binding_escapes(db, &a.cont, binder, dup_sites)),
    }
}

/// Whether `id` is a chain of `Core::Proj`/`Core::SumPayload`/`Core::SumExpect` extractions ultimately
/// rooted at the NODE `root` (`id == root`) — the node-keyed twin of `payload_or_proj_chain_roots_at_binder`
/// (which bottoms at a `LocalRef`/`Param`). A match PAYLOAD binder lowers to `Core::SumPayload { scrutinee:
/// <the-match-scrutinee-node-id> }`, so a payload use is a chain whose ROOT is the scrutinee's own node id —
/// not a `LocalRef`. Used by the wrapper-scrutinee shell-drop's consumed-child analysis.
pub(super) fn payload_proj_chain_roots_at_node(db: &mut Db, id: StructId, root: StructId) -> bool {
    if id == root {
        return true;
    }
    match core_of(db, id) {
        Core::Proj { operand, .. }
        | Core::SumPayload {
            scrutinee: operand, ..
        }
        | Core::SumExpect {
            scrutinee: operand, ..
        } => payload_proj_chain_roots_at_node(db, operand, root),
        _ => false,
    }
}

/// Collect every `Core::SumPayload`/`Core::Proj` OCCURRENCE (by node id) in the sum-match rooted at
/// `cont` that (a) is a chain rooted at the scrutinee NODE `scrut`, (b) extracts a COMPOUND heap leaf
/// (`get_op` None — a real handle, not an unboxed scalar), and (c) sits in a CONSUMING position. These
/// are the child extractions that MOVE a handle out of the scrutinee shell; to let the shell be deep-
/// `drop`'d after the match WITHOUT double-freeing a moved-out child, each such occurrence is `dup`'d
/// (inserted into `dup_sites`) so the shell keeps its own reference. Computed in the UPFRONT dup-sites
/// pass (NOT mid-emit — the emit's `Core::SumPayload` child-dup reads `dup_sites`, and `collect_used_ops`
/// reads the same set to decide the `dup` IMPORT, so the two must agree before emit runs). The RECORDING
/// twin of [`sum_payload_child_escapes_cont`] (same borrow/consume classification; see it for semantics).
pub(super) fn collect_consuming_payload_sites_cont(
    db: &mut Db,
    cont: &crate::core::SumCont,
    scrut: StructId,
    out: &mut HashSet<StructId>,
) {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            collect_consuming_payload_sites_expr(db, *body, scrut, true, out)
        }
        crate::core::SumCont::Guarded { body, els, .. } => {
            collect_consuming_payload_sites_expr(db, *body, scrut, true, out);
            collect_consuming_payload_sites_cont(db, els, scrut, out);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            collect_consuming_payload_sites_cont(db, then_, scrut, out);
            collect_consuming_payload_sites_cont(db, els, scrut, out);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms {
                collect_consuming_payload_sites_cont(db, &a.cont, scrut, out);
            }
        }
    }
}

/// The expression walker for [`collect_consuming_payload_sites_cont`] — same borrow/consume classification
/// as [`sum_payload_child_escapes_expr`], but INSERTS each consuming compound-leaf scrutinee-child site
/// into `out` rather than returning a bool.
pub(super) fn collect_consuming_payload_sites_expr(
    db: &mut Db,
    id: StructId,
    scrut: StructId,
    consuming: bool,
    out: &mut HashSet<StructId>,
) {
    // MEMO (v-memory-safety sign-off, front-3 sibling): each subtree's contributed site-set is a pure fn of
    // (id, scrut, consuming) + the immutable graph (no sibling live_after, no read of `out`), so memoize it.
    // Cache each node's OWN COMPLETE contribution computed into a FRESH accumulator (NOT a delta vs the shared
    // `out` — that would capture sibling inserts + corrupt the cache); splice on a hit via `out.extend`
    // (equivalent to re-walking since the walk's only effect is idempotent, order-independent set-inserts).
    // Linearizes the ~643× DAG-as-tree re-walk (the cont-walker calls this on the SAME shared scrutinee-expr
    // per arm). NO in_progress/tainted — acyclic by spec (Core::Call recurses only into args) + the memo
    // writes only AFTER the recursion returns, so no cycle-artifact can be cached (see `Db::payload_sites_memo`).
    // The inner worker's recursion calls THIS wrapper, so nested/shared subtrees memoize.
    if let Some(cached) = db.payload_sites_memo.get(&(id, scrut, consuming)) {
        out.extend(cached.iter().copied());
        return;
    }
    let mut own: HashSet<StructId> = HashSet::new();
    collect_consuming_payload_sites_expr_inner(db, id, scrut, consuming, &mut own);
    db.payload_sites_memo
        .insert((id, scrut, consuming), own.iter().copied().collect());
    out.extend(own);
}

/// The unmemoized worker of [`collect_consuming_payload_sites_expr`]. Its recursive
/// `collect_consuming_payload_sites_expr(...)` calls resolve to the MEMOIZED wrapper above, so a shared
/// scrutinee-subtree reached via many continuation arms is computed once and spliced thereafter.
fn collect_consuming_payload_sites_expr_inner(
    db: &mut Db,
    id: StructId,
    scrut: StructId,
    consuming: bool,
    out: &mut HashSet<StructId>,
) {
    if payload_proj_chain_roots_at_node(db, id, scrut) {
        let scalar_leaf = matches!(get_op(db, id), Ok(Some(_)));
        if consuming && !scalar_leaf {
            out.insert(id);
        }
        return;
    }
    match core_of(db, id) {
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            collect_consuming_payload_sites_expr(db, operand, scrut, false, out)
        }
        Core::Proj { operand, .. } => {
            let scalar_element = matches!(get_op(db, id), Ok(Some(_)));
            collect_consuming_payload_sites_expr(db, operand, scrut, !scalar_element, out)
        }
        Core::ListAt { list, index, .. }
        | Core::BytesAt {
            bytes: list, index, ..
        } => {
            collect_consuming_payload_sites_expr(db, list, scrut, false, out);
            collect_consuming_payload_sites_expr(db, index, scrut, false, out);
        }
        Core::StrAt { string, index, .. } => {
            collect_consuming_payload_sites_expr(db, string, scrut, false, out);
            collect_consuming_payload_sites_expr(db, index, scrut, false, out);
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            collect_consuming_payload_sites_expr(db, string, scrut, false, out);
            collect_consuming_payload_sites_expr(db, start, scrut, false, out);
            collect_consuming_payload_sites_expr(db, end, scrut, false, out);
        }
        // BORROWING compares/ops (mirror `binding_escapes` arm-for-arm): each reads both operands in place
        // (dropping only an OWNED temporary), so a scrutinee-child reached DIRECTLY as an operand is BORROWED,
        // never moved out — descend with `consuming=false` so it is NOT marked a consumed-child dup site.
        // `Core::ValueCmp` (the runtime compound-ordering `<`/`<=`/`>`/`>=` walk — the blessed heap-order op)
        // borrows both operands exactly like `Core::ValueEq`; without it here a shell child compared via `<`
        // in a match arm fell to the `_ =>` CONSUMING fallback and was wrongly marked a dup site (the deep
        // shell drop happened to absorb the over-dup in the cases tested, but the divergence from
        // `binding_escapes` is a latent over-retain — keep the two walks identical).
        Core::ValueEq { lhs, rhs }
        | Core::StrCmp { lhs, rhs, .. }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. }
        | Core::BigIntBinOp { lhs, rhs, .. }
        | Core::RationalBinOp { lhs, rhs, .. } => {
            collect_consuming_payload_sites_expr(db, lhs, scrut, false, out);
            collect_consuming_payload_sites_expr(db, rhs, scrut, false, out);
        }
        // `Map.lookup`/`Set.contains` BORROW BOTH operands (core.rs: "the boxed key/elem is dropped after") —
        // a HEAP key/elem (a Bytes/String/compound handle) is read in place for the hash+compare and retained
        // by NOTHING (no box for an already-heap key), so a scrutinee-child reached AS the key/elem is BORROWED,
        // never moved out → descend with `consuming=false`, exactly like the `value-eq`/`value-cmp` compare
        // operands above. (The prior `consuming=true` here over-marked such a probe key as a consumed-child dup
        // site → the shell-reclaim child-dup was UNBALANCED → a 1-cell leak on the slice-view-as-CHAMP-key
        // BORROWED-PROBE, 13-strings:1408; the op only borrows the key, so no dup is owed — seam (b) of the
        // CHAMP-key reclaim, v-mem-safety-co-designed.) A SCALAR key copies out (get-int) → not a heap child →
        // unaffected by the flag.
        Core::MapLookup { map, key, .. } => {
            collect_consuming_payload_sites_expr(db, map, scrut, false, out);
            collect_consuming_payload_sites_expr(db, key, scrut, false, out);
        }
        Core::MapSize { map } => collect_consuming_payload_sites_expr(db, map, scrut, false, out),
        Core::SetContains { set, elem, .. } => {
            collect_consuming_payload_sites_expr(db, set, scrut, false, out);
            collect_consuming_payload_sites_expr(db, elem, scrut, false, out);
        }
        // `Map.remove`/`Set.remove` BORROW the key/elem (core.rs) but CONSUME the collection: descend the
        // key/elem BORROWED (false) and the collection CONSUMING (true, safe default — a slice-view is never
        // the collection). Mirrors the `arm_borrows_heap_subvalue` relax for the same ops.
        Core::MapRemove { map, key, .. } => {
            collect_consuming_payload_sites_expr(db, map, scrut, true, out);
            collect_consuming_payload_sites_expr(db, key, scrut, false, out);
        }
        Core::SetRemove { set, elem, .. } => {
            collect_consuming_payload_sites_expr(db, set, scrut, true, out);
            collect_consuming_payload_sites_expr(db, elem, scrut, false, out);
        }
        Core::SetLen { set } => collect_consuming_payload_sites_expr(db, set, scrut, false, out),
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => {
            collect_consuming_payload_sites_expr(db, lhs, scrut, false, out);
            collect_consuming_payload_sites_expr(db, rhs, scrut, false, out);
        }
        Core::Convert { operand, .. } | Core::Not { operand } => {
            collect_consuming_payload_sites_expr(db, operand, scrut, consuming, out)
        }
        // `Bytes.compact` is a SAME-HANDLE IN-PLACE canonicalize (op_bytes_compact returns the SAME handle,
        // rc-neutral) — an IDENTITY transform for the consume/borrow question: it PASSES THROUGH its operand's
        // status. A compact whose RESULT is a borrowed key-op operand (a CHAMP probe key) reads its operand as
        // a borrow → NOT a consumed-child dup site (seam (b): without this, a `Map.lookup { key: Bytes.compact
        // view }` fell to the `_ =>` CONSUMING default and re-marked the view even after the key was relaxed).
        Core::BytesCompact { operand } => {
            collect_consuming_payload_sites_expr(db, operand, scrut, consuming, out)
        }
        Core::If { cond, then_, else_ } => {
            collect_consuming_payload_sites_expr(db, cond, scrut, false, out);
            collect_consuming_payload_sites_expr(db, then_, scrut, consuming, out);
            collect_consuming_payload_sites_expr(db, else_, scrut, consuming, out);
        }
        Core::Seq { stmts, tail } => {
            for s in stmts.iter() {
                collect_consuming_payload_sites_expr(db, *s, scrut, false, out);
            }
            collect_consuming_payload_sites_expr(db, tail, scrut, consuming, out);
        }
        Core::Block { body, .. } => {
            collect_consuming_payload_sites_expr(db, body, scrut, consuming, out)
        }
        Core::Break { value } => {
            collect_consuming_payload_sites_expr(db, value, scrut, consuming, out)
        }
        // A CLOSURE APPLICATION: the `closure` operand is APPLIED (its env is read in place — a BORROW; the
        // apply does NOT move the closure reference out of the arm), so an extracted scrutinee-child closure
        // reached HERE does NOT escape → it must NOT be shell-reclaim-dup'd (the outer shell-drop's cascade
        // already reclaims it; the spurious dup was the List.at/String.at extraction leak, v-mem-endorsed
        // option (b)). Without this arm the default `_` marks the closure `consuming=true` → a spurious dup
        // → the extracted closure ref survives rc1 (clA2/clA3). The ARGS are genuinely CONSUMED by the call.
        // (A closure that ESCAPES via a CONTAINER — `#list(f)`/`(tuple f …)` — is NOT reached here: it flows
        // through a ctor whose default `consuming=true` KEEPS its dup, so clE1/clE3 stay sound.)
        Core::CallClosure { closure, args } => {
            collect_consuming_payload_sites_expr(db, closure, scrut, false, out);
            for &a in args.iter() {
                collect_consuming_payload_sites_expr(db, a, scrut, true, out);
            }
        }
        Core::Let { bindings, body } => {
            for (_, v) in bindings.iter() {
                collect_consuming_payload_sites_expr(db, *v, scrut, true, out);
            }
            collect_consuming_payload_sites_expr(db, body, scrut, consuming, out);
        }
        Core::MatchSum { scrutinee: s, root } => {
            collect_consuming_payload_sites_expr(db, s, scrut, false, out);
            collect_consuming_payload_sites_cont(db, &root, scrut, out);
        }
        Core::Match { scrutinee: s, arms } => {
            collect_consuming_payload_sites_expr(db, s, scrut, false, out);
            for a in &arms {
                if let Some(g) = a.guard {
                    collect_consuming_payload_sites_expr(db, g, scrut, false, out);
                }
                collect_consuming_payload_sites_expr(db, a.body, scrut, consuming, out);
            }
        }
        Core::MatchList { scrutinee: s, arms } => {
            collect_consuming_payload_sites_expr(db, s, scrut, false, out);
            for a in &arms {
                collect_consuming_payload_sites_expr(db, a.body, scrut, consuming, out);
            }
        }
        _ => {
            for c in core_child_ids(db, id) {
                collect_consuming_payload_sites_expr(db, c, scrut, true, out);
            }
        }
    }
}

/// UPFRONT pass companion of the wrapper-scrutinee shell reclaim: walk `id` for every `Core::MatchSum`
/// whose scrutinee is a NON-reusable (computed) OWNED boxed-sum with COMPOUND payloads, and collect its
/// arms' CONSUMING scrutinee-child extraction sites into `dup_sites`. Run BEFORE emit (alongside
/// `collect_dup_sites`), so the `dup` IMPORT decision (`collect_used_ops`) and the emit's child-dup read
/// the SAME set. A reusable/borrowed/all-scalar scrutinee contributes nothing (the emit reclaims those
/// with no dup, or leaves a borrowed scrutinee to its owner). This is what makes the deep shell drop in
/// BOTH the tail and non-tail `MatchSum` emits safe for a consumed-child arm.
/// §5-disjointness for the NON-TAIL SPINE dup: whether any arm's TAIL-position expression is a `Call` that
/// consumes a payload-of-`scrut` as an arg — §5's self-loop-tail spine shape (`(rec tail …)`), where
/// `emit_loop_iteration` already dup(rest)s the carried payload. The non-tail-spine dup MUST skip such a
/// match (else double-dup → leak). Follows result-position tails (`If`/`Let`/`Seq`/`Block`/`Break`); a
/// non-tail nested call (`(+ 1 (rec m))`) is NOT a tail call → returns false → the non-tail dup fires.
pub(super) fn sum_cont_payload_consumed_in_tail_call(
    db: &mut Db,
    cont: &crate::core::SumCont,
    scrut: StructId,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => expr_tail_is_call_consuming_payload(db, *body, scrut),
        crate::core::SumCont::Guarded { body, els, .. } => {
            expr_tail_is_call_consuming_payload(db, *body, scrut)
                || sum_cont_payload_consumed_in_tail_call(db, els, scrut)
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            sum_cont_payload_consumed_in_tail_call(db, then_, scrut)
                || sum_cont_payload_consumed_in_tail_call(db, els, scrut)
        }
        crate::core::SumCont::Switch { arms, .. } => arms
            .iter()
            .any(|a| sum_cont_payload_consumed_in_tail_call(db, &a.cont, scrut)),
    }
}

pub(super) fn expr_tail_is_call_consuming_payload(
    db: &mut Db,
    id: StructId,
    scrut: StructId,
) -> bool {
    match core_of(db, id) {
        Core::Call { args, .. } => args
            .iter()
            .any(|&a| payload_proj_chain_roots_at_node(db, a, scrut)),
        Core::If { then_, else_, .. } => {
            expr_tail_is_call_consuming_payload(db, then_, scrut)
                || expr_tail_is_call_consuming_payload(db, else_, scrut)
        }
        Core::Let { body, .. } => expr_tail_is_call_consuming_payload(db, body, scrut),
        Core::Seq { tail, .. } => expr_tail_is_call_consuming_payload(db, tail, scrut),
        Core::Block { body, .. } => expr_tail_is_call_consuming_payload(db, body, scrut),
        Core::Break { value } => expr_tail_is_call_consuming_payload(db, value, scrut),
        _ => false,
    }
}

/// The number of DISTINCT `Core::MatchSum` nodes in `top_body` whose scrutinee is a direct reference to
/// `binder`. Used to detect a SHARED (borrowed) sum param: the non-tail-spine reclaim owns a param's shell
/// ONLY when that param is matched EXACTLY ONCE (the single match holds its last owned ref, so the emit's
/// tail-MatchSum param-slot drop reclaims it). A param matched by MORE THAN ONE MatchSum is SHARED/borrowed
/// — its shell is NOT emit-reclaimed and `mark_binder_dups` already owns the dup of each shared payload
/// extraction; the shell-reclaim dup-pass must NOT also mark that extraction, else the SAME SumPayload node
/// is claimed by both collectors (the exactly-one-of B2 invariant breaks — a benign double-mark, dedup'd by
/// the union set in emit, but a real collector over-claim). `count_param_consumes==0` does NOT catch this
/// (a match scrutinee borrow is not a consume), so this is the missing shared-scrutinee guard. Cheap: one
/// cycle-guarded walk.
///
/// SOUNDNESS SCOPE (v-mem-safety, #4888 close): this is NOT a COMPLETE sharing oracle — it counts only
/// `MatchSum` scrutinees and MISSES other borrows of the param, notably `Core::ValueEq` (structural `=` on
/// a Sum lowers to a `champ_eq` runtime borrow, not a `MatchSum`). It is a SUFFICIENT (restrictive) sharing
/// OVER-APPROXIMATION that is SAFE **only because** the non-tail-spine param-shell reclaim it gates is
/// TAIL-POSITION-emit-gated: `param_reclaim` fires ONLY when the `MatchSum` IS the fn's tail/result position
/// (so nothing uses the param after the match) and the shell `drop` is placed AFTER `emit_sum_cont` (so an
/// in-arm `ValueEq` borrows the LIVE shell + yields a non-aliasing scalar `Bool`, shell dropped after). A
/// param matched once but `=`-compared AFTER the match ⟹ the match is NON-tail ⟹ the reclaim never fires ⟹
/// no UAF (benign dup-without-drop leak only). NOTE: DO NOT reuse `count_matchsum_over_binder` as a "complete
/// sharing" signal in a NON-tail-gated reclaim context — a `ValueEq`-after-a-non-tail-reclaimed-match would
/// be a real UAF; add a matched-once+eq (ValueEq-aware) predicate FIRST if you ever move the reclaim off
/// tail-position gating.
pub(super) fn count_matchsum_over_binder(
    db: &mut Db,
    top_body: StructId,
    binder: StructId,
) -> usize {
    fn go(
        db: &mut Db,
        id: StructId,
        binder: StructId,
        seen: &mut HashSet<StructId>,
        n: &mut usize,
    ) {
        if !seen.insert(id) {
            return;
        }
        if let Core::MatchSum { scrutinee, .. } = core_of(db, id)
            && is_ref_to(db, scrutinee, binder)
        {
            *n += 1;
        }
        for c in core_child_ids(db, id) {
            go(db, c, binder, seen, n);
        }
    }
    let mut seen = HashSet::new();
    let mut n = 0usize;
    go(db, top_body, binder, &mut seen, &mut n);
    n
}

pub(super) fn collect_shell_reclaim_child_dups(
    db: &mut Db,
    id: StructId,
    dup_sites: &mut HashSet<StructId>,
) {
    // SHARING-AWARE: a shared Core `StructId` reached under N parents would otherwise be re-descended N
    // times (the exponential DAG re-walk — v-core-opt profile: this walk was ~55% of one self-host body's
    // 291M node-visits). Each `MatchSum`'s contribution here is a function of the NODE ALONE (its own
    // scrutinee + root) and lands in a SET of site node-ids, so visiting a node once vs. N times yields the
    // IDENTICAL `dup_sites` set — a node-id visited-set is idempotent and BYTE-NEUTRAL. (Contrast the
    // multiplicity-sensitive `mark_binder_dups`, whose per-occurrence retain placement a naive visited-set
    // WOULD corrupt into a dropped dup → UAF; that one stays Perceus-blocked.)
    let mut seen = HashSet::new();
    // `id` is the TOP function body — threaded down as `top_body` so the non-tail-spine param membership
    // (count_param_consumes over the WHOLE body) is computed self-contained, matching BOTH callers
    // (select_function_of + collect_used_ops) without threading params/self_def into this walk.
    collect_shell_reclaim_child_dups_seen(db, id, id, dup_sites, &mut seen);
}

pub(super) fn collect_shell_reclaim_child_dups_seen(
    db: &mut Db,
    id: StructId,
    top_body: StructId,
    dup_sites: &mut HashSet<StructId>,
    seen: &mut HashSet<StructId>,
) {
    if !seen.insert(id) {
        return;
    }
    if let Core::MatchSum { scrutinee, root } = core_of(db, id) {
        let scrut_ty = type_of(db, scrutinee);
        let compound_boxed = is_heap_type(&scrut_ty)
            && !ty_is_enum_disc(db, &scrut_ty)
            && !sum_has_only_scalar_payloads(db, &scrut_ty);
        // Existing STASHED-owned path: a computed owned compound boxed sum whose shell the emit deep-drops.
        let owned_compound_boxed = compound_boxed
            && matches!(
                heap_operand_ownership(db, scrutinee),
                Ok(HandleOwnership::Owned)
            );
        // NON-TAIL SPINE path (v-mem-safety-signed-off): a PARAM scrutinee consumed ONLY by the match
        // (count_param_consumes over the WHOLE body == 0) — the emit's tail-MatchSum param-slot drop reclaims
        // its shell, so its CONSUMED spine payload must ALSO be dup'd here (else the deep-drop double-frees
        // the moved payload). RELAXED vs the emit's strict set (NO epilogue-dropped exclusion here): the
        // dup-pass over-includes (a param the emit won't drop → dup-without-drop = a LEAK, never a UAF), so
        // dup ⊇ drop = every emit drop has its dup = no double-free. heap_operand_ownership(Param)==Borrowed,
        // so this is DISJOINT from owned_compound_boxed above (no double dup).
        // NON-TAIL SPINE param (INC1): the dup-pass predicate, EXTRACTED to `is_nontail_spine_param` so the
        // DROP-side collector (`collect_nontail_compound_reclaim_binders`) shares it verbatim (dup ⟺ drop).
        // The boundary exclusion is `!body_is_capturing_lifted` (NOT bare `db.lifted`): a lifted COMBINATOR
        // (empty captures — a named recursive def hoisted to the funcref table, called directly, callee-owned)
        // IS reclaimable (the BST del-min/insert 29/13/3 leak class); only a genuine CAPTURING closure's
        // closure-arg params stay excluded. (Prior `!top_is_lifted` over-excluded the combinators = the leak.)
        let nontail_spine_param = is_nontail_spine_param(db, top_body, scrutinee, &root);
        if owned_compound_boxed || nontail_spine_param {
            collect_consuming_payload_sites_cont(db, &root, scrutinee, dup_sites);
        }
    }
    for child in core_child_ids(db, id) {
        collect_shell_reclaim_child_dups_seen(db, child, top_body, dup_sites, seen);
    }
}

/// UPFRONT pass companion of the RUNTIME ROW-OP field-copy (breaker #45 UAF): a `Record.without`/`project`/
/// `extend`/`pop`/`merge` over a RUNTIME record operand `r` lowers (`lower_record_project` etc.) to a
/// self-keyed materialize `Core::Let{(r,r), Core::Record{ field ↦ (. r field) … }}` — the operand is
/// evaluated once (the `(r,r)` binding) and each kept field is a `Core::Proj{operand: r, …}` reading it
/// back. The `Let`'s escape-gated drop reclaims `r` after the record is built. But a HEAP-handle field
/// (`get_op` None — a String/Bytes/List/Map/Set/Sum/nested-record/BigInt/Rational/Symbol) is copied by a
/// BORROWING `arr-get` (no rc++), so when `r` is dropped its owned CHAMP/heap node cascades a drop to that
/// child → the freshly-built record holds a DANGLING field (breaker #45: `(Record.extend (Record.without r
/// (qty)) …)` over a map-borne record traps oob / silently reads a freed field). The binder-later-use dup
/// pass (`mark_binder_dups`) does NOT catch it: `r` is often SINGLE-USE (recomputed per row op), so there is
/// no "later use" to trigger a retain — the free comes from the operand's OWN materialize-`Let` drop,
/// independent of any later use. So mark each heap-handle field `Core::Proj` off the materialize binder as a
/// dup site: the emit (`Core::Proj` arm) then `dup`s the `arr-get` result (rc++), so the field owns an
/// independent reference that survives `r`'s drop — the FINDING#20 borrow-outlives-container-drop remedy,
/// one level down (the extracted FIELD, not the record). SCOPED precisely: only a self-keyed materialize
/// `Let{(k,k)}` whose body is a `Core::Record` (the row-op signature — a `lower_runtime_compare`
/// materialize's body is a nested `if`, not a record), only fields that are `Core::Proj{operand: k}` off
/// THAT binder, and only when the field is a HEAP HANDLE (`get_op` None). A SCALAR field COPIES out via
/// `get-int`/`get-bool` (no handle) → NOT marked (a `dup` on a non-handle scalar would corrupt, not leak). A
/// FRESH/const-record row op folds to a direct `Core::Record` with no materialize-`Let` (no operand drop) →
/// no self-keyed `Let` here → not marked (the list-borne / fresh-record controls stay green). Mirrors
/// `collect_shell_reclaim_child_dups`'s structure; run alongside it so the emit's child-`dup` + the `dup`
/// IMPORT decision (`collect_used_ops`) read the SAME set.
pub(super) fn collect_row_op_field_dups(
    db: &mut Db,
    id: StructId,
    dup_sites: &mut HashSet<StructId>,
) {
    // SHARING-AWARE, same rationale as `collect_shell_reclaim_child_dups`: a self-keyed materialize `Let`'s
    // contribution is node-intrinsic (its own bindings/body/fields) and lands in a SET of field node-ids, so
    // a node-id visited-set collapses the shared-DAG re-descent while keeping `dup_sites` byte-identical.
    let mut seen = HashSet::new();
    collect_row_op_field_dups_seen(db, id, dup_sites, &mut seen);
}

pub(super) fn collect_row_op_field_dups_seen(
    db: &mut Db,
    id: StructId,
    dup_sites: &mut HashSet<StructId>,
    seen: &mut HashSet<StructId>,
) {
    if !seen.insert(id) {
        return;
    }
    if let Core::Let { bindings, body } = core_of(db, id)
        // A self-keyed single binding `(k, k)` — the materialize-once row-op operand signature.
        && let [(bk, bv)] = &bindings[..]
        && bk == bv
    {
        let binder = *bk;
        // The materialize body must be a `Core::Record` (the row-op result); a `lower_runtime_compare`
        // materialize wraps a nested `if`, not a record, so it is skipped.
        if let Core::Record { fields } = core_of(db, body) {
            for &field in fields.values() {
                // A field read back from the operand is a `Core::Proj{operand: binder}`. Mark it only when
                // the projected value is a genuine HEAP HANDLE — a scalar field copies out (`get_op` Some,
                // `get-int`/`get-bool`/…) and must NOT be dup'd (rc++ on a non-handle corrupts). The proj
                // must root at THIS binder.
                //
                // WARNING: `get_op` returns `Ok(None)` for BOTH a heap handle AND `Ty::Unit` (the inline `IMM_UNIT`
                // sentinel — PR#914 Copilot): a Unit field-proj is `Drop`'d INLINE by the `Core::Proj`
                // emitter (never reaches the dup branch), so marking it a dup site imports `dup` with no
                // matching emit → breaks "import exactly the ops we call" (a spurious import that can perturb
                // op resolution). So EXCLUDE Unit explicitly — mark only a `get_op`-None field that is NOT
                // `Ty::Unit` (a real heap-handle field: String/Bytes/List/Map/Set/Sum/nested-record/BigInt/
                // Rational/Symbol). A Unit field owns no reference, so `r`'s drop can't dangle it anyway.
                if let Core::Proj { operand, .. } = core_of(db, field)
                    && operand == binder
                    && matches!(get_op(db, field), Ok(None))
                    && !matches!(type_of(db, field).strip_nominal(), crate::ty::Ty::Unit)
                {
                    dup_sites.insert(field);
                }
            }
        }
    }
    for child in core_child_ids(db, id) {
        collect_row_op_field_dups_seen(db, child, dup_sites, seen);
    }
}

/// (2) ROPE/SLICE-VIEW reclaim (co-owned with v-mem-safety): mark each `Core::SumExpect` whose extracted
/// COMPOUND Bytes view is consumed by EXACTLY ONE `Core::BytesAt` (a scalar-returning borrow — `bytes-get`
/// yields a raw Int64, no handle escapes) and does NOT escape, and whose SCRUTINEE is an OWNED producer (a
/// `Bytes.slice`/computed Option, so the SumExpect emit's `reclaim_shell` fires). Such a view is
/// scalar-extracted-DEAD: `compound_dupd` (the SumExpect emit) dup's the view at extract + drops the Some
/// shell (freeing it, view back to rc1 owned), and the sole `Bytes.at`'s `reclaim_bytes` drops the now-owned
/// view AFTER its len+get borrows (its existing liveness point) — reclaiming BOTH cells (the bar3/bar4
/// 10-bytes leak: the Option shell + the slice view). This is v-mem-safety's dup-at-extract + shell-drop +
/// liveness-view-drop plan onto the two existing emit hooks, coupled through THIS dedicated set (both hooks
/// consult it, never re-derived — the single source of truth, kept disjoint from `dup_sites` so `reclaim_bytes`
/// fires the view-drop ONLY for this reason, never conflated with a `mark_binder_dups`/B1 mark = the b2
/// double-mark class).
///
/// SINGLE-CONSUMER (count == 1): `reclaim_bytes` is PER-`Bytes.at`-op, so a view read by MULTIPLE `Bytes.at`
/// would double-drop → left UNMARKED (leak-over-UAF; a future last-use-hoist recovers it). VIEW-ESCAPE
/// MUST-HOLD: a view that escapes (returned / passed as a handle / read by a non-`Bytes.at` consumer) has
/// `count != 1` for the direct-`Bytes.at`-operand shape OR its sole use is not a `Bytes.at`, so it is not
/// marked → not reclaimed (v-mem-safety's #4917 SumExpect-escape control must stay leaking).
pub(super) fn collect_sumexpect_view_reclaim(
    db: &mut Db,
    body: StructId,
    view_set: &mut HashSet<StructId>,
    shell_set: &mut HashSet<StructId>,
) {
    let mut seen = HashSet::new();
    collect_sumexpect_view_reclaim_seen(db, body, body, view_set, shell_set, &mut seen);
}

/// Whether `scrutinee` is a SINGLE-heap-payload owned Some PRODUCER eligible for the (2) reclaim (Option A):
/// `String.at` (Some char-view) or `Bytes.slice` (Some bytes-view) — both return a fresh `Some(one view)`,
/// inherently single-heap-payload (the load-bearing fence: dup==1 balances the single Some→payload cascade),
/// and Owned (a fresh sum-new the SumExpect emit's `reclaim_shell` frees). (Option B would generalize the
/// producer set with an explicit #heap-payloads==1 check; A scopes to these two inherently-single producers.)
pub(super) fn is_owned_single_view_producer(db: &mut Db, scrutinee: StructId) -> bool {
    // `String.at` / `Bytes.slice` ALWAYS return a fresh `Some(one view)` — owned + single-heap-payload BY
    // CONSTRUCTION (that IS the load-bearing fence for Option A). We check the NODE KIND directly rather than
    // `heap_operand_ownership` because `StrAt` is deliberately NOT globally-Owned (to avoid perturbing the
    // MatchSum Stage-B path — see the heap_operand_ownership note); being a StrAt/BytesSlice node is itself
    // the owned-single-view proof, and the SumExpect `reclaim_shell` treats it as owned LOCALLY via the set.
    matches!(
        core_of(db, scrutinee),
        Core::StrAt { .. } | Core::BytesSlice { .. }
    )
}

/// Whether `parent`'s use of `target` is a SCALAR-returning read WITH a view-drop hook — a `Bytes.at`
/// (→reclaim_bytes) or `String.scalar-len` (→the StrScalarLen reclaim). Such a consumer lets us OWN + drop
/// the view (VIEW-set, net -1). Any OTHER single consumer (a `Call`/op that takes the view onward) leaves the
/// view to that consumer (SHELL-set, net 0). This is the two-set partition by consumer-kind (disjoint).
pub(super) fn is_view_scalar_read_consumer(
    db: &mut Db,
    parent: StructId,
    target: StructId,
) -> bool {
    match core_of(db, parent) {
        Core::BytesAt { bytes, .. } => bytes == target,
        Core::StrScalarLen { operand } => operand == target,
        _ => false,
    }
}

pub(super) fn collect_sumexpect_view_reclaim_seen(
    db: &mut Db,
    id: StructId,
    top_body: StructId,
    view_set: &mut HashSet<StructId>,
    shell_set: &mut HashSet<StructId>,
    seen: &mut HashSet<StructId>,
) {
    if !seen.insert(id) {
        return;
    }
    // A fresh `Option.expect` extraction (`Core::SumExpect`) over an Owned single-view Some producer, used
    // EXACTLY ONCE (single-consumer + no-escape: count > 1 ⟹ multi-consumer/escape ⟹ neither set). Classify
    // by its single consumer's kind — the disjoint two-set partition.
    if let Core::SumExpect { scrutinee, .. } = core_of(db, id)
        && is_owned_single_view_producer(db, scrutinee)
        && is_heap_type(&type_of(db, id))
        && count_node_refs(db, top_body, id) == 1
    {
        match single_parent_of(db, top_body, id) {
            // Consumer is a scalar-read with a view-drop hook (Bytes.at / String.scalar-len) → VIEW-set:
            // compound_dupd dups + shell-drops, and the consumer's reclaim drops the now-owned view (net -1).
            Some(parent) if is_view_scalar_read_consumer(db, parent, id) => {
                view_set.insert(id);
            }
            // Consumer is a Call/op that takes the view onward → SHELL-set: compound_dupd's dup (+1) exactly
            // compensates the shell-drop cascade (-1) = NET-0 on the view (consumer owns it), only the
            // orphaned shell freed. No view-drop on our side. (None = escape/body-result → neither.)
            Some(_) => {
                shell_set.insert(id);
            }
            None => {}
        }
    }
    for child in core_child_ids(db, id) {
        collect_sumexpect_view_reclaim_seen(db, child, top_body, view_set, shell_set, seen);
    }
}

/// The SINGLE node in `body` that has `target` as a direct child, or `None` if `target` is unreferenced
/// (escapes as the body result) or referenced more than once (callers gate on `count_node_refs == 1`, so
/// `None` here means the body-result/escape case). Used to classify a single-consumer SumExpect's consumer.
pub(super) fn single_parent_of(db: &mut Db, body: StructId, target: StructId) -> Option<StructId> {
    fn go(
        db: &mut Db,
        id: StructId,
        target: StructId,
        seen: &mut HashSet<StructId>,
        found: &mut Option<StructId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        for c in core_child_ids(db, id) {
            if c == target {
                *found = Some(id);
            }
            go(db, c, target, seen, found);
        }
    }
    let mut seen = HashSet::new();
    let mut found = None;
    go(db, body, target, &mut seen, &mut found);
    found
}

/// The number of times `target` appears as a direct CHILD of any node reachable from `body` — a
/// whole-body use count (sharing-aware: each parent node is visited once, and `target` counted once per
/// child SLOT it occupies, so `(f target target)` counts 2). Used by [`collect_sumexpect_view_reclaim`] to
/// prove a SumExpect extraction is single-consumer + non-escaping (`== 1`).
pub(super) fn count_node_refs(db: &mut Db, body: StructId, target: StructId) -> usize {
    fn go(
        db: &mut Db,
        id: StructId,
        target: StructId,
        seen: &mut HashSet<StructId>,
        n: &mut usize,
    ) {
        if !seen.insert(id) {
            return;
        }
        for c in core_child_ids(db, id) {
            if c == target {
                *n += 1;
            }
            go(db, c, target, seen, n);
        }
    }
    let mut seen = HashSet::new();
    let mut n = 0usize;
    go(db, body, target, &mut seen, &mut n);
    n
}

/// Perceus RETAIN placement: the set of `Core::LocalRef`/`Core::Param` OCCURRENCES (keyed by their own
/// node id) whose reference is CONSUMED at that occurrence while the binding has a LATER live use on the
/// same control-flow path — so the occurrence must be `dup`'d (rc++) before the consuming op runs, or the
/// op's in-place FBIP reuse (a uniquely-owned `vec-push`/`map-insert`/… mutates its operand) corrupts the
/// value the later use reads.
///
/// The single closing `drop` a `Core::Let` emits (gated by `!binding_escapes`) reclaims a binding whose
/// LAST use borrows; it does NOT account for a binding consumed EARLY and read again. Without a dup there,
/// `(let ((e L)) (+ (List.len (List.push e 9)) (List.len e)))` mutates `e` through the push and the right
/// `List.len e` reads the grown list — a silent wrong value (the same defect for `Map.insert`/`Set.insert`
/// and for a shared PARAMETER across two recursive-call operands). A dup at the consuming occurrence gives
/// the consumer its OWN reference and leaves the binding's reference intact for the later use; the existing
/// escape-gated drop still reclaims the survivor exactly once. The single-use consume (the FBIP fast path,
/// `(List.len (List.push e 9))` with `e` used once) is untouched — no later use, so no dup.
//= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
//# A value MUST NOT be observably mutated through one reference while it is read through another in a way the executable semantics leaves unspecified.
// The dup/reuse decision this computes is a function of the SOURCE STRUCTURE ALONE — the consuming
// occurrences, their control-flow paths, and the escape/borrow classification — never of any runtime or
// nondeterministic input, so whether an op reuses its operand's storage in place (FBIP) or a dup forces
// fresh storage is deterministic and cannot introduce nondeterminism into observable behavior:
//= spec/capabilities/memory-and-resource-model.md#reuse-is-not-observable
//# A decision to reuse a value's storage or to allocate fresh storage MUST be a deterministic function of the source, so that reuse does not introduce nondeterminism into a program's observable behavior.
//= spec/capabilities/memory-and-resource-model.md#sharing-is-not-observable
//# A decision to share a value's storage or to copy it MUST be a deterministic function of the source, so that sharing does not introduce nondeterminism into a program's observable behavior.
/// hcz CAPTURE-ESCAPE dup sites: mark the `Core::Captured` occurrence of each COMPOUND (heap) closure capture
/// that is read EXACTLY ONCE in `body` and whose sole read ESCAPES (flows out via the body's return / a
/// consuming use, per [`capture_escapes_via_body`]). Such a read must `dup` so the returned value owns an
/// independent ref — else the monolithic env-cell drop double-frees it (the hcz1/hcz2 UAF). SINGLE-READ +
/// HEAP + ESCAPES guard (v-memory-safety-signed): with exactly one occurrence, that read IS the escaping
/// consume (no borrow occurrence to over-dup); a scalar capture unboxes/copies (no rc); a multi-read escaping
/// compound capture is left UNMARKED (a tracked RESIDUAL double-free — see the `captured_escape_dup_sites`
/// field doc). Runs for EVERY body — a non-closure body has no `Core::Captured` occurrence, so the map is
/// empty and this is a no-op. Called from BOTH `select_function_of` (to EMIT the dup) and the op-collection
/// pass (to IMPORT `OP_DUP`) so the emit and the import agree exactly — the dup-site import/emit discipline.
pub(super) fn collect_captured_escape_dup_sites(
    db: &mut Db,
    body: StructId,
    sites: &mut HashSet<StructId>,
) {
    let mut by_index: HashMap<usize, Vec<StructId>> = HashMap::new();
    collect_captured_occurrences(db, body, &mut by_index);
    for (index, occs) in by_index {
        // PER-OCCURRENCE escape-dup (Perceus borrowed-parameter rule, #5857 Increment A). Emit one dup
        // PER ESCAPING occurrence of the capture, NOT one total. A capture used in N>1 escaping positions
        // (`#tuple(a a)`) needs a dup at EACH escaping read so the returned value owns N independent refs
        // and the monolithic closure-cell drop nets each ref to a live rc; the old `occs.len() != 1` punt
        // emitted ZERO dups for a multi-occurrence capture → the closure-drop freed the shared cell while
        // the result still held N refs = the hczm1 over-free (release `unreachable`). This brings the
        // capture collector up to the binder path (`mark_binder_dups`), which already handles
        // per-occurrence multiplicity correctly; each occurrence is a DISTINCT `Core::Captured` node, so
        // inserting the escaping ones into the site set yields exactly one dup per escaping read (no
        // multiset needed — that is Increment B, for a single node consumed twice).
        //
        // PRE-GATE on the per-index escape query: an index NONE of whose reads escape (every read a borrow,
        // e.g. `List.len a` / a scalar field read — hcd1/hcd2) contributes no dup, skip it whole.
        if !capture_escapes_via_body(db, body, index) {
            continue;
        }
        for occ in occs {
            // A scalar capture unboxes to a raw value (no heap handle, no refcount) → no dup needed.
            if !is_heap_type(&type_of(db, occ)) {
                continue;
            }
            // THIS occurrence escapes (a consuming / result / call-arg position) vs is only BORROWED (a
            // `Proj` operand — relaxed to `tail_borrowed`). Keyed on the specific node (`EscapeTarget::
            // Node`), so a capture both PROJECTED and ESCAPED (hczm2) dups ONLY the escaping occurrence —
            // the projection is a borrow. Reuses the exact borrow-vs-consume walk the snowflake
            // SumPayload-escape dup (#5833) already drives per node.
            if binding_escapes_dup_aware(db, body, EscapeTarget::Node(occ), false, None) {
                sites.insert(occ);
            }
        }
    }
}

/// Collect every `Core::Captured { index }` occurrence in `body`, grouped by capture slot INDEX. Recurses via
/// [`core_child_ids`] (the same uniform Core-child walk `collect_retain_candidate_binders` uses).
pub(super) fn collect_captured_occurrences(
    db: &mut Db,
    id: StructId,
    by_index: &mut HashMap<usize, Vec<StructId>>,
) {
    // MEMO (v-memory-safety sign-off; simplest of the reclaim family — key = id ALONE, no context). Each
    // subtree's Captured-occurrence contribution is a pure fn of id + the immutable graph. MULTIPLICITY: the
    // walk APPENDS per visit (no dedup), so cache each node's OWN contribution in a FRESH accumulator (NOT a
    // delta vs the shared `by_index`) and MERGE=APPEND on EVERY hit — a shared subtree reached via N paths
    // gives N appends = the SAME per-visit multiplicity the ~570× re-walk produced (merging once-per-distinct
    // would UNDER-count). DFS order preserved (own-first then children in core_child_ids order, extended in
    // walk order). NO in_progress/tainted — acyclic by spec (Core::Call args-only) + memo writes post-return.
    if let Some(cached) = db.captured_occ_memo.get(&id) {
        for (index, occs) in cached {
            by_index
                .entry(*index)
                .or_default()
                .extend(occs.iter().copied());
        }
        return;
    }
    let mut own: HashMap<usize, Vec<StructId>> = HashMap::new();
    collect_captured_occurrences_inner(db, id, &mut own);
    for (index, occs) in &own {
        by_index
            .entry(*index)
            .or_default()
            .extend(occs.iter().copied());
    }
    db.captured_occ_memo.insert(id, own);
}

/// The unmemoized worker of [`collect_captured_occurrences`]. Its recursive calls resolve to the MEMOIZED
/// wrapper above, so a shared subtree reached via many lifted-body walks is computed once + spliced (appended)
/// thereafter — preserving per-visit multiplicity and DFS order.
fn collect_captured_occurrences_inner(
    db: &mut Db,
    id: StructId,
    by_index: &mut HashMap<usize, Vec<StructId>>,
) {
    if let Core::Captured { index, .. } = core_of(db, id) {
        by_index.entry(index).or_default().push(id);
    }
    for child in core_child_ids(db, id) {
        collect_captured_occurrences(db, child, by_index);
    }
}

/// SumPayload-ESCAPE dup sites — the BOUNDARY-OWNED twin of [`collect_captured_escape_dup_sites`] (hcz). In a
/// LIFTED body (`db.lifted` — boundary-owned params the direct-call boundary BUILT + `drop_after`s), a
/// `SumPayload`/`Proj` extraction of a boundary-owned PARAM scrutinee whose HEAP payload ESCAPES the fn via a
/// result ctor MUST be `dup`'d: else the caller's boundary `drop_after` of the arg cascades into the escaped
/// payload, freeing it while the returned ctor still holds an uncounted ref (the snowflake
/// `lower(six-fold(ball))` rc-underflow — lower's `Sphere` arm `sum-payload(r) -> sum-new(OSphere(r))` moves
/// `r` out un-dup'd, and lower being lifted, the CALLER drops the arg). ORTHOGONAL to the shell-reclaim /
/// boundary-owned drop fence: `nontail_spine_param` EXCLUDES boundary-owned (`top_is_lifted`, the "40 corpus
/// traps" double-free fence — correctly, the caller drops). This ADDS a `dup` for the escaping payload, NEVER
/// a shell-drop, so that fence stays clean. Only fires on ESCAPE (`binding_escapes_dup_aware`) — a matched-
/// then-discarded payload does not escape → no dup → no over-retain. Gated on `db.lifted` ONLY (the exclusion
/// complement): the fn-OWNED case already dups its escaping payload via `nontail_spine_param`, so no double-
/// dup. `db.lifted` is checkable in BOTH `select_function_of` (emit) and `collect_used_ops` (import) → the
/// dup emit + `OP_DUP` import agree exactly. Empty + no-op for a non-lifted body.
pub(super) fn collect_sumpayload_escape_dup_sites(
    db: &mut Db,
    body: StructId,
    sites: &mut HashSet<StructId>,
) {
    if !db.lifted.iter().any(|l| l.body == body) {
        return;
    }
    // #5833 OVER-MARK FIX (v-memory-safety-signed-off gate (a), 177 over-retention): the escape query below
    // (`binding_escapes_dup_aware`'s Proj arm) CONSERVATIVELY treats a nested-compound projection
    // `(. p 1)` as its operand `p` ESCAPING — a `dup`-worthy alias-out. That bias is correct+safe for the
    // ~78 reclaim-DECISION callers (there escape=true means "don't reclaim" = a LEAK), but here it is a
    // FALSE POSITIVE: when the projected child is read only to SCALARS (`(. (. p 1) 0)`), `p` is
    // borrow-then-dead — it does NOT truly escape — so the escape-dup is spurious + unbalanced (nothing
    // consumes it), and the boundary drop of the arg then hits `p` at rc>1 and cannot free it (the 177
    // leak + the corpus-21 nested-tuple 0→2). Gate the dup with the SAME payload-safety
    // `nontail_param_payload_ok` uses (a scoped fix on THIS collector, NOT the shared escape query — that
    // would flip the 78 reclaim callers toward a UAF): SUPPRESS the escape-dup for an extraction whose root
    // param is matched ONLY by PAYLOAD-SAFE (borrow-then-dead) MatchSums; KEEP it for a REAL escape (the
    // payload returned whole / fed to a consuming ctor/op — the snowflake `sum-new(OSphere r)`, the
    // `List.push (. t 0)` FBIP shape). Default-KEEP (operator leak>UAF): a param with ANY non-payload-safe
    // match keeps its dups.
    let payload_safe = payload_safe_match_param_binders(db, body);
    let mut nodes = Vec::new();
    collect_payload_extraction_nodes(db, body, &mut nodes);
    for node in nodes {
        // HEAP payload only — a scalar unboxes/copies out (no rc handle, no dup).
        if !is_heap_type(&type_of(db, node)) {
            continue;
        }
        // Rooted at a boundary-owned PARAM (the caller `drop_after`s it) — NOT a fresh owned local (that
        // case is the owned-scrutinee shell-reclaim's child-dup, not this boundary-owned complement).
        if !payload_extraction_roots_at_param(db, node) {
            continue;
        }
        // SUPPRESS (default-KEEP): the extraction's root param is matched only by payload-safe
        // (borrow-then-dead) MatchSums → the escape query's compound-projection "escape" is spurious.
        if let Some(binder) = extraction_root_param_binder(db, node)
            && payload_safe.contains(&binder)
        {
            continue;
        }
        // The extracted payload ESCAPES via a result ctor / the return (not borrow-only).
        if binding_escapes_dup_aware(db, body, EscapeTarget::Node(node), false, None) {
            sites.insert(node);
        }
    }
}

/// Param binders whose EVERY `MatchSum` in `body` is PAYLOAD-SAFE (borrow-then-dead) per
/// [`nontail_param_payload_ok`] — their match-extracted payloads never escape (no construct-compound, no
/// re-match, no payload-in-result), so the [`collect_sumpayload_escape_dup_sites`] escape-dup is a false
/// positive (a compound projection read only to scalars). DEFAULT-KEEP: a binder with ANY non-payload-safe
/// match is EXCLUDED (its escape-dups are kept — leak > UAF). `never_diverges = false` so the payload
/// escape clauses (not the divergence gate) decide safety here.
pub(super) fn payload_safe_match_param_binders(db: &mut Db, body: StructId) -> HashSet<StructId> {
    let mut safe: HashSet<StructId> = HashSet::new();
    let mut excluded: HashSet<StructId> = HashSet::new();
    collect_payload_safe_match_binders(db, body, &mut safe, &mut excluded);
    safe.retain(|b| !excluded.contains(b));
    safe
}

pub(super) fn collect_payload_safe_match_binders(
    db: &mut Db,
    id: StructId,
    safe: &mut HashSet<StructId>,
    excluded: &mut HashSet<StructId>,
) {
    if let Core::MatchSum { scrutinee, root } = core_of(db, id)
        && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, scrutinee)
    {
        let scrut_ty = type_of(db, scrutinee);
        if is_heap_type(&scrut_ty) {
            if nontail_param_payload_ok(db, scrutinee, &scrut_ty, false, &root) {
                safe.insert(binder);
            } else {
                excluded.insert(binder);
            }
        }
    }
    for child in core_child_ids(db, id) {
        collect_payload_safe_match_binders(db, child, safe, excluded);
    }
}

/// The `Core::Param`/`Core::LocalRef` binder a `SumPayload`/`Proj` extraction chain bottoms at (the twin of
/// [`payload_extraction_roots_at_param`] that RETURNS the binder), or `None` for a non-param root.
pub(super) fn extraction_root_param_binder(db: &mut Db, id: StructId) -> Option<StructId> {
    match core_of(db, id) {
        Core::Param { binder } | Core::LocalRef { binder } => Some(binder),
        Core::SumPayload { scrutinee, .. }
        | Core::Proj {
            operand: scrutinee, ..
        } => extraction_root_param_binder(db, scrutinee),
        _ => None,
    }
}

/// Collect every `Core::SumPayload` / heap `Core::Proj` extraction node in `body` (the sites
/// [`collect_sumpayload_escape_dup_sites`] considers). Uniform `core_child_ids` walk.
pub(super) fn collect_payload_extraction_nodes(db: &mut Db, id: StructId, out: &mut Vec<StructId>) {
    if matches!(core_of(db, id), Core::SumPayload { .. } | Core::Proj { .. }) {
        out.push(id);
    }
    for child in core_child_ids(db, id) {
        collect_payload_extraction_nodes(db, child, out);
    }
}

/// Whether a `Core::SumPayload`/`Core::Proj` extraction's scrutinee/operand CHAIN bottoms at a `Core::Param`
/// (a boundary-owned arg in a lifted body), vs a fresh owned local / computed value.
pub(super) fn payload_extraction_roots_at_param(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Param { .. } => true,
        Core::SumPayload { scrutinee, .. }
        | Core::Proj {
            operand: scrutinee, ..
        } => payload_extraction_roots_at_param(db, scrutinee),
        _ => false,
    }
}

pub(super) fn collect_dup_sites(
    db: &mut Db,
    body: StructId,
    binders: &[StructId],
    sites: &mut HashSet<StructId>,
) {
    // Build the shared occurrence oracle ONCE (a single cycle-guarded pass, O(N × binders/64)) so each
    // binder's walk prunes binder-free subtrees in O(1) instead of the per-binder full re-walk — the
    // O(binders × body-nodes)→~O(N) traversal-share fix for the sread-eval provider-emit cliff. The
    // per-binder loop stays (each binder's dup/live_after logic is unchanged), but a subtree the binder
    // never enters is now skipped at its root rather than fully descended.
    let index: HashMap<StructId, usize> =
        binders.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let bitsets = build_occurrence_bitsets(db, body, &index);
    // Install the oracle under an RAII guard that RESTORES THE PREVIOUS value on scope-exit — including a
    // PANIC unwind (a `mark_binder_dups` assertion firing is a routine test-harness outcome). A bare
    // set-then-`= None` would, on an unwind between the two, leave the thread-local `Some(stale)`, and a
    // later `collect_dup_sites` on the SAME THREAD would read the stale oracle → a false "binder absent"
    // early-prune → a DROPPED dup site → a latent leak/UAF (the process-global-state-contamination class,
    // cf. `process-global-atomic-metric-counter-contaminated-by-parallel-test-harness`). Restoring the
    // PREVIOUS value (not unconditionally `None`) is also nesting-safe. (Copilot PR#942 id 3687515459.)
    let _oracle_guard = OracleGuard::install((index, bitsets));
    for &binder in binders {
        // Compute the borrow-only verdict for THIS binder over the whole body (the BASE escape query,
        // `dup_sites=None` — deliberately NOT `Some(sites)`: `sites` is what we are computing, and the
        // dqe17 root is exactly the `dup_sites`↔`binding_escapes` circularity, so gate on the dup-FREE
        // query). `false` (escapes on some path) ⇒ the binder's projection dup is LEGITIMATE (a
        // conditional-escape binder like dqe17's `a` needs the taken-path dup for arm-conditional reclaim);
        // `true` (never escapes) ⇒ a pure borrow-only binder (dqe4's `a`) whose compound-projection chain
        // must NOT mint a spurious unbalanced dup. The `Core::Proj` arm reads this via `binder_never_escapes`.
        let never_escapes =
            !binding_escapes_dup_aware(db, body, EscapeTarget::Binder(binder), false, None);
        let _ne_guard = NeverEscapesGuard::install(never_escapes);
        // The dqe7/8-vs-dqe17 discriminator: when the binder DOES escape (`!never_escapes`), does it escape on
        // EVERY path (an unconditional straight-line consume — its own consume-dup covers the refcount, so the
        // compound-projection keep-alive dup is SURPLUS → suppress) or only CONDITIONALLY (an arm-only escape —
        // the dup is needed for the non-escape path → keep)? Sound under-approximation (wrong ⇒ keep the dup).
        // Read by the `Core::Proj` arm via `binder_must_escapes`.
        let must_escapes = binder_must_escape(db, body, binder, false);
        let _me_guard = MustEscapesGuard::install(must_escapes);
        // The body's result position CONSUMES (the value is returned / escapes), so the top-level call is
        // `consuming: true`; nothing is used after the whole body, so `live_after: false`.
        mark_binder_dups(db, body, binder, true, false, sites);
    }
    // `_oracle_guard` drops here (or on unwind), restoring the prior oracle value.
}

/// RAII guard for [`DUP_OCCURRENCE_ORACLE`]: swaps in a new oracle on `install`, restores the PREVIOUS
/// value on `Drop` (normal return OR panic unwind), so a panicking `mark_binder_dups` can never leave a
/// stale oracle to contaminate the next same-thread `collect_dup_sites`. Restoring the previous value
/// (rather than `None`) keeps it nesting-safe.
pub(super) struct OracleGuard {
    prev: Option<DupOccurrenceOracle>,
}

impl OracleGuard {
    pub(super) fn install(oracle: DupOccurrenceOracle) -> Self {
        let prev = DUP_OCCURRENCE_ORACLE.with(|o| o.borrow_mut().replace(oracle));
        OracleGuard { prev }
    }
}

impl Drop for OracleGuard {
    fn drop(&mut self) {
        DUP_OCCURRENCE_ORACLE.with(|o| *o.borrow_mut() = self.prev.take());
    }
}

/// The active `collect_dup_sites` occurrence oracle: `(binder → bit-index, node → occurrence-bitset)` from
/// [`build_occurrence_bitsets`].
pub(super) type DupOccurrenceOracle = (HashMap<StructId, usize>, HashMap<StructId, Vec<u64>>);

thread_local! {
    // The SHARED occurrence oracle for the active `collect_dup_sites` run. Set for the duration of one
    // `collect_dup_sites` call, consulted by `mark_binder_dups_inner` for the O(1) occurrence EARLY-PRUNE
    // (skip a subtree where the current binder's bit is 0 — that subtree marks NO site for it and returns
    // "did not occur", identical to the full walk, so the Perceus site-set is preserved). Thread-local
    // rather than a threaded param because `mark_binder_dups`'s ~40-arm recursion + its capturing closures
    // would each need the extra argument; select runs single-threaded per function, so this is safe. `None`
    // outside a `collect_dup_sites` run → no prune (the walk behaves exactly as before).
    pub(super) static DUP_OCCURRENCE_ORACLE: std::cell::RefCell<Option<DupOccurrenceOracle>> =
        const { std::cell::RefCell::new(None) };
}

/// Whether `binder` PROVABLY does not occur in the subtree at `id`, per the active occurrence oracle — used
/// by [`mark_binder_dups_inner`] to prune a binder-free subtree in O(1). Returns `false` (do NOT prune) when
/// there is no oracle, the node was not memoized (a tainted/cyclic subtree — walk it, don't risk a false
/// prune), or the binder is not tracked. Only a DEFINITE all-zero bit for the binder prunes.
pub(super) fn binder_absent_in_subtree(binder: StructId, id: StructId) -> bool {
    DUP_OCCURRENCE_ORACLE.with(|o| {
        let o = o.borrow();
        let Some((index, bitsets)) = o.as_ref() else {
            return false;
        };
        let Some(&i) = index.get(&binder) else {
            return false;
        };
        let Some(bits) = bitsets.get(&id) else {
            return false; // not memoized (tainted/cyclic) — don't prune
        };
        (bits[i / 64] >> (i % 64)) & 1 == 0
    })
}

thread_local! {
    // Whether the CURRENT `mark_binder_dups` binder PROVABLY never escapes the body (a borrow-only binding) —
    // set per-binder in `collect_dup_sites` before each `mark_binder_dups` call (the binder is fixed for the
    // whole recursion, so a single flag is correct), consulted by `mark_binder_dups_inner`'s `Core::Proj` arm
    // to GATE the compound-projection consuming-transparency. Thread-local (not a threaded param) for the same
    // reason as `DUP_OCCURRENCE_ORACLE`: `mark_binder_dups`'s ~40-arm recursion + capturing closures would each
    // need the extra argument, and select runs single-threaded per function. Defaults to `false` (conservative
    // — keep the dup, the SAFE leak-not-UAF direction) outside a `collect_dup_sites` run.
    static BINDER_NEVER_ESCAPES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the active `mark_binder_dups` binder provably never escapes its body (set by `collect_dup_sites`).
/// Read by the `Core::Proj` arm to decide the dqe17-safe dup-suppression (see there).
fn binder_never_escapes() -> bool {
    BINDER_NEVER_ESCAPES.with(|c| c.get())
}

/// RAII guard that sets [`BINDER_NEVER_ESCAPES`] for one `mark_binder_dups` binder and RESTORES the prior value
/// on drop (normal return OR panic unwind), so a nested `collect_dup_sites` can never leave a stale flag to
/// mis-gate the enclosing run's remaining binders.
struct NeverEscapesGuard {
    prev: bool,
}

impl NeverEscapesGuard {
    fn install(never_escapes: bool) -> Self {
        let prev = BINDER_NEVER_ESCAPES.with(|c| c.replace(never_escapes));
        NeverEscapesGuard { prev }
    }
}

impl Drop for NeverEscapesGuard {
    fn drop(&mut self) {
        BINDER_NEVER_ESCAPES.with(|c| c.set(self.prev));
    }
}

thread_local! {
    // Whether the CURRENT `mark_binder_dups` binder PROVABLY MUST-escapes (is consumed on EVERY control path,
    // per the sound under-approximation `binder_must_escape`) — set per-binder in `collect_dup_sites` alongside
    // `BINDER_NEVER_ESCAPES`, consulted by `mark_binder_dups_inner`'s `Core::Proj` arm to distinguish an
    // UNCONDITIONAL straight-line consume (dqe7/8 — suppress the surplus projection dup) from a CONDITIONAL
    // arm-escape (dqe17 — keep the dup for the non-escape path). Thread-local for the same reason as
    // `BINDER_NEVER_ESCAPES`. Defaults to `false` (conservative — keep the dup, the SAFE leak-not-UAF
    // direction) outside a `collect_dup_sites` run.
    static BINDER_MUST_ESCAPES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the active `mark_binder_dups` binder provably MUST-escapes its body (set by `collect_dup_sites`).
/// Read by the `Core::Proj` arm for the dqe7/8-vs-dqe17 dup-suppression discriminator (see there).
fn binder_must_escapes() -> bool {
    BINDER_MUST_ESCAPES.with(|c| c.get())
}

/// RAII guard for [`BINDER_MUST_ESCAPES`] — sets it for one `mark_binder_dups` binder and RESTORES the prior
/// value on drop (normal return OR panic unwind), the `NeverEscapesGuard` twin (nesting-safe).
struct MustEscapesGuard {
    prev: bool,
}

impl MustEscapesGuard {
    fn install(must_escapes: bool) -> Self {
        let prev = BINDER_MUST_ESCAPES.with(|c| c.replace(must_escapes));
        MustEscapesGuard { prev }
    }
}

impl Drop for MustEscapesGuard {
    fn drop(&mut self) {
        BINDER_MUST_ESCAPES.with(|c| c.set(self.prev));
    }
}

/// The EXACT occurrence bit for `binder` at subtree `id` from the active oracle — `Some(true/false)` when the
/// node is MEMOIZED in the bitset (identical to [`binder_occurs`] by construction; see the `build_occurrence_
/// bitsets` cross-check test), `None` when there is no oracle / the binder is untracked / the node is a
/// tainted-cyclic MISS (fall back to the walking `binder_occurs`). Lets the `seq` pre-pass read a child's
/// occurrence in O(1) off the ONCE-built oracle instead of a fresh per-`seq` `binder_occurs` re-scan — the fix
/// for the super-linear reclaim blowup where nested `seq`s re-walked overlapping subtrees (db-query-diff
/// `cdz test` hang). Exactness-preserving: the oracle bit EQUALS `binder_occurs` for memoized nodes, and a
/// MISS falls back to the exact walk, so the Perceus site-set is byte-identical to the pre-fix behaviour.
fn binder_occurs_via_oracle(binder: StructId, id: StructId) -> Option<bool> {
    DUP_OCCURRENCE_ORACLE.with(|o| {
        let o = o.borrow();
        let (index, bitsets) = o.as_ref()?;
        let &i = index.get(&binder)?;
        let bits = bitsets.get(&id)?; // MISS (tainted/cyclic, un-memoized) → None → caller walks
        Some((bits[i / 64] >> (i % 64)) & 1 == 1)
    })
}

/// Collect every HEAP-typed binder whose multi-use inside `id` could need a retain: each `Core::Let`
/// BINDER declared in the subtree, and each PARAMETER referenced by a `Core::Param` occurrence (a scalar
/// binding owns no heap cell, so its multi-use needs no dup — a scalar is re-read from its slot freely).
/// De-dups a parameter referenced more than once. Used to seed `collect_dup_sites` — from `select_function`
/// (which then emits the dups) AND from `collect_used_ops` (which must import `OP_DUP` iff a dup site
/// exists), so the two agree on the retain set. Walks every child (a binding/reference nests anywhere).
/// Read the `Core` at `id` BY REFERENCE (no clone) and run `f` over it — the borrow twin of the common
/// `f(&core_of(db, id))` clone. During emit the tree is fully lowered/memoized, so the common path borrows
/// the memoized node from the column; falls back to the cloning `core_of` only for an un-memoized node or
/// when a `core_override` is installed (which `core_of` itself prefers). `f` returns a `Copy`/owned value
/// (it cannot hold the borrow or touch `db` mutably), so the borrow is released before the caller resumes —
/// which is what lets a RECURSIVE walk classify a node by borrow, then recurse (needing `&mut Db`) with no
/// per-node Core clone. Byte-identical to `f(&core_of(db, id))`.
pub(super) fn with_core_ref<R>(db: &mut Db, id: StructId, f: impl FnOnce(&Core) -> R) -> R {
    if db.core_override.is_empty()
        && let crate::arena::Slot::Filled(c) = db.core.get(id)
    {
        return f(c);
    }
    f(&core_of(db, id))
}

pub(super) fn collect_retain_candidate_binders(db: &mut Db, id: StructId, out: &mut Vec<StructId>) {
    // Classify the node's binder(s) by BORROW (no Core clone), then do the `type_of` checks + recursion
    // (both need `&mut Db`) after the borrow is released. `Let` collects its binder occ ids; `Param` its one.
    enum Cand {
        Let(Vec<StructId>),
        Param(StructId),
        None,
    }
    let cand = with_core_ref(db, id, |c| match c {
        Core::Let { bindings, .. } => Cand::Let(bindings.iter().map(|(b, _)| *b).collect()),
        Core::Param { binder } => Cand::Param(*binder),
        _ => Cand::None,
    });
    match cand {
        Cand::Let(binders) => {
            for binder in binders {
                // `is_heap_type_for_retain`: a still-`Var` binder type counts as a candidate (leak-safe;
                // avoids the demand-order UAF where a not-yet-ground heap payload was skipped). The dup/drop
                // EMISSION is concrete-type-gated, so a Var that solves to a scalar emits nothing.
                if is_heap_type_for_retain(&type_of(db, binder)) {
                    out.push(binder);
                }
            }
        }
        Cand::Param(binder) => {
            if is_heap_type_for_retain(&type_of(db, binder)) && !out.contains(&binder) {
                out.push(binder);
            }
        }
        Cand::None => {}
    }
    for child in core_child_ids(db, id) {
        collect_retain_candidate_binders(db, child, out);
    }
}

/// Whether any `Core::LocalRef`/`Core::Param` occurrence of `binder` WITHIN `id` is a Perceus RETAIN site
/// (present in `dup_sites`) — i.e. the binder's slot reference is DUP'd (survives that consume) somewhere in
/// `id`. The D1 drop-elide guard: a binder consumed into a later sibling initializer as a PURE MOVE (no dup
/// anywhere in that init) is fully subsumed by the later binding, so its own scope drop double-frees it and
/// must be elided; but if it has a dup_site there (the 980 shape: a then-arm MOVE-OUT plus an else-arm concat
/// that DUP'd the binder), its slot reference SURVIVES and the existing drop is its sole reclaim — do NOT
/// elide. Bottoms at the binder's own leaf occurrences; recurses every child via [`core_child_ids`].
pub(super) fn binder_has_dup_site_in(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    dup_sites: &HashSet<StructId>,
) -> bool {
    if with_core_ref(
        db,
        id,
        |c| matches!(c, Core::LocalRef { binder: b } | Core::Param { binder: b } if *b == binder),
    ) && dup_sites.contains(&id)
    {
        return true;
    }
    core_child_ids(db, id)
        .into_iter()
        .any(|c| binder_has_dup_site_in(db, c, binder, dup_sites))
}

/// Every immediate child NODE id of a Core node (all sub-expression occurrences, regardless of position).
/// Used by `collect_heap_let_binders` to find nested `let`s; positions do not matter here. Also drives
/// `layout::collect_closure_call_sigs` (the extra closure-application functype collection) — hence `pub`.
pub fn core_child_ids(db: &mut Db, id: StructId) -> Vec<StructId> {
    // REVERTED the #5600 memoized-column BORROW fast path: it caused a 68-case host-import-drop regression
    // on 28-wit-abi (bisected — reading `db.core.get(id)` directly diverged from `core_of(db, id)` for the
    // layout callee/closure walks that seed `layout.order`, so callees were missed and their host imports
    // dropped). Always go through `core_of` (the authoritative read — override → memo → captured-ref →
    // lower-on-demand), matching by reference via `child_ids_of` so the split still avoids re-cloning inside
    // the match. Behaviorally the exact pre-#5600 `match core_of(db, id)`. (A correct borrow fast path can be
    // re-derived once the divergence is understood + gated against the 28-wit-abi corpus.)
    let mut cs: Vec<StructId> = Vec::new();
    let c = core_of(db, id);
    child_ids_of(&c, &mut cs);
    cs
}

/// Extract the child occurrence ids of `c` into `cs` BY REFERENCE (no clone). The child set of each `Core`
/// variant — the sub-expression `StructId`s, all `Copy` — for the emit-side tree walks. Split out of
/// [`core_child_ids`] so the memoized-column borrow fast path there needs no `Core` clone.
pub(super) fn child_ids_of(c: &Core, cs: &mut Vec<StructId>) {
    match c {
        Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand }
        | Core::BytesCompact { operand }
        | Core::Blake3Of { operand }
        | Core::AstPrint { operand, .. }
        | Core::AstEncode { operand, .. }
        | Core::AstDecode { operand, .. }
        | Core::MapSize { map: operand }
        | Core::SetLen { set: operand }
        | Core::SetToList { set: operand, .. }
        | Core::MapToList { map: operand, .. }
        | Core::Proj { operand, .. }
        | Core::SumPayload {
            scrutinee: operand, ..
        }
        | Core::SumExpect {
            scrutinee: operand, ..
        }
        | Core::BigIntOfI64 { value: operand }
        | Core::RationalOfIntWiden { value: operand }
        | Core::BigIntToI64 { operand }
        | Core::CharToInt { operand }
        | Core::IntToCharChecked { operand, .. }
        | Core::RationalNum { operand }
        | Core::RationalDen { operand }
        | Core::StrFromBytes { bytes: operand, .. }
        | Core::StrToBytes { string: operand }
        | Core::NfcNormalize { string: operand }
        // `Value.encode`/`decode` each have one child NODE: the value / bytes operand. The `desc` bytes
        // and the `disc_some`/`disc_none` are scalars baked into the op, not sub-expression occurrences.
        | Core::ValueEncode { value: operand, .. }
        | Core::ValueDecode { bytes: operand, .. }
        | Core::Convert { operand, .. }
        | Core::Not { operand } => cs.push(*operand),
        // A `BinIntRead`/`BinRestRead` reads `bytes`, plus a `off_plus` size-sum child (§4a dynamic offset).
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => {
            cs.push(*bytes);
            if let Some(op) = off_plus {
                cs.push(*op);
            }
        }
        // A `BinSizedRead` has children: the sliced bytes, the runtime length read, and (§4a) `off_plus`.
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => {
            cs.push(*bytes);
            cs.push(*len);
            if let Some(op) = off_plus {
                cs.push(*op);
            }
        }
        Core::ListAt {
            list: a, index: b, ..
        }
        | Core::BytesAt {
            bytes: a, index: b, ..
        }
        | Core::StrAt {
            string: a,
            index: b,
            ..
        }
        | Core::StrScalarAt {
            operand: a,
            index: b,
            ..
        }
        | Core::BigIntBinOp { lhs: a, rhs: b, .. }
        | Core::BigIntCmp { lhs: a, rhs: b, .. }
        | Core::RationalBinOp { lhs: a, rhs: b, .. }
        | Core::RationalCmp { lhs: a, rhs: b, .. }
        | Core::RationalOfInts { num: a, den: b }
        | Core::ValueEq { lhs: a, rhs: b }
        | Core::ValueCmp { lhs: a, rhs: b, .. }
        | Core::ValueEqShaped { lhs: a, rhs: b, .. }
        | Core::BytesConcat { lhs: a, rhs: b }
        | Core::ListConcat { lhs: a, rhs: b }
        | Core::ListPush { list: a, elem: b }
        | Core::ListPrepend { list: a, elem: b }
        | Core::MapLookup { map: a, key: b, .. }
        | Core::MapRemove { map: a, key: b, .. }
        | Core::SetInsert {
            set: a, elem: b, ..
        }
        | Core::SetRemove {
            set: a, elem: b, ..
        }
        | Core::SetContains {
            set: a, elem: b, ..
        }
        | Core::SetAlgebra { lhs: a, rhs: b, .. }
        | Core::Arith { lhs: a, rhs: b, .. }
        | Core::Compare { lhs: a, rhs: b, .. }
        | Core::StrCmp { lhs: a, rhs: b, .. }
        | Core::FloatCompare { lhs: a, rhs: b, .. }
        | Core::And { lhs: a, rhs: b, .. } => {
            cs.push(*a);
            cs.push(*b);
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            cs.push(*bytes);
            cs.push(*start);
            cs.push(*len);
        }
        Core::StrSlice {
            string, start, end, ..
        } => {
            cs.push(*string);
            cs.push(*start);
            cs.push(*end);
        }
        Core::ListUpdate { list, index, elem } => {
            cs.push(*list);
            cs.push(*index);
            cs.push(*elem);
        }
        Core::MapInsert { map, key, val, .. } => {
            cs.push(*map);
            cs.push(*key);
            cs.push(*val);
        }
        Core::Tuple { elems }
        | Core::ListNew { elems }
        | Core::BytesOf { elems }
        | Core::SetOf { elems, .. } => cs.extend(elems.iter().copied()),
        Core::SumNew { payloads, .. } => cs.extend(payloads.iter().copied()),
        Core::Record { fields } => cs.extend(fields.values().copied()),
        Core::MapNew { entries, .. } => {
            for (k, v) in entries.iter().copied() {
                cs.push(k);
                cs.push(v);
            }
        }
        Core::BinBuild { segs } => cs.extend(segs.iter().map(|s| s.value)),
        Core::BinBitsBuild { fields } => cs.extend(fields.iter().map(|f| f.value)),
        Core::Call { args, .. } | Core::HostCall { args, .. } => cs.extend(args.iter().copied()),
        Core::CallClosure { closure, args } => {
            cs.push(*closure);
            cs.extend(args.iter().copied());
        }
        Core::Closure { captures, .. } => cs.extend(captures.iter().copied()),
        Core::Seq { stmts, tail } => {
            cs.extend(stmts.iter().copied());
            cs.push(*tail);
        }
        // A boundary block's child is its body; a break's child is its value.
        Core::Block { body, .. } => cs.push(*body),
        Core::Break { value } => cs.push(*value),
        Core::Let { bindings, body } => {
            cs.extend(bindings.iter().map(|(_, v)| *v));
            cs.push(*body);
        }
        Core::If { cond, then_, else_ } => {
            cs.push(*cond);
            cs.push(*then_);
            cs.push(*else_);
        }
        Core::Match { scrutinee, arms } => {
            cs.push(*scrutinee);
            for a in arms {
                if let Some(g) = a.guard {
                    cs.push(g);
                }
                cs.push(a.body);
            }
        }
        Core::MatchList { scrutinee, arms } => {
            cs.push(*scrutinee);
            for a in arms {
                if let Some(g) = a.guard {
                    cs.push(g);
                }
                cs.push(a.body);
            }
        }
        Core::MatchSum { scrutinee, root } => {
            cs.push(*scrutinee);
            cont_child_ids(root, cs);
        }
        Core::LocalRef { .. }
        | Core::Param { .. }
        | Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
        | Core::Captured { .. }
        | Core::Poison(_) => {}
    }
}

/// Collect the body/guard/`cond` occurrence ids of a sum-match continuation (the arms `core_child_ids`
/// reaches through a `MatchSum`). The `path` steps carry no occurrence.
pub(super) fn cont_child_ids(cont: &crate::core::SumCont, cs: &mut Vec<StructId>) {
    match cont {
        crate::core::SumCont::Leaf(body) => cs.push(*body),
        crate::core::SumCont::Guarded { cond, body, els } => {
            cs.push(*cond);
            cs.push(*body);
            cont_child_ids(els, cs);
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            cont_child_ids(then_, cs);
            cont_child_ids(els, cs);
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms.iter() {
                cont_child_ids(&a.cont, cs);
            }
        }
    }
}

/// Walk `id` in EVALUATION order marking dup sites for `binder` (see [`collect_dup_sites`]), and RETURN
/// whether `binder` occurs anywhere in the subtree. `consuming` is whether the reference reaching `id` is
/// in a consuming position (a constructor element, a call argument, a persistent-collection operand, the
/// escaping result) vs a borrow (a `Proj`/`ListLen`/`Map.size`/… read operand); `live_after` is whether
/// `binder` has a use AFTER `id` completes on the current path. A consuming `LocalRef`/`Param` occurrence
/// of `binder` with `live_after` is a dup site. Sequential operands are processed RIGHT-TO-LEFT, folding
/// each returned "occurred" into `live_after` for its earlier siblings (so an earlier consuming operand
/// sees a later sibling's use); branches (`if`/`match` arms) are independent paths, each processed with
/// the SAME incoming `live_after` (a use in a sibling arm is not "later" on this arm's path). The position
/// (borrow-vs-consume) of each child mirrors [`binding_escapes`] exactly (its `tail_borrowed` = borrow).
/// Does `binder` occur anywhere in the subtree at `id`? A CHEAP occurrence-only walk (a plain membership
/// scan over `core_child_ids` — NO site marking, NO borrow/consume/liveness logic), used by `seq`'s
/// pre-pass to decide whether a sibling references the binder. WARNING: It must NOT call `mark_binder_dups`: that
/// full two-pass walk, invoked from every `seq` level's pre-pass, is EXPONENTIAL on a deeply-nested term
/// (a `push(push(push(xs)))` chain re-walks its inner subtree once per enclosing level, 2^depth). This
/// occurrence scan visits each node of the subtree ONCE per call (the enclosing `seq` levels make it
/// O(depth × subtree) overall — polynomial, not exponential). Memoized via `cache` so a shared subterm
/// (a DAG re-walk) is not re-scanned within one query.
/// A CYCLIC Core graph (the new nullary-ctor mutual-recursion SCC can produce one) would recurse forever
/// here — the memoization above guards COMPLETED subtrees, but a node re-entered while still on the stack
/// has no cache entry yet. An `in_progress` guard breaks the cycle SOUNDLY: a back-edge to a node still
/// being computed contributes NO occurrence (any real occurrence is a `LocalRef`/`Param` LEAF, which has
/// no children and so is never on a cycle — it is always reachable via FORWARD edges), so returning
/// `false` for the back-edge can never hide a real occurrence. But a result whose computation TOUCHED a
/// back-edge is `tainted` — its `false` may be a cycle artifact for that path (a sibling querying the same
/// node off the stack must recompute it) — so only fully-determined (untainted) results are memoized. The
/// acyclic DAG bulk still memoizes, preserving the polynomial bound the doc above describes.
pub(super) fn binder_occurs(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    cache: &mut HashMap<StructId, bool>,
) -> bool {
    let mut in_progress: HashSet<StructId> = HashSet::new();
    binder_occurs_rec(db, id, binder, cache, &mut in_progress).0
}

/// SHARED OCCURRENCE ORACLE (the O(N) traversal-share substrate for `collect_dup_sites`, replacing the
/// O(binders × body-nodes) per-binder re-walk — the sread-eval ~1360-def provider-emit cliff). Computes,
/// in ONE cycle-guarded post-order pass over the body, a `node → {which of the tracked binders occur in
/// the subtree rooted there}` map, as a fixed-width BITSET keyed by each binder's index in `binders`. A
/// later occurrence query is then an O(1) bit test instead of a fresh O(N) walk per (node, binder). Mirrors
/// [`binder_occurs_rec`]'s memo/taint discipline EXACTLY so the bit for binder `i` at node `n` equals
/// `binder_occurs(n, binders[i])` — a `tainted` (cycle-back-edge) subtree result is NOT cached (a future
/// entry off-stack could reach an occurrence through the back-edge), only a definite bit is memoized.
///
/// The bitset is `Vec<u64>` words ((B+63)/64 per node); B is the tracked-binder count. Build cost is
/// O(N × B/64) — a ~64× constant win over the O(N × B) per-binder re-walk (the pragmatic operator-target
/// fix: it only speeds the OCCURRENCE lookups, never changes WHICH dup sites are marked, so the Perceus
/// site-set is preserved by construction). A binder that is not in `index` (a `Core::LocalRef`/`Param` to
/// something outside the tracked set) contributes no bit.
pub(super) fn build_occurrence_bitsets(
    db: &mut Db,
    body: StructId,
    index: &HashMap<StructId, usize>,
) -> HashMap<StructId, Vec<u64>> {
    // Width by the MAX index value + 1, NOT `index.len()`: `index` may collapse a duplicate binder id
    // (a Let-binder can appear twice in the caller's `binders` — only params are de-duped there), so the
    // distinct-key count can be smaller than the highest assigned index. Sizing by `len()` would then
    // under-allocate and the leaf `bits[i/64]` would panic for the high index (observed on the ~1360-def
    // sread-eval closure).
    let words = index
        .values()
        .copied()
        .max()
        .map_or(0, |m| m + 1)
        .div_ceil(64);
    let mut memo: HashMap<StructId, Vec<u64>> = HashMap::new();
    let mut in_progress: HashSet<StructId> = HashSet::new();
    occurrence_bitset_rec(db, body, index, words, &mut memo, &mut in_progress);
    memo
}

/// Returns `(bits, tainted)` for the subtree at `id`. `tainted` iff a cyclic back-edge was hit on this
/// path (then the result is NOT memoized — same rule as [`binder_occurs_rec`]).
pub(super) fn occurrence_bitset_rec(
    db: &mut Db,
    id: StructId,
    index: &HashMap<StructId, usize>,
    words: usize,
    memo: &mut HashMap<StructId, Vec<u64>>,
    in_progress: &mut HashSet<StructId>,
) -> (Vec<u64>, bool) {
    if let Some(bits) = memo.get(&id) {
        return (bits.clone(), false);
    }
    if in_progress.contains(&id) {
        // Cyclic back-edge: no NEW occurrence on this path; taint so the caller does not memoize a
        // cycle-artifact all-zeros.
        return (vec![0u64; words], true);
    }
    let (bits, tainted) = match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => {
            let mut bits = vec![0u64; words];
            if let Some(&i) = index.get(&b) {
                bits[i / 64] |= 1u64 << (i % 64);
            }
            (bits, false)
        }
        _ => {
            in_progress.insert(id);
            let mut bits = vec![0u64; words];
            let mut tainted = false;
            for c in core_child_ids(db, id) {
                let (cb, t) = occurrence_bitset_rec(db, c, index, words, memo, in_progress);
                for (w, cw) in bits.iter_mut().zip(cb.iter()) {
                    *w |= *cw;
                }
                tainted |= t;
            }
            in_progress.remove(&id);
            (bits, tainted)
        }
    };
    // A definite result (no taint on this path) is safe to memoize; a tainted one is withheld exactly as
    // `binder_occurs_rec` withholds a tainted `false`.
    if !tainted {
        memo.insert(id, bits.clone());
    }
    (bits, tainted)
}

/// Returns `(occurs, tainted)` — `tainted` iff the walk hit an `in_progress` back-edge, in which case the
/// result is NOT cached (see [`binder_occurs`]).
pub(super) fn binder_occurs_rec(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    cache: &mut HashMap<StructId, bool>,
    in_progress: &mut HashSet<StructId>,
) -> (bool, bool) {
    if let Some(&hit) = cache.get(&id) {
        return (hit, false);
    }
    if in_progress.contains(&id) {
        // Back-edge in a cyclic Core graph: no NEW occurrence on this path; taint so the caller does not
        // memoize a cycle-artifact `false`.
        return (false, true);
    }
    // Read the node's leaf-binder (if it IS a `LocalRef`/`Param`) by BORROW — no clone; the recursive `_`
    // arm needs `&mut Db`, so classify first, then recurse after the borrow is released.
    let leaf_binder = with_core_ref(db, id, |c| match c {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => Some(*b),
        _ => None,
    });
    let (here, tainted) = match leaf_binder {
        Some(b) => (b == binder, false),
        None => {
            in_progress.insert(id);
            let mut occurred = false;
            let mut tainted = false;
            for c in core_child_ids(db, id) {
                let (o, t) = binder_occurs_rec(db, c, binder, cache, in_progress);
                occurred |= o;
                tainted |= t;
            }
            in_progress.remove(&id);
            (occurred, tainted)
        }
    };
    // A definite `true` (a real leaf occurrence reached via forward edges) is correct regardless of taint,
    // so it is always safe to memoize; only a `tainted` `false` is withheld (it may be a cycle artifact —
    // a future query entering this node OFF the stack could reach an occurrence through the back-edge).
    if here || !tainted {
        cache.insert(id, here);
    }
    (here, tainted)
}

pub(super) fn mark_binder_dups(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    consuming: bool,
    live_after: bool,
    sites: &mut HashSet<StructId>,
) -> bool {
    // Thin entry: every position EXCEPT a `Proj`'s own operand is a "top" position for child-dup marking.
    mark_binder_dups_inner(db, id, binder, consuming, live_after, false, sites)
}

/// Whether `id` is a chain of nested-compound `Core::Proj`s ultimately rooted at `binder` — `binder`
/// itself (`(. binder k)`), or a projection of such a chain (`(. (. binder j) k)`, arbitrarily deep). Each
/// intermediate `Proj` is a BORROW (`arr-get` returns a child handle into the parent), so every child in
/// the chain aliases a cell that lives inside `binder`; a consuming op on the innermost child would
/// FBIP-mutate it while `binder` still owns it. Used by [`mark_binder_dups_inner`] to decide a child-retain
/// (`dup`) site. Only follows `Proj` links (not `SumPayload`/`ListAt`/… — those have their own retain
/// paths); bottoms out at the `LocalRef`/`Param` for `binder`.
pub(super) fn proj_chain_roots_at_binder(db: &mut Db, id: StructId, binder: StructId) -> bool {
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => b == binder,
        Core::Proj { operand, .. } => proj_chain_roots_at_binder(db, operand, binder),
        _ => false,
    }
}

/// Whether `id` is a chain of BORROWING heap-child extractions (`Core::Proj` `arr-get`, `Core::SumPayload`
/// `sum-payload`/`arr-get`, OR `Core::SumExpect` `sum-payload`, in any mix) ultimately rooted at `binder`.
/// Each intermediate step is a BORROW that returns a handle to a cell living INSIDE `binder` (no rc++), so
/// the extracted leaf aliases `binder`'s storage under its single refcount. A consuming op on the leaf would
/// FBIP-mutate it while `binder` still owns it — the [`Core::SumPayload`]/[`Core::SumExpect`]/[`Core::Proj`]
/// child-retain sites in [`mark_binder_dups_inner`] use this to decide a `dup`. The `SumPayload`/`SumExpect`
/// analogue of [`proj_chain_roots_at_binder`]; bottoms out at the `LocalRef`/`Param` for `binder`. Following
/// `SumExpect` too is load-bearing for a CHAINED extraction — `(Option.expect (Option.expect s))` over a
/// threaded `(Option (Option (List …)))`: the outer expect's scrutinee is the inner expect, which must
/// resolve through to the root `s` so the consuming op on the leaf retains.
pub(super) fn payload_or_proj_chain_roots_at_binder(
    db: &mut Db,
    id: StructId,
    binder: StructId,
) -> bool {
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => b == binder,
        Core::Proj { operand, .. }
        | Core::SumPayload {
            scrutinee: operand, ..
        }
        | Core::SumExpect {
            scrutinee: operand, ..
        } => payload_or_proj_chain_roots_at_binder(db, operand, binder),
        _ => false,
    }
}

/// The worker of [`mark_binder_dups`]. `in_proj_operand` is set ONLY when `id` is the aggregate operand of
/// an enclosing `Core::Proj` (an `arr-get`-borrowed intermediate) — used to suppress a redundant child-dup
/// mark on a nested projection in a chain (only the OUTERMOST consuming projection dups its child). Every
/// other recursion resets it to `false` (via the `mark_binder_dups` wrapper the closures call).
pub(super) fn mark_binder_dups_inner(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    consuming: bool,
    live_after: bool,
    in_proj_operand: bool,
    sites: &mut HashSet<StructId>,
) -> bool {
    // O(1) EARLY-PRUNE (the traversal-share win): if the occurrence oracle proves `binder` does not occur
    // anywhere in this subtree, it marks NO site here and its occurrence-result is `false` — identical to
    // what the full descent would compute, but without walking the (binder-free) subtree. Site-set
    // preserving by construction (a subtree with no `binder` occurrence has no `LocalRef`/`Param` for it to
    // mark, and every dup-site predicate requires the chain to root at `binder`). Skipped when there is no
    // oracle or the node wasn't memoized (tainted/cyclic) — then the walk proceeds exactly as before.
    if binder_absent_in_subtree(binder, id) {
        return false;
    }
    // A borrowing child position — recurse borrowing, threading `live_after` unchanged.
    let borrow = |db: &mut Db, c: StructId, la: bool, s: &mut HashSet<StructId>| {
        mark_binder_dups(db, c, binder, false, la, s)
    };
    // A consuming child position — recurse consuming.
    let consume = |db: &mut Db, c: StructId, la: bool, s: &mut HashSet<StructId>| {
        mark_binder_dups(db, c, binder, true, la, s)
    };
    // A SEQUENTIAL group of (child, is_borrow) evaluated left-to-right, ALL SIMULTANEOUSLY LIVE before the
    // enclosing op runs (a call pushes every arg onto the stack, then consumes them; a constructor likewise
    // holds all elements). Because they are simultaneously live, a CONSUMING occurrence of `binder` in one
    // child must retain (`dup`) if `binder` also occurs in ANY OTHER child — left OR right — not only a
    // later (right) one: an EARLIER (left) child's `local.get binder` leaves a handle on the stack that a
    // later child's consuming op (e.g. `List.push binder`) would FBIP-mutate in place at rc==1, corrupting
    // the earlier child's already-stacked handle (the self-recursive-call `(f … base … (push base) …)`
    // shape — base threaded unchanged in one arg AND consumed in a sibling). So a child's incoming
    // `live_after` must include whether `binder` occurs in any OTHER child of THIS group. Two-pass: first
    // detect occurrence in every child (a cheap pre-walk that marks no sites — `probe_only`), then process
    // each child with `la || (binder occurs in some other child)`. Still fold right-to-left within the pass
    // (preserves the later-sibling propagation), but seed each child's `la` from the group-wide occurrence
    // EXCLUDING itself. Returns whether `binder` occurred in any of them.
    let seq = |db: &mut Db,
               children: &[(StructId, bool)],
               la_in: bool,
               s: &mut HashSet<StructId>|
     -> bool {
        // Pre-pass: does `binder` occur in each child? Use the CHEAP occurrence scan (`binder_occurs`), NOT
        // `mark_binder_dups` — the latter's full two-pass walk, invoked from every nested `seq`'s pre-pass,
        // is EXPONENTIAL on a deep term (a `push(push(push(xs)))` chain: each level re-walks its inner
        // subtree, 2^depth — a real cdz-compile timeout at depth ~30). The occurrence scan is memoized and
        // marks no sites; the real site marking happens in the main pass below with the correct `live_after`.
        let mut occ_cache: HashMap<StructId, bool> = HashMap::new();
        let mut occurs: Vec<bool> = Vec::with_capacity(children.len());
        for &(c, _) in children.iter() {
            // Read the child's occurrence off the ONCE-built oracle (O(1)) when live+memoized — identical to
            // `binder_occurs` by construction — falling back to the walking scan ONLY on an oracle MISS
            // (no oracle / untracked binder / tainted-cyclic node). This is the fix for the super-linear
            // reclaim blowup: the fresh-per-`seq` `occ_cache` re-scanned overlapping subtrees at every nested
            // `seq` (db-query-diff `cdz test` hang); the oracle makes each child O(1). Exactness-preserving.
            let o = binder_occurs_via_oracle(binder, c)
                .unwrap_or_else(|| binder_occurs(db, c, binder, &mut occ_cache));
            occurs.push(o);
        }
        let any = occurs.iter().any(|&o| o);
        // Main pass, right-to-left so a later sibling's use still flows into an earlier one's `live_after`;
        // additionally seed each child's `la` with "binder occurs in some OTHER child" (the left-sibling
        // case the one-directional fold misses for simultaneously-live operands).
        let mut la = la_in;
        for i in (0..children.len()).rev() {
            let (c, is_borrow) = children[i];
            let other = any && occurs.iter().enumerate().any(|(k, &o)| k != i && o);
            let here = mark_binder_dups(db, c, binder, !is_borrow, la || other, s);
            la = la || here;
        }
        any
    };
    // A BRANCH group: a leading sequential prefix (cond/scrutinee, evaluated before the arms) then N arms,
    // each an independent path with the SAME incoming `live_after`. The prefix's `live_after` includes any
    // arm's use (an arm runs after the prefix). Returns whether `binder` occurred anywhere.
    match core_of(db, id) {
        Core::LocalRef { binder: b } | Core::Param { binder: b } => {
            if b == binder {
                if consuming && live_after {
                    sites.insert(id);
                }
                return true;
            }
            false
        }
        // Borrowing reads: the operand is borrowed (a scalar element `Proj`, a length, a lookup's map, …).
        Core::ListLen { operand } | Core::BytesLen { operand } | Core::StrScalarLen { operand } => {
            borrow(db, operand, live_after, sites)
        }
        // `Blake3.of` BORROWS its Bytes operand (an inspector, like `Bytes.len`).
        Core::Blake3Of { operand } => borrow(db, operand, live_after, sites),
        // `Ast.print` (runtime) BORROWS its Ast operand (an inspector; the baked `discs` is not a node).
        Core::AstPrint { operand, .. } => borrow(db, operand, live_after, sites),
        // `Ast.encode` (runtime) BORROWS its Ast operand (an inspector; the baked `discs` is not a node).
        Core::AstEncode { operand, .. } => borrow(db, operand, live_after, sites),
        // `Ast.decode` (runtime) BORROWS its Bytes operand (the baked `discs` is not a node; the fresh Ast
        // result is owned by the Ok wrap).
        Core::AstDecode { operand, .. } => borrow(db, operand, live_after, sites),
        // `Bytes.compact` (adv-66) is NOT a borrow — it lowers to the SAME runtime `bytes-compact` op as
        // `StrToBytes` (op_bytes_compact: flattens the rope IN PLACE and returns the SAME handle), so it
        // CONSUMES its operand and hands the handle back as the result. `binding_escapes_dup_aware` already
        // treats it consuming (the operand escapes into the result); this pass must AGREE — a binding
        // consumed by `Bytes.compact` that has a LATER live use needs a `dup` before the compact, exactly
        // like `StrToBytes` below. Misclassifying it as a borrow meant `(let ((flat (Bytes.compact rope)))
        // …)` never dup'd `rope`: the compact consumes rope's handle + returns it as `flat` (an ALIAS), so a
        // later consuming use of `flat` (`Bytes.concat flat …`) FBIP-freed the handle while `rope` (the same
        // handle) was still read (`= rope flat` / `< rope …`) → use-after-free / OOB (a wasm-only miscompile).
        Core::BytesCompact { operand } => consume(db, operand, live_after, sites),
        Core::Proj { operand, .. } => {
            let scalar_element = matches!(get_op(db, id), Ok(Some(_)));
            // A NESTED-COMPOUND projection (`get_op` None — `arr-get` returns the child HANDLE, a BORROW of
            // the parent) in a CONSUMING position, whose parent `binder` is STILL LIVE afterward, needs a
            // `dup` of the CHILD — not of the binder. The parent is already retained by its own occurrence's
            // dup, but that bumps the AGGREGATE's rc, not the child's: `arr-get` does not rc++ the child, so
            // the child has rc 1 (only the parent's array cell refs it) and the consuming op (e.g. `vec-push`)
            // FBIP-mutates it in place, corrupting a LATER re-projection `(. binder k)` that reads the same
            // child. Dup the child here so the consumer takes the persistent (copy) path and the parent's
            // array stays intact. Marked at THIS Proj node (its own id); the emit `dup`s the arr-get result.
            // The operand must resolve to the (live) `binder` through a CHAIN of nested-compound projections
            // (each an intermediate BORROW — `(. binder k)`, or `(. (. binder j) k)` two deep, …), which all
            // alias the SAME leaf child living inside `binder`'s cells. A projection off a nested/COMPUTED
            // operand (a call result, a fresh constructor) is a different owned handle handled by the
            // `reclaim` path. Scalar elements COPY out, so they never alias — no dup (the FBIP fast path and
            // scalar reads stay untouched). Only the OUTERMOST consuming projection marks (a chain's
            // intermediate projection is reached below as an `arr-get`-borrowed operand — `in_proj_operand`
            // suppresses a redundant child-dup there).
            if consuming
                && !scalar_element
                && !in_proj_operand
                && live_after
                && proj_chain_roots_at_binder(db, operand, binder)
            {
                sites.insert(id);
            }
            // Recurse for BINDER-marking (the aggregate's own dup), flagging that `operand` is a projection
            // operand (borrowed) so a nested `Proj` there does not re-mark a child-dup site.
            //
            // The consuming flag for the operand recursion is dqe17-SAFE-GATED. The bare `!scalar_element`
            // was a bug: a SCALAR-BOTTOMED chain through COMPOUND intermediates (`(. (. (. a 1) 1) 1)`)
            // reset consuming to TRUE at each compound step (`!scalar_element` ignores the incoming
            // `consuming`), so the root binder's `LocalRef` was reached consuming+live_after → a SPURIOUS
            // binder-dup. For a pure BORROW-ONLY binder (dqe4's `a`: projection + `= a b`, both borrows)
            // that dup is unbalanced (only the let-epilogue drop) → the binder ends rc1 → its owned tree
            // LEAKS (dqe4-8). BUT the same dup is LEGITIMATE for a binder that ESCAPES on some arm (dqe17's
            // `a`, returned in the then-arm): removing it also removes its `dup_sites` entry, which flips
            // `binding_escapes` to "escapes" and SUPPRESSES the arm-conditional epilogue drop → dqe17
            // regresses clean→leak. So make the compound-projection consuming-transparency
            // (`!scalar_element && consuming`) fire ONLY when the binder PROVABLY never escapes any path
            // (`binder_never_escapes`, the base `dup_sites=None` query computed in `collect_dup_sites`);
            // otherwise keep the original `!scalar_element` to preserve the escape dup. Equivalent:
            // `!scalar_element && (consuming || !never_escapes)`. borrow-only ⇒ `&& consuming` (drop the
            // spurious dup); escaping ⇒ `!scalar_element` (keep it). A genuine consuming compound proj
            // (incoming `consuming` true) is unchanged either way — no under-retain / UAF.
            //
            // dqe7/8 REFINEMENT: an ESCAPING binder (`!never_escapes`) that escapes UNCONDITIONALLY — consumed
            // on EVERY path by a straight-line op (`Map.insert`-key / `Set` elem) — ALSO has a spurious
            // projection keep-alive dup: its consume already carries its own dup, so the compound-projection
            // dup here is surplus and LEAKS (`a` ends rc 1). Suppress it too, gated on `binder_must_escapes`
            // (the sound all-paths under-approximation). dqe17's `a` escapes only CONDITIONALLY (one `if`/match
            // arm), so `must_escapes == false` there → its arm-conditional dup is PRESERVED (no regression).
            // Net gate: `!scalar_element && (consuming || (!never_escapes && !must_escapes))`. A wrong
            // `must_escapes` under-approximates to `false` ⇒ keep the dup ⇒ leak-not-UAF (the safe direction).
            let never_escapes = binder_never_escapes();
            let must_escapes = binder_must_escapes();
            mark_binder_dups_inner(
                db,
                operand,
                binder,
                !scalar_element && (consuming || (!never_escapes && !must_escapes)),
                live_after,
                true,
                sites,
            )
        }
        Core::MapSize { map } => borrow(db, map, live_after, sites),
        Core::SetLen { set } => borrow(db, set, live_after, sites),
        Core::SetToList { set, .. } => borrow(db, set, live_after, sites),
        Core::MapToList { map, .. } => borrow(db, map, live_after, sites),
        Core::SumPayload { scrutinee, .. } => {
            // A sum-match payload binder lowers to `Core::SumPayload` at EACH use (lower.rs), and
            // `sum-payload`/`arr-get` BORROW the scrutinee's payload (no rc++). A payload that is a COMPOUND
            // heap child (`get_op` None — the leaf is a handle, not an unboxed scalar) in a CONSUMING position,
            // whose scrutinee `binder` is STILL LIVE afterward, needs a `dup` of the CHILD — exactly like the
            // nested-compound `Proj` case above (and the `RestFrom` step's dup). Without it, a consuming op
            // (`List.push`/`Bytes.concat`/…) FBIP-mutates the child at rc==1 while the still-live scrutinee
            // (matched again, or threaded to a self-call) still references it → the scrutinee reads the grown
            // value (drift). `proj_chain_roots_at_binder`'s SumPayload analogue confirms the scrutinee resolves
            // to the live `binder` through a chain of borrowing payload/proj extractions. A scalar payload
            // COPIES out (no alias) so it never dups — the FBIP fast path and scalar reads stay untouched.
            // Marked at THIS node's id; the emit `dup`s the extracted child.
            let scalar_leaf = matches!(get_op(db, id), Ok(Some(_)));
            // A path ending in `RestFrom` is a list-tail slice (`vec-drop`) — the emit's `RestFrom` step
            // ALREADY dups the scrutinee before consuming (see the emit), so this node must NOT also mark a
            // child-dup (a double-dup + a slot conflict). Only a `Payload`/`Elem` COMPOUND leaf extraction
            // (a borrowing `sum-payload`/`arr-get` returning a handle) needs this retain.
            let ends_in_rest = matches!(
                core_of(db, id),
                Core::SumPayload { path, .. }
                    if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_)))
            );
            if consuming
                && !scalar_leaf
                && !ends_in_rest
                && !in_proj_operand
                && live_after
                && payload_or_proj_chain_roots_at_binder(db, scrutinee, binder)
            {
                sites.insert(id);
            }
            // Recurse for BINDER-marking on the scrutinee (borrowed), flagging it as a projection operand so a
            // nested payload/proj there does not re-mark a redundant child-dup (only the outermost consuming
            // extraction dups).
            mark_binder_dups_inner(db, scrutinee, binder, false, live_after, true, sites)
        }
        Core::SumExpect { scrutinee, .. } => {
            // The `SumExpect` twin of the `SumPayload` child-retain above: `Option.expect`/`Result.expect`
            // reads `sum-payload` (a BORROW, no rc++) of the present variant. A COMPOUND payload
            // (`get_op` None) consumed while the scrutinee `binder` is STILL LIVE (a self-recursive call
            // threading the Option, or a re-expect in the same expression) must `dup` the extracted child —
            // else the consuming op FBIP-mutates the shared payload at rc==1 and the still-live scrutinee
            // reads the grown value (drift). Same shape as `SumPayload`; no `RestFrom` case (a `SumExpect`
            // reads exactly one payload). A scalar payload COPIES out → never a site (FBIP fast path intact).
            let scalar_leaf = matches!(get_op(db, id), Ok(Some(_)));
            if consuming
                && !scalar_leaf
                && !in_proj_operand
                && live_after
                && payload_or_proj_chain_roots_at_binder(db, scrutinee, binder)
            {
                sites.insert(id);
            }
            mark_binder_dups_inner(db, scrutinee, binder, false, live_after, true, sites)
        }
        // `List.at`/`Bytes.at` BORROW the sequence; the index is a scalar (consume position, no heap).
        Core::ListAt { list, index, .. } => {
            seq(db, &[(list, true), (index, false)], live_after, sites)
        }
        Core::BytesAt { bytes, index, .. } => {
            seq(db, &[(bytes, true), (index, false)], live_after, sites)
        }
        // `String.at` CONSUMES its string (the Some branch slices out of it); the index is scalar.
        Core::StrAt { string, index, .. } => {
            seq(db, &[(string, false), (index, false)], live_after, sites)
        }
        Core::StrScalarAt { operand, index, .. } => {
            seq(db, &[(operand, false), (index, false)], live_after, sites)
        }
        // `String.slice` likewise CONSUMES its string (the Some branch `dup`s + slices out of it); the
        // start/end bounds are scalars.
        Core::StrSlice {
            string, start, end, ..
        } => seq(
            db,
            &[(string, false), (start, false), (end, false)],
            live_after,
            sites,
        ),
        // `String.from-bytes` CONSUMES its bytes operand (`str-from-bytes` transfers it out as the String).
        Core::StrFromBytes { bytes, .. } => consume(db, bytes, live_after, sites),
        // `String.to-bytes` CONSUMES its string operand (`bytes-compact` transfers it out as the Bytes).
        Core::StrToBytes { string } => consume(db, string, live_after, sites),
        // `str-nfc-normalize` CONSUMES its string operand (returns it or a fresh normalized leaf).
        Core::NfcNormalize { string } => consume(db, string, live_after, sites),
        // BigInt/Rational arith/cmp BORROW their handle operands (`tail_borrowed: true` in `binding_escapes`).
        Core::BigIntBinOp { lhs, rhs, .. }
        | Core::BigIntCmp { lhs, rhs, .. }
        | Core::RationalBinOp { lhs, rhs, .. }
        | Core::RationalCmp { lhs, rhs, .. } => {
            seq(db, &[(lhs, true), (rhs, true)], live_after, sites)
        }
        Core::BigIntOfI64 { value } | Core::RationalOfIntWiden { value } => {
            consume(db, value, live_after, sites)
        }
        Core::BigIntToI64 { operand } => borrow(db, operand, live_after, sites),
        // `Value.encode`/`decode` BORROW their value/bytes operand (inspector / fresh-construct — the result
        // retains no reference to it; the emit drops an owned-temporary operand + the baked desc after the op).
        Core::ValueEncode { value: operand, .. } | Core::ValueDecode { bytes: operand, .. } => {
            borrow(db, operand, live_after, sites)
        }
        // `Char.to-int` reads its i32-scalar Char operand and yields an i64 — a scalar read, no handle
        // retained, so it BORROWS (like `BigIntToI64`).
        Core::CharToInt { operand } | Core::IntToCharChecked { operand, .. } => {
            borrow(db, operand, live_after, sites)
        }
        // `rational-num`/`rational-den` BORROW the Rational operand (return a fresh BigInt handle).
        Core::RationalNum { operand } | Core::RationalDen { operand } => {
            borrow(db, operand, live_after, sites)
        }
        Core::RationalOfInts { num, den } => {
            seq(db, &[(num, false), (den, false)], live_after, sites)
        }
        // `value-eq`/`value-eq-shaped` BORROW both operands.
        Core::ValueEq { lhs, rhs }
        | Core::ValueCmp { lhs, rhs, .. }
        | Core::ValueEqShaped { lhs, rhs, .. } => {
            seq(db, &[(lhs, true), (rhs, true)], live_after, sites)
        }
        // Consuming constructors / ops: every operand is consumed into the result.
        Core::BytesConcat { lhs, rhs } | Core::ListConcat { lhs, rhs } => {
            seq(db, &[(lhs, false), (rhs, false)], live_after, sites)
        }
        Core::BytesSlice {
            bytes, start, len, ..
        } => seq(
            db,
            &[(bytes, false), (start, false), (len, false)],
            live_after,
            sites,
        ),
        Core::Tuple { elems } | Core::ListNew { elems } | Core::BytesOf { elems } => {
            let cs: Vec<(StructId, bool)> = elems.iter().map(|&e| (e, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::SumNew { payloads, .. } => {
            let cs: Vec<(StructId, bool)> = payloads.iter().map(|&p| (p, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::Record { fields } => {
            let cs: Vec<(StructId, bool)> = fields.values().map(|&v| (v, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::BinBuild { segs } => {
            let cs: Vec<(StructId, bool)> = segs.iter().map(|s| (s.value, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::BinBitsBuild { fields } => {
            let cs: Vec<(StructId, bool)> = fields.iter().map(|f| (f.value, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        // A `BinIntRead` BORROWS its bytes (`bytes-get` copies each byte out, retaining nothing); a
        // `BinRestRead` DUPs the scrutinee and slices the COPY (the original survives). So each occurrence
        // BORROWS the shared scrutinee — recurse with `borrow`, NOT `consume`. `consume` inserted a spurious
        // `dup` at each read of a still-live scrutinee binder (a bin-match reads the scrutinee once per field
        // probe), so the frame's rc was bumped past its single closing drop and LEAKED one frame per match —
        // the dup-placement twin of the `binding_escapes` borrow classification for these ops.
        // `off_plus` (§4a dynamic offset) is a scalar `BinIntRead` decode (borrows). Child ORDER must match
        // the operand-list arm above: `[bytes, off_plus]`.
        Core::BinIntRead {
            bytes, off_plus, ..
        }
        | Core::BinRestRead {
            bytes, off_plus, ..
        } => match off_plus {
            None => borrow(db, bytes, live_after, sites),
            Some(op) => seq(db, &[(bytes, true), (op, false)], live_after, sites),
        },
        // A `BinSizedRead` BORROWS its bytes (DUP-then-`bytes-slice` the copy — original survives, like
        // `BinRestRead`) and reads its runtime length + `off_plus` (scalar `BinIntRead` decodes). Mark the
        // bytes borrowed (`(bytes, true)`) so a still-live scrutinee binder gets no spurious dup. Child ORDER
        // must match the operand-list arm above: `[bytes, len, off_plus]`.
        Core::BinSizedRead {
            bytes,
            off_plus,
            len,
            ..
        } => match off_plus {
            None => seq(db, &[(bytes, true), (len, false)], live_after, sites),
            Some(op) => seq(
                db,
                &[(bytes, true), (len, false), (op, false)],
                live_after,
                sites,
            ),
        },
        Core::ListPush { list, elem } | Core::ListPrepend { list, elem } => {
            seq(db, &[(list, false), (elem, false)], live_after, sites)
        }
        Core::ListUpdate { list, index, elem } => seq(
            db,
            &[(list, false), (index, false), (elem, false)],
            live_after,
            sites,
        ),
        Core::MapNew { entries, .. } => {
            let mut cs: Vec<(StructId, bool)> = Vec::with_capacity(entries.len() * 2);
            for &(k, v) in entries.iter() {
                cs.push((k, false));
                cs.push((v, false));
            }
            seq(db, &cs, live_after, sites)
        }
        Core::MapInsert { map, key, val, .. } => seq(
            db,
            &[(map, false), (key, false), (val, false)],
            live_after,
            sites,
        ),
        // `Map.lookup` BORROWS the map; the key is consumed into an owned temporary.
        Core::MapLookup { map, key, .. } => {
            seq(db, &[(map, true), (key, false)], live_after, sites)
        }
        Core::MapRemove { map, key, .. } => {
            seq(db, &[(map, false), (key, false)], live_after, sites)
        }
        Core::SetOf { elems, .. } => {
            let cs: Vec<(StructId, bool)> = elems.iter().map(|&e| (e, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::SetInsert { set, elem, .. } | Core::SetRemove { set, elem, .. } => {
            seq(db, &[(set, false), (elem, false)], live_after, sites)
        }
        // `Set.contains` BORROWS the set; the element is consumed into an owned temporary.
        Core::SetContains { set, elem, .. } => {
            seq(db, &[(set, true), (elem, false)], live_after, sites)
        }
        Core::SetAlgebra { lhs, rhs, .. } => {
            seq(db, &[(lhs, false), (rhs, false)], live_after, sites)
        }
        // Arithmetic / logical: both operands consumed positions (scalars anyway; a heap binding can only
        // reach here through a producer, which resets to consuming — matches `binding_escapes`'s `false`).
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::FloatCompare { lhs, rhs, .. }
        | Core::And { lhs, rhs, .. } => seq(db, &[(lhs, false), (rhs, false)], live_after, sites),
        // `StrCmp` BORROWS both operands (heap String/Symbol handles; drops only an OWNED temporary — the
        // `ValueEq` contract, NOT the scalar-compare group whose operands are always scalars). A let-bound
        // String reaching a StrCmp operand as a direct `LocalRef` is BORROWED, so mark it `true` like
        // `ValueEq` above — else its dup/drop accounting is wrong (a missing drop → leak).
        Core::StrCmp { lhs, rhs, .. } => seq(db, &[(lhs, true), (rhs, true)], live_after, sites),
        Core::Convert { operand, .. } | Core::Not { operand } => {
            consume(db, operand, live_after, sites)
        }
        // A runtime call / host call CONSUMES each argument (callee-owns-args). Args evaluate left-to-right.
        Core::Call { args, .. } | Core::HostCall { args, .. } => {
            let cs: Vec<(StructId, bool)> = args.iter().map(|&a| (a, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        Core::CallClosure { closure, args } => {
            let mut cs: Vec<(StructId, bool)> = Vec::with_capacity(args.len() + 1);
            cs.push((closure, false));
            cs.extend(args.iter().map(|&a| (a, false)));
            seq(db, &cs, live_after, sites)
        }
        Core::Closure { captures, .. } => {
            let cs: Vec<(StructId, bool)> = captures.iter().map(|&c| (c, false)).collect();
            seq(db, &cs, live_after, sites)
        }
        // A sequencing block: statements then the tail, all sequential (each statement's value is dropped,
        // so a bare consuming statement is a consume position).
        Core::Seq { stmts, tail } => {
            let mut cs: Vec<(StructId, bool)> = stmts.iter().map(|&s| (s, false)).collect();
            cs.push((tail, false));
            seq(db, &cs, live_after, sites)
        }
        // A boundary block's body / a break's value is a single sequential value position (the body's
        // value is the block's; the break value flows out as the block's value on the abortive path).
        Core::Block { body, .. } => {
            mark_binder_dups(db, body, binder, consuming, live_after, sites)
        }
        Core::Break { value } => mark_binder_dups(db, value, binder, consuming, live_after, sites),
        // A `let`: the initializers are sequential-before the body (a `let*` later init may name an earlier
        // one). The body's position is the enclosing `consuming` (the let's value flows to where the let is
        // used). NOTE the INNER binder shadows nothing here — we track ONE outer `binder`; an inner binding
        // with the same id is impossible (each binder is a distinct node).
        Core::Let { bindings, body } => {
            let body_occurs = mark_binder_dups(db, body, binder, consuming, live_after, sites);
            // Each initializer is evaluated before the body; the body (and later inits) may use `binder`.
            let mut la = live_after || body_occurs;
            let mut any = body_occurs;
            for (_, v) in bindings.iter().rev() {
                let here = mark_binder_dups(db, *v, binder, false, la, sites);
                la = la || here;
                any = any || here;
            }
            any
        }
        // `if`: the condition is evaluated first (borrow — a bool test never consumes a heap ref into the
        // result); the two branches are INDEPENDENT paths, each with the incoming `live_after`.
        //
        // CROSS-ARM RETAIN (v-memory-safety ruling, narrowed): a binder CONSUMED in one if-arm needs a `dup`
        // iff it is ALIASED AS THE OTHER ARM'S BORROWED RESULT and the if-RESULT is DROPPED (`!consuming`).
        // Then the other arm yields the binder unchanged as the join value; because the if-result is dropped
        // (borrowed later, not moved out), the reclaim inserts an UNCONDITIONAL post-join drop of that join
        // value, which on the surviving arm's path aliases the binder — so on THIS arm's path (where the
        // binder was consumed) consume + that same drop = double-free unless the consume retained a fresh ref.
        //
        // "Aliased as the other arm's borrowed result" = `binding_escapes(arm, tb=false) &&
        // !binding_escapes(arm, tb=true)`: the binder leaves the arm ONLY as its borrowed result (it escapes
        // with a CONSUMING tail but NOT with a BORROWED tail), i.e. the arm's tail expression IS the binder /
        // a borrow-chain rooted at it. This is STRICTLY NARROWER than "net-survives the other arm" (the first
        // wiring, which over-dupped +11 corpus leak-increases): it excludes (i) a binder BORROWED INTERNALLY
        // whose result is something else (`(if c (byte-len r1) (concat r1 y))` — byte-len escapes neither way
        // → not an alias → the reclaim's per-path conditional drop already handles it, no dup), and (ii)
        // BOTH-arms-consume (`(if c (concat r1 a) (concat r1 b))` — concat escapes both ways → not an alias →
        // each path owns its own ref, no double-free). The `!consuming` gate keeps the FBIP tail-return
        // accumulator dup-free (its if-result is MOVED OUT, so there is no post-join drop to double with).
        //
        // Verified (cdz-run --report-live-objects, debug-counters runtime): 980 (then=keep=r1 alias, else=
        // concat r1 consume) → the else-consume dups, mode2 double-free UAF eliminated; the two FBIP no-dup
        // unit tests stay dup-free; a binder consumed-in-one-arm-and-ABSENT-from-the-other stays dup-free; and
        // the earlier +11 corpus leak-increases (recursion/accumulator/dedup shapes) are gone (back to base).
        // Does NOT reach a PRE-`if` multi-consume (D1) — already handled by cross-let live_after.
        Core::If { cond, then_, else_ } => {
            // The binder is aliased as `arm`'s borrowed result (escapes via a consuming tail, not a borrowed
            // tail — i.e. the tail IS the binder / a borrow-chain rooted at it), and this if's result is
            // DROPPED (`!consuming`). `binder_occurs` short-circuits the walks when the binder is absent.
            let mut occ = HashMap::new();
            let aliases_as_result =
                |db: &mut Db, arm: StructId, occ: &mut HashMap<StructId, bool>| {
                    !consuming
                        && binder_occurs(db, arm, binder, occ)
                        && binding_escapes(db, arm, binder, false)
                        && !binding_escapes(db, arm, binder, true)
                };
            let else_alias = aliases_as_result(db, else_, &mut occ);
            let mut occ2 = HashMap::new();
            let then_alias = aliases_as_result(db, then_, &mut occ2);
            // IF-JOIN-SHARED-CHILD family fix (v-mem-directed, NARROWED after the 980 ROPE UAF co-verify):
            // when the binder reaches BOTH arms' results as a MOVE-OUT or an IN-PLACE-REUSED builder BASE
            // (`escapes_as_reuse_base_or_moveout`) AND is FULLY SUBSUMED by the if-result (`!live_after`: its
            // ONLY post-if reach is through `keep`), the cross-arm dup is SPURIOUS: the two arms are MUTUALLY
            // EXCLUSIVE, so the binder (rc1, fully-subsumed) is used on exactly ONE arm at runtime and need
            // NOT survive across them. Skipping the dup lets the arm's builder REUSE the rc1 binder IN PLACE
            // (the binder's slot BECOMES `keep`), so the binder's own post-if scope drop is exactly `keep`'s
            // reclaim — one balanced drop, no leak, no double-free. (Dropping the dup was the family's mode-2
            // over-retain: 712/MAP-insert/LIST mode-2 leak = the dup's extra ref left the shared interior
            // over-retained. The scope drop is KEPT — it reclaims the reused `keep`; only the DUP is removed.)
            //
            // CRITICAL FENCE (`escapes_as_reuse_base_or_moveout`, NOT a bare consuming-escape): the load-
            // bearing premise is that the builder REUSES the base's storage in place so the base slot BECOMES
            // `keep`. That holds for `List.push`/`prepend`/`update`, `Map.insert`/`remove`, `Set.insert`/
            // `remove` — but is FALSE for `String.concat`→`bytes-concat` / `List.concat` / any ctor, which
            // ALLOCATE a fresh node holding the binder as a CHILD. There the binder is a distinct cell `keep`
            // references, so skipping the dup + keeping its scope drop DOUBLE-FREES it (the 980 rope mode-2
            // `unreachable` UAF the earlier bare `binding_escapes(.., false)` gate wrongly reached — it fires
            // for a concat child too). `!live_after` alone is not enough; the escape MODE must be reuse-in-
            // place. Conservative: a shape not proven reuse-base/move-out keeps the dup (benign known-leak).
            let then_reuse = escapes_as_reuse_base_or_moveout(db, then_, binder);
            let else_reuse = escapes_as_reuse_base_or_moveout(db, else_, binder);
            let escapes_into_if_result = !consuming && !live_after && then_reuse && else_reuse;
            // SKIP the spurious cross-arm dup when the binder escapes-into-the-if-result (fully subsumed):
            // do NOT propagate the sibling alias into the arm's `live_after` (that propagation is what marks
            // the cross-arm consume a dup_site).
            let (then_la, else_la) = if escapes_into_if_result {
                (live_after, live_after)
            } else {
                (live_after || else_alias, live_after || then_alias)
            };
            let then_occurs = mark_binder_dups(db, then_, binder, consuming, then_la, sites);
            let else_occurs = mark_binder_dups(db, else_, binder, consuming, else_la, sites);
            let cond_la = live_after || then_occurs || else_occurs;
            let cond_occurs = mark_binder_dups(db, cond, binder, false, cond_la, sites);
            cond_occurs || then_occurs || else_occurs
        }
        // A scalar `match`: the scrutinee is evaluated first; each arm (guard + body) is an independent
        // path. A guard is evaluated before its body (both on the arm's path), so within an arm the guard's
        // `live_after` includes the body's use.
        Core::Match { scrutinee, arms } => {
            let mut arms_occur = false;
            for a in arms.iter() {
                let body_occurs =
                    mark_binder_dups(db, a.body, binder, consuming, live_after, sites);
                if let Some(g) = a.guard {
                    let g_occurs =
                        mark_binder_dups(db, g, binder, false, live_after || body_occurs, sites);
                    arms_occur = arms_occur || g_occurs;
                }
                arms_occur = arms_occur || body_occurs;
            }
            let scrutinee_occurs = mark_binder_dups(
                db,
                scrutinee,
                binder,
                false,
                live_after || arms_occur,
                sites,
            );
            scrutinee_occurs || arms_occur
        }
        // A LIST match: the scrutinee is consumed (a rest arm's `vec-split` consumes the handle); each arm
        // body/guard is an independent path.
        Core::MatchList { scrutinee, arms } => {
            let mut arms_occur = false;
            for a in arms.iter() {
                let body_occurs =
                    mark_binder_dups(db, a.body, binder, consuming, live_after, sites);
                if let Some(g) = a.guard {
                    let g_occurs =
                        mark_binder_dups(db, g, binder, false, live_after || body_occurs, sites);
                    arms_occur = arms_occur || g_occurs;
                }
                arms_occur = arms_occur || body_occurs;
            }
            let scrutinee_occurs =
                mark_binder_dups(db, scrutinee, binder, true, live_after || arms_occur, sites);
            scrutinee_occurs || arms_occur
        }
        // A SUM match: the scrutinee is evaluated first; the continuation's arms are independent paths.
        Core::MatchSum { scrutinee, root } => {
            let cont_occurs = mark_cont_dups(db, &root, binder, consuming, live_after, sites);
            let scrutinee_occurs = mark_binder_dups(
                db,
                scrutinee,
                binder,
                false,
                live_after || cont_occurs,
                sites,
            );
            scrutinee_occurs || cont_occurs
        }
        // Leaves / non-binding nodes.
        Core::ConstInt(_)
        | Core::ConstRational(_, _)
        | Core::ConstBool(_)
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::ConstChar(_)
        | Core::ConstFloat(_)
        | Core::ConstFloatNan
        | Core::ConstFloatInf
        | Core::Unit
        | Core::Trap
        | Core::TrapDivZero
        | Core::TrapOverflow
        | Core::Captured { .. }
        | Core::Poison(_) => false,
    }
}

/// Mark dup sites through a sum-match CONTINUATION (mirrors `cont_binding_escapes`): every leaf body /
/// guarded arm / literal-test / nested switch is an independent path, each processed with the incoming
/// `consuming`/`live_after`. The `path` steps (`Payload`/`Elem`) are heap reads carrying no binding.
/// Returns whether `binder` occurs anywhere in the continuation.
pub(super) fn mark_cont_dups(
    db: &mut Db,
    cont: &crate::core::SumCont,
    binder: StructId,
    consuming: bool,
    live_after: bool,
    sites: &mut HashSet<StructId>,
) -> bool {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            mark_binder_dups(db, *body, binder, consuming, live_after, sites)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            let body_occurs = mark_binder_dups(db, *body, binder, consuming, live_after, sites);
            let els_occurs = mark_cont_dups(db, els, binder, consuming, live_after, sites);
            // The guard is evaluated before the guarded body (same path); the fall-through `els` is a
            // separate path. The guard only reads (never consumes into the result).
            let cond_occurs =
                mark_binder_dups(db, *cond, binder, false, live_after || body_occurs, sites);
            body_occurs || els_occurs || cond_occurs
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            let then_occurs = mark_cont_dups(db, then_, binder, consuming, live_after, sites);
            let els_occurs = mark_cont_dups(db, els, binder, consuming, live_after, sites);
            then_occurs || els_occurs
        }
        crate::core::SumCont::Switch { arms, .. } => {
            let mut occurs = false;
            for a in arms.iter() {
                occurs =
                    mark_cont_dups(db, &a.cont, binder, consuming, live_after, sites) || occurs;
            }
            occurs
        }
    }
}

// ============================================================================
// INC1: recursive-sum owned-param-shell reclaim — SOUND SELECTION (recovered from f455bf3bdb,
// adapted to current main: count_param_consumes now takes the count_restfrom arg). The reclaim EMIT
// half is the existing single-op_drop param_reclaim (op_drop cascades — no bespoke recursive drop).
// ============================================================================

/// Whether `body` is a CAPTURING-CLOSURE lifted body — in `db.lifted` AND with a non-empty capture set (an
/// `(env, param…)` closure whose params ARE built + `drop_after`'d by the caller at the `call_indirect`
/// boundary). The refined boundary-owned discriminator (INC1): a lifted COMBINATOR (empty captures — a named
/// recursive def like BST del-min/insert hoisted to the funcref table but called DIRECTLY, no env slot) is
/// NOT caller-`drop_after`'d — it receives an OWNED (dup'd) param and must reclaim it itself. A genuine
/// capturing closure stays excluded. NOTE: this refines ONLY the INC1 nontail-spine SELECTION; it does NOT
/// change the global `is_boundary_owned` (which the 05:18721 surplus/caller-drop still read as db.lifted).
pub(super) fn body_is_capturing_lifted(db: &Db, body: StructId) -> bool {
    db.lifted
        .iter()
        .any(|l| l.body == body && !l.captures.is_empty())
}

/// Whether a `MatchSum` over `scrutinee` (continuation `root`) in function body `top_body` is the NON-TAIL
/// SPINE PARAM case for a COMPOUND payload: an owned-by-flow compound boxed-sum PARAM matched exactly once +
/// consumed ONLY by the match, NOT a capturing-closure body, NOT §5 self-loop-tail-consumed. The tail-
/// `MatchSum` emit reclaims its shell via the param SLOT, so its consumed spine payload must be dup'd in
/// lockstep. (Recovered from f455bf3bdb; count_param_consumes gets the current `, true` arg.)
pub(super) fn is_nontail_spine_param(
    db: &mut Db,
    top_body: StructId,
    scrutinee: StructId,
    root: &crate::core::SumCont,
) -> bool {
    let scrut_ty = type_of(db, scrutinee);
    let compound_boxed = is_heap_type(&scrut_ty)
        && !ty_is_enum_disc(db, &scrut_ty)
        && !sum_has_only_scalar_payloads(db, &scrut_ty);
    compound_boxed
        && !body_is_capturing_lifted(db, top_body)
        && matches!(core_of(db, scrutinee), Core::Param { binder } | Core::LocalRef { binder } if {
            let mut seen2 = HashSet::new();
            let mut total = 0usize;
            count_param_consumes(db, top_body, binder, &mut seen2, &mut total, true);
            total == 0 && count_matchsum_over_binder(db, top_body, binder) <= 1
        })
        && !sum_cont_payload_consumed_in_tail_call(db, root, scrutinee)
}

/// Collect the PARAM BINDERS whose owned-shell the tail-`MatchSum` emit reclaims via the param slot for a
/// COMPOUND payload (INC1 — the drop side of the [`is_nontail_spine_param`] dup side). Shares the exact
/// predicate so `nontail_compound_reclaim_binders` ⟺ the dups `collect_shell_reclaim_child_dups` marks.
pub(super) fn collect_nontail_compound_reclaim_binders(
    db: &mut Db,
    body: StructId,
    out: &mut HashSet<StructId>,
) {
    let mut seen = HashSet::new();
    collect_nontail_compound_reclaim_binders_seen(db, body, body, out, &mut seen);
}

fn collect_nontail_compound_reclaim_binders_seen(
    db: &mut Db,
    id: StructId,
    top_body: StructId,
    out: &mut HashSet<StructId>,
    seen: &mut HashSet<StructId>,
) {
    if !seen.insert(id) {
        return;
    }
    if let Core::MatchSum { scrutinee, root } = core_of(db, id)
        && is_nontail_spine_param(db, top_body, scrutinee, &root)
        && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, scrutinee)
    {
        out.insert(binder);
    }
    for child in core_child_ids(db, id) {
        collect_nontail_compound_reclaim_binders_seen(db, child, top_body, out, seen);
    }
}

/// Whether the callee function `body` will INC1 non-tail-spine-reclaim its param `param_binder` — i.e. `body`
/// contains a `MatchSum` over `param_binder` that [`is_nontail_spine_param`] selects (the callee drops that
/// owned recursive-sum scrutinee's shell itself, at every recursion frame). Used by `call_arg_caller_drops`
/// (APPROACH B): the caller-drop YIELDS to this — callee-reclaim covers ALL frames (top + inner, inner
/// reachable only by the callee), the caller-drop covers only the top, so the callee is the COMPLETE owner.
/// Making this exclusion UNCONDITIONAL (not gated on whether the caller-drop fires today) is future-proof:
/// a later `param_escapes_body` change can never silently double-free an INC1-reclaimed param.
pub(super) fn def_inc1_reclaims_param(db: &mut Db, body: StructId, param_binder: StructId) -> bool {
    fn go(
        db: &mut Db,
        id: StructId,
        top_body: StructId,
        pb: StructId,
        seen: &mut HashSet<StructId>,
    ) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Core::MatchSum { scrutinee, root } = core_of(db, id)
            && matches!(core_of(db, scrutinee), Core::Param { binder } | Core::LocalRef { binder } if binder == pb)
            && is_nontail_spine_param(db, top_body, scrutinee, &root)
        {
            return true;
        }
        core_child_ids(db, id)
            .into_iter()
            .any(|c| go(db, c, top_body, pb, seen))
    }
    let mut seen = HashSet::new();
    go(db, body, body, param_binder, &mut seen)
}
