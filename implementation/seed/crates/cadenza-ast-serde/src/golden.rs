//! Golden byte tests of the typed mapping that spec/contracts/ast-typed-values.md pins: one test
//! for each row of the contract, built from the row's worked example, so that the bytes a typed
//! value encodes to and the value a decoder reads back from them are the contract's. A change to
//! the mapping that alters a row's bytes fails that row's test.

use crate::{from_arenas, from_bytes, to_arenas, to_bytes};
use cadenza_ast::ast::{Builder, CompoundCtor, IntValue, Leaf, Radix};
use cadenza_ast::codec;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The eight-byte container header, which a worked example writes as `H`.
const HEADER: [u8; 8] = *b"cdzast\x00\x01";

const F64_300: &str = "H 01 06 00 00 00 00 00 00 00 00 00 02 01 2C 01 00 00 00";
const F64_ONE_AND_A_HALF: &str = "H 01 06 00 FF FF FF FF FF FF FF FF 01 0F 01 00 00 00";
const F64_MINUS_A_QUARTER: &str = "H 01 06 01 FF FF FF FF FF FF FF FE 01 19 01 00 00 00";
const F64_NEGATIVE_ZERO: &str = "H 01 06 01 00 00 00 00 00 00 00 00 00 01 00 00 00";
const F32_TENTH: &str = "H 01 06 00 FF FF FF FF FF FF FF FF 01 01 01 00 00 00";
const F32_300: &str = "H 01 06 00 00 00 00 00 00 00 00 02 01 03 01 00 00 00";
const LIST: &str = "H 03 14 00 01 01 00 01 02 04 00 00 00 01 00 02 01 03 00 01 02 03";
const TUPLE: &str = "H 03 15 00 01 01 07 01 61 04 00 00 00 01 00 02 01 03 00 01 02 03";
const MAP: &str = concat!(
    "H 04 17 19 07 01 61 00 01 01",
    " 06 00 00 00 01 00 02 00 03 01 03 01 02 03 01 02 00 04 05"
);
const RECORD: &str = concat!(
    "H 06 16 19 07 01 78 00 01 01 07 01 79 00 01 02",
    " 0A 00 00 00 01 00 02 00 03 01 03 01 02 03 00 01 00 04 00 05 01 03 05 06 07 01 03 00 04 08 09"
);
const RENAMED: &str = concat!(
    "H 04 16 19 07 01 79 00 01 02",
    " 06 00 00 00 01 00 02 00 03 01 03 01 02 03 01 02 00 04 05"
);
const EMPTY: &str = "H 01 0A 05 45 6D 70 74 79 02 00 00 01 01 00 01";
const CIRCLE: &str = "H 02 0A 06 43 69 72 63 6C 65 00 01 05 03 00 00 00 01 01 02 00 01 02";
const PAIR: &str = concat!(
    "H 03 0A 04 50 61 69 72 00 01 01 00 01 02",
    " 04 00 00 00 01 00 02 01 03 00 01 02 03"
);
const RECT: &str = concat!(
    "H 07 0A 04 52 65 63 74 16 19 07 01 77 00 01 01 07 01 68 00 01 02",
    " 0C 00 00 00 01 00 02 00 03 00 04 01 03 02 03 04",
    " 00 02 00 05 00 06 01 03 06 07 08 01 03 01 05 09 01 02 00 0A 0B"
);
const WIDE: &str = concat!(
    "H 08 16 19 07 01 78 00 01 01 07 05 65 78 74 72 61 07 05 69 74 65 6D 73 14 00 01 02",
    " 11 00 00 00 01 00 02 00 03 01 03 01 02 03 00 01 00 04 00 00 00 01 00 05 00 06 00 07",
    " 01 02 0A 0B 01 03 08 09 0C 01 02 07 0D 01 03 05 06 0E 01 03 00 04 0F 10"
);
const NAME_HEADED: &str = concat!(
    "H 04 0A 06 72 65 63 6F 72 64 19 07 01 78 00 01 01",
    " 06 00 00 00 01 00 02 00 03 01 03 01 02 03 01 02 00 04 05"
);
const NEWER: &str = concat!(
    "H 06 16 19 07 01 78 00 01 01 07 04 6E 6F 74 65 07 02 68 69",
    " 0B 00 00 00 01 00 02 00 03 01 03 01 02 03 00 01 00 04",
    " 00 05 01 01 07 01 03 05 06 08 01 03 00 04 09 0A"
);

/// The bytes of a worked example as the contract writes it: `H` for the header, then each byte as
/// two hexadecimal digits, separated by white space.
fn example(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for word in text.split_whitespace() {
        if word == "H" {
            bytes.extend_from_slice(&HEADER);
        } else {
            let byte = u8::from_str_radix(word, 16).expect("two hexadecimal digits");
            bytes.push(byte);
        }
    }
    bytes
}

