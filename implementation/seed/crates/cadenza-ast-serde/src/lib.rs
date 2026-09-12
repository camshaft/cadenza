//! `cadenza-ast-serde` — a serde data format whose serialized bytes ARE the canonical binary-AST.
//!
//! serde defines a `Serializer`/`Deserializer` trait model that concrete formats (serde_json,
//! ciborium, …) implement; this crate is one more such format, with `cadenza_ast`'s binary-AST as its
//! wire. The entry points:
//!
//! - [`to_bytes`] drives serde's serialization over a `T: Serialize`, building a `cadenza_ast` value
//!   tree via [`Builder`], then `cadenza_ast::codec::encode`s it — so the OUTPUT is byte-for-byte the
//!   canonical binary-AST (the one data-exchange format; this is NOT a competing wire).
//! - [`from_bytes`] `cadenza_ast::codec::decode`s the binary-AST and drives serde's (type-directed)
//!   deserialization over the resulting tree.
//! - [`to_arenas`] / [`from_arenas`] are the same, one step in from the bytes — for a caller that
//!   already holds an [`Arenas`] (e.g. to compare a derived encoding against a hand-written one during
//!   the `*_wire.rs` migration).
//!
//! Any `#[derive(Serialize, Deserialize)]` type thus round-trips through binary-AST for free. The
//! value-model mapping is documented on [`ser`]; [`de`] is its type-directed dual.
//!
//! Scope: this is a HOST-SIDE / tooling convenience layer (std-only). The canonical wire remains the
//! hand-written `cadenza_ast::codec`; this crate USES it and never forks it.

pub mod de;
pub mod error;
pub mod ser;

pub use de::AstDeserializer;
pub use error::{Error, Result};
pub use ser::AstSerializer;

use cadenza_ast::ast::{Arenas, Builder};
use cadenza_ast::codec;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Serialize `value` into a `cadenza_ast` value tree (an [`Arenas`]) via serde. The tree is built with
/// leaf deduplication (the `Builder`'s intern) exactly as every other binary-AST producer does; the
/// caller `codec::encode`s it (or uses [`to_bytes`]).
pub fn to_arenas<T>(value: &T) -> Result<Arenas>
where
    T: ?Sized + Serialize,
{
    let mut builder = Builder::new();
    let root = value.serialize(AstSerializer {
        builder: &mut builder,
    })?;
    Ok(builder.finish(root))
}

/// Serialize `value` to canonical binary-AST bytes.
pub fn to_bytes<T>(value: &T) -> Result<Vec<u8>>
where
    T: ?Sized + Serialize,
{
    Ok(codec::encode(&to_arenas(value)?))
}

/// Deserialize a `T` from a `cadenza_ast` value tree previously produced by (or equivalent to) this
/// format.
pub fn from_arenas<T>(arenas: &Arenas) -> Result<T>
where
    T: DeserializeOwned,
{
    T::deserialize(AstDeserializer {
        arenas,
        id: arenas.root,
    })
}

