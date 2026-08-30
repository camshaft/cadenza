//! The `KIND_FUNC_LAYOUT` wire — the emitted-function layout a `FuncLayout` query reports (each reachable
//! def's absolute wasm func-index + a content-hash of its AST subtree, in func-index order, preceded by a
//! `defs-begin` marker carrying the def-region base). Carried as canonical BINARY AST (`cadenza_ast::codec`),
//! the SAME wire every compile-boundary artifact speaks (operator P0, seq-284: "Binary AST everywhere" — no
//! bespoke TAB text). The producer (`rcdzc`'s `Query::FuncLayout`) calls [`encode`]; the consumer (`cdz
//! func-layout`) calls [`decode`] then [`render_text`] to print the historical TAB rows verbatim (the
//! compile-reuse witness diffs that stdout, so the text stays byte-stable) — doing ZERO string parsing.
//!
//! Shape (see the `Query::FuncLayout` docs for the field semantics):
//! - A DECLINE (no export AND no `@test` to anchor emit) is an EMPTY root list `()` — [`render_text`] emits
//!   the empty string, exactly as the old text wire did.
//! - A LAID-OUT program is `(list [Int import_base] [ (list of row-forms) ])`, one row-form per def:
//!   `(list [Str name] [Int content_hash] <[Int func_index] present ONLY when the def has an assigned slot>)`.
//!   The func-index is OMITTED (a 2-child row) rather than a `-1` sentinel when the def has no slot (the
//!   no-sentinel-ints directive) — [`render_text`] renders the absent index as the historical `-`.
//!
//! TOTAL on decode: a non-AST payload, a wrong-shape tree, or an out-of-range operand yields `None`.

use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix, Struct, StructId};

/// One reachable definition's layout row: its source name, the stable content-hash of its `(def …)` AST
/// subtree, and its absolute wasm func-index (`None` for an emitted def with no assigned slot — rendered
/// `-`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FuncLayoutRow {
    pub name: String,
    pub hash: u64,
    pub idx: Option<u32>,
}

/// The emitted-function layout: the def-region base plus the ordered rows. `laid_out` is `false` for a
/// DECLINE (neither an export nor a `@test` anchored a layout) — [`render_text`] then emits the empty
/// string, the total-query "no layout" result the old text wire signalled with empty bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FuncLayout {
    pub import_base: u32,
    pub laid_out: bool,
    pub rows: Vec<FuncLayoutRow>,
}