/// Check one worked example: `value` encodes to the example's bytes, and those bytes decode back
/// to `value`.
fn golden<T>(value: &T, text: &str)
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = example(text);
    assert_eq!(to_bytes(value).expect("encode"), bytes, "{value:?}");
    let back: T = from_bytes(&bytes).expect("decode the example");
    assert_eq!(&back, value, "the value the example decodes to");
}

/// A byte sequence: its `Serialize` and `Deserialize` declare the value as raw bytes, where a
/// `Vec<u8>` alone is a list of small integers.
#[derive(PartialEq, Debug)]
struct Raw(Vec<u8>);

impl Serialize for Raw {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Raw, D::Error> {
        deserializer.deserialize_byte_buf(RawVisitor)
    }
}

struct RawVisitor;

impl serde::de::Visitor<'_> for RawVisitor {
    type Value = Raw;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a byte sequence")
    }

    fn visit_bytes<E: serde::de::Error>(self, bytes: &[u8]) -> Result<Raw, E> {
        Ok(Raw(bytes.to_vec()))
    }
}

/// A named unit.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Marker;

/// A wrapper of one positional part.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Meters(u8);

/// A record of two fields.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Point {
    x: u8,
    y: u8,
}

/// A record whose field `x` writes under the name `y`.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Renamed {
    #[serde(rename = "y")]
    x: u8,
}

/// An enumeration with a variant of each payload shape.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
enum Shape {
    Empty,
    Circle(u8),
    Pair(u8, u8),
    Rect { w: u8, h: u8 },
}

/// A record type that declares the field `x` alone.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Narrow {
    x: u8,
}

/// `Narrow` with the field `extra` added, which holds a record that holds a list.
#[derive(Serialize)]
struct Wide {
    x: u8,
    extra: Inner,
}

#[derive(Serialize)]
struct Inner {
    items: Vec<u8>,
}

/// `Narrow` marked to refuse unknown fields.
#[derive(Deserialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
struct Strict {
    x: u8,
}

/// `Narrow` with the optional field `note` added.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Newer {
    x: u8,
    #[serde(default)]
    note: Option<String>,
}

/// `Shape`'s variant `Rect` with the field `depth` added.
#[derive(Serialize)]
enum WideShape {
    Rect { w: u8, h: u8, depth: u8 },
}

/// A record that holds a typed value of each row.
#[derive(Serialize)]
struct Every {
    flags: (bool, bool),
    ints: (u8, i8),
    floats: (f64, f32, f64, f64, f64),
    character: char,
    text: String,
    raw: Raw,
    unit: (),
    absent: Option<u8>,
    present: Option<u8>,
    tuple: (u8, u8),
    list: Vec<u8>,
    map: BTreeMap<String, u8>,
    wrapper: Meters,
    shapes: Vec<Shape>,
}

#[test]
fn a_boolean_is_its_boolean_leaf() {
    golden(&true, "H 01 09 01 00 00 00");
    golden(&false, "H 01 08 01 00 00 00");
}

#[test]
fn an_integer_is_a_decimal_integer_leaf() {
    golden(&300u16, "H 01 00 02 01 2C 01 00 00 00");
    golden(&-1i8, "H 01 03 01 01 01 00 00 00");
    golden(&0i64, "H 01 00 00 01 00 00 00");
}

#[test]
fn a_whole_64_bit_binary_floating_point_value_is_its_integer_decimal() {
    golden(&300.0f64, F64_300);
}

#[test]
fn a_fractional_64_bit_binary_floating_point_value_is_its_shortest_decimal() {
    golden(&1.5f64, F64_ONE_AND_A_HALF);
    golden(&-0.25f64, F64_MINUS_A_QUARTER);
}

#[test]
fn a_32_bit_binary_floating_point_value_is_its_shortest_decimal() {
    golden(&0.1f32, F32_TENTH);
    golden(&300.0f32, F32_300);
}

#[test]
fn the_sign_of_a_float_leaf_is_the_values_sign_bit() {
    let bytes = example(F64_NEGATIVE_ZERO);
    assert_eq!(to_bytes(&-0.0f64).expect("encode"), bytes);
    let back: f64 = from_bytes(&bytes).expect("decode");
    assert_eq!(back.to_bits(), (-0.0f64).to_bits());
}

