//! `cdz doc-module` — the TYPE-ENRICHED doc-module extraction (cadenza-docs I2, assembly half).
//!
//! Architecture (design §4.2, ruled option C): the doc-item PROJECTION is single-sourced in
//! `cadenza_syntax::doc_item::project` (I1, structural — name/sig/doc/kind/visibility, no types); the
//! RESOLVED types come from `rcdzc`'s sidecar (`Query::ExportedTypes`), which the pure compiler core can
//! produce but cadenza-syntax cannot reach (it can't depend on rcdzc). This bin — the ONE place both
//! crates meet — MERGES them: run the structural projection, ask the sidecar for each export's resolved
//! type, and graft a `(ty <sub-ast>)` into the matching `doc-item`.
//!
//! This module is the pure MERGE half: given the projected doc-module arena + the sidecar's
//! `export-types` blob bytes, it decodes each type and grafts it in. It is deliberately independent of
//! the sidecar QUERY symbol (the caller drives `run_sidecar` and hands the blob here) so it is testable
//! standalone against a synthetic blob.
//!
//! ## The `export-types` wire (locked with v-inference)
//! ```text
//! bytes = u32_le count
//!         then `count` records, each:
//!           u32_le name_len | name (UTF-8) | u32_le ty_len | ty_bytes
//! ```
//! `ty_bytes` is a FULL `cdzast\x00\x01` artifact (`codec::encode` of a standalone arena rooted at the
//! resolved type's `encode_ty_payload`), so `codec::decode(ty_bytes)` yields an arena whose ROOT is the
//! `(ty …)` payload subtree. An export whose type does not resolve is OMITTED from the blob (so a
//! `doc-item` whose name is absent from the map simply gets no `(ty …)` — the graceful-degrade rule).

// The `cdz doc-module` HANDLER that calls these (drive `run_sidecar(Query::ExportedTypes)` → feed the
// blob to `parse_export_types` → `merge_types`) is wired in a follow-up once v-inference's
// `rcdzc::sidecar::Query::ExportedTypes` + `KIND_EXPORT_TYPES` land on trunk (queued as b6cb71019).
// Until then this merge half is dead-but-tested (its `#[cfg(test)]` suite exercises the full
// parse+graft against a synthetic blob); the allow is removed when the handler makes it live.
#![allow(dead_code)]

use cadenza_syntax::ast::{Arenas, Builder, Struct, StructId};
use std::collections::BTreeMap;

/// Parse the sidecar `export-types` blob into a map of export name → the decoded type arena (whose root
/// is the `(ty …)` payload). A malformed/truncated blob yields an empty map rather than an error — a
/// missing type map just means no `doc-item` gets a `(ty …)` (honest degrade, never a crash on a doc
/// build). `None` entries (a type that didn't decode) are skipped.
pub fn parse_export_types(blob: &[u8]) -> BTreeMap<String, Arenas> {
    let mut out = BTreeMap::new();
    let mut r = Reader::new(blob);
    let Some(count) = r.u32_le() else {
        return out;
    };
    for _ in 0..count {
        let Some(name) = r.len_prefixed().and_then(|b| std::str::from_utf8(b).ok()) else {
            break; // truncated — stop, keep what parsed
        };
        let name = name.to_string();
        let Some(ty_bytes) = r.len_prefixed() else {
            break;
        };
        if let Some(arena) = cadenza_syntax::codec::decode(ty_bytes) {
            out.insert(name, arena);
        }
    }
    out
}

/// Merge resolved types into the structural doc-module: for each `doc-item`, if its `(name …)` is in
/// `types`, append a `(ty <sub-ast>)` child (the decoded type arena's root subtree, grafted in) as a
/// sibling to `(sig …)`. An item whose name is absent gets no `(ty …)`. Returns a fresh doc-module
/// arena (the input is not mutated). Non-`doc-item` children (`(module-doc …)`, the head/name) are
/// copied verbatim.
pub fn merge_types(structural: &Arenas, types: &BTreeMap<String, Arenas>) -> Arenas {
    let root = structural.root;
    let Struct::List(children) = structural.get(root) else {
        // Unexpected shape — copy verbatim (never panic).
        let mut b = Builder::new();
        let r = copy_subtree(&mut b, structural, root);
        return b.finish(r);
    };
    let children = children.clone();

    let mut b = Builder::new();
    let mut new_children = Vec::with_capacity(children.len());
    for child in children {
        if structural.as_form(child, "doc-item").is_some() {
            new_children.push(enrich_item(&mut b, structural, child, types));
        } else {
            new_children.push(copy_subtree(&mut b, structural, child));
        }
    }
    let new_root = b.list(new_children);
    b.finish(new_root)
}