/// Encode the layout as the `KIND_FUNC_LAYOUT` artifact bytes — canonical binary AST (see module docs).
/// Round-trips with [`decode`].
pub fn encode(fl: &FuncLayout) -> Vec<u8> {
    let mut b = Builder::new();
    let root = if !fl.laid_out {
        b.list(Vec::new()) // decline → empty root list
    } else {
        let base = int_leaf(&mut b, fl.import_base);
        let row_forms: Vec<StructId> = fl
            .rows
            .iter()
            .map(|r| {
                let name = b.atom_leaf(Leaf::Str(r.name.as_str().into()));
                let hash = b.atom_leaf(Leaf::Int {
                    value: IntValue::from_u128(u128::from(r.hash)),
                    radix: Radix::Dec,
                });
                let mut kids = vec![name, hash];
                if let Some(i) = r.idx {
                    kids.push(int_leaf(&mut b, i)); // present → append; absent → omit (no -1 sentinel)
                }
                b.list(kids)
            })
            .collect();
        let rows_list = b.list(row_forms);
        b.list(vec![base, rows_list])
    };
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_FUNC_LAYOUT` bytes back into a [`FuncLayout`] — the inverse of [`encode`], read via the
/// shared `cadenza_ast::codec`. TOTAL: a non-AST / wrong-shape / out-of-range payload yields `None`.
pub fn decode(bytes: &[u8]) -> Option<FuncLayout> {
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::List(cols) = a.get(a.root).clone() else {
        return None;
    };
    // Empty root list = a decline (no layout) — a defined, total result, not a malformed tree.
    if cols.is_empty() {
        return Some(FuncLayout {
            import_base: 0,
            laid_out: false,
            rows: Vec::new(),
        });
    }
    if cols.len() != 2 {
        return None;
    }
    let import_base = u32::try_from(a.as_int(cols[0])?.to_i64()?).ok()?;
    let Struct::List(row_forms) = a.get(cols[1]).clone() else {
        return None;
    };
    let mut rows = Vec::with_capacity(row_forms.len());
    for f in row_forms {
        let Struct::List(kids) = a.get(f) else {
            return None;
        };
        if kids.len() < 2 || kids.len() > 3 {
            return None;
        }
        let name = a.as_str(kids[0])?.to_string();
        let hash = u64::try_from(a.as_int(kids[1])?.to_u128()?).ok()?;
        let idx = match kids.get(2) {
            Some(&k) => Some(u32::try_from(a.as_int(k)?.to_i64()?).ok()?),
            None => None,
        };
        rows.push(FuncLayoutRow { name, hash, idx });
    }
    Some(FuncLayout {
        import_base,
        laid_out: true,
        rows,
    })
}

/// Render a [`FuncLayout`] to the historical TAB text a `cdz func-layout` prints — the SINGLE source of
/// truth for the human/witness output, so the CLI stays byte-stable across the binary-AST conversion. A
/// DECLINE renders the empty string; a laid-out program renders the `defs-begin\t<import_base>\t-` marker
/// then one `<idx-or-"-">\t<hash:016x>\t<name>` row per def.
pub fn render_text(fl: &FuncLayout) -> String {
    if !fl.laid_out {
        return String::new();
    }
    let mut text = format!("defs-begin\t{}\t-\n", fl.import_base);
    for r in &fl.rows {
        let idx = r.idx.map_or_else(|| "-".to_string(), |i| i.to_string());
        text.push_str(&format!("{idx}\t{:016x}\t{}\n", r.hash, r.name));
    }
    text
}

/// An `Ast.Int` (decimal) leaf for an `import_base` / `func_index` — the same integer-atom encoding the
/// sibling compile-boundary wires (`link_map`, `sidecar`, `spans`) use.
fn int_leaf(b: &mut Builder, n: u32) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(i64::from(n)),
        radix: Radix::Dec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST func-layout round-trips exactly (operator P0 seq-284: no bespoke TAB): a laid-out
    // program with a slotted row AND a slot-less (idx=None) row, and a DECLINE, all survive encode→decode,
    // and render_text reproduces the exact historical TAB text on both sides.
    #[test]
    fn func_layout_binary_ast_round_trips() {
        let fl = FuncLayout {
            import_base: 3,
            laid_out: true,
            rows: vec![
                FuncLayoutRow {
                    name: "main".into(),
                    hash: 0x0123_4567_89ab_cdef,
                    idx: Some(3),
                },
                FuncLayoutRow {
                    name: "helper".into(),
                    hash: u64::MAX,
                    idx: None, // an emitted def with no slot → rendered `-`, no sentinel on the wire
                },
            ],
        };
        assert_eq!(decode(&encode(&fl)), Some(fl.clone()));
        assert_eq!(
            render_text(&fl),
            "defs-begin\t3\t-\n3\t0123456789abcdef\tmain\n-\tffffffffffffffff\thelper\n"
        );

        // A decline round-trips to a laid_out=false layout and renders the empty string.
        let decline = FuncLayout {
            import_base: 0,
            laid_out: false,
            rows: Vec::new(),
        };
        assert_eq!(decode(&encode(&decline)), Some(decline.clone()));
        assert_eq!(render_text(&decline), "");

        // Garbage / wrong-shape payloads decode to None (total, graceful-degrade — never panics).
        assert_eq!(decode(b"not a binary-ast tree"), None);
        let mut b = Builder::new();
        let bad = b.atom_leaf(Leaf::Str("nope".into())); // a bare Str root (not a list)
        assert_eq!(decode(&cadenza_ast::codec::encode(&b.finish(bad))), None);
    }
}
