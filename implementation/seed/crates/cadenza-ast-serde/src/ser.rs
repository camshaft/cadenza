//! The serde `Serializer` that builds a `cadenza_ast` value tree.
//!
//! Every `serialize_*` returns the [`StructId`] of the node it built into the shared [`Builder`]; the
//! top-level driver ([`crate::to_bytes`]) then `finish`es the builder at that root and
//! `codec::encode`s it. spec/contracts/ast-typed-values.md pins the value-model mapping as a
//! frozen contract, with a worked byte example for each row that the `golden` tests check, and the
//! [`crate::de`] reader reads it back. In brief, with `<list>`, `<tuple>`, `<map>` and `<record>`
//! for the constructor-head leaves and `=` for the field-pair head leaf:
//!
//! - primitives -> the matching leaf atom (`Bool`/`Int`/`Float`/`Char`/`Str`/`Bytes`);
//! - `unit` / `unit_struct` / `None` -> the empty list `()`; `Some(v)` -> the one-element list
//!   `(v)`;
//! - `seq` -> `(<list> e...)`, `tuple`/`tuple_struct` -> `(<tuple> e...)`;
//! - `map` -> `(<map> (= k v)...)`, `struct` -> `(<record> (= "field" v)...)`, a field name being a
//!   `Str` key;
//! - an enum variant -> `(VariantName payload...)`: the head is the variant `Name`, the tail is the
//!   payload (empty for a unit variant, one node for newtype, the elements for a tuple variant, and
//!   one `(<record> (= "field" v)...)` node for a struct variant).

use crate::error::{Error, Result};
use cadenza_ast::ast::{Builder, CompoundCtor, Decimal, IntValue, Leaf, Radix, StructId};
use serde::ser;
use serde::ser::Serialize;
use std::sync::Arc;

//= spec/contracts/ast-typed-values.md#a-whole-64-bit-binary-floating-point-value-is-its-integer-decimal
//# A finite 64-bit binary floating-point value that is a whole number MUST be written as an atom of a float leaf whose exponent is zero and whose significand is the value's absolute value.
//= spec/contracts/ast-typed-values.md#a-fractional-64-bit-binary-floating-point-value-is-its-shortest-decimal
//# A finite 64-bit binary floating-point value that is not a whole number MUST be written as an atom of a float leaf whose significand and exponent are those of the value's shortest decimal.
//= spec/contracts/ast-typed-values.md#the-sign-of-a-float-leaf-is-the-value-s-sign-bit
//# The negative flag of the float leaf written for a finite binary floating-point value MUST be set exactly when the value's sign bit is set, so that negative zero is a negative float leaf with an empty significand.
//= spec/contracts/ast-typed-values.md#a-not-a-number-value-is-the-not-a-number-leaf
//# A not-a-number value of either binary floating-point format MUST be written as an atom of the not-a-number leaf, kind `17`, whatever its sign and payload.
//= spec/contracts/ast-typed-values.md#an-infinity-is-its-infinity-leaf
//# A positive infinity of either binary floating-point format MUST be written as an atom of the positive-infinity leaf, kind `18`.
//= spec/contracts/ast-typed-values.md#an-infinity-is-its-infinity-leaf
//# A negative infinity of either binary floating-point format MUST be written as an atom of the negative-infinity leaf, kind `19`.
/// A finite `f64` is an exact decimal; a non-finite one is a payloadless marker leaf.
fn f64_leaf(v: f64) -> Leaf {
    if v.is_nan() {
        Leaf::FloatNan
    } else if v.is_infinite() {
        Leaf::FloatInf {
            negative: v.is_sign_negative(),
        }
    } else {
        // Finite by the branches above, so `from_f64` yields `Some`.
        Leaf::Float(Decimal::from_f64(v).expect("finite f64 has an exact decimal"))
    }
}

