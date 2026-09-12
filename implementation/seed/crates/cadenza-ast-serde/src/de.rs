//! The serde `Deserializer` that reads a decoded `cadenza_ast` value tree.
//!
//! Deserialization is TYPE-DIRECTED: the target type's `Deserialize` impl calls the `deserialize_*`
//! method for the shape it expects, and this reader extracts that shape from the AST node at its
//! current [`StructId`]. The mapping is the exact dual of [`crate::ser`] (see that module's header).
//! A borrowed `&'de Arenas` backs the reader, so strings/bytes are handed to visitors by reference.

use crate::error::{Error, Result};
use cadenza_ast::ast::{Arenas, CompoundCtor, IntValue, Leaf, Struct, StructId};
use serde::de::{
    self, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};

/// Fold a canonical (minimal, big-endian) magnitude into a `u128`; error if it is wider than 128 bits.
fn mag_to_u128(iv: &IntValue) -> Result<u128> {
    if iv.magnitude.len() > 16 {
        return Err(Error::IntOutOfRange);
    }
    let mut v: u128 = 0;
    for &b in &iv.magnitude {
        v = (v << 8) | b as u128;
    }
    Ok(v)
}

/// The unsigned value of `iv`; a negative non-zero integer is out of range for an unsigned target.
fn int_u128(iv: &IntValue) -> Result<u128> {
    if iv.negative && !iv.magnitude.is_empty() {
        return Err(Error::IntOutOfRange);
    }
    mag_to_u128(iv)
}

/// The signed value of `iv`, if it fits `i128`.
fn int_i128(iv: &IntValue) -> Result<i128> {
    let mag = mag_to_u128(iv)?;
    if iv.negative {
        // `-mag` must fit i128: mag <= 2^127. `(2^127) as i128` wraps to i128::MIN, whose
        // `wrapping_neg` is itself — exactly the value we want for mag == 2^127.
        if mag > (i128::MAX as u128) + 1 {
            return Err(Error::IntOutOfRange);
        }
        Ok((mag as i128).wrapping_neg())
    } else {
        if mag > i128::MAX as u128 {
            return Err(Error::IntOutOfRange);
        }
        Ok(mag as i128)
    }
}

/// The serde `Deserializer` over a borrowed [`Arenas`], positioned at one node.
#[derive(Clone, Copy)]
pub struct AstDeserializer<'de> {
    pub(crate) arenas: &'de Arenas,
    pub(crate) id: StructId,
}

impl<'de> AstDeserializer<'de> {
    fn at(&self, id: StructId) -> AstDeserializer<'de> {
        AstDeserializer {
            arenas: self.arenas,
            id,
        }
    }

    /// This node with any leading type-ascription `(: value type)` peeled off — the canonical value-form
    /// readers ignore the ascription token ("decode by structure, not names"; operator 2026-09-11), so we
    /// do too. Iterates to peel nested ascriptions. Every node-access below goes through `cur()`, so an
    /// ascribed value at ANY position decodes identically to a bare one — which is what lets this reader
    /// replace a hand-written, ascription-tolerant platform decoder without changing the producer's bytes.
    /// (This crate's own serializer never emits ascription, so `cur()` is a no-op on its own output.)
    fn cur(&self) -> StructId {
        let mut id = self.id;
        while let Some(&inner) = self.arenas.as_form(id, ":").and_then(|tail| tail.first()) {
            id = inner;
        }
        id
    }

    fn node(&self) -> &'de Struct {
        &self.arenas.structure[self.cur().0 as usize]
    }

    /// The leaf at `id` if it is an `Atom`, else `None`.
    fn leaf_at(&self, id: StructId) -> Option<&'de Leaf> {
        match &self.arenas.structure[id.0 as usize] {
            Struct::Atom(l) => Some(&self.arenas.leaves[l.0 as usize]),
            Struct::List(_) => None,
        }
    }

    fn atom_leaf(&self) -> Result<&'de Leaf> {
        self.leaf_at(self.cur())
            .ok_or_else(|| Error::UnexpectedShape("expected a leaf atom, found a list".into()))
    }

    fn as_list(&self) -> Result<&'de [StructId]> {
        match self.node() {
            Struct::List(items) => Ok(items),
            Struct::Atom(_) => Err(Error::UnexpectedShape(
                "expected a list, found a leaf atom".into(),
            )),
        }
    }

    fn read_int(&self) -> Result<&'de IntValue> {
        match self.atom_leaf()? {
            Leaf::Int { value, .. } => Ok(value),
            other => Err(Error::UnexpectedShape(format!(
                "expected an integer atom, found {other:?}"
            ))),
        }
    }

    /// The children of a compound `(<ctor> child…)` after its head. Recognizes BOTH the native ctor-LEAF
    /// head this crate's serializer emits AND the shadowable NAME-alias head the platform value-form uses
    /// (`("record" …)` etc.), via [`Arenas::compound_form_of`] — so a platform-produced value decodes
    /// without changing its bytes. Reads through `cur()`, so an ascribed compound is accepted too.
    fn compound_children(&self, expect: CompoundCtor) -> Result<&'de [StructId]> {
        self.arenas
            .compound_form_of(self.cur(), expect)
            .ok_or_else(|| Error::UnexpectedShape(format!("expected a {expect:?} compound")))
    }

    /// The `f64` a float leaf denotes (finite decimal, or a non-finite marker).
    fn read_f64(&self) -> Result<f64> {
        match self.atom_leaf()? {
            Leaf::Float(d) => Ok(f64::from_bits(d.to_f64_bits())),
            Leaf::FloatNan => Ok(f64::NAN),
            Leaf::FloatInf { negative } => Ok(if *negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }),
            other => Err(Error::UnexpectedShape(format!(
                "expected a float atom, found {other:?}"
            ))),
        }
    }
}