/// Deserialize a `T` from canonical binary-AST bytes.
pub fn from_bytes<T>(bytes: &[u8]) -> Result<T>
where
    T: DeserializeOwned,
{
    let arenas = codec::decode(bytes).ok_or(Error::MalformedBinaryAst)?;
    from_arenas(&arenas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::fmt::Debug;

    /// Round-trip `v` through bytes and assert equality, and that the bytes decode as a valid AST.
    fn round<T>(v: T)
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        let bytes = to_bytes(&v).expect("serialize");
        assert!(
            codec::decode(&bytes).is_some(),
            "output is not a valid canonical binary-AST for {v:?}"
        );
        let back: T = from_bytes(&bytes).expect("deserialize");
        assert_eq!(v, back, "round-trip mismatch");
    }

    #[test]
    fn primitives() {
        round(true);
        round(false);
        round(0i32);
        round(-1i32);
        round(42u8);
        round(i8::MIN);
        round(i8::MAX);
        round(i16::MIN);
        round(i32::MIN);
        round(i64::MIN);
        round(i64::MAX);
        round(u64::MAX);
        round(u32::MAX);
        round(i128::MIN);
        round(i128::MAX);
        round(u128::MAX);
        round('a');
        round('é');
        round('🦀');
        round(String::from("hello, world"));
        round(String::new());
    }

    #[test]
    fn floats() {
        round(0.0f64);
        round(-0.0f64);
        round(1.5f64);
        round(-0.25f64);
        round(100.0f64);
        round(f64::MIN);
        round(f64::MAX);
        round(1.2345678901234567f64);
        round(0.0f32);
        round(1.5f32);
        round(-2.75f32);
        round(f32::MAX);
    }

    #[test]
    fn non_finite_floats() {
        // NaN is not `PartialEq`-equal to itself, so check the classification directly.
        let nan: f64 = from_bytes(&to_bytes(&f64::NAN).unwrap()).unwrap();
        assert!(nan.is_nan());
        let pinf: f64 = from_bytes(&to_bytes(&f64::INFINITY).unwrap()).unwrap();
        assert_eq!(pinf, f64::INFINITY);
        let ninf: f64 = from_bytes(&to_bytes(&f64::NEG_INFINITY).unwrap()).unwrap();
        assert_eq!(ninf, f64::NEG_INFINITY);
        let nan32: f32 = from_bytes(&to_bytes(&f32::NAN).unwrap()).unwrap();
        assert!(nan32.is_nan());
    }

    #[test]
    fn options_and_unit() {
        round(Some(5i32));
        round(None::<i32>);
        round(Some(String::from("x")));
        round(None::<String>);
        round(Some(Some(1i32)));
        round(());
    }

    #[test]
    fn sequences_and_tuples() {
        round(vec![1i32, 2, 3]);
        round(Vec::<i32>::new());
        round(vec![vec![1i32, 2], vec![3]]);
        round((1i32, "two".to_string(), 3.0f64));
        round((true,));
        round([1u8, 2, 3, 4]);
    }

    #[test]
    fn maps() {
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), 1i32);
        m.insert("b".to_string(), 2);
        round(m);
        round(BTreeMap::<String, i32>::new());
        let mut nested = BTreeMap::new();
        nested.insert(1i32, vec![true, false]);
        round(nested);
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Point {
        x: i32,
        y: i32,
        label: String,
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Wrapper(u32);

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Pair(i32, i32);

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Unit;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Nested {
        inner: Point,
        tags: Vec<String>,
        maybe: Option<Box<Nested>>,
    }

    #[test]
    fn structs() {
        round(Point {
            x: -3,
            y: 7,
            label: "origin-ish".into(),
        });
        round(Wrapper(99));
        round(Pair(1, -1));
        round(Unit);
        round(Nested {
            inner: Point {
                x: 1,
                y: 2,
                label: "p".into(),
            },
            tags: vec!["a".into(), "b".into()],
            maybe: Some(Box::new(Nested {
                inner: Point {
                    x: 0,
                    y: 0,
                    label: "".into(),
                },
                tags: vec![],
                maybe: None,
            })),
        });
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    enum Shape {
        Empty,
        Circle(f64),
        Rect(f64, f64),
        Named { name: String, sides: u32 },
    }

    #[test]
    fn enums() {
        round(Shape::Empty);
        round(Shape::Circle(2.5));
        round(Shape::Rect(3.0, 4.0));
        round(Shape::Named {
            name: "square".into(),
            sides: 4,
        });
        round(vec![
            Shape::Empty,
            Shape::Circle(1.0),
            Shape::Named {
                name: "tri".into(),
                sides: 3,
            },
        ]);
        round(Some(Shape::Rect(1.0, 2.0)));
    }

    #[test]
    fn int_out_of_range_is_an_error() {
        // A u64::MAX value cannot deserialize into a u8. Because the full u64 is handed to serde's
        // `visit_u64`, serde's own width-range check rejects it (a descriptive "expected u8"), which
        // is exactly the behavior we want — an out-of-range integer is a hard error, not a silent
        // truncation.
        let bytes = to_bytes(&(u64::MAX)).unwrap();
        let r: Result<u8> = from_bytes(&bytes);
        assert!(
            r.is_err(),
            "u64::MAX must not deserialize into u8, got {r:?}"
        );
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let r: Result<i32> = from_bytes(&[0xff, 0x00, 0x13, 0x37]);
        assert_eq!(r, Err(Error::MalformedBinaryAst));
    }

    #[test]
    fn output_is_deterministic() {
        // The same value serializes to identical bytes every time (byte-determinism — the property
        // the *_wire.rs byte-equality migration will lean on).
        let v = Point {
            x: 5,
            y: 6,
            label: "det".into(),
        };
        assert_eq!(to_bytes(&v).unwrap(), to_bytes(&v).unwrap());
    }

    // ── Shape goldens ─────────────────────────────────────────────────────────────────────────
    // Pin the EXACT AST tree shape the serializer produces for each serde kind (the mapping
    // contract documented on `ser`). These lock the format's wire: any future Serializer change
    // that alters a shape fails here, which is the invariant every byte-neutral `*_wire.rs`
    // migration depends on. `render` walks the built `Arenas` into a compact s-expr string.

    fn render(a: &cadenza_ast::ast::Arenas) -> String {
        render_node(a, a.root)
    }

    fn render_node(a: &cadenza_ast::ast::Arenas, id: cadenza_ast::ast::StructId) -> String {
        use cadenza_ast::ast::Struct;
        match &a.structure[id.0 as usize] {
            Struct::Atom(l) => render_leaf(&a.leaves[l.0 as usize]),
            Struct::List(items) => {
                let parts: Vec<String> = items.iter().map(|&c| render_node(a, c)).collect();
                format!("({})", parts.join(" "))
            }
        }
    }

    fn render_leaf(l: &cadenza_ast::ast::Leaf) -> String {
        use cadenza_ast::ast::{CompoundCtor, Leaf};
        match l {
            Leaf::Bool(b) => b.to_string(),
            Leaf::Int { value, .. } => {
                let mut m: u128 = 0;
                for &byte in &value.magnitude {
                    m = (m << 8) | byte as u128;
                }
                if value.negative {
                    format!("-{m}")
                } else {
                    format!("{m}")
                }
            }
            Leaf::Str(s) => format!("{:?}", &**s),
            Leaf::Char(c) => format!("#\\{c}"),
            Leaf::Bytes(b) => format!("bytes{}", b.len()),
            Leaf::Float(_) | Leaf::FloatNan | Leaf::FloatInf { .. } => "<float>".to_string(),
            Leaf::Name(n) => (**n).to_string(),
            Leaf::Ctor(c) => match c {
                CompoundCtor::List => "list",
                CompoundCtor::Tuple => "tuple",
                CompoundCtor::Record => "record",
                CompoundCtor::Map => "map",
                CompoundCtor::Set => "set",
            }
            .to_string(),
            Leaf::FieldPair => "=".to_string(),
            Leaf::Member => ".".to_string(),
            other => format!("<{other:?}>"),
        }
    }

    #[test]
    fn shape_primitives() {
        assert_eq!(render(&to_arenas(&5u8).unwrap()), "5");
        assert_eq!(render(&to_arenas(&-3i32).unwrap()), "-3");
        assert_eq!(render(&to_arenas(&true).unwrap()), "true");
        assert_eq!(render(&to_arenas(&"hi").unwrap()), "\"hi\"");
    }

    #[test]
    fn shape_option_and_unit() {
        // The AST-native Option idiom: None → the empty list, Some(v) → the one-element list.
        assert_eq!(render(&to_arenas(&None::<u8>).unwrap()), "()");
        assert_eq!(render(&to_arenas(&Some(7u8)).unwrap()), "(7)");
        assert_eq!(render(&to_arenas(&()).unwrap()), "()");
    }

    #[test]
    fn shape_seq_and_tuple() {
        assert_eq!(
            render(&to_arenas(&vec![1u8, 2, 3]).unwrap()),
            "(list 1 2 3)"
        );
        assert_eq!(render(&to_arenas(&Vec::<u8>::new()).unwrap()), "(list)");
        assert_eq!(render(&to_arenas(&(1u8, 2u8)).unwrap()), "(tuple 1 2)");
    }

    #[test]
    fn shape_struct_map() {
        let p = Point {
            x: 1,
            y: 2,
            label: "p".into(),
        };
        assert_eq!(
            render(&to_arenas(&p).unwrap()),
            "(record (= \"x\" 1) (= \"y\" 2) (= \"label\" \"p\"))"
        );
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), 1u8);
        m.insert("b".to_string(), 2u8);
        assert_eq!(
            render(&to_arenas(&m).unwrap()),
            "(map (= \"a\" 1) (= \"b\" 2))"
        );
    }

    #[test]
    fn shape_newtype_and_tuple_struct() {
        assert_eq!(render(&to_arenas(&Wrapper(99)).unwrap()), "99"); // newtype is transparent
        assert_eq!(render(&to_arenas(&Pair(1, -1)).unwrap()), "(tuple 1 -1)");
        assert_eq!(render(&to_arenas(&Unit).unwrap()), "()");
    }

    #[test]
    fn shape_enum_variants() {
        assert_eq!(render(&to_arenas(&Shape::Empty).unwrap()), "(Empty)");
        assert_eq!(
            render(&to_arenas(&Shape::Circle(2.5)).unwrap()),
            "(Circle <float>)"
        );
        assert_eq!(
            render(&to_arenas(&Shape::Rect(3.0, 4.0)).unwrap()),
            "(Rect <float> <float>)"
        );
        assert_eq!(
            render(
                &to_arenas(&Shape::Named {
                    name: "sq".into(),
                    sides: 4
                })
                .unwrap()
            ),
            "(Named (= \"name\" \"sq\") (= \"sides\" 4))"
        );
    }
}