//= spec/contracts/ast-typed-values.md#a-32-bit-binary-floating-point-value-is-its-shortest-decimal
//# A finite 32-bit binary floating-point value MUST be written as an atom of a float leaf whose significand and exponent are those of the value's shortest decimal, whether or not it is a whole number.
//= spec/contracts/ast-typed-values.md#the-sign-of-a-float-leaf-is-the-value-s-sign-bit
//# The negative flag of the float leaf written for a finite binary floating-point value MUST be set exactly when the value's sign bit is set, so that negative zero is a negative float leaf with an empty significand.
//= spec/contracts/ast-typed-values.md#a-not-a-number-value-is-the-not-a-number-leaf
//# A not-a-number value of either binary floating-point format MUST be written as an atom of the not-a-number leaf, kind `17`, whatever its sign and payload.
//= spec/contracts/ast-typed-values.md#an-infinity-is-its-infinity-leaf
//# A positive infinity of either binary floating-point format MUST be written as an atom of the positive-infinity leaf, kind `18`.
//= spec/contracts/ast-typed-values.md#an-infinity-is-its-infinity-leaf
//# A negative infinity of either binary floating-point format MUST be written as an atom of the negative-infinity leaf, kind `19`.
/// The `f32` twin of [`f64_leaf`] — uses the `f32`-precise shortest-decimal decomposition so a
/// round-tripped `f32` reconstructs bit-exactly.
fn f32_leaf(v: f32) -> Leaf {
    if v.is_nan() {
        Leaf::FloatNan
    } else if v.is_infinite() {
        Leaf::FloatInf {
            negative: v.is_sign_negative(),
        }
    } else {
        Leaf::Float(Decimal::from_f32(v).expect("finite f32 has an exact decimal"))
    }
}

/// The serde `Serializer` over a borrowed [`Builder`]. `Ok = StructId` (the node just built).
pub struct AstSerializer<'a> {
    pub(crate) builder: &'a mut Builder,
}

impl<'a> AstSerializer<'a> {
    //= spec/contracts/ast-typed-values.md#an-integer-is-a-decimal-integer-leaf
    //# An integer of any width MUST be written as an atom of a decimal integer leaf whose magnitude is the integer's absolute value, kind `0` for zero and every positive integer and kind `3` for every negative integer.
    fn int_atom(self, value: IntValue) -> StructId {
        self.builder.atom_leaf(Leaf::Int {
            value,
            radix: Radix::Dec,
        })
    }
}

