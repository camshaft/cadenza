//! The `KIND_PARAM_MANIFEST` wire — the `@param` WIDGET MANIFEST a `ParamManifest` query answers (one record
//! per `@param` site, the data a host renders controls from). Carried as ONE canonical BINARY AST value
//! (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator P0, seq-284/254:
//! "Binary AST everywhere. No exceptions." + "I want the full type ast!"). The producer (`rcdzc`'s
//! `Query::ParamManifest`) builds each site's declared type as a FULL structured `Ty` sub-AST via
//! `rcdzc::eval::encode_ty_payload` (NOT a `Ty::render_name` string) and calls [`encode`]; the consumer
//! (`cdz param-manifest`) calls [`decode`] and renders each type back to its display string via
//! `cadenza_syntax::render_ty::render_ty` (byte-identical to `Ty::render_name`), doing zero string
//! parsing.
//!
//! Shape: a root `(param-manifest <site>…)` list, one site form per `@param`, in scan order:
//! `(param <Str name> <widget> <ty-payload> <range> <options> <default> <Int name-node>)` where
//! - `widget` is a `Str` (the widget atom) or an empty list `()` when absent;
//! - `ty-payload` is the resolved type sub-AST grafted verbatim (the `encode_ty_payload` shape — the same
//!   `(-> …)`/`(Sum …)`/`(Record …)`/scalar payload `result_types_wire`/`export_types_wire` carry);
//! - `range` is `(list [Int lo, Int hi])` (both arena node ids) or `()` when absent;
//! - `options` / `default` are an `Int` arena node id or `()` when absent (NO `-1` sentinel — the old `-`);
//! - `name-node` is the param NAME occurrence node id (always present).
//!
//! TOTAL on decode: a malformed tree / wrong-shape site form is skipped, never a crash.

use crate::graft::copy_from;
use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};

/// One `@param` site the manifest describes. `ty` is a STANDALONE arena rooted at the site's resolved-type
/// payload sub-AST (as the producer extracts it via `encode_ty_payload`); a consumer renders it with
/// `cadenza_syntax::render_ty::render_ty(&site.ty, site.ty.root)`. `widget`/`range`/`options`/`default`
/// are `None` when the `@param` kv is absent; `range`/`options`/`default`/`name_node` are arena node ids the
/// consumer maps to source via the shared `StructId` space + span table.
#[derive(Clone, PartialEq, Debug)]
pub struct ParamSite {
    pub name: String,
    pub widget: Option<String>,
    pub ty: Arenas,
    pub range: Option<(u32, u32)>,
    pub options: Option<u32>,
    pub default: Option<u32>,
    pub name_node: u32,
}