#[cfg(test)]
mod migration_scenarios {
    // Real-world derived-struct behaviors the platform/tooling migrations rely on: optional fields
    // via #[serde(default)], unknown-field skipping, deny_unknown_fields, and #[serde(rename)] for
    // kebab-case wire keys. These pin that the format handles the kinds of structs a config / host
    // wire migration produces — not just the tidy round-trip cases.
    use crate::{from_bytes, to_arenas, to_bytes};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize)]
    struct OnlyA {
        a: i32,
    }

    #[derive(Deserialize, PartialEq, Debug)]
    struct WithDefault {
        a: i32,
        #[serde(default)]
        b: i32,
    }

    #[test]
    fn missing_field_uses_serde_default() {
        // `OnlyA` serializes to a record with just `a`; deserializing as `WithDefault` must fill `b`
        // from #[serde(default)] rather than erroring on the absent key.
        let bytes = to_bytes(&OnlyA { a: 1 }).unwrap();
        let got: WithDefault = from_bytes(&bytes).unwrap();
        assert_eq!(got, WithDefault { a: 1, b: 0 });
    }

    #[test]
    fn genuinely_missing_required_field_errors() {
        // Without a default, an absent required field is a hard error (serde derive's job; our reader
        // simply must not fabricate it).
        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct NeedsBoth {
            a: i32,
            b: i32,
        }
        let bytes = to_bytes(&OnlyA { a: 1 }).unwrap();
        let got: Result<NeedsBoth, _> = from_bytes(&bytes);
        assert!(got.is_err(), "missing required `b` must error, got {got:?}");
    }

    #[derive(Serialize)]
    struct Extra {
        a: i32,
        extra: i32,
    }

    #[derive(Deserialize, PartialEq, Debug)]
    struct JustA {
        a: i32,
    }

    #[test]
    fn unknown_fields_are_ignored_by_default() {
        // A record carrying an unknown `extra` field deserializes into a struct that does not declare
        // it — serde's default skips unknown fields (which drives our deserialize_ignored_any).
        let bytes = to_bytes(&Extra { a: 7, extra: 99 }).unwrap();
        let got: JustA = from_bytes(&bytes).unwrap();
        assert_eq!(got, JustA { a: 7 });
    }

    #[derive(Deserialize, PartialEq, Debug)]
    #[serde(deny_unknown_fields)]
    struct StrictA {
        a: i32,
    }

    #[test]
    fn deny_unknown_fields_rejects_extra() {
        let bytes = to_bytes(&Extra { a: 7, extra: 99 }).unwrap();
        let got: Result<StrictA, _> = from_bytes(&bytes);
        assert!(
            got.is_err(),
            "deny_unknown_fields must reject the extra field, got {got:?}"
        );
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Renamed {
        #[serde(rename = "the-field")]
        x: i32,
    }

    #[test]
    fn rename_changes_the_record_key() {
        // The wire field name follows #[serde(rename)] — needed for kebab-case wire keys (e.g.
        // `cas-url`, `root-router` in the http control frames).
        let arenas = to_arenas(&Renamed { x: 5 }).unwrap();
        let has_renamed_key = arenas
            .leaves
            .iter()
            .any(|l| matches!(l, cadenza_ast::ast::Leaf::Str(s) if &**s == "the-field"));
        assert!(
            has_renamed_key,
            "the renamed key 'the-field' must appear as a Str leaf"
        );
        // And it still round-trips by the renamed key.
        let bytes = to_bytes(&Renamed { x: 5 }).unwrap();
        let back: Renamed = from_bytes(&bytes).unwrap();
        assert_eq!(back, Renamed { x: 5 });
    }
}

