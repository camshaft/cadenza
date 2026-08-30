//! Dynamic-key map operations
//!
//! Map collection of (key, value) handle pairs, stored verbatim.

use super::*;

// ─── Map: dynamic-key collection of (key, value) handle pairs, stored verbatim ──────────
// Pairs are flattened into `handles` as [k0, v0, k1, v1, …]; pair count = handles.len() / 2. OOB
// pair index into a valid map traps; null is benign.

pub(crate) fn op_map_alloc(len: u32) -> Handle {
    alloc(vec![Handle::NULL; (len as usize) * 2], Vec::new())
}
/// Write the (key, value) pair at `index` and return the map handle (for convenient threading). OOB
/// pair index into a valid map traps; null is a no-op.
pub(crate) fn op_map_set(m: Handle, index: u32, key: Handle, value: Handle) -> Handle {
    if is_immediate(m) {
        return m; // defensive (mirrors the map readers): a map is never an immediate; return the
        // handle unchanged (no-op write), never deref the tagged bits
    }
    match unsafe { m.node_mut() } {
        None => {}
        Some(n) => {
            let base = (index as usize) * 2;
            if base + 1 < n.handles.len() {
                n.handles.set(base, key);
                n.handles.set(base + 1, value);
            } else {
                trap_oob();
            }
        }
    }
    m
}
pub(crate) fn op_map_key(m: Handle, index: u32) -> Handle {
    if is_immediate(m) {
        return Handle::NULL; // defensive: a map is never an immediate; benign default like null-in
    }
    match unsafe { m.node_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get((index as usize) * 2) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
pub(crate) fn op_map_val(m: Handle, index: u32) -> Handle {
    if is_immediate(m) {
        return Handle::NULL; // defensive: a map is never an immediate; benign default like null-in
    }
    match unsafe { m.node_ref() } {
        None => Handle::NULL,
        Some(n) => match n.handles.get((index as usize) * 2 + 1) {
            Some(&h) => h,
            None => trap_oob(),
        },
    }
}
pub(crate) fn op_map_len(m: Handle) -> u32 {
    with_node(m, 0, |n| (n.handles.len() / 2) as u32)
}