#[test]
fn a_not_a_number_value_is_the_not_a_number_leaf() {
    let bytes = example("H 01 11 01 00 00 00");
    assert_eq!(to_bytes(&f64::NAN).expect("encode"), bytes);
    assert_eq!(to_bytes(&-f64::NAN).expect("encode"), bytes);
    assert_eq!(to_bytes(&f32::NAN).expect("encode"), bytes);
    let back: f64 = from_bytes(&bytes).expect("decode");
    assert!(back.is_nan());
}

#[test]
fn an_infinity_is_its_infinity_leaf() {
    golden(&f64::INFINITY, "H 01 12 01 00 00 00");
    golden(&f64::NEG_INFINITY, "H 01 13 01 00 00 00");
    golden(&f32::INFINITY, "H 01 12 01 00 00 00");
    golden(&f32::NEG_INFINITY, "H 01 13 01 00 00 00");
}

#[test]
fn a_character_is_a_character_leaf() {
    golden(&'\u{e9}', "H 01 0D 02 C3 A9 01 00 00 00");
}

#[test]
fn a_text_is_a_string_leaf() {
    let value = String::from("hi");
    golden(&value, "H 01 07 02 68 69 01 00 00 00");
    let unnormalized = String::from("e\u{301}");
    golden(&unnormalized, "H 01 07 03 65 CC 81 01 00 00 00");
}

#[test]
fn a_byte_sequence_is_a_bytes_leaf() {
    golden(&Raw(vec![0x00, 0xFF]), "H 01 0B 02 00 FF 01 00 00 00");
}

#[test]
fn a_unit_is_the_empty_list() {
    golden(&(), "H 00 01 01 00 00");
    golden(&Marker, "H 00 01 01 00 00");
}

#[test]
fn an_absent_optional_value_is_the_empty_list() {
    golden(&None::<u8>, "H 00 01 01 00 00");
}

#[test]
fn a_present_optional_value_is_a_list_of_one() {
    golden(&Some(5u8), "H 01 00 01 05 02 00 00 01 01 00 01");
}

#[test]
fn a_list_is_the_list_constructor_head_then_its_elements() {
    let value = Vec::from([1u8, 2]);
    golden(&value, LIST);
}

#[test]
fn a_tuple_is_the_tuple_constructor_head_then_its_parts() {
    let value = (1u8, String::from("a"));
    golden(&value, TUPLE);
}

#[test]
fn a_map_is_the_map_constructor_head_then_one_field_pair_per_entry() {
    let value = BTreeMap::from([(String::from("a"), 1u8)]);
    golden(&value, MAP);
}

#[test]
fn a_record_is_the_record_constructor_head_then_one_field_pair_per_field() {
    golden(&Point { x: 1, y: 2 }, RECORD);
}

#[test]
fn a_record_fields_key_is_a_string_leaf_of_its_name() {
    golden(&Renamed { x: 2 }, RENAMED);
}

#[test]
fn a_wrapper_is_the_tree_of_the_value_it_wraps() {
    golden(&Meters(5), "H 01 00 01 05 01 00 00 00");
}

#[test]
fn a_variant_without_a_payload_is_a_list_of_its_name() {
    golden(&Shape::Empty, EMPTY);
}

#[test]
fn a_variant_with_one_payload_value_is_its_name_then_that_value() {
    golden(&Shape::Circle(5), CIRCLE);
}

#[test]
fn a_variant_with_positional_parts_is_its_name_then_each_part() {
    golden(&Shape::Pair(1, 2), PAIR);
}

#[test]
fn a_variant_with_named_fields_is_its_name_then_one_record() {
    golden(&Shape::Rect { w: 1, h: 2 }, RECT);
}

#[test]
fn the_typed_mapping_writes_only_its_listed_leaf_kinds() {
    let every = Every {
        flags: (true, false),
        ints: (1, -1),
        floats: (1.5, 0.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY),
        character: 'a',
        text: String::from("a"),
        raw: Raw(vec![1]),
        unit: (),
        absent: None,
        present: Some(1),
        tuple: (1, 2),
        list: vec![1],
        map: BTreeMap::from([(String::from("a"), 1)]),
        wrapper: Meters(1),
        shapes: vec![
            Shape::Empty,
            Shape::Circle(1),
            Shape::Pair(1, 2),
            Shape::Rect { w: 1, h: 2 },
        ],
    };
    let arenas = to_arenas(&every).expect("encode");
    let mut kinds = BTreeSet::new();
    for leaf in &arenas.leaves {
        let mut b = Builder::new();
        let atom = b.atom_leaf(leaf.clone());
        // The encoding of one atom is the header, the leaf count `01`, then the leaf's kind byte.
        kinds.insert(codec::encode(&b.finish(atom))[9]);
    }
    let names: Vec<String> = kinds.iter().map(|kind| kind.to_string()).collect();
    let listed = names.join(" ");
    assert_eq!(listed, "0 3 6 7 8 9 10 11 13 17 18 19 20 21 22 23 25");
}

