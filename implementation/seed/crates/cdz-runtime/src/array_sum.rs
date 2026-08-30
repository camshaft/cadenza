//! Positional array and sum operations
//!
//! The ONE runtime shape for TUPLE, RECORD, and LIST + Sum discriminant/payload.

use super::*;

// ─── Positional array — the ONE runtime shape for TUPLE, RECORD, and LIST ───────────────
// Elements live in `handles`. Access by an out-of-bounds index into a valid array TRAPS; a null
// handle is benign (returns NULL / no-op).

pub(crate) fn op_arr_alloc(len: u32) -> Handle {
    // Normalize-on-construct (P1b): the empty array (arity 0) IS the unit value, and unit ALWAYS
    // inlines — no heap Node. A non-empty array still allocates its slots.
    if len == 0 {
        return imm_unit();
    }
    // A small (≤cap) tuple/record builds its slots INLINE (no transient heap Vec that would be copied
    // into the inline arm and freed) — the node is then just its Box (the inline-handles WIN for the
    // dominant ≤2-arity products). Wider arrays keep the heap Vec.
    if (len as usize) <= INLINE_HANDLES_CAP {
        return alloc_raw(Handles::inline_nulls(len as usize), Raw::from(Vec::new()));
    }
    alloc(vec![Handle::NULL; len as usize], Vec::new())
}
/// Write an element handle and return the array handle (for convenient threading). OOB into a valid
/// array traps; null is a no-op.
pub(crate) fn op_arr_set(arr: Handle, index: u32, elem: Handle) -> Handle {
    if is_immediate(arr) {
        return arr; // an immediate array (inline unit) has no slots; elem is stored, not deref'd
    }
    match unsafe { arr.node_mut() } {
        None => {}
        Some(n) => match n.handles.get_mut(index as usize) {
            Some(slot) => *slot = elem,
            None => trap_oob(),
        },
    }
    arr
}
pub(crate) fn op_arr_get(arr: Handle, index: u32) -> Handle {
    if is_immediate(arr) {
        trap_oob(); // an immediate array (inline unit) has 0 slots — any index is OOB
    }
    match unsafe { arr.node_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get(index as usize) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
pub(crate) fn op_arr_len(arr: Handle) -> u32 {
    if is_immediate(arr) {
        return 0; // inline unit has 0 elements
    }
    with_node(arr, 0, |n| n.handles.len() as u32)
}

// ─── Sum: a discriminant (in `raw`) plus a payload handle (in `handles`) ────────────────
// `sum-payload` is TOTAL (no runtime index): a mismatched node with no handle yields NULL.

pub(crate) fn op_sum_new(disc: u32, payload: Handle) -> Handle {
    // Build BOTH the 4-byte disc raw AND the 1-element handles INLINE (no transient heap Vec for
    // either) — a sum node is then just the node Box, 1 alloc instead of 2 (was 3 before inline-raw).
    alloc_raw(
        Handles::inline_from(&[payload]),
        Raw::inline(&disc.to_le_bytes()),
    )
}
pub(crate) fn op_sum_disc(h: Handle) -> u32 {
    if is_immediate(h) {
        return 0; // cross-kind totality: a sum is never itself an immediate
    }
    with_node(h, 0, |n| read_disc(&n.raw))
}
/// The discriminant of a sum value FOR A DESCRIPTOR-GUIDED WALK (compare + render) — decodes an ALL-NULLARY
/// sum that was boxed as an Int IMMEDIATE (SOUNDNESS #43). A nullary variant boxes via `box-int` (enum-disc
/// → OP_BOX_INT); a small disc (0/1/2…) fixnum_fits, so `op_box_int` returns an immediate carrying the disc
/// as its int value, NOT a heap sum node. `op_sum_disc` returns 0 for ANY immediate (its documented cross-
/// kind-totality contract, relied on by the render/decode/WIT callers + pinned tests), so the shape-guided
/// Sum arms (value_cmp_shaped + value-encode) MUST decode the disc from the immediate's value here instead —
/// else every nullary key/element reads disc 0 (wrong sort order in to-list; wrong variant in render). A
/// payload-carrying variant is a real heap node → `op_sum_disc` reads its stored disc. Kept SEPARATE from
/// `op_sum_disc` on purpose: only the descriptor-walk callers know the operand is a sum (so an immediate is
/// an enum-disc, not a cross-kind int); `op_sum_disc`'s blanket-0 stays correct for its other callers.
pub(crate) fn sum_disc_shaped(h: Handle) -> u32 {
    if is_immediate(h) {
        // A nullary-sum enum-disc is boxed via `box-int`, so the immediate is INT-tagged; `imm_as_int` is
        // only valid for an int-tagged immediate (a unit/bool immediate would arithmetic-shift to a garbage
        // disc). GUARD on the int tag (PR#889 Copilot, defensive): a non-int immediate under a Sum shape is a
        // MALFORMED descriptor/value pairing — return `u32::MAX` (out of any `variants` range) so the caller's
        // `variants.get(disc)?` DECLINES cleanly (the descriptor-walk contract) rather than garbage-decoding.
        match imm_kind(h) {
            ImmKind::Int => imm_as_int(h) as u32,
            _ => u32::MAX,
        }
    } else {
        op_sum_disc(h)
    }
}
pub(crate) fn op_sum_payload(h: Handle) -> Handle {
    if is_immediate(h) {
        return Handle::NULL; // cross-kind totality: a sum is never itself an immediate
    }
    with_node(h, Handle::NULL, |n| {
        n.handles.first().copied().unwrap_or(Handle::NULL)
    })
}