#[cfg(test)]
mod canonical_value_form {
    //! FOUNDATION for the broad decoder-migration sweep (operator 2026-09-12: any struct decoding
    //! binary-AST should use serde). The platform / `Value.encode` produce records with NAME-keyed
    //! field pairs (`(record (= Name"field" v)…)`) — NOT the `Str`-keyed form this crate's own
    //! serializer emits. These tests build the platform-canonical shape DIRECTLY via `Builder` (the
    //! same primitives `cadenza_value::record` uses) and prove `from_arenas` decodes it into a derived
    //! struct — i.e. a hand-written platform decoder can be replaced by `cadenza_ast_serde::from_bytes`
    //! WITHOUT changing the producer's bytes. Where a platform convention (Option/enum/ctor-elision)
    //! diverges, this module is where the compatibility gets pinned as the Deserializer is extended.

    use crate::from_arenas;
    use cadenza_ast::ast::{Builder, CompoundCtor, IntValue, Leaf, Radix, StructId};
    use serde::Deserialize;
    use std::sync::Arc;

    fn int(b: &mut Builder, v: i64) -> StructId {
        b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(v),
            radix: Radix::Dec,
        })
    }
    fn string(b: &mut Builder, s: &str) -> StructId {
        b.atom_leaf(Leaf::Str(Arc::from(s)))
    }
    /// A NAME-keyed field pair `(= Name"key" value)` — the platform's record-field form.
    fn field(b: &mut Builder, key: &str, value: StructId) -> StructId {
        let k = b.name(key);
        b.field_pair(k, value)
    }

    #[derive(Deserialize, PartialEq, Debug)]
    struct Point {
        x: i32,
        y: i32,
        label: String,
    }

    #[test]
    fn decodes_a_name_keyed_record() {
        // Build `(record (= Name"x" 1) (= Name"y" 2) (= Name"label" "p"))` exactly as the platform's
        // value-form builder would, then decode it via serde. Field keys are NAME atoms, not Str.
        let mut b = Builder::new();
        let vx = int(&mut b, 1);
        let fx = field(&mut b, "x", vx);
        let vy = int(&mut b, 2);
        let fy = field(&mut b, "y", vy);
        let vlabel = string(&mut b, "p");
        let flabel = field(&mut b, "label", vlabel);
        let rec = b.compound(CompoundCtor::Record, &[fx, fy, flabel]);
        let arenas = b.finish(rec);
        let got: Point = from_arenas(&arenas).expect("decode name-keyed record");
        assert_eq!(
            got,
            Point {
                x: 1,
                y: 2,
                label: "p".into()
            }
        );
    }

    #[test]
    fn decodes_with_fields_out_of_declaration_order() {
        // The platform emits fields NAME-SORTED, not in declaration order; serde matches by key, so a
        // reordered record still decodes. (label, x, y) order here vs (x, y, label) declared.
        let mut b = Builder::new();
        let vlabel = string(&mut b, "q");
        let flabel = field(&mut b, "label", vlabel);
        let vx = int(&mut b, 7);
        let fx = field(&mut b, "x", vx);
        let vy = int(&mut b, 8);
        let fy = field(&mut b, "y", vy);
        let rec = b.compound(CompoundCtor::Record, &[flabel, fx, fy]);
        let arenas = b.finish(rec);
        let got: Point = from_arenas(&arenas).expect("decode reordered record");
        assert_eq!(
            got,
            Point {
                x: 7,
                y: 8,
                label: "q".into()
            }
        );
    }

    #[derive(Deserialize, PartialEq, Debug)]
    struct Config {
        listen: String,
        #[serde(default)]
        credential: Option<String>,
        inner: Nested,
    }

    #[derive(Deserialize, PartialEq, Debug)]
    struct Nested {
        n: i32,
    }

    #[test]
    fn decodes_omitted_optional_and_nested_record() {
        // A platform record that OMITS an absent optional field (the common convention) + a NESTED
        // record. `#[serde(default)]` on the Option fills None; the nested record decodes recursively.
        let mut b = Builder::new();
        let vlisten = string(&mut b, "127.0.0.1:8080");
        let listen = field(&mut b, "listen", vlisten);
        let vn = int(&mut b, 42);
        let fn_ = field(&mut b, "n", vn);
        let nrec = b.compound(CompoundCtor::Record, &[fn_]);
        let inner = field(&mut b, "inner", nrec);
        // NOTE: no `credential` field at all — it must default to None.
        let rec = b.compound(CompoundCtor::Record, &[listen, inner]);
        let arenas = b.finish(rec);
        let got: Config = from_arenas(&arenas).expect("decode omitted-optional + nested record");
        assert_eq!(
            got,
            Config {
                listen: "127.0.0.1:8080".into(),
                credential: None,
                inner: Nested { n: 42 },
            }
        );
    }

    /// Wrap `value` in a root type-ascription `(: value SomeType)` — the reader must peel it.
    fn ascribe(b: &mut Builder, value: StructId) -> StructId {
        let colon = b.name(":");
        let ty = b.name("SomeType");
        b.list(vec![colon, value, ty])
    }

    #[test]
    fn decodes_ascribed_root_record() {
        let mut b = Builder::new();
        let vx = int(&mut b, 5);
        let fx = field(&mut b, "x", vx);
        let vy = int(&mut b, 6);
        let fy = field(&mut b, "y", vy);
        let vl = string(&mut b, "asc");
        let fl = field(&mut b, "label", vl);
        let rec = b.compound(CompoundCtor::Record, &[fx, fy, fl]);
        let asc = ascribe(&mut b, rec); // (: (record …) SomeType)
        let arenas = b.finish(asc);
        let got: Point = from_arenas(&arenas).expect("decode ascribed record");
        assert_eq!(
            got,
            Point {
                x: 5,
                y: 6,
                label: "asc".into()
            }
        );
    }

    #[test]
    fn decodes_present_optional_as_bare_value() {
        // Platform/canonical convention for `Option`: a PRESENT optional field carries its BARE value
        // (NOT this crate's `(v)` wrapper) ⇒ Some; an ABSENT field ⇒ None (via #[serde(default)], see
        // `decodes_omitted_optional_and_nested_record`). This is how e.g. cdz-cas-http config encodes
        // `read-credential`, so the reader must accept it.
        let mut b = Builder::new();
        let vlisten = string(&mut b, "0.0.0.0:9000");
        let listen = field(&mut b, "listen", vlisten);
        let vcred = string(&mut b, "secret");
        let cred = field(&mut b, "credential", vcred); // PRESENT, bare Str value
        let vn = int(&mut b, 1);
        let fn_ = field(&mut b, "n", vn);
        let nrec = b.compound(CompoundCtor::Record, &[fn_]);
        let inner = field(&mut b, "inner", nrec);
        let rec = b.compound(CompoundCtor::Record, &[listen, cred, inner]);
        let arenas = b.finish(rec);
        let got: Config = from_arenas(&arenas).expect("decode present bare optional");
        assert_eq!(
            got,
            Config {
                listen: "0.0.0.0:9000".into(),
                credential: Some("secret".into()),
                inner: Nested { n: 1 },
            }
        );
    }
}
