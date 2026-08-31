//! Decoder for the runtime rc-TRACE drain — the ATTRIBUTION complement to the `--guarded-all` /
//! live-objects DETECTION (which only tells you a case leaks/traps, not WHICH handle).
//!
//! The `.#rctrace-runtime` variant (features `debug-counters` + `rc-trace-export`, world
//! `runtime-debug`) records one event per `op_alloc`/`op_dup`/`op_drop` when armed via the
//! `rc-trace-enable(true)` export, and `rc-trace-drain() -> list<u8>` returns them as a flat array of
//! 20-byte LITTLE-ENDIAN fixed records (contract settled with v-runtime + v-nix, 2026-08-31):
//!
//! ```text
//!   byte 0     : op        (0=ALLOC, 1=DUP, 2=DROP, 3=MARK_IMMORTAL — a census-exit, not a leak)
//!   byte 1     : tag       (0=Leaf, 1=Sum, 2=Compound — structural, tagless runtime)
//!   byte 2     : freed     (0/1; meaningful on DROP)
//!   byte 3     : _pad
//!   bytes 4..8 : node            u32 LE  (unique per ALLOC — the leak identity)
//!   bytes 8..12: rc_before       u32 LE
//!   bytes 12..16: rc_after       u32 LE
//!   bytes 16..20: cascade_parent u32 LE  (0xFFFF_FFFF = none / root drop)
//! ```
//!
//! A LEAK is a node with an ALLOC but no census-EXIT — neither a DROP reaching `freed = 1` NOR a
//! MARK_IMMORTAL (an immortal build-once static the RC never frees is not a leak) — [`leak_summary`]. This module is
//! the pure decode/summary half of `cdz-run --rc-trace`; the export-call wiring (instantiate the
//! runtime-debug world, `rc-trace-enable` pre-run, `rc-trace-drain`/`rc-trace-truncated` post-run) is
//! added when the `.#rctrace-runtime` variant + the debug-trace WIT land on main.

/// The RC op an event records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RcOp {
    Alloc,
    Dup,
    Drop,
    /// The node left the census AS IMMORTAL (`op_mark_immortal`/`_deep`) — NOT a leak and NOT a freed
    /// drop; a census-EXIT that [`leak_summary`] excludes from the leak set (same as a freed drop).
    MarkImmortal,
}

/// The node's structural tag (the runtime is tagless; this is the shape class the trace records).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTag {
    Leaf,
    Sum,
    Compound,
    /// A tag byte the current contract doesn't define — kept (not rejected) so a runtime that grows the
    /// tag set doesn't make the decoder reject an otherwise-valid trace; rendered as `tag=<n>`.
    Other(u8),
}

/// One decoded 20-byte rc-trace record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RcEvent {
    pub op: RcOp,
    pub tag: NodeTag,
    /// Meaningful only on `Drop`: the drop reached refcount 0 and freed the cell.
    pub freed: bool,
    /// Unique per ALLOC — the leak identity.
    pub node: u32,
    pub rc_before: u32,
    pub rc_after: u32,
    /// `Some(parent)` when this drop was cascaded from a dying parent compound; `None` for a root drop
    /// (the contract's `0xFFFF_FFFF` sentinel).
    pub cascade_parent: Option<u32>,
}

/// The fixed rc-trace record width (bytes).
pub const RECORD_LEN: usize = 20;
const CASCADE_NONE: u32 = 0xFFFF_FFFF;

/// Why a drain buffer failed to decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeErr {
    /// The buffer length is not a whole number of 20-byte records.
    Ragged { len: usize },
    /// An `op` byte outside 0..=2 at record index `i`.
    BadOp { index: usize, op: u8 },
}