//= spec/contracts/ast-typed-values.md#the-leaf-kinds-the-typed-mapping-writes
//# An encoder MUST write a typed value with leaves of kinds `0`, `3`, `6`, `7`, `8`, `9`, `10`, `11`, `13`, `17`, `18`, `19`, `20`, `21`, `22`, `23` and `25` alone.
impl<'a> ser::Serializer for AstSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    type SerializeSeq = SeqSerializer<'a>;
    type SerializeTuple = SeqSerializer<'a>;
    type SerializeTupleStruct = SeqSerializer<'a>;
    type SerializeTupleVariant = VariantSeqSerializer<'a>;
    type SerializeMap = MapSerializer<'a>;
    type SerializeStruct = StructSerializer<'a>;
    type SerializeStructVariant = StructVariantSerializer<'a>;

    //= spec/contracts/ast-typed-values.md#a-boolean-is-its-boolean-leaf
    //# A boolean MUST be written as an atom of the boolean leaf of its value, kind `8` for false and kind `9` for true.
    fn serialize_bool(self, v: bool) -> Result<StructId> {
        Ok(self.builder.atom_leaf(Leaf::Bool(v)))
    }

    fn serialize_i8(self, v: i8) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_i128(v as i128)))
    }
    fn serialize_i16(self, v: i16) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_i128(v as i128)))
    }
    fn serialize_i32(self, v: i32) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_i128(v as i128)))
    }
    fn serialize_i64(self, v: i64) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_i128(v as i128)))
    }
    fn serialize_i128(self, v: i128) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_i128(v)))
    }

    fn serialize_u8(self, v: u8) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_u128(v as u128)))
    }
    fn serialize_u16(self, v: u16) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_u128(v as u128)))
    }
    fn serialize_u32(self, v: u32) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_u128(v as u128)))
    }
    fn serialize_u64(self, v: u64) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_u128(v as u128)))
    }
    fn serialize_u128(self, v: u128) -> Result<StructId> {
        Ok(self.int_atom(IntValue::from_u128(v)))
    }

    fn serialize_f32(self, v: f32) -> Result<StructId> {
        Ok(self.builder.atom_leaf(f32_leaf(v)))
    }
    fn serialize_f64(self, v: f64) -> Result<StructId> {
        Ok(self.builder.atom_leaf(f64_leaf(v)))
    }

    //= spec/contracts/ast-typed-values.md#a-character-is-a-character-leaf
    //# A character, one Unicode scalar value, MUST be written as an atom of a character leaf, kind `13`, holding that scalar.
    fn serialize_char(self, v: char) -> Result<StructId> {
        Ok(self.builder.atom_leaf(Leaf::Char(v)))
    }

    //= spec/contracts/ast-typed-values.md#a-text-is-a-string-leaf
    //# A text MUST be written as an atom of a string leaf, kind `7`, holding the text's UTF-8 bytes as given, without normalization.
    fn serialize_str(self, v: &str) -> Result<StructId> {
        Ok(self.builder.atom_leaf(Leaf::Str(Arc::from(v))))
    }

    //= spec/contracts/ast-typed-values.md#a-byte-sequence-is-a-bytes-leaf
    //# A byte sequence MUST be written as an atom of a bytes leaf, kind `11`, holding its bytes.
    fn serialize_bytes(self, v: &[u8]) -> Result<StructId> {
        Ok(self.builder.atom_leaf(Leaf::Bytes(Arc::from(v))))
    }

    //= spec/contracts/ast-typed-values.md#an-absent-optional-value-is-the-empty-list
    //# An absent optional value MUST be written as the empty list `()`.
    fn serialize_none(self) -> Result<StructId> {
        Ok(self.builder.list(Vec::new()))
    }

    //= spec/contracts/ast-typed-values.md#a-present-optional-value-is-a-list-of-one
    //# A present optional value MUST be written as a list of one child, the tree of the value it holds.
    fn serialize_some<T>(self, value: &T) -> Result<StructId>
    where
        T: ?Sized + Serialize,
    {
        let node = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        Ok(self.builder.list(vec![node]))
    }

    //= spec/contracts/ast-typed-values.md#a-unit-is-the-empty-list
    //# A unit, the unit value or a named unit, MUST be written as the empty list `()`.
    fn serialize_unit(self) -> Result<StructId> {
        Ok(self.builder.list(Vec::new()))
    }

    //= spec/contracts/ast-typed-values.md#a-unit-is-the-empty-list
    //# A unit, the unit value or a named unit, MUST be written as the empty list `()`.
    fn serialize_unit_struct(self, _name: &'static str) -> Result<StructId> {
        Ok(self.builder.list(Vec::new()))
    }

    //= spec/contracts/ast-typed-values.md#an-enumeration-variant-is-headed-by-a-name-leaf-of-its-name
    //# The first child of the list an enumeration variant is written as MUST be an atom of a name leaf, kind `10`, holding the variant's name as its type declares it for writing.
    //= spec/contracts/ast-typed-values.md#a-variant-without-a-payload-is-a-list-of-its-name
    //# A variant without a payload MUST be written as a list of one child, the atom of its name.
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<StructId> {
        let head = self.builder.name(variant);
        Ok(self.builder.list(vec![head]))
    }

    //= spec/contracts/ast-typed-values.md#a-wrapper-is-the-tree-of-the-value-it-wraps
    //# A wrapper MUST be written as the tree of its one positional part, with nothing added.
    fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> Result<StructId>
    where
        T: ?Sized + Serialize,
    {
        // Transparent, per serde convention: a newtype struct is its inner value.
        value.serialize(self)
    }

    //= spec/contracts/ast-typed-values.md#an-enumeration-variant-is-headed-by-a-name-leaf-of-its-name
    //# The first child of the list an enumeration variant is written as MUST be an atom of a name leaf, kind `10`, holding the variant's name as its type declares it for writing.
    //= spec/contracts/ast-typed-values.md#a-variant-with-one-payload-value-is-its-name-then-that-value
    //# A variant whose payload is one value MUST be written as a list of two children: the atom of its name, then the tree of the payload.
    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<StructId>
    where
        T: ?Sized + Serialize,
    {
        let head = self.builder.name(variant);
        let node = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        Ok(self.builder.list(vec![head, node]))
    }

    //= spec/contracts/ast-typed-values.md#a-list-is-the-list-constructor-head-then-its-elements
    //# A list MUST be written as a list whose first child is an atom of the list constructor head, kind `20`, followed by the tree of each element in order.
    fn serialize_seq(self, _len: Option<usize>) -> Result<SeqSerializer<'a>> {
        Ok(SeqSerializer {
            builder: self.builder,
            ctor: CompoundCtor::List,
            elems: Vec::new(),
        })
    }

    //= spec/contracts/ast-typed-values.md#a-tuple-is-the-tuple-constructor-head-then-its-parts
    //# A tuple MUST be written as a list whose first child is an atom of the tuple constructor head, kind `21`, followed by the tree of each part in order.
    fn serialize_tuple(self, _len: usize) -> Result<SeqSerializer<'a>> {
        Ok(SeqSerializer {
            builder: self.builder,
            ctor: CompoundCtor::Tuple,
            elems: Vec::new(),
        })
    }

    //= spec/contracts/ast-typed-values.md#a-tuple-is-the-tuple-constructor-head-then-its-parts
    //# A tuple MUST be written as a list whose first child is an atom of the tuple constructor head, kind `21`, followed by the tree of each part in order.
    fn serialize_tuple_struct(self, _name: &'static str, _len: usize) -> Result<SeqSerializer<'a>> {
        Ok(SeqSerializer {
            builder: self.builder,
            ctor: CompoundCtor::Tuple,
            elems: Vec::new(),
        })
    }

    //= spec/contracts/ast-typed-values.md#an-enumeration-variant-is-headed-by-a-name-leaf-of-its-name
    //# The first child of the list an enumeration variant is written as MUST be an atom of a name leaf, kind `10`, holding the variant's name as its type declares it for writing.
    //= spec/contracts/ast-typed-values.md#a-variant-with-positional-parts-is-its-name-then-each-part
    //# A variant whose payload is two or more positional parts MUST be written as a list of the atom of its name followed by the tree of each part in order.
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<VariantSeqSerializer<'a>> {
        let head = self.builder.name(variant);
        Ok(VariantSeqSerializer {
            builder: self.builder,
            items: vec![head],
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<MapSerializer<'a>> {
        Ok(MapSerializer {
            builder: self.builder,
            entries: Vec::new(),
            pending_key: None,
        })
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<StructSerializer<'a>> {
        Ok(StructSerializer {
            builder: self.builder,
            entries: Vec::new(),
        })
    }

    //= spec/contracts/ast-typed-values.md#an-enumeration-variant-is-headed-by-a-name-leaf-of-its-name
    //# The first child of the list an enumeration variant is written as MUST be an atom of a name leaf, kind `10`, holding the variant's name as its type declares it for writing.
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<StructVariantSerializer<'a>> {
        let head = self.builder.name(variant);
        Ok(StructVariantSerializer {
            builder: self.builder,
            head,
            entries: Vec::new(),
        })
    }
}