/// Rebuild one `doc-item`, copying its children verbatim and appending `(ty <sub-ast>)` when the item's
/// name is in `types`.
fn enrich_item(
    b: &mut Builder,
    structural: &Arenas,
    item: StructId,
    types: &BTreeMap<String, Arenas>,
) -> StructId {
    let Struct::List(kids) = structural.get(item) else {
        return copy_subtree(b, structural, item);
    };
    let kids = kids.clone();
    let mut new_kids: Vec<StructId> = kids
        .iter()
        .map(|&k| copy_subtree(b, structural, k))
        .collect();

    if let Some(name) = item_name(structural, item)
        && let Some(ty_arena) = types.get(&name)
    {
        // Graft the decoded type arena's root subtree in as the `(ty …)` payload.
        let ty_head = b.name("ty");
        let ty_payload = copy_from(b, ty_arena, ty_arena.root);
        let ty_node = b.list(vec![ty_head, ty_payload]);
        new_kids.push(ty_node);
    }

    b.list(new_kids)
}

/// The `(name "…")` string of a `doc-item`.
fn item_name(a: &Arenas, item: StructId) -> Option<String> {
    let Struct::List(kids) = a.get(item) else {
        return None;
    };
    for &c in kids {
        if let Some(args) = a.as_form(c, "name")
            && let Some(&v) = args.first()
        {
            return a.as_str(v).map(str::to_string);
        }
    }
    None
}

/// Copy the subtree rooted at `id` of `src` into builder `b` (same arena family). Iterative post-order.
fn copy_subtree(b: &mut Builder, src: &Arenas, id: StructId) -> StructId {
    copy_from(b, src, id)
}

/// Copy the subtree rooted at `id` of any `src` `Arenas` into `b`, returning the new root id.
/// Iterative post-order so a deep type/item can't overflow the native stack.
fn copy_from(b: &mut Builder, src: &Arenas, id: StructId) -> StructId {
    enum Job {
        Visit(StructId),
        EmitList(usize),
    }
    let mut jobs = vec![Job::Visit(id)];
    let mut results: Vec<StructId> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(sid) => match src.get(sid) {
                Struct::Atom(lid) => {
                    let leaf = src.leaf(*lid).clone();
                    let n = b.atom_leaf(leaf);
                    results.push(n);
                }
                Struct::List(kids) => {
                    jobs.push(Job::EmitList(kids.len()));
                    for &k in kids.iter().rev() {
                        jobs.push(Job::Visit(k));
                    }
                }
            },
            Job::EmitList(n) => {
                let kids = results.split_off(results.len() - n);
                let node = b.list(kids);
                results.push(node);
            }
        }
    }
    results.pop().expect("copy_from leaves a root")
}