impl std::fmt::Display for DecodeErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeErr::Ragged { len } => write!(
                f,
                "rc-trace drain is {len} bytes — not a whole number of {RECORD_LEN}-byte records"
            ),
            DecodeErr::BadOp { index, op } => {
                write!(
                    f,
                    "rc-trace record {index}: unknown op byte {op} (expected 0=ALLOC/1=DUP/2=DROP)"
                )
            }
        }
    }
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Decode a `rc-trace-drain()` buffer into its events. Errors on a ragged length or an unknown op byte;
/// an unknown TAG byte is preserved as [`NodeTag::Other`] (forward-compatible with a grown tag set).
pub fn decode(bytes: &[u8]) -> Result<Vec<RcEvent>, DecodeErr> {
    if !bytes.len().is_multiple_of(RECORD_LEN) {
        return Err(DecodeErr::Ragged { len: bytes.len() });
    }
    let mut out = Vec::with_capacity(bytes.len() / RECORD_LEN);
    for (index, rec) in bytes.chunks_exact(RECORD_LEN).enumerate() {
        let op = match rec[0] {
            0 => RcOp::Alloc,
            1 => RcOp::Dup,
            2 => RcOp::Drop,
            3 => RcOp::MarkImmortal,
            other => return Err(DecodeErr::BadOp { index, op: other }),
        };
        let tag = match rec[1] {
            0 => NodeTag::Leaf,
            1 => NodeTag::Sum,
            2 => NodeTag::Compound,
            other => NodeTag::Other(other),
        };
        let cascade = le_u32(&rec[16..20]);
        out.push(RcEvent {
            op,
            tag,
            freed: rec[2] != 0,
            node: le_u32(&rec[4..8]),
            rc_before: le_u32(&rec[8..12]),
            rc_after: le_u32(&rec[12..16]),
            cascade_parent: (cascade != CASCADE_NONE).then_some(cascade),
        });
    }
    Ok(out)
}

/// The leaking nodes: every node with an ALLOC but no DROP that reached `freed = 1`. Returned sorted +
/// deduped (a node allocs once, but be robust to a malformed trace). This is the attribution payload —
/// each node# here is a handle the emit leaked (a missing/unbalanced drop).
pub fn leak_summary(events: &[RcEvent]) -> Vec<u32> {
    use std::collections::BTreeSet;
    let mut allocated: BTreeSet<u32> = BTreeSet::new();
    // Nodes that EXITED the census legitimately: a freed DROP (reclaimed) OR a MARK_IMMORTAL (left the
    // census as immortal — a build-once static the RC never frees). Neither is a leak.
    let mut exited: BTreeSet<u32> = BTreeSet::new();
    for e in events {
        match e.op {
            RcOp::Alloc => {
                allocated.insert(e.node);
            }
            RcOp::Drop if e.freed => {
                exited.insert(e.node);
            }
            RcOp::MarkImmortal => {
                exited.insert(e.node);
            }
            _ => {}
        }
    }
    allocated.difference(&exited).copied().collect()
}

