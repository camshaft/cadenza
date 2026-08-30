//! The `KIND_INSTANTIATIONS` wire — the disposition + specialization report an `Instantiations` query
//! answers for one definition (what the compiler DID with it, plus every concrete monomorphization).
//! Carried as canonical BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact
//! speaks (operator P0, seq-284: "Binary AST everywhere" — no bespoke TAB text). The producer (`rcdzc`'s
//! `Query::Instantiations`) calls [`encode`]; the consumers (`cdz instantiations` CLI + the LSP
//! instantiations code-lens) call [`decode`] and build their own rendering from the STRUCTURED fields —
//! neither hand-rolls a `\t`/`+`/`;` parse.
//!
//! Shape (see the `Query::Instantiations` docs for the field semantics):
//! - An UNKNOWN name is an EMPTY root list `()` — decodes to `known == false` (the CLI reports "no such
//!   definition"), distinct from a known def with no instances (which always has a disposition).
//! - A KNOWN def is `(list [ (list [Str disposition…]) , (list [ instance-form… ]) , <Int name_node>? ])`:
//!   the dispositions (the `+`-joined set), the instances, and the def's NAME-occurrence node id as an
//!   OPTIONAL LAST `Int` (OMITTED — rather than a `-1` sentinel — when the signature is malformed, the old
//!   text wire's `-`). Each instance-form is `(list [Str spec_name] [ (list [Str arg…]) ])` — the
//!   specialization's mint name + its per-argument descriptors (the old `;`-joined list).
//!
//! TOTAL on decode: a non-AST payload, a wrong-shape tree, or an out-of-range operand yields `None`.

use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};

/// One concrete monomorphization of the queried definition: the specialization's mint name and its
/// per-argument descriptors (`n: Int64`, `const name = VALUE`, …) — the fields the old `inst` row carried
/// as a `;`-joined list, now a proper list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Instance {
    pub spec_name: String,
    pub args: Vec<String>,
}

/// The disposition report for one definition. `known` is `false` for a name that resolves to no
/// definition (the empty-root-list answer — the CLI's "no such definition"); a known def always carries
/// at least one disposition. `name_node` is the def's NAME-occurrence node id a consumer maps to a source
/// location (`None` when the signature is malformed — the old `-`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Instantiations {
    pub known: bool,
    pub name_node: Option<u32>,
    pub dispositions: Vec<String>,
    pub instances: Vec<Instance>,
}

/// Encode the report as the `KIND_INSTANTIATIONS` artifact bytes — canonical binary AST (see module docs).
/// Round-trips with [`decode`].
pub fn encode(i: &Instantiations) -> Vec<u8> {
    let mut b = Builder::new();
    let root = if !i.known {
        b.list(Vec::new()) // unknown name → empty root list
    } else {
        let disp = i
            .dispositions
            .iter()
            .map(|d| b.atom_leaf(Leaf::Str(d.as_str().into())))
            .collect();
        let disp_list = b.list(disp);
        let inst_forms: Vec<StructId> = i
            .instances
            .iter()
            .map(|inst| {
                let name = b.atom_leaf(Leaf::Str(inst.spec_name.as_str().into()));
                let args = inst
                    .args
                    .iter()
                    .map(|a| b.atom_leaf(Leaf::Str(a.as_str().into())))
                    .collect();
                let args_list = b.list(args);
                b.list(vec![name, args_list])
            })
            .collect();
        let inst_list = b.list(inst_forms);
        let mut kids = vec![disp_list, inst_list];
        if let Some(n) = i.name_node {
            kids.push(b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(i64::from(n)),
                radix: Radix::Dec,
            }));
        }
        b.list(kids)
    };
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_INSTANTIATIONS` bytes back into an [`Instantiations`] — the inverse of [`encode`], read
/// via the shared `cadenza_ast::codec`. TOTAL: a non-AST / wrong-shape / out-of-range payload yields `None`.
pub fn decode(bytes: &[u8]) -> Option<Instantiations> {
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::List(cols) = a.get(a.root).clone() else {
        return None;
    };
    // Empty root list = an unknown name (a defined "no such definition" answer, not a malformed tree).
    if cols.is_empty() {
        return Some(Instantiations {
            known: false,
            name_node: None,
            dispositions: Vec::new(),
            instances: Vec::new(),
        });
    }
    if cols.len() < 2 || cols.len() > 3 {
        return None;
    }
    let Struct::List(disp_forms) = a.get(cols[0]).clone() else {
        return None;
    };
    let mut dispositions = Vec::with_capacity(disp_forms.len());
    for d in disp_forms {
        dispositions.push(a.as_str(d)?.to_string());
    }
    let Struct::List(inst_forms) = a.get(cols[1]).clone() else {
        return None;
    };
    let mut instances = Vec::with_capacity(inst_forms.len());
    for f in inst_forms {
        instances.push(decode_instance(&a, f)?);
    }
    let name_node = match cols.get(2) {
        Some(&n) => Some(u32::try_from(a.as_int(n)?.to_i64()?).ok()?),
        None => None,
    };
    Some(Instantiations {
        known: true,
        name_node,
        dispositions,
        instances,
    })
}

/// Decode one instance form `(list [Str spec_name] [ (list [Str arg…]) ])`.
fn decode_instance(a: &Arenas, form: StructId) -> Option<Instance> {
    let Struct::List(kids) = a.get(form) else {
        return None;
    };
    if kids.len() != 2 {
        return None;
    }
    let spec_name = a.as_str(kids[0])?.to_string();
    let Struct::List(arg_forms) = a.get(kids[1]).clone() else {
        return None;
    };
    let mut args = Vec::with_capacity(arg_forms.len());
    for arg in arg_forms {
        args.push(a.as_str(arg)?.to_string());
    }
    Some(Instance { spec_name, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST instantiations report round-trips exactly (operator P0 seq-284: no bespoke TAB): a
    // specialized def with a `+`-joined disposition set, two instances (one with a multi-arg list, one with
    // an empty arg list), and a present name_node; a known-but-unspecialized def with no instances and an
    // absent name_node; and an unknown name (known=false). Plus garbage → None.
    #[test]
    fn instantiations_binary_ast_round_trips() {
        let specialized = Instantiations {
            known: true,
            name_node: Some(7),
            dispositions: vec!["specialized".into(), "inlined".into()],
            instances: vec![
                Instance {
                    spec_name: "loopn#mono5".into(),
                    args: vec!["n: Int64".into(), "x: String".into()],
                },
                Instance {
                    spec_name: "loopn#mono6".into(),
                    args: Vec::new(),
                },
            ],
        };
        assert_eq!(decode(&encode(&specialized)), Some(specialized.clone()));

        let plain = Instantiations {
            known: true,
            name_node: None, // a malformed signature → the old `-`
            dispositions: vec!["emitted".into()],
            instances: Vec::new(),
        };
        assert_eq!(decode(&encode(&plain)), Some(plain.clone()));

        let unknown = Instantiations {
            known: false,
            name_node: None,
            dispositions: Vec::new(),
            instances: Vec::new(),
        };
        assert_eq!(decode(&encode(&unknown)), Some(unknown.clone()));

        // Garbage / wrong-shape payloads decode to None (total, graceful-degrade — never panics).
        assert_eq!(decode(b"not a binary-ast tree"), None);
        let mut b = Builder::new();
        let bad = b.atom_leaf(Leaf::Str("nope".into())); // a bare Str root (not a list)
        assert_eq!(decode(&cadenza_ast::codec::encode(&b.finish(bad))), None);
    }
}