impl<'de> de::Deserializer<'de> for AstDeserializer<'de> {
    type Error = Error;

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.atom_leaf()? {
            Leaf::Bool(b) => visitor.visit_bool(*b),
            other => Err(Error::UnexpectedShape(format!(
                "expected a bool atom, found {other:?}"
            ))),
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(int_i128(self.read_int()?)? as i64)
    }
    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(int_i128(self.read_int()?)? as i64)
    }
    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(int_i128(self.read_int()?)? as i64)
    }
    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let v = int_i128(self.read_int()?)?;
        if v < i64::MIN as i128 || v > i64::MAX as i128 {
            return Err(Error::IntOutOfRange);
        }
        visitor.visit_i64(v as i64)
    }
    fn deserialize_i128<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i128(int_i128(self.read_int()?)?)
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u64(int_u128(self.read_int()?)? as u64)
    }
    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u64(int_u128(self.read_int()?)? as u64)
    }
    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u64(int_u128(self.read_int()?)? as u64)
    }
    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let v = int_u128(self.read_int()?)?;
        if v > u64::MAX as u128 {
            return Err(Error::IntOutOfRange);
        }
        visitor.visit_u64(v as u64)
    }
    fn deserialize_u128<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u128(int_u128(self.read_int()?)?)
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_f32(self.read_f64()? as f32)
    }
    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_f64(self.read_f64()?)
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.atom_leaf()? {
            Leaf::Char(c) => visitor.visit_char(*c),
            Leaf::BadChar(_) => Err(Error::BadChar),
            other => Err(Error::UnexpectedShape(format!(
                "expected a char atom, found {other:?}"
            ))),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.atom_leaf()? {
            Leaf::Str(s) => visitor.visit_borrowed_str(s),
            other => Err(Error::UnexpectedShape(format!(
                "expected a string atom, found {other:?}"
            ))),
        }
    }
    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.atom_leaf()? {
            Leaf::Bytes(b) => visitor.visit_borrowed_bytes(b),
            other => Err(Error::UnexpectedShape(format!(
                "expected a bytes atom, found {other:?}"
            ))),
        }
    }
    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        // Two option encodings are accepted:
        //  - this crate's own: `None` → `()` (empty list), `Some(v)` → `(v)` (one-element list);
        //  - the platform/canonical convention: an optional record FIELD is simply PRESENT with its
        //    BARE value (⇒ `Some`) or ABSENT (⇒ `None`, already handled by `#[serde(default)]` before we
        //    are even called). So a present value that is NOT the `()`/`(v)` form IS the `Some` payload.
        // Disambiguation is unambiguous for every value this crate emits (its `Some` is always a
        // one-element list) and for platform values (a `Some` payload is a bare atom or a ctor-headed
        // compound, i.e. ≥2 list items) — the only theoretical clash is a payload that is itself a
        // 1-element list, which neither producer emits as an option value.
        match self.node() {
            Struct::List(items) if items.is_empty() => visitor.visit_none(),
            Struct::List(items) if items.len() == 1 => {
                let inner = items[0];
                visitor.visit_some(self.at(inner))
            }
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let items = self.as_list()?;
        if items.is_empty() {
            visitor.visit_unit()
        } else {
            Err(Error::UnexpectedShape(
                "expected unit (the empty list)".into(),
            ))
        }
    }
    fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        // Transparent — the serializer wrote the inner value directly at this node.
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let items = self.compound_children(CompoundCtor::List)?;
        visitor.visit_seq(SeqReader {
            arenas: self.arenas,
            items,
            idx: 0,
        })
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let items = self.compound_children(CompoundCtor::Tuple)?;
        visitor.visit_seq(SeqReader {
            arenas: self.arenas,
            items,
            idx: 0,
        })
    }
    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let entries = self.compound_children(CompoundCtor::Map)?;
        visitor.visit_map(MapReader {
            arenas: self.arenas,
            entries,
            idx: 0,
            value: None,
        })
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let entries = self.compound_children(CompoundCtor::Record)?;
        visitor.visit_map(MapReader {
            arenas: self.arenas,
            entries,
            idx: 0,
            value: None,
        })
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        let items = self.as_list()?;
        let (&head, payload) = items
            .split_first()
            .ok_or_else(|| Error::UnexpectedShape("expected an enum (non-empty list)".into()))?;
        visitor.visit_enum(EnumReader {
            arenas: self.arenas,
            name_id: head,
            payload,
        })
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        // Field-name keys are `Str`; enum variant tags are `Name`. Both surface as a borrowed str.
        match self.atom_leaf()? {
            Leaf::Str(s) => visitor.visit_borrowed_str(s),
            Leaf::Name(n) => visitor.visit_borrowed_str(n),
            other => Err(Error::UnexpectedShape(format!(
                "expected an identifier (Str/Name atom), found {other:?}"
            ))),
        }
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        self.deserialize_any(visitor)
    }

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        // Best-effort self-describing read — used by generic/`IgnoredAny` consumers. Typed derives
        // never route through here (they call the specific `deserialize_*`).
        match self.node() {
            Struct::Atom(l) => match &self.arenas.leaves[l.0 as usize] {
                Leaf::Bool(b) => visitor.visit_bool(*b),
                Leaf::Int { value, .. } => {
                    if value.negative {
                        visitor.visit_i128(int_i128(value)?)
                    } else {
                        visitor.visit_u128(int_u128(value)?)
                    }
                }
                Leaf::Float(_) | Leaf::FloatNan | Leaf::FloatInf { .. } => {
                    visitor.visit_f64(self.read_f64()?)
                }
                Leaf::Char(c) => visitor.visit_char(*c),
                Leaf::Str(s) => visitor.visit_borrowed_str(s),
                Leaf::Sym(s) => visitor.visit_borrowed_str(s),
                Leaf::Name(n) => visitor.visit_borrowed_str(n),
                Leaf::Bytes(b) => visitor.visit_borrowed_bytes(b),
                other => Err(Error::UnexpectedShape(format!(
                    "deserialize_any: unsupported leaf {other:?}"
                ))),
            },
            Struct::List(items) => {
                let ctor = items.first().and_then(|&h| self.leaf_at(h));
                match ctor {
                    Some(Leaf::Ctor(CompoundCtor::Map)) => self.deserialize_map(visitor),
                    Some(Leaf::Ctor(CompoundCtor::Record)) => visitor.visit_map(MapReader {
                        arenas: self.arenas,
                        entries: &items[1..],
                        idx: 0,
                        value: None,
                    }),
                    Some(Leaf::Ctor(_)) => visitor.visit_seq(SeqReader {
                        arenas: self.arenas,
                        items: &items[1..],
                        idx: 0,
                    }),
                    // A bare list: unit `()` or an opaque sequence of children.
                    _ if items.is_empty() => visitor.visit_unit(),
                    _ => visitor.visit_seq(SeqReader {
                        arenas: self.arenas,
                        items,
                        idx: 0,
                    }),
                }
            }
        }
    }
}