/// A `seq` / `tuple` / `tuple_struct` — collects element nodes, then wraps them in a compound ctor.
pub struct SeqSerializer<'a> {
    builder: &'a mut Builder,
    ctor: CompoundCtor,
    elems: Vec<StructId>,
}

impl<'a> ser::SerializeSeq for SeqSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let node = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        self.elems.push(node);
        Ok(())
    }

    //= spec/contracts/ast-typed-values.md#a-decoder-may-accept-a-name-head-in-place-of-a-constructor-head
    //# An encoder MUST NOT write a name leaf in place of a constructor head.
    fn end(self) -> Result<StructId> {
        Ok(self.builder.compound(self.ctor, &self.elems))
    }
}

impl<'a> ser::SerializeTuple for SeqSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<StructId> {
        ser::SerializeSeq::end(self)
    }
}

impl<'a> ser::SerializeTupleStruct for SeqSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<StructId> {
        ser::SerializeSeq::end(self)
    }
}

/// A `tuple_variant` `(VariantName e…)` — the head Name is already pushed; elements append.
pub struct VariantSeqSerializer<'a> {
    builder: &'a mut Builder,
    items: Vec<StructId>,
}

impl<'a> ser::SerializeTupleVariant for VariantSeqSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let node = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        self.items.push(node);
        Ok(())
    }

    fn end(self) -> Result<StructId> {
        Ok(self.builder.list(self.items))
    }
}

/// A `map` `(<map> (= k v)...)`: keys and values arrive separately, so we stash the key node until its
/// value arrives, then push a `(= k v)` field pair.
pub struct MapSerializer<'a> {
    builder: &'a mut Builder,
    entries: Vec<StructId>,
    pending_key: Option<StructId>,
}