/// A human-readable trace: one line per event + a trailing LEAK SUMMARY. `truncated` reflects
/// `rc-trace-truncated()` (the run exceeded the 64Ki-event buffer → the trace holds only the first N).
pub fn render(events: &[RcEvent], truncated: bool) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    if truncated {
        s.push_str(
            "!! rc-trace TRUNCATED — the run exceeded the 64Ki-event buffer; the trace below is the \
             FIRST events only, the leak summary is INCOMPLETE.\n",
        );
    }
    for e in events {
        let op = match e.op {
            RcOp::Alloc => "ALLOC",
            RcOp::Dup => "DUP  ",
            RcOp::Drop => "DROP ",
            RcOp::MarkImmortal => "IMMOR",
        };
        let tag = match e.tag {
            NodeTag::Leaf => "Leaf".to_string(),
            NodeTag::Sum => "Sum".to_string(),
            NodeTag::Compound => "Compound".to_string(),
            NodeTag::Other(n) => format!("tag={n}"),
        };
        let _ = write!(
            s,
            "{op} node#{} {tag} rc {}->{}",
            e.node, e.rc_before, e.rc_after
        );
        if e.op == RcOp::Drop && e.freed {
            s.push_str(" [freed]");
        }
        if let Some(p) = e.cascade_parent {
            let _ = write!(s, " [cascade<-node#{p}]");
        }
        s.push('\n');
    }
    let leaks = leak_summary(events);
    if leaks.is_empty() {
        s.push_str("LEAK SUMMARY: none — every ALLOC reached a freed DROP.\n");
    } else {
        let _ = write!(
            s,
            "LEAK SUMMARY: {} leaked node(s) (ALLOC, no freed DROP): ",
            leaks.len()
        );
        s.push_str(
            &leaks
                .iter()
                .map(|n| format!("node#{n}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one 20-byte LE record.
    fn rec(
        op: u8,
        tag: u8,
        freed: u8,
        node: u32,
        before: u32,
        after: u32,
        parent: u32,
    ) -> [u8; 20] {
        let mut r = [0u8; 20];
        r[0] = op;
        r[1] = tag;
        r[2] = freed;
        r[4..8].copy_from_slice(&node.to_le_bytes());
        r[8..12].copy_from_slice(&before.to_le_bytes());
        r[12..16].copy_from_slice(&after.to_le_bytes());
        r[16..20].copy_from_slice(&parent.to_le_bytes());
        r
    }

    #[test]
    fn decode_one_alloc_record_reads_every_field_le() {
        let bytes = rec(0, 2, 0, 0x0A0B_0C0D, 0, 1, CASCADE_NONE);
        let evs = decode(&bytes).unwrap();
        assert_eq!(evs.len(), 1);
        let e = evs[0];
        assert_eq!(e.op, RcOp::Alloc);
        assert_eq!(e.tag, NodeTag::Compound);
        assert!(!e.freed);
        assert_eq!(e.node, 0x0A0B_0C0D);
        assert_eq!(e.rc_before, 0);
        assert_eq!(e.rc_after, 1);
        assert_eq!(e.cascade_parent, None); // 0xFFFFFFFF sentinel → None
    }

    #[test]
    fn decode_drop_carries_freed_and_cascade_parent() {
        let bytes = rec(2, 0, 1, 7, 1, 0, 42);
        let e = decode(&bytes).unwrap()[0];
        assert_eq!(e.op, RcOp::Drop);
        assert!(e.freed);
        assert_eq!(e.cascade_parent, Some(42));
    }

    #[test]
    fn unknown_tag_is_preserved_not_rejected() {
        let e = decode(&rec(1, 9, 0, 1, 1, 2, CASCADE_NONE)).unwrap()[0];
        assert_eq!(e.tag, NodeTag::Other(9));
        assert_eq!(e.op, RcOp::Dup);
    }

    #[test]
    fn ragged_and_bad_op_error() {
        assert_eq!(decode(&[0u8; 19]), Err(DecodeErr::Ragged { len: 19 }));
        assert_eq!(
            decode(&rec(5, 0, 0, 1, 0, 1, CASCADE_NONE)),
            Err(DecodeErr::BadOp { index: 0, op: 5 })
        );
    }

    #[test]
    fn leak_summary_flags_alloc_without_freed_drop() {
        // node#1: alloc → dropped-freed = clean. node#2: alloc, only a non-freeing drop (rc 2->1) = LEAK.
        let mut buf = Vec::new();
        buf.extend_from_slice(&rec(0, 0, 0, 1, 0, 1, CASCADE_NONE)); // alloc 1
        buf.extend_from_slice(&rec(0, 0, 0, 2, 0, 1, CASCADE_NONE)); // alloc 2
        buf.extend_from_slice(&rec(2, 0, 1, 1, 1, 0, CASCADE_NONE)); // drop 1 freed
        buf.extend_from_slice(&rec(2, 0, 0, 2, 2, 1, CASCADE_NONE)); // drop 2 NOT freed (still shared)
        let evs = decode(&buf).unwrap();
        assert_eq!(leak_summary(&evs), vec![2]);
        assert!(render(&evs, false).contains("node#2"));
        assert!(render(&evs, false).contains("1 leaked node"));
    }

    #[test]
    fn mark_immortal_is_a_census_exit_not_a_leak() {
        // node#1: alloc → MARK_IMMORTAL (op 3, rc→IMMORTAL) = census-exit, NOT a leak (dqe17 fix).
        // node#2: alloc, never exited = a real leak.
        let mut buf = Vec::new();
        buf.extend_from_slice(&rec(0, 1, 0, 1, 0, 1, CASCADE_NONE)); // alloc 1 (Sum)
        buf.extend_from_slice(&rec(3, 1, 0, 1, 1, u32::MAX, CASCADE_NONE)); // mark-immortal 1
        buf.extend_from_slice(&rec(0, 0, 0, 2, 0, 1, CASCADE_NONE)); // alloc 2, never exits
        let evs = decode(&buf).unwrap();
        assert_eq!(evs[1].op, RcOp::MarkImmortal); // op 3 decodes (no BadOp)
        assert_eq!(leak_summary(&evs), vec![2]); // 1 excluded (immortal), 2 flagged
        assert!(render(&evs, false).contains("IMMOR node#1"));
    }

    #[test]
    fn no_leak_when_all_freed() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&rec(0, 0, 0, 1, 0, 1, CASCADE_NONE));
        buf.extend_from_slice(&rec(2, 0, 1, 1, 1, 0, CASCADE_NONE));
        assert!(leak_summary(&decode(&buf).unwrap()).is_empty());
        assert!(render(&decode(&buf).unwrap(), false).contains("none"));
    }

    #[test]
    fn truncated_banner_and_empty() {
        assert!(render(&[], true).contains("TRUNCATED"));
        assert_eq!(decode(&[]).unwrap().len(), 0);
    }
}
