//! The `KIND_RESULT_TYPES` map wire — each boundary export's COMPILED result type as a FULL structured
//! `Ty` sub-AST, carried in ONE canonical BINARY AST value (`cadenza_ast::codec`), the SAME wire every
//! compile-boundary artifact speaks (operator seq-254/seq-284: "Binary AST everywhere. No exceptions." +
//! "I don't want the type names being rendered by the compiler — I want the full type ast!"). The
//! producer (`rcdzc`'s `compile`, emitting both the standalone artifact + the `cdz-result-type` component
//! custom section) builds each export's type via `rcdzc::eval::encode_ty_payload` and calls
//! [`encode_result_types`]; the consumer (`cdz-run`, which reads the section to disambiguate a WIT-erased
//! leaf at render — a `list<u8>` as `Bytes` vs `List UInt8`, a `string` as a `Symbol`) calls
//! [`decode_result_types`], which returns a flat [`DecodedTy`] so the render path needs NO `cadenza-ast`
//! arena-walk (it operates on a plain enum). ONE shared codec, so neither side hand-rolls a parser and NO
//! render-name string ever rides the wire — structured-is-truth.
//!
//! Shape: a root `(result-types <result-type>…)` list, one `(result-type <Str name> <ty-payload>)` form
//! per boundary export, in export order. `<ty-payload>` is the resolved type sub-AST grafted verbatim —
//! the same `(-> …)` / `(Sum …)` / `(Record …)` / `(List …)` / scalar payload `encode_ty_payload` builds
//! (mirrors `export_types_wire`). TOTAL on decode: a malformed / wrong-shape form is skipped, and any
//! type head the consumer does not case on decodes to [`DecodedTy::Other`] (a render-blind default), never
//! a crash.

use cadenza_ast::ast::{Arenas, Builder, Struct, StructId};

/// A consumer-facing, `cadenza-ast`-free projection of a resolved result type — ONLY the distinctions a
/// runtime render needs to disambiguate a WIT-erased leaf (`Bytes` vs `List UInt8`, `Symbol` vs a
/// compound). The codec walks the structured `Ty` payload into this flat enum on decode, so the render
/// path (`cdz-run`) matches a plain value and never touches the arena. Any type the render does not
/// special-case (`Map`, `Int`/`Float`/`Bool`/…, `Nominal`, an arrow) folds to [`DecodedTy::Other`] — a
/// type-blind render, exactly as a `None` result type rendered before this wire existed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DecodedTy {
    /// The `Bytes` leaf — render a `list<u8>` as `b"…"`, NOT `#list`.
    Bytes,
    /// The `Symbol` leaf — render a string-shaped value as `#"…"`, NOT `"…"`.
    Symbol,
    /// `List elem` — the element type threads for nested disambiguation.
    List(Box<DecodedTy>),
    /// `Set elem`.
    Set(Box<DecodedTy>),
    /// `Tuple e…` — each element type in order.
    Tuple(Vec<DecodedTy>),
    /// `Record (: name T)…` — each field's type by name, in canonical (encoded) order.
    Record(Vec<(String, DecodedTy)>),
    /// `Option inner` (a `Sum` named `Option`) — the payload type threads into a `(Some p)` render.
    Option(Box<DecodedTy>),
    /// `Result ok err` (a `Sum` named `Result`) — the ok/err payload types thread into `(Ok p)`/`(Err e)`.
    Result(Box<DecodedTy>, Box<DecodedTy>),
    /// Any type the render does not special-case — a type-blind render (the pre-wire default).
    Other,
}