impl<'a> ser::SerializeMap for MapSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let node = key.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        self.pending_key = Some(node);
        Ok(())
    }

    //= spec/contracts/ast-typed-values.md#a-field-pair-is-the-field-pair-head-a-key-and-a-value
    //# A field pair MUST be written as a list of three children: an atom of the field-pair head, kind `25`, then the tree of the key, then the tree of the value.
    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let key = self
            .pending_key
            .take()
            .ok_or_else(|| Error::Message("map value serialized before its key".into()))?;
        let value = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        let pair = self.builder.field_pair(key, value);
        self.entries.push(pair);
        Ok(())
    }

    //= spec/contracts/ast-typed-values.md#a-map-is-the-map-constructor-head-then-one-field-pair-per-entry
    //# A map MUST be written as a list whose first child is an atom of the map constructor head, kind `23`, followed by one field pair for each entry, holding the entry's key and value, in the order the map presents its entries.
    //= spec/contracts/ast-typed-values.md#a-decoder-may-accept-a-name-head-in-place-of-a-constructor-head
    //# An encoder MUST NOT write a name leaf in place of a constructor head.
    fn end(self) -> Result<StructId> {
        Ok(self.builder.compound(CompoundCtor::Map, &self.entries))
    }
}

/// A `struct` `(<record> (= "field" v)...)`: each field key is a `Str` leaf holding the field name.
pub struct StructSerializer<'a> {
    builder: &'a mut Builder,
    entries: Vec<StructId>,
}

impl<'a> ser::SerializeStruct for StructSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    //= spec/contracts/ast-typed-values.md#a-field-pair-is-the-field-pair-head-a-key-and-a-value
    //# A field pair MUST be written as a list of three children: an atom of the field-pair head, kind `25`, then the tree of the key, then the tree of the value.
    //= spec/contracts/ast-typed-values.md#a-record-field-s-key-is-a-string-leaf-of-its-name
    //# The key of a record field's field pair MUST be written as an atom of a string leaf holding the field's name as the record's type declares it for writing.
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let key_node = self.builder.atom_leaf(Leaf::Str(Arc::from(key)));
        let value_node = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        let pair = self.builder.field_pair(key_node, value_node);
        self.entries.push(pair);
        Ok(())
    }

    //= spec/contracts/ast-typed-values.md#a-record-is-the-record-constructor-head-then-one-field-pair-per-field
    //# A record MUST be written as a list whose first child is an atom of the record constructor head, kind `22`, followed by one field pair for each of its fields, in the order the record's type declares them.
    //= spec/contracts/ast-typed-values.md#a-decoder-may-accept-a-name-head-in-place-of-a-constructor-head
    //# An encoder MUST NOT write a name leaf in place of a constructor head.
    fn end(self) -> Result<StructId> {
        Ok(self.builder.compound(CompoundCtor::Record, &self.entries))
    }
}

/// A `struct_variant` `(VariantName (<record> (= "field" v)...))`: the head Name, then one record
/// of the field pairs.
pub struct StructVariantSerializer<'a> {
    builder: &'a mut Builder,
    head: StructId,
    entries: Vec<StructId>,
}

impl<'a> ser::SerializeStructVariant for StructVariantSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

    //= spec/contracts/ast-typed-values.md#a-field-pair-is-the-field-pair-head-a-key-and-a-value
    //# A field pair MUST be written as a list of three children: an atom of the field-pair head, kind `25`, then the tree of the key, then the tree of the value.
    //= spec/contracts/ast-typed-values.md#a-record-field-s-key-is-a-string-leaf-of-its-name
    //# The key of a record field's field pair MUST be written as an atom of a string leaf holding the field's name as the record's type declares it for writing.
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        let key_node = self.builder.atom_leaf(Leaf::Str(Arc::from(key)));
        let value_node = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        let pair = self.builder.field_pair(key_node, value_node);
        self.entries.push(pair);
        Ok(())
    }

    //= spec/contracts/ast-typed-values.md#a-variant-with-named-fields-is-its-name-then-one-record
    //# A variant whose payload is named fields MUST be written as a list of two children: the atom of its name, then the tree of a record holding those fields.
    //= spec/contracts/ast-typed-values.md#a-decoder-may-accept-a-name-head-in-place-of-a-constructor-head
    //# An encoder MUST NOT write a name leaf in place of a constructor head.
    fn end(self) -> Result<StructId> {
        // A struct variant is `(VariantName (record (= "field" v)…))` — the variant name applied to a
        // single Record payload. This matches the canonical value-form sum shape `Variant(Record(…))`
        // (e.g. `type Event = Exited(Record(reducer, schema, reason)) | …`), so a Rust struct-variant
        // enum decodes from / encodes to the same bytes a `.cdz` schema value uses — no need to model it
        // as a newtype-variant-wrapping-a-struct in Rust.
        let record = self.builder.compound(CompoundCtor::Record, &self.entries);
        Ok(self.builder.list(vec![self.head, record]))
    }
}