/// A little-endian, length-prefixed byte reader for the `export-types` blob. Total: every read returns
/// `None` on insufficient bytes rather than panicking (a truncated blob degrades to a partial map).
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    fn u32_le(&mut self) -> Option<u32> {
        let end = self.pos.checked_add(4)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    /// A `u32_le` length followed by that many bytes.
    fn len_prefixed(&mut self) -> Option<&'a [u8]> {
        let len = self.u32_le()? as usize;
        let end = self.pos.checked_add(len)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_syntax::ast::{Builder, Leaf};
    use cadenza_syntax::codec;

    /// Build a `(ty <payload>)`-less type arena — a standalone `(-> a b)` — and its `\x00\x01` bytes,
    /// as the sidecar would emit for one export's resolved type.
    fn arrow_ty_bytes() -> Vec<u8> {
        let mut b = Builder::new();
        let head = b.name("->");
        let a = b.name("a");
        let bb = b.name("b");
        let root = b.list(vec![head, a, bb]);
        codec::encode(&b.finish(root))
    }

    /// Frame an export-types blob from (name, ty_bytes) records, per the locked wire.
    fn frame(records: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for (name, ty) in records {
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(ty.len() as u32).to_le_bytes());
            out.extend_from_slice(ty);
        }
        out
    }

    /// A minimal structural doc-module `(doc-module "m" (doc-item (name "f") (sig (f x)) (kind def)))`.
    fn structural_one_item() -> Arenas {
        let mut b = Builder::new();
        let dm = b.name("doc-module");
        let mname = b.atom_leaf(Leaf::Str("m".into()));
        // doc-item
        let di = b.name("doc-item");
        let nh = b.name("name");
        let nv = b.atom_leaf(Leaf::Str("f".into()));
        let name_node = b.list(vec![nh, nv]);
        let sh = b.name("sig");
        let sf = b.name("f");
        let sx = b.name("x");
        let sig_sub = b.list(vec![sf, sx]);
        let sig_node = b.list(vec![sh, sig_sub]);
        let kh = b.name("kind");
        let kv = b.name("def");
        let kind_node = b.list(vec![kh, kv]);
        let item = b.list(vec![di, name_node, sig_node, kind_node]);
        let root = b.list(vec![dm, mname, item]);
        b.finish(root)
    }

    #[test]
    fn parse_export_types_reads_the_locked_wire() {
        let blob = frame(&[("f", arrow_ty_bytes()), ("g", arrow_ty_bytes())]);
        let map = parse_export_types(&blob);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("f") && map.contains_key("g"));
        // each decoded arena's root is the (-> a b) subtree
        let f = &map["f"];
        assert_eq!(f.head_name(f.root), Some("->"));
    }

    #[test]
    fn parse_export_types_is_total_on_a_truncated_blob() {
        // A count of 3 but only one full record + a truncated second → keep the one that parsed, no panic.
        let mut blob = frame(&[("f", arrow_ty_bytes())]);
        blob[0..4].copy_from_slice(&3u32.to_le_bytes()); // claim 3, supply 1
        blob.extend_from_slice(&5u32.to_le_bytes()); // a dangling name_len with no bytes
        let map = parse_export_types(&blob);
        assert_eq!(
            map.len(),
            1,
            "keeps the fully-parsed record, drops the truncated"
        );
        assert!(map.contains_key("f"));
    }

    #[test]
    fn merge_grafts_ty_for_a_named_item_and_omits_for_absent() {
        let structural = structural_one_item();
        // types has "f" → the item gets (ty (-> a b)); an absent name gets nothing.
        let mut types = BTreeMap::new();
        types.insert("f".to_string(), codec::decode(&arrow_ty_bytes()).unwrap());
        let merged = merge_types(&structural, &types);

        // Find the single doc-item + assert it now has a (ty …) whose payload is (-> a b).
        let dm = merged
            .as_form(merged.root, "doc-module")
            .expect("doc-module");
        let item = dm
            .iter()
            .copied()
            .find(|&c| merged.as_form(c, "doc-item").is_some())
            .expect("a doc-item");
        let Struct::List(kids) = merged.get(item) else {
            panic!("item is a list")
        };
        let ty_child = kids
            .iter()
            .find(|&&k| merged.as_form(k, "ty").is_some())
            .copied()
            .expect("item has a (ty …) child");
        let ty_payload = merged.as_form(ty_child, "ty").unwrap()[0];
        assert_eq!(
            merged.head_name(ty_payload),
            Some("->"),
            "(ty …) payload is the grafted arrow type"
        );
        // sig is still present (ty is additive, sibling to sig — not a rewrite).
        assert!(
            kids.iter().any(|&k| merged.as_form(k, "sig").is_some()),
            "(sig …) is preserved alongside (ty …)"
        );
    }

    #[test]
    fn merge_omits_ty_when_no_type_resolved() {
        // Empty types map (every export's type declined) → the item keeps sig/name/kind, no (ty …).
        let structural = structural_one_item();
        let merged = merge_types(&structural, &BTreeMap::new());
        let dm = merged.as_form(merged.root, "doc-module").unwrap();
        let item = dm
            .iter()
            .copied()
            .find(|&c| merged.as_form(c, "doc-item").is_some())
            .unwrap();
        let Struct::List(kids) = merged.get(item) else {
            panic!()
        };
        assert!(
            !kids.iter().any(|&k| merged.as_form(k, "ty").is_some()),
            "no (ty …) when the type didn't resolve"
        );
        assert!(kids.iter().any(|&k| merged.as_form(k, "name").is_some()));
    }

    #[test]
    fn merged_doc_module_round_trips_through_the_codec() {
        // The type-enriched doc-module is ordinary cdzast — encodes → \x00\x01 → decodes identically.
        let structural = structural_one_item();
        let mut types = BTreeMap::new();
        types.insert("f".to_string(), codec::decode(&arrow_ty_bytes()).unwrap());
        let merged = merge_types(&structural, &types);
        let bytes = codec::encode(&merged);
        assert_eq!(&bytes[..8], b"cdzast\x00\x01");
        let decoded = codec::decode(&bytes).expect("merged doc-module decodes");
        assert!(merged.structurally_eq(&decoded), "round-trip identical");
    }
}
