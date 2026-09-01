//! Heap-operand OWNERSHIP classification (heap_operand_ownership + join_arm_ownership +
//! sum_cont_ownership) — extracted from select.rs to keep it under xtask_support::MAX_SOURCE_BYTES
//! (512 KiB). Pure code move, behavior-neutral. `use super::*` brings the select module items
//! (HandleOwnership, Core, core_of, Reject, is_bigint_valued, Ty, ...) into scope, as the sibling
//! select/* submodules do.
use super::*;

pub(super) fn heap_operand_ownership(db: &mut Db, id: StructId) -> Result<HandleOwnership, Reject> {
    match core_of(db, id) {
        // Constructors and calls produce a fresh owned reference (ownership transfers out). A map
        // construction/update (`map-empty`+inserts, `map-insert`, `map-remove`) returns a fresh owned
        // map handle exactly like a list/tuple constructor — the `value-eq` emit drops it after the compare.
        Core::SumNew { .. }
        | Core::Tuple { .. }
        | Core::Record { .. }
        | Core::ListNew { .. }
        // A list PRODUCER returns a FRESH owned list handle exactly like a `ListNew` constructor:
        // `vec-push` (`ListPush`) CONSUMES its input list + element and returns a NEW owned list (the old
        // version untouched — persistence); `vec-concat` (`ListConcat`) consumes both operands and hands
        // back a new vector; `vec-update` (`ListUpdate`) returns the updated list as a new handle. So such a
        // list as a BORROWING-op operand (`List.len`/`List.at`/`= `) is an owned temporary the emit reclaims
        // after the borrow — the LIST analog of the `BytesConcat`/`BytesSlice` producers below. WITHOUT
        // this, `List.len (List.push (build …) x)` / `List.len (List.concat a b)` / `List.at (List.update l
        // i x) j` fell to the `_ => decline` default, so the reclaim gate never fired and the fresh list
        // LEAKED one vector per call (value correct — a leak, not a miscompile). Classifying `ListPush`
        // Owned reclaims only the fresh PUSH RESULT that a borrowing op leaves un-consumed; the ACCUMULATOR
        // pattern `(build i n (List.push acc i))` returns the push as its TAIL (never a borrowing-op
        // operand), so its ownership is never consulted here and the FBIP single-consume fast path is
        // untouched. A shared/borrowed `acc` (rc>1) is already `dup`'d before the push by the Perceus retain,
        // so `vec-push` produces a genuinely fresh result — dropping it can never double-free the live `acc`.
        | Core::ListPush { .. }
        | Core::ListPrepend { .. }
        | Core::ListConcat { .. }
        | Core::ListUpdate { .. }
        // A fallible READ that returns an `(Option T)` builds a FRESH owned `sum-new` Some shell (or a
        // nullary None) around the extracted/copied payload: `List.at`/`Bytes.at` (`vec-get`/`bytes-get`
        // into `Some`), `Map.lookup` (the looked-up value dup'd into `Some`). So the Option RESULT is an
        // owned temporary — when it is a `Option.expect`/match SCRUTINEE, the `SumExpect`/`MatchSum` emit
        // reclaims the shell after the payload read (else the shell LEAKS one cell per call — the
        // live-objects-gate find). These BORROW their collection (that reclaim is handled at the op's own
        // emit); here we classify the fallible RESULT they hand out, which is a fresh owned sum.
        | Core::ListAt { .. }
        | Core::BytesAt { .. }
        // NOTE: `String.at` (`Core::StrAt`) also returns a FRESH owned `Some(char-view)`, but it is
        // DELIBERATELY NOT classified Owned here — a GLOBAL reclassification perturbs the MatchSum Stage-B
        // extraction-consume reclaim (a String.at-view consumed by an allowlisted `String.concat` nets +1 =
        // a latent Stage-B imbalance, v-mem-safety's separate follow-up). Instead the (2) SumExpect view/shell
        // reclaim treats a `StrAt` scrutinee as owned LOCALLY (in the SumExpect `reclaim_shell` emit, gated by
        // the `sumexpect_view_reclaim`/`sumexpect_shell_reclaim` membership) — same local>global discipline as
        // the reclaim_bytes-local (2) check, so the MatchSum/value-eq/Stage-B consumers see StrAt UNCHANGED.
        | Core::MapLookup { .. }
        | Core::MapNew { .. }
        | Core::MapInsert { .. }
        | Core::MapRemove { .. }
        // `Map.merge` (`MapMerge`) returns a FRESH owned map handle — `map-merge` CONSUMES both operands and
        // hands back a new CHAMP map (the map analog of `ListConcat`). As a BORROWING-op operand (`Map.len
        // (Map.merge a b)` / `= (Map.merge …) m`) it is an owned temporary the emit must reclaim after the
        // borrow. WITHOUT this it fell to `_ => decline`, so the reclaim gate never fired and the fresh
        // merged map LEAKED its root cell (value correct — a leak, not a miscompile; WAT-confirmed: the
        // map-merge result was consumed by a borrowing `map-size` with no drop). The merge's ENTRIES are
        // dup'd from the operands (or immortal statics), so dropping only the fresh spine never double-frees.
        | Core::MapMerge { .. }
        // A constant string/bytes materializes a FRESH owned byte-leaf handle (`bytes-alloc`+`bytes-set`,
        // see the `Core::ConstStr`/`ConstBytes`/`BytesOf` emit), so — like a constructor — the `value-eq`
        // emit drops it after the borrowing compare. This is the `(= h "+")` shape: comparing a runtime
        // payload string against a constant-string literal.
        | Core::ConstStr(_)
        | Core::ConstBytes(_)
        | Core::BytesOf { .. }
        // A runtime Bytes/String PRODUCER returns a FRESH owned handle: `bytes-concat`/`bytes-slice`/
        // `bytes-compact` each consume their operand(s) and hand back a new sequence; `str-from-bytes`
        // transfers the validated buffer out as a String; `str-to-bytes` (= `bytes-compact`) flattens the
        // string's byte-rope out as a fresh Bytes leaf. So such a value as a DIRECT `value-eq` operand is
        // owned and the emit drops it after the borrowing compare — the `(= (String.to-bytes s) b"…")` shape
        // the compiler-in-Cadenza codec's byte round-trip compares.
        | Core::BytesConcat { .. }
        | Core::BytesSlice { .. }
        | Core::BytesCompact { .. }
        // `str-slice` (`String.slice`) returns a FRESH owned `(Option String)` — a fallible read that copies
        // the slice range into a new `Some(str)` (or `None`), exactly like `BytesAt`/`BytesSlice` above. So a
        // `String.slice` result used as a MatchSum SCRUTINEE is an OWNED computed boxed-sum, and its shell +
        // consumed payload must be reclaimed by the shell-reclaim child-dup path. WITHOUT this arm it fell to
        // the `_ => decline` ("ownership this backend cannot yet prove") default, so `sum_shell_reclaim_ok` /
        // `collect_shell_reclaim_child_dups` bailed and a payload CONSUMED MORE THAN ONCE off the slice (the
        // adv54b class: `tail` = `String.slice`'s payload fed to two `String.to-bytes` under one
        // `Bytes.concat`) got NO child-`dup` → the shared payload's ref was released twice → DOUBLE-FREE.
        | Core::StrSlice { .. }
        | Core::StrFromBytes { .. }
        | Core::StrToBytes { .. }
        // `str-nfc-normalize` (FINDING #23) hands out a FRESH owned String handle: it CONSUMES its operand and
        // returns either that same handle (already NFC) or a fresh normalized leaf with the original dropped —
        // either way an owned reference transfers out, exactly like `StrToBytes`/`BytesConcat`. So as a
        // borrowing-op operand it is Owned and the emit reclaims it after the borrow.
        | Core::NfcNormalize { .. }
        // `Blake3.of` on a runtime Bytes (`hash-blake3`, op 91) BORROWS its operand and returns a FRESH
        // owned 32-byte Bytes leaf. So a `Blake3.of` result used as a `value-eq` operand — the P4
        // dispatch shape `(= (Blake3.of msg-contract) id)` / `(= (Blake3.of x) (Blake3.of y))` — is an
        // owned temporary the borrow must reclaim. WITHOUT this it fell to `_ => decline` ("ownership this
        // backend cannot yet prove"), blocking a runtime-digest equality (a completeness gap, not a
        // miscompile) even though a `Bytes.of` of the same content compares fine.
        | Core::Blake3Of { .. }
        // `Ast.encode` (op 93) BORROWS its Ast operand and returns a FRESH owned Bytes document; `Ast.print`
        // (op 92) likewise returns a FRESH owned String. So such a result used as a BORROWING-op operand — a
        // bare `(= (Ast.encode a) (Ast.encode b))`, or the PROBE of `Set.contains`/`Map.lookup` keyed by
        // arena Bytes (`(Set.contains s (Ast.encode …))`) — is an owned temporary the borrow must reclaim.
        // WITHOUT this both fell to `_ => decline` ("ownership this backend cannot yet prove"): CHAMP
        // construction dedups arena Bytes fine (the constructor path never consults this), but the query-
        // entry / bare-compare lowering DECLINED (breaker xf3/xf4 + beq bare-`=`). The Ast twin of `Blake3Of`
        // / `ValueEncode` above.
        | Core::AstEncode { .. }
        | Core::AstPrint { .. }
        // A runtime `(bin …)` construction builds a FRESH owned Bytes on the rope heap (`bytes-alloc` +
        // per-segment range-check-and-write, exactly like `BytesOf`), so as a `value-eq` operand it is
        // Owned and the emit drops it after the borrowing compare. WITHOUT this a runtime `(bin …)` result
        // — spec'd a Bytes value — fell to the `_ => decline` below, so `(= (bin (u8 v)) b"…")` DECLINED
        // "an ownership this backend cannot yet prove" even though a `Bytes.of` of the same content compares
        // fine (a completeness gap, not a miscompile). `BinBitsBuild` (a `(bits v k)` run) is the same.
        | Core::BinBuild { .. }
        | Core::BinBitsBuild { .. }
        // A dependent-size / final-rest bin-match PAYLOAD read returns a FRESH owned Bytes: `BinSizedRead`
        // and `BinRestRead` both emit `dup(scrutinee); bytes-slice(copy, off, len)`, and `bytes-slice`
        // CONSUMES the dup'd copy + hands back a NEW owned slice (exactly like `BytesSlice`). So the slice
        // as a borrowing-op operand (`Bytes.len payload` — the payload binder lowers to an INLINE
        // `BinSizedRead`, so `BytesLen`'s operand IS this node) is an owned temporary the borrow must
        // reclaim. WITHOUT this the slice fell to `_ => decline`, `BytesLen`'s reclaim gate never fired,
        // and a fresh slice LEAKED one Bytes cell per dependent-size match (value-correct — a leak, not a
        // miscompile; witnessed by `dependent_size_bin_match_payload_read_leaves_no_live_objects`). The
        // Bytes analog of the `ListConcat`/`ListPush` producer gap fixed earlier.
        | Core::BinSizedRead { .. }
        | Core::BinRestRead { .. }
        // `Value.encode` returns a FRESH owned `Bytes` document handle (the runtime `value-encode` op mints
        // a new doc, borrowing its value operand), and `Value.decode` returns a FRESH owned value handle
        // (`sum-new(Some, decoded)` over the fresh `value-decode` result, or the nullary `None`). So a
        // `Value.encode`/`Value.decode` RESULT used as a borrowing-op operand — the round-trip `(let ((bs
        // (Value.encode v))) (Value.decode bs) …)` where `bs` feeds `value-decode` (a borrow) — is an owned
        // temporary the borrow must reclaim. WITHOUT this it fell to `_ => decline` ("ownership this backend
        // cannot yet prove"), blocking the whole R2 round-trip at emit (v-inference's decode-grounding got it
        // past typing, then this fired). The Bytes/value analog of the collection-producer arms below.
        | Core::ValueEncode { .. }
        | Core::ValueDecode { .. }
        // `Char.from-int` (`IntToCharChecked`) wraps the boxed codepoint into a FRESH owned `(Option Char)`
        // sum (`disc_some`/`disc_none`), exactly like `ValueDecode`'s `sum-new(Some, …)`/`None`. So its result
        // used as a MatchSum SCRUTINEE — `(match (Char.from-int n) ((Some c) …) ((None _) …))` — is an OWNED
        // computed boxed-sum whose Option SHELL the shell-reclaim must drop. WITHOUT this it fell to the
        // `_ => decline` default, so `sum_shell_reclaim_ok` bailed and the Option shell LEAKED one cell per
        // match (arm- AND payload-independent — the shell, not the boxed Char; v-mem's chfi triage). The Char
        // twin of the `StrSlice`(Option String) / `ValueDecode`(Option value) fresh-owned-sum producer arms.
        | Core::IntToCharChecked { .. }
        // A set construction/update/algebra (`set-empty`+inserts, `set-insert`, `set-remove`, union/
        // intersection/difference) returns a fresh owned set handle — the `value-eq` emit drops it.
        | Core::SetOf { .. }
        | Core::SetInsert { .. }
        | Core::SetRemove { .. }
        | Core::SetAlgebra { .. }
        // A collection ENUMERATION returns a FRESH owned `List` handle: `map-to-list` materializes the map's
        // entries as a new `List (Tuple k v)` (canonical key order), `set-to-list` the set's elements as a new
        // `List`. So the enumeration RESULT as a borrowing-op operand (`List.len (Map.to-list m)`) is an owned
        // temporary the borrow must reclaim. WITHOUT this they fell to `_ => decline`, the borrowing consumer's
        // reclaim gate (e.g. `ListLen`) never fired, and the fresh result list (+ its boxed entries) LEAKED
        // per call — value-correct, a leak not a miscompile; the enumeration analog of the `ListConcat`/
        // `ListUpdate` producer gap. (This governs the to-list RESULT; a leak of an owned-temporary SOURCE
        // handed TO to-list is a separate face — the to-list op borrows its source — tracked by
        // `map_or_set_to_list_over_an_owned_temporary_source_leaks_it_known_gap`.)
        | Core::MapToList { .. }
        | Core::SetToList { .. }
        // A BigInt PRODUCER returns a fresh owned handle: `bigint-of-i64` mints a leaf, and each
        // `bigint-add`/`-sub`/`-mul`/`-div` re-boxes a normalized result (the operands are borrowed, the
        // result is new). So a BigInt operand that is itself the result of another BigInt op is owned —
        // the enclosing op drops it after borrowing. (`BigIntToI64` returns an i64 scalar, never a handle,
        // so it is not a heap operand and never reaches here.)
        | Core::BigIntOfI64 { .. }
        | Core::BigIntBinOp { .. }
        // `rational-num`/`rational-den` return a FRESH owned BigInt handle (they unbox the Rational's
        // component + re-box it, borrowing the Rational), so a `Rational.numerator`/`denominator` result
        // used as a borrowing-op operand (e.g. `Int64.of (Rational.numerator r)`) is Owned — the enclosing
        // op drops it after borrowing.
        | Core::RationalNum { .. }
        | Core::RationalDen { .. }
        // A Rational PRODUCER likewise returns a fresh owned handle: `rational-of` (`RationalOfInts`/
        // `RationalOfIntWiden`) builds a new 2-handle node, and each `rational-add`/…-`div` re-normalizes
        // into a new node. So a Rational operand that is itself a Rational op's result is owned.
        | Core::RationalOfInts { .. }
        | Core::RationalOfIntWiden { .. }
        | Core::RationalBinOp { .. }
        // A HOST/PEER call returning a COMPOUND yields a fresh OWNED handle (a peer-bound effect returns a
        // runtime value the consumer now owns — the shared-runtime handle transport, U5/U11), exactly like
        // a defined-func `Call`. So a peer-returned compound projected/consumed here is an owned temporary
        // the enclosing op reclaims (U13) rather than leaking until run-end.
        | Core::HostCall { .. }
        | Core::Call { .. } => Ok(HandleOwnership::Owned),
        // A CONSTANT typed `BigInt` materializes to a FRESH owned handle at `emit` (the `Core::ConstInt`
        // arm routes a BigInt-typed constant through `bigint-of-i64`), exactly like `ConstStr` above — so
        // as a borrowing-op operand it is Owned and the emit drops it. This is what lets `Int64.of (if c
        // (BigInt.of 1) (BigInt.of 2))` narrow a BigInt-valued `if` whose branches are constant BigInts.
        // A constant BigInt (bare OR a BigInt-inner quantity — `is_bigint_valued` peels the `Qty`)
        // materializes to a FRESH owned handle at emit (the `Core::ConstInt` arm routes it through
        // `bigint-of-i64`), exactly like `ConstStr` — so as a borrowing bigint-op operand it is Owned and
        // the emit drops it. Covers `(+ (Qty.of (BigInt.of v) m) (Qty.of (BigInt.of 100) m))` (runtime +
        // constant BigInt quantity) and `Int64.of (if c (BigInt.of 1) (BigInt.of 2))`.
        Core::ConstInt(_) if is_bigint_valued(db, id) => Ok(HandleOwnership::Owned),
        // A CONSTANT Rational likewise materializes to a FRESH owned handle at `emit` (`bigint-of-i64` ×2
        // + `rational-of`), so as a borrowing-op operand it is Owned.
        Core::ConstRational(_, _) => Ok(HandleOwnership::Owned),
        // A `Core::Closure` builds a FRESH owned cell (`arr-alloc` of the code index + the dup'd captures),
        // ownership transferring out exactly like a `SumNew`/`Tuple`/`Record` constructor — so as the operand
        // of a BORROWING op it is an owned temporary the enclosing op reclaims after the borrow. This is the
        // SITE-A part (a): the eta-closure `((if c (T.Mk 0) (T.Mk 10)) 5)` distributes the projection into the
        // `if`, so the `CallClosure` operand is an `If`-of-partial-ctors whose arms are each a fresh owned
        // closure; the `Core::If` join (`join_arm_ownership`) can now flip that operand to Owned, which is the
        // precondition v-effects' CallClosure emit (part b) gates its `cell_slot` drop on. Classifying a
        // Closure Owned is UNCONDITIONALLY correct (a freshly-built cell is genuinely owned); the SOUNDNESS
        // that keeps a SHARED/curried/re-applied/forced-thunk cell from being wrongly dropped lives ENTIRELY
        // in v-effects' emit gate (full-arity apply + non-escaping result), NOT here — this arm only states
        // "the cell is owned", never "the cell may be dropped". No reclaim consumer today takes a Fn-typed
        // operand except that CallClosure emit, so until part (b) lands this arm is observably inert (a Fn
        // cannot be a List.len/value-eq/map-key/sum-scrutinee operand — those are type errors), which is what
        // lets part (a) land independently.
        Core::Closure { .. } => Ok(HandleOwnership::Owned),
        // A reference to a parameter or a kept `let`-binding — the owner elsewhere reclaims it.
        Core::Param { .. } | Core::LocalRef { .. } => Ok(HandleOwnership::Borrowed),
        // A list REST-BINDER read (`(list _ .. r)` → a `SumPayload` whose path's SOLE/final step is a
        // `RestFrom`) is the EXCEPTION: its emit is `dup(scrutinee); vec-drop(k)`, and `vec-drop` CONSUMES
        // the dup'd copy and hands back a NEW owned tail vector (the scrutinee's own count nets to zero —
        // see the `RestFrom` emit). So the extracted rest is a FRESH OWNED temporary, exactly like a
        // `ListConcat`/`ListUpdate` producer — NOT a borrow. A borrowing consumer (`List.len r`) must
        // therefore reclaim it after the borrow (else the fresh tail LEAKS one vector + its shared leaf per
        // read — the lm5 live-objects find), and as an arm RESULT it transfers out (Owned). Each reference
        // re-emits its own `dup`+`vec-drop` (an inline `SumPayload`, one per occurrence), so classifying it
        // Owned drops exactly one fresh vector per emit — never a double-free. Any OTHER path (a plain
        // `Payload`/`Elem` read) still BORROWS, as below.
        Core::SumPayload { path, .. }
            if matches!(path.last(), Some(crate::core::PathStep::RestFrom(_))) =>
        {
            Ok(HandleOwnership::Owned)
        }
        // A payload/element READ (`sum-payload`/`arr-get`) BORROWS its operand — the enclosing compound
        // owns the sub-value, so the read yields a borrowed handle the `value-eq` emit must NOT drop
        // (`sum-payload`/`arr-get` read without transferring ownership; see `binding_escapes`). This is
        // the shape a recursive tree-walker compares — `(= h "+")` where `h` is a variant's tuple-payload
        // element bound via `SumPayload`. `SumExpect` (an `Option.expect` payload read) borrows likewise.
        // (A `String.at` `Some` payload is a rope slice, but it is COMPACTED at the producer — the `StrAt`
        // Some-branch flattens the slice before wrapping it — so the extracted payload is a flat leaf that
        // `value-eq` compares correctly without reclassifying this borrow as owned; see `Core::StrAt` emit.)
        Core::SumPayload { .. } | Core::SumExpect { .. } | Core::Proj { .. } => {
            Ok(HandleOwnership::Borrowed)
        }
        // Control flow: the operand's value is produced on one of several paths, so its ownership is the
        // JOIN of the reachable results — OWNED only when EVERY path provably yields a fresh owned
        // temporary (so the single post-compare drop is correct on all paths), else BORROWED. Classifying
        // BORROWED is always leak-safe: the emit then does NOT drop the operand, so a path that actually
        // produced an owned temporary merely LEAKS it (the conservative bias `binding_escapes` states — a
        // false "borrowed" only leaks) rather than risk freeing a borrowed path's still-live value under
        // its owner (a double-free). This mirrors the standalone-function path exactly: a body returning a
        // borrowed match payload leaves the value un-dropped and leaks the scrutinee it borrows from.
        //
        // `if` joins both arms; `let` forwards its body; a `match` (scalar / sum / list) joins its arm
        // bodies (`join_arm_ownership` / `sum_cont_ownership`). A bare-`Leaf`-rooted sum match folds to its
        // body in `lower` and never reaches here as a `MatchSum`.
        Core::If { then_, else_, .. } => Ok(join_arm_ownership(db, [then_, else_])),
        Core::Let { body, .. } => heap_operand_ownership(db, body),
        Core::Match { arms, .. } => {
            Ok(join_arm_ownership(db, arms.iter().map(|a| a.body)))
        }
        Core::MatchList { arms, .. } => {
            Ok(join_arm_ownership(db, arms.iter().map(|a| a.body)))
        }
        Core::MatchSum { root, .. } => Ok(sum_cont_ownership(db, &root)),
        // When the operand's ownership (its aliasing status — whether the enclosing op may reclaim it or must
        // leave it to another owner) cannot be established by any arm above, DECLINE rather than emit a
        // component whose dup/drop placement would be a guess: the aliasing discipline could not be proven
        // safe here, so refusing is the sound outcome, not an unchecked emit with unspecified aliasing.
        //= spec/capabilities/memory-and-resource-model.md#aliasing-is-statically-disciplined
        //# The compiler MUST reject a program whose aliasing the memory discipline cannot establish as safe, rather than emit a component with unspecified aliasing behavior.
        _ => Err(Reject::unsupported(
            "borrowing op operand has an ownership this backend cannot prove",
        )),
    }
}