/// Encode the export→result-type map as the `KIND_RESULT_TYPES` artifact / `cdz-result-type` section
/// bytes — ONE canonical binary AST value (see module docs). Each entry's `Arenas` is a standalone arena
/// ROOTED at that export's type payload sub-AST (as the `rcdzc` producer extracts it via
/// `encode_ty_payload`); its root subtree is grafted verbatim into the shared response arena. Order is
/// preserved. Round-trips with [`decode_result_types`].
pub fn encode_result_types(entries: &[(String, Arenas)]) -> Vec<u8> {
    let mut b = Builder::new();
    let mut forms: Vec<StructId> = Vec::with_capacity(entries.len());
    for (name, ty_arena) in entries {
        let head = b.name("result-type");
        let name_node = b.atom_leaf(cadenza_ast::ast::Leaf::Str(name.as_str().into()));
        let payload = copy_from(&mut b, ty_arena, ty_arena.root);
        forms.push(b.list(vec![head, name_node, payload]));
    }
    let rt_head = b.name("result-types");
    let mut children = Vec::with_capacity(forms.len() + 1);
    children.push(rt_head);
    children.extend(forms);
    let root = b.list(children);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_RESULT_TYPES` bytes back into export name → [`DecodedTy`] pairs — the inverse of
/// [`encode_result_types`], read via the shared `cadenza_ast::codec`. The structured `Ty` payload is
/// walked into the flat `DecodedTy` HERE (on the codec side), so the consumer never walks the arena.
/// TOTAL: a malformed tree / wrong-shape form is skipped; an unrecognized type head decodes to
/// [`DecodedTy::Other`].
pub fn decode_result_types(bytes: &[u8]) -> Vec<(String, DecodedTy)> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Some(forms) = a.as_form(a.root, "result-types") else {
        return Vec::new();
    };
    forms
        .to_vec()
        .iter()
        .filter_map(|&f| decode_one(&a, f))
        .collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<(String, DecodedTy)> {
    let tail = a.as_form(form, "result-type")?;
    let name = a.as_str(*tail.first()?)?.to_string();
    let payload = *tail.get(1)?;
    Some((name, decode_ty(a, payload)))
}

/// Walk a resolved-type payload subtree (the `encode_ty_payload` shape) into a flat [`DecodedTy`]. TOTAL:
/// any head the render does not need decodes to `Other`. Recurses only through the shapes the consumer
/// disambiguates (List/Set/Tuple/Record/Option/Result), so a deep type stays shallow on the enum side.
fn decode_ty(a: &Arenas, id: StructId) -> DecodedTy {
    match a.get(id) {
        // A bare leaf type-name: only `Bytes` / `Symbol` matter to the render; every other scalar
        // (`Bool`/`Unit`/`Char`/`String`/`BigInt`/`Rational`) is render-blind → `Other`.
        Struct::Atom(_) => match a.as_name(id) {
            Some("Bytes") => DecodedTy::Bytes,
            Some("Symbol") => DecodedTy::Symbol,
            _ => DecodedTy::Other,
        },
        Struct::List(kids) => {
            let Some(head) = kids.first().and_then(|&h| a.as_name(h)) else {
                return DecodedTy::Other;
            };
            match head {
                "List" => match kids.get(1) {
                    Some(&elem) => DecodedTy::List(Box::new(decode_ty(a, elem))),
                    None => DecodedTy::Other,
                },
                "Set" => match kids.get(1) {
                    Some(&elem) => DecodedTy::Set(Box::new(decode_ty(a, elem))),
                    None => DecodedTy::Other,
                },
                "Tuple" => DecodedTy::Tuple(kids[1..].iter().map(|&c| decode_ty(a, c)).collect()),
                "Record" => {
                    // Each field is the shared ascription node `(: name T)`.
                    let mut fields = Vec::new();
                    for &f in &kids[1..] {
                        if let Struct::List(fk) = a.get(f)
                            && fk.len() == 3
                            && a.as_name(fk[0]) == Some(":")
                            && let Some(fname) = a.as_name(fk[1])
                        {
                            fields.push((fname.to_string(), decode_ty(a, fk[2])));
                        }
                    }
                    DecodedTy::Record(fields)
                }
                // A sum type-value `(Sum NAME <decl> arg…)`; recognize the two the render threads a payload
                // type into (Option/Result) by NAME — args follow the name + decl (indices 3, 4). Every
                // other sum (and Map / Nominal / arrow / numeric heads) is render-blind → `Other`.
                "Sum" => match kids.get(1).and_then(|&n| a.as_name(n)) {
                    Some("Option") => match kids.get(3) {
                        Some(&inner) => DecodedTy::Option(Box::new(decode_ty(a, inner))),
                        None => DecodedTy::Other,
                    },
                    Some("Result") => match (kids.get(3), kids.get(4)) {
                        (Some(&ok), Some(&err)) => DecodedTy::Result(
                            Box::new(decode_ty(a, ok)),
                            Box::new(decode_ty(a, err)),
                        ),
                        _ => DecodedTy::Other,
                    },
                    _ => DecodedTy::Other,
                },
                _ => DecodedTy::Other,
            }
        }
    }
}

/// Copy the subtree rooted at `id` of `src` into builder `b`, returning the new root id. Iterative
/// post-order so a deep type payload can't overflow the native stack (mirrors `export_types_wire::copy_from`).
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

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, Leaf};

    /// A standalone type-payload arena rooted at `root_build(&mut b)`'s node — as the producer extracts
    /// one export's `encode_ty_payload` subtree.
    fn ty(root_build: impl FnOnce(&mut Builder) -> StructId) -> Arenas {
        let mut b = Builder::new();
        let root = root_build(&mut b);
        b.finish(root)
    }

    #[test]
    fn result_types_full_ty_binary_ast_round_trips() {
        // g : Bytes (the Bytes-leaf disambiguation), greet : Symbol, xs : (List (UInt 8)) (a byte list that
        // is NOT Bytes), opt : (Sum Option <decl> Bytes), pair : (Tuple Symbol Bytes).
        let entries = vec![
            ("g".to_string(), ty(|b| b.name("Bytes"))),
            ("greet".to_string(), ty(|b| b.name("Symbol"))),
            (
                "xs".to_string(),
                ty(|b| {
                    let head = b.name("List");
                    let uint = b.name("UInt");
                    let w = b.atom_leaf(Leaf::Int {
                        value: cadenza_ast::ast::IntValue::from_i64(8),
                        radix: cadenza_ast::ast::Radix::Dec,
                    });
                    let inner = b.list(vec![uint, w]);
                    b.list(vec![head, inner])
                }),
            ),
            (
                "opt".to_string(),
                ty(|b| {
                    let head = b.name("Sum");
                    let nm = b.name("Option");
                    let decl = b.atom_leaf(Leaf::Int {
                        value: cadenza_ast::ast::IntValue::from_i64(0),
                        radix: cadenza_ast::ast::Radix::Dec,
                    });
                    let inner = b.name("Bytes");
                    b.list(vec![head, nm, decl, inner])
                }),
            ),
            (
                "pair".to_string(),
                ty(|b| {
                    let head = b.name("Tuple");
                    let s = b.name("Symbol");
                    let by = b.name("Bytes");
                    b.list(vec![head, s, by])
                }),
            ),
        ];
        let decoded = decode_result_types(&encode_result_types(&entries));
        assert_eq!(
            decoded,
            vec![
                ("g".to_string(), DecodedTy::Bytes),
                ("greet".to_string(), DecodedTy::Symbol),
                (
                    "xs".to_string(),
                    DecodedTy::List(Box::new(DecodedTy::Other))
                ),
                (
                    "opt".to_string(),
                    DecodedTy::Option(Box::new(DecodedTy::Bytes))
                ),
                (
                    "pair".to_string(),
                    DecodedTy::Tuple(vec![DecodedTy::Symbol, DecodedTy::Bytes])
                ),
            ]
        );
    }

    #[test]
    fn record_fields_decode_by_name() {
        // (Record (: id Symbol) (: raw Bytes)) — field types recovered by name, in order.
        let arena = ty(|b| {
            let head = b.name("Record");
            let c1 = b.name(":");
            let n1 = b.name("id");
            let t1 = b.name("Symbol");
            let f1 = b.list(vec![c1, n1, t1]);
            let c2 = b.name(":");
            let n2 = b.name("raw");
            let t2 = b.name("Bytes");
            let f2 = b.list(vec![c2, n2, t2]);
            b.list(vec![head, f1, f2])
        });
        let decoded = decode_result_types(&encode_result_types(&[("r".to_string(), arena)]));
        assert_eq!(
            decoded,
            vec![(
                "r".to_string(),
                DecodedTy::Record(vec![
                    ("id".to_string(), DecodedTy::Symbol),
                    ("raw".to_string(), DecodedTy::Bytes),
                ])
            )]
        );
    }

    #[test]
    fn empty_and_garbage_are_total() {
        assert!(decode_result_types(&encode_result_types(&[])).is_empty());
        assert!(decode_result_types(b"not a binary-ast tree").is_empty());
    }

    #[test]
    fn unrecognized_head_folds_to_other() {
        // A Map / an arrow / a bare numeric are render-blind → Other.
        let arrow = ty(|b| {
            let h = b.name("->");
            let p = b.name("Bytes");
            let r = b.name("Symbol");
            b.list(vec![h, p, r])
        });
        let decoded = decode_result_types(&encode_result_types(&[("f".to_string(), arrow)]));
        assert_eq!(decoded, vec![("f".to_string(), DecodedTy::Other)]);
    }
}