/// Reads a compound's children as a serde `seq` / `tuple`.
struct SeqReader<'de> {
    arenas: &'de Arenas,
    items: &'de [StructId],
    idx: usize,
}

impl<'de> SeqAccess<'de> for SeqReader<'de> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: DeserializeSeed<'de>,
    {
        if self.idx >= self.items.len() {
            return Ok(None);
        }
        let d = AstDeserializer {
            arenas: self.arenas,
            id: self.items[self.idx],
        };
        self.idx += 1;
        seed.deserialize(d).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len() - self.idx)
    }
}

/// Reads `(= k v)` field-pair entries as a serde `map` / `struct`.
struct MapReader<'de> {
    arenas: &'de Arenas,
    entries: &'de [StructId],
    idx: usize,
    value: Option<StructId>,
}

impl<'de> MapReader<'de> {
    /// The `(key, value)` node ids of the field-pair entry at `id` — `(= k v)` is a 3-element list.
    fn pair(&self, id: StructId) -> Result<(StructId, StructId)> {
        match &self.arenas.structure[id.0 as usize] {
            Struct::List(items) if items.len() == 3 => Ok((items[1], items[2])),
            _ => Err(Error::UnexpectedShape(
                "expected a (= key value) field-pair entry".into(),
            )),
        }
    }
}