#[test]
fn a_decoder_refuses_an_integer_its_target_cant_hold() {
    let too_large: crate::Result<u8> = from_bytes(&example("H 01 00 02 01 2C 01 00 00 00"));
    assert!(too_large.is_err(), "{too_large:?}");
    let negative: crate::Result<u8> = from_bytes(&example("H 01 03 01 01 01 00 00 00"));
    assert!(negative.is_err(), "{negative:?}");
}

#[test]
fn a_decoder_rounds_a_float_leaf_to_its_targets_nearest_value() {
    // The decimal 1 x 10^-1, the shortest decimal of the 32-bit value 0.1.
    let bytes = example(F32_TENTH);
    let wide: f64 = from_bytes(&bytes).expect("decode into a 64-bit target");
    assert_eq!(wide.to_bits(), 0.1f64.to_bits());
    let narrow: f32 = from_bytes(&bytes).expect("decode into a 32-bit target");
    assert_eq!(narrow.to_bits(), 0.1f32.to_bits());
}

#[test]
fn a_decoder_skips_a_record_field_its_type_doesnt_declare() {
    let wide = Wide {
        x: 1,
        extra: Inner { items: vec![2] },
    };
    let bytes = example(WIDE);
    assert_eq!(to_bytes(&wide).expect("encode"), bytes);
    let got: Narrow = from_bytes(&bytes).expect("skip the field `extra`");
    assert_eq!(got, Narrow { x: 1 });
}

#[test]
fn a_decoder_skips_an_undeclared_field_whatever_tree_it_holds() {
    // The record `(<record> (= "x" 1) (= "member" (. object field)) (= "wide" N))`, where N is an
    // integer of 17 bytes: its two undeclared fields hold trees that no typed value is written as,
    // a member access and an integer wider than every machine integer, and the skip reads neither.
    let mut b = Builder::new();
    let x_key = b.atom_leaf(Leaf::Str(Arc::from("x")));
    let x_value = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(1),
        radix: Radix::Dec,
    });
    let x = b.field_pair(x_key, x_value);
    let member_key = b.atom_leaf(Leaf::Str(Arc::from("member")));
    let object = b.name("object");
    let field = b.name("field");
    let member_value = b.member(object, field);
    let member = b.field_pair(member_key, member_value);
    let wide_key = b.atom_leaf(Leaf::Str(Arc::from("wide")));
    let wide_value = b.atom_leaf(Leaf::Int {
        value: IntValue {
            negative: false,
            magnitude: vec![1; 17],
        },
        radix: Radix::Dec,
    });
    let wide = b.field_pair(wide_key, wide_value);
    let record = b.compound(CompoundCtor::Record, &[x, member, wide]);
    let arenas = b.finish(record);
    let got: Narrow = from_arenas(&arenas).expect("skip the undeclared fields");
    assert_eq!(got, Narrow { x: 1 });
}

#[test]
fn a_decoder_skips_an_undeclared_field_of_a_variant_with_named_fields() {
    let wide = WideShape::Rect {
        w: 1,
        h: 2,
        depth: 3,
    };
    let bytes = to_bytes(&wide).expect("encode");
    let got: Shape = from_bytes(&bytes).expect("skip the field `depth`");
    assert_eq!(got, Shape::Rect { w: 1, h: 2 });
}

#[test]
fn a_type_that_refuses_unknown_fields_refuses_them() {
    let Err(crate::Error::Message(message)) = from_bytes::<Strict>(&example(WIDE)) else {
        panic!("the field `extra` is refused");
    };
    assert!(message.starts_with("unknown field `extra`"), "{message}");
}

#[test]
fn a_decoder_may_accept_a_name_head_in_place_of_a_constructor_head() {
    let got: Narrow = from_bytes(&example(NAME_HEADED)).expect("read the name head `record`");
    assert_eq!(got, Narrow { x: 1 });
}

#[test]
fn a_field_added_to_a_record_type_is_optional() {
    let newer = Newer {
        x: 1,
        note: Some(String::from("hi")),
    };
    let bytes = example(NEWER);
    assert_eq!(to_bytes(&newer).expect("encode"), bytes);
    let earlier: Narrow = from_bytes(&bytes).expect("the earlier type skips the field `note`");
    assert_eq!(earlier, Narrow { x: 1 });
    let earlier_bytes = to_bytes(&Narrow { x: 1 }).expect("encode");
    let read: Newer = from_bytes(&earlier_bytes).expect("the newer type reads an earlier value");
    assert_eq!(read, Newer { x: 1, note: None });
}
