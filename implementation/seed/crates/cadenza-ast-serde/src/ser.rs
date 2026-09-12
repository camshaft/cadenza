//! The serde `Serializer` that builds a `cadenza_ast` value tree.
//!
//! Every `serialize_*` returns the [`StructId`] of the node it built into the shared [`Builder`]; the
//! top-level driver ([`crate::to_bytes`]) then `finish`es the builder at that root and
//! `codec::encode`s it. The value-model mapping (mirrored by the [`crate::de`] reader):
//!
//! - primitives → the matching leaf atom (`Bool`/`Int`/`Float`/`Char`/`Str`/`Bytes`);
//! - `unit` / `unit_struct` / `None` → the empty list `()`; `Some(v)` → the one-element list `(v)`;
//! - `seq` → `("list" e…)`, `tuple`/`tuple_struct` → `("tuple" e…)`;
//! - `map` → `("map" (= k v)…)`, `struct` → `("record" (= "field" v)…)` (field name a `Str` key);
//! - an enum variant → `(VariantName payload…)`: the head is the variant `Name`, the tail is the
//!   payload (empty for a unit variant, one node for newtype, the elements for a tuple variant, the
//!   `(= "field" v)` pairs for a struct variant).

use crate::error::{Error, Result};
use cadenza_ast::ast::{Builder, CompoundCtor, Decimal, IntValue, Leaf, Radix, StructId};
use serde::ser;
use serde::ser::Serialize;
use std::sync::Arc;

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
    fn int_atom(self, value: IntValue) -> StructId {
        self.builder.atom_leaf(Leaf::Int {
            value,
            radix: Radix::Dec,
        })
    }
}

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

    fn serialize_char(self, v: char) -> Result<StructId> {
        Ok(self.builder.atom_leaf(Leaf::Char(v)))
    }

    fn serialize_str(self, v: &str) -> Result<StructId> {
        Ok(self.builder.atom_leaf(Leaf::Str(Arc::from(v))))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<StructId> {
        Ok(self.builder.atom_leaf(Leaf::Bytes(Arc::from(v))))
    }

    fn serialize_none(self) -> Result<StructId> {
        Ok(self.builder.list(Vec::new()))
    }

    fn serialize_some<T>(self, value: &T) -> Result<StructId>
    where
        T: ?Sized + Serialize,
    {
        let node = value.serialize(AstSerializer {
            builder: &mut *self.builder,
        })?;
        Ok(self.builder.list(vec![node]))
    }

    fn serialize_unit(self) -> Result<StructId> {
        Ok(self.builder.list(Vec::new()))
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<StructId> {
        Ok(self.builder.list(Vec::new()))
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<StructId> {
        let head = self.builder.name(variant);
        Ok(self.builder.list(vec![head]))
    }

    fn serialize_newtype_struct<T>(self, _name: &'static str, value: &T) -> Result<StructId>
    where
        T: ?Sized + Serialize,
    {
        // Transparent, per serde convention: a newtype struct is its inner value.
        value.serialize(self)
    }

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

    fn serialize_seq(self, _len: Option<usize>) -> Result<SeqSerializer<'a>> {
        Ok(SeqSerializer {
            builder: self.builder,
            ctor: CompoundCtor::List,
            elems: Vec::new(),
        })
    }

    fn serialize_tuple(self, _len: usize) -> Result<SeqSerializer<'a>> {
        Ok(SeqSerializer {
            builder: self.builder,
            ctor: CompoundCtor::Tuple,
            elems: Vec::new(),
        })
    }

    fn serialize_tuple_struct(self, _name: &'static str, _len: usize) -> Result<SeqSerializer<'a>> {
        Ok(SeqSerializer {
            builder: self.builder,
            ctor: CompoundCtor::Tuple,
            elems: Vec::new(),
        })
    }

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

/// A `map` `("map" (= k v)…)` — keys and values arrive separately, so we stash the key node until its
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

    fn end(self) -> Result<StructId> {
        Ok(self.builder.compound(CompoundCtor::Map, &self.entries))
    }
}

/// A `struct` `("record" (= "field" v)…)` — each field key is a `Str` leaf holding the field name.
pub struct StructSerializer<'a> {
    builder: &'a mut Builder,
    entries: Vec<StructId>,
}

impl<'a> ser::SerializeStruct for StructSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

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

    fn end(self) -> Result<StructId> {
        Ok(self.builder.compound(CompoundCtor::Record, &self.entries))
    }
}

/// A `struct_variant` `(VariantName (= "field" v)…)` — the head Name plus record-style field pairs.
pub struct StructVariantSerializer<'a> {
    builder: &'a mut Builder,
    head: StructId,
    entries: Vec<StructId>,
}

impl<'a> ser::SerializeStructVariant for StructVariantSerializer<'a> {
    type Ok = StructId;
    type Error = Error;

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