impl<'de> MapAccess<'de> for MapReader<'de> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: DeserializeSeed<'de>,
    {
        if self.idx >= self.entries.len() {
            return Ok(None);
        }
        let (k, v) = self.pair(self.entries[self.idx])?;
        self.value = Some(v);
        let d = AstDeserializer {
            arenas: self.arenas,
            id: k,
        };
        seed.deserialize(d).map(Some)
    }

    fn next_value_seed<Vv>(&mut self, seed: Vv) -> Result<Vv::Value>
    where
        Vv: DeserializeSeed<'de>,
    {
        let v = self
            .value
            .take()
            .ok_or_else(|| Error::Message("map value requested before its key".into()))?;
        self.idx += 1;
        let d = AstDeserializer {
            arenas: self.arenas,
            id: v,
        };
        seed.deserialize(d)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len() - self.idx)
    }
}

/// Reads an enum `(VariantName payload…)`.
struct EnumReader<'de> {
    arenas: &'de Arenas,
    name_id: StructId,
    payload: &'de [StructId],
}

impl<'de> EnumAccess<'de> for EnumReader<'de> {
    type Error = Error;
    type Variant = VariantReader<'de>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant)>
    where
        V: DeserializeSeed<'de>,
    {
        let d = AstDeserializer {
            arenas: self.arenas,
            id: self.name_id,
        };
        let variant = seed.deserialize(d)?;
        Ok((
            variant,
            VariantReader {
                arenas: self.arenas,
                payload: self.payload,
            },
        ))
    }
}

struct VariantReader<'de> {
    arenas: &'de Arenas,
    payload: &'de [StructId],
}

impl<'de> VariantAccess<'de> for VariantReader<'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        if self.payload.is_empty() {
            Ok(())
        } else {
            Err(Error::UnexpectedShape(
                "expected a unit variant (no payload)".into(),
            ))
        }
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value>
    where
        T: DeserializeSeed<'de>,
    {
        match self.payload {
            [only] => seed.deserialize(AstDeserializer {
                arenas: self.arenas,
                id: *only,
            }),
            _ => Err(Error::UnexpectedShape(
                "expected a newtype variant (one payload node)".into(),
            )),
        }
    }

    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(SeqReader {
            arenas: self.arenas,
            items: self.payload,
            idx: 0,
        })
    }

    fn struct_variant<V>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(MapReader {
            arenas: self.arenas,
            entries: self.payload,
            idx: 0,
            value: None,
        })
    }
}

/// Allow `AstDeserializer` to be used where an `IntoDeserializer` is expected (uniformity; not
/// currently required by the driver, but cheap and idiomatic).
impl<'de> IntoDeserializer<'de, Error> for AstDeserializer<'de> {
    type Deserializer = Self;
    fn into_deserializer(self) -> Self {
        self
    }
}