/// Encode the `@param` sites as the `KIND_PARAM_MANIFEST` artifact bytes — ONE canonical binary AST value
/// (see module docs). Each site's `ty` arena is grafted verbatim. Round-trips with [`decode`].
pub fn encode(sites: &[ParamSite]) -> Vec<u8> {
    let mut b = Builder::new();
    let mut forms = Vec::with_capacity(sites.len() + 1);
    forms.push(b.name("param-manifest"));
    for s in sites {
        let head = b.name("param");
        let name = b.atom_leaf(Leaf::Str(s.name.as_str().into()));
        let widget = match &s.widget {
            Some(w) => b.atom_leaf(Leaf::Str(w.as_str().into())),
            None => b.list(Vec::new()),
        };
        let ty = copy_from(&mut b, &s.ty, s.ty.root);
        let range = match s.range {
            Some((lo, hi)) => {
                let l = int_leaf(&mut b, lo);
                let h = int_leaf(&mut b, hi);
                b.list(vec![l, h])
            }
            None => b.list(Vec::new()),
        };
        let options = opt_int(&mut b, s.options);
        let default = opt_int(&mut b, s.default);
        let name_node = int_leaf(&mut b, s.name_node);
        forms.push(b.list(vec![
            head, name, widget, ty, range, options, default, name_node,
        ]));
    }
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_PARAM_MANIFEST` bytes back into the `@param` sites — the inverse of [`encode`], read via
/// the shared `cadenza_ast::codec`. Each site's type sub-AST is extracted into its own standalone [`Arenas`]
/// (so a consumer renders it independently). TOTAL: a malformed tree / wrong-shape site is skipped.
pub fn decode(bytes: &[u8]) -> Vec<ParamSite> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Some(forms) = a.as_form(a.root, "param-manifest") else {
        return Vec::new();
    };
    forms
        .to_vec()
        .iter()
        .filter_map(|&f| decode_one(&a, f))
        .collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<ParamSite> {
    let t = a.as_form(form, "param")?;
    if t.len() != 7 {
        return None;
    }
    let name = a.as_str(t[0])?.to_string();
    // An empty list `()` (widget absent) yields `None` from `as_str`; a `Str` yields the widget atom.
    let widget = a.as_str(t[1]).map(str::to_string);
    // Extract the type payload subtree (child 2) into its own standalone arena for independent rendering.
    let ty = {
        let mut b = Builder::new();
        let root = copy_from(&mut b, a, t[2]);
        b.finish(root)
    };
    let range = decode_range(a, t[3]);
    let options = decode_opt_int(a, t[4]);
    let default = decode_opt_int(a, t[5]);
    let name_node = u32::try_from(a.as_int(t[6])?.to_i64()?).ok()?;
    Some(ParamSite {
        name,
        widget,
        ty,
        range,
        options,
        default,
        name_node,
    })
}

/// A `(list [Int lo, Int hi])` decodes to `Some((lo, hi))`; anything else (an empty list `()`) to `None`.
fn decode_range(a: &Arenas, id: StructId) -> Option<(u32, u32)> {
    let Struct::List(kids) = a.get(id) else {
        return None;
    };
    if kids.len() != 2 {
        return None;
    }
    let lo = u32::try_from(a.as_int(kids[0])?.to_i64()?).ok()?;
    let hi = u32::try_from(a.as_int(kids[1])?.to_i64()?).ok()?;
    Some((lo, hi))
}

/// An `Int` node decodes to `Some(u32)`; an empty list `()` (absent) to `None`.
fn decode_opt_int(a: &Arenas, id: StructId) -> Option<u32> {
    let v = a.as_int(id)?.to_i64()?;
    u32::try_from(v).ok()
}

/// An `Int` node id leaf, or an empty list `()` when the value is absent (no `-1` sentinel).
fn opt_int(b: &mut Builder, v: Option<u32>) -> StructId {
    match v {
        Some(n) => int_leaf(b, n),
        None => b.list(Vec::new()),
    }
}

/// An `Ast.Int` (decimal) leaf for an arena node id — the same integer-atom encoding the sibling wires use.
fn int_leaf(b: &mut Builder, n: u32) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(i64::from(n)),
        radix: Radix::Dec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standalone type-payload arena rooted at `root_build(&mut b)`'s node — as the producer extracts a
    /// site's `encode_ty_payload` subtree.
    fn ty(root_build: impl FnOnce(&mut Builder) -> StructId) -> Arenas {
        let mut b = Builder::new();
        let root = root_build(&mut b);
        b.finish(root)
    }

    // The binary-AST param manifest round-trips exactly (operator P0 seq-284: full Ty AST, no render_name on
    // the wire): a site with a widget + range + options + default + a scalar type, and a site with everything
    // absent + a compound `(-> …)` type, both survive encode→decode with the type sub-AST grafted verbatim.
    #[test]
    fn param_manifest_binary_ast_round_trips() {
        let sites = vec![
            ParamSite {
                name: "size".to_string(),
                widget: Some("slider".to_string()),
                ty: ty(|b| b.name("Int64")),
                range: Some((10, 11)),
                options: Some(12),
                default: Some(13),
                name_node: 5,
            },
            ParamSite {
                name: "fn".to_string(),
                widget: None,
                ty: ty(|b| {
                    let h = b.name("->");
                    let p = b.name("Int64");
                    let r = b.name("Int64");
                    b.list(vec![h, p, r])
                }),
                range: None,
                options: None,
                default: None,
                name_node: 9,
            },
        ];
        assert_eq!(decode(&encode(&sites)), sites);
        // Empty manifest + garbage both total.
        assert!(decode(&encode(&[])).is_empty());
        assert!(decode(b"not a binary-ast tree").is_empty());
    }
}