/// The JOIN of several result positions' ownership for a borrowing-op operand (see
/// [`heap_operand_ownership`]): [`HandleOwnership::Owned`] iff EVERY body is provably `Owned`, otherwise
/// [`HandleOwnership::Borrowed`]. A body whose ownership cannot be proven counts as `Borrowed` — the
/// leak-safe join value, so an unhandled arm shape never declines the whole match (it just leaves the
/// operand un-dropped, a leak, never a double-free). Empty (a match with no arms cannot reach a value)
/// is `Borrowed` — the safe default.
fn join_arm_ownership(db: &mut Db, bodies: impl IntoIterator<Item = StructId>) -> HandleOwnership {
    for body in bodies {
        if !matches!(heap_operand_ownership(db, body), Ok(HandleOwnership::Owned)) {
            return HandleOwnership::Borrowed;
        }
    }
    HandleOwnership::Owned
}

/// Ownership of a sum-match CONTINUATION as a borrowing-op operand — the join over every LEAF body the
/// decision tree can reach (mirrors `cont_child_ids`): a `Guarded` arm joins its body with the
/// fall-through `els`, a `LitTest` joins its `then_`/`els`, a `Switch` joins all its arms'
/// continuations. `Owned` iff every reachable leaf is provably `Owned`, else `Borrowed` (leak-safe).
fn sum_cont_ownership(db: &mut Db, cont: &crate::core::SumCont) -> HandleOwnership {
    match cont {
        crate::core::SumCont::Leaf(body) => {
            if matches!(
                heap_operand_ownership(db, *body),
                Ok(HandleOwnership::Owned)
            ) {
                HandleOwnership::Owned
            } else {
                HandleOwnership::Borrowed
            }
        }
        crate::core::SumCont::Guarded { body, els, .. } => {
            match (
                heap_operand_ownership(db, *body),
                sum_cont_ownership(db, els),
            ) {
                (Ok(HandleOwnership::Owned), HandleOwnership::Owned) => HandleOwnership::Owned,
                _ => HandleOwnership::Borrowed,
            }
        }
        crate::core::SumCont::LitTest { then_, els, .. } => {
            match (sum_cont_ownership(db, then_), sum_cont_ownership(db, els)) {
                (HandleOwnership::Owned, HandleOwnership::Owned) => HandleOwnership::Owned,
                _ => HandleOwnership::Borrowed,
            }
        }
        crate::core::SumCont::Switch { arms, .. } => {
            for a in arms.iter() {
                if sum_cont_ownership(db, &a.cont) == HandleOwnership::Borrowed {
                    return HandleOwnership::Borrowed;
                }
            }
            HandleOwnership::Owned
        }
    }
}
