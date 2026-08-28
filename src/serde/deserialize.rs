//! serde support (feature `serde`): deserialize any `T: Deserialize` from
//! KVD nodes, and deserialize [`Node`] itself.
//!
//! Shapes are enforced strictly: an integer target accepts only `Shape::Int`
//! text, a string target only `Shape::Str`, and so on — there is no
//! YAML-style coercion. Underscore separators (`1_000`) are accepted
//! wherever the grammar allows them.

use crate::serde::error::{float_text, SerdeError};
use crate::value::{Map, Node, Scalar, Shape};
use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use serde::Deserialize;
use std::fmt;
use std::str::FromStr;

fn describe(node: &Node) -> de::Unexpected<'_> {
    match node {
        Node::Scalar(s) => match s.shape {
            Shape::Int => de::Unexpected::Other("int"),
            Shape::Float => de::Unexpected::Other("float"),
            Shape::Bool => de::Unexpected::Other("bool"),
            Shape::Str => de::Unexpected::Str(&s.text),
            Shape::Null => de::Unexpected::Unit,
        },
        Node::Map(_) => de::Unexpected::Map,
        Node::List(_) => de::Unexpected::Seq,
    }
}

/// Parses `s` as a number, tolerating `_` digit separators (spec §3).
fn parse_number<T: FromStr>(s: &str) -> Option<T> {
    let compact: String = s.chars().filter(|c| *c != '_').collect();
    compact.parse::<T>().ok()
}

/// Visits a scalar according to its shape (the `deserialize_any` path).
fn visit_by_shape<'de, V: Visitor<'de>>(
    s: &'de Scalar,
    visitor: V,
) -> Result<V::Value, SerdeError> {
    match s.shape {
        Shape::Str => visitor.visit_borrowed_str(&s.text),
        Shape::Int => match parse_number::<i64>(&s.text) {
            Some(n) => visitor.visit_i64(n),
            None => match parse_number::<u64>(&s.text) {
                Some(n) => visitor.visit_u64(n),
                None => Err(de::Error::invalid_value(
                    de::Unexpected::Str(&s.text),
                    &"an integer that fits in 64 bits",
                )),
            },
        },
        Shape::Float => match parse_number::<f64>(&s.text) {
            Some(x) => visitor.visit_f64(x),
            None => Err(de::Error::invalid_value(
                de::Unexpected::Str(&s.text),
                &"a float",
            )),
        },
        Shape::Bool => match s.text.as_str() {
            "true" => visitor.visit_bool(true),
            "false" => visitor.visit_bool(false),
            _ => Err(de::Error::invalid_value(
                de::Unexpected::Str(&s.text),
                &"`true` or `false`",
            )),
        },
        Shape::Null => visitor.visit_unit(),
    }
}

/// serde deserializer borrowing a KVD node tree.
pub struct NodeDe<'de> {
    node: &'de Node,
}

impl<'de> NodeDe<'de> {
    /// Wraps a borrowed node.
    pub fn new(node: &'de Node) -> Self {
        NodeDe { node }
    }

    fn expect_scalar(&self, expected: &'static str) -> Result<&'de Scalar, SerdeError> {
        self.node
            .as_scalar()
            .ok_or_else(|| de::Error::invalid_type(describe(self.node), &expected))
    }
}

impl<'de> Deserializer<'de> for NodeDe<'de> {
    type Error = SerdeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        match self.node {
            Node::Scalar(s) => visit_by_shape(s, visitor),
            Node::Map(m) => visitor.visit_map(MapDe::new(m)),
            Node::List(l) => visitor.visit_seq(SeqDe::new(l)),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        let s = self.expect_scalar("a bool")?;
        match (s.shape, s.text.as_str()) {
            (Shape::Bool, "true") => visitor.visit_bool(true),
            (Shape::Bool, "false") => visitor.visit_bool(false),
            _ => Err(de::Error::invalid_type(describe(self.node), &"a bool")),
        }
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.signed_int(visitor)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.signed_int(visitor)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.signed_int(visitor)
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.signed_int(visitor)
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.unsigned_int(visitor)
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.unsigned_int(visitor)
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.unsigned_int(visitor)
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.unsigned_int(visitor)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        let s = self.expect_scalar("a float")?;
        match s.shape {
            Shape::Float => match parse_number::<f64>(&s.text) {
                Some(x) => visitor.visit_f64(x),
                None => Err(de::Error::invalid_value(
                    de::Unexpected::Str(&s.text),
                    &"a finite float",
                )),
            },
            _ => Err(de::Error::invalid_type(describe(self.node), &"a float")),
        }
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.deserialize_f64(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        let s = self.expect_scalar("a string")?;
        match s.shape {
            Shape::Str => visitor.visit_borrowed_str(&s.text),
            _ => Err(de::Error::invalid_type(describe(self.node), &"a string")),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        match self.node {
            Node::Scalar(s) if s.shape == Shape::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        match self.node {
            Node::Scalar(s) if s.shape == Shape::Null => visitor.visit_unit(),
            _ => Err(de::Error::invalid_type(describe(self.node), &"null")),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        match self.node {
            Node::List(l) => visitor.visit_seq(SeqDe::new(l)),
            _ => Err(de::Error::invalid_type(describe(self.node), &"a list")),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        match self.node {
            Node::Map(m) => visitor.visit_map(MapDe::new(m)),
            _ => Err(de::Error::invalid_type(describe(self.node), &"a map")),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        match self.node {
            // Bare string: unit variant (`status: active`).
            Node::Scalar(s) if s.shape == Shape::Str => {
                visitor.visit_enum(UnitVariantDe { name: &s.text })
            }
            // Single-entry map: externally tagged variant with payload.
            Node::Map(m) => {
                let mut iter = m.iter();
                let Some((key, value)) = iter.next() else {
                    return Err(de::Error::custom(
                        "cannot deserialize an enum from an empty map",
                    ));
                };
                if iter.next().is_some() {
                    return Err(de::Error::custom(
                        "externally tagged enums require exactly one map entry",
                    ));
                }
                visitor.visit_enum(PayloadVariantDe { name: key, value })
            }
            _ => Err(de::Error::invalid_type(
                describe(self.node),
                &"an enum (bare string or single-entry map)",
            )),
        }
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        let s = self.expect_scalar("a single character")?;
        match s.shape {
            Shape::Str if s.text.chars().count() == 1 => {
                visitor.visit_char(s.text.chars().next().unwrap())
            }
            _ => Err(de::Error::invalid_type(
                describe(self.node),
                &"a single character",
            )),
        }
    }

    /// Bytes arrive as a list of integer nodes (KVD has no byte-string
    /// type); the byte visitors accept sequences.
    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        visitor.visit_unit()
    }
}

impl<'de> NodeDe<'de> {
    fn signed_int<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        let s = self.expect_scalar("an integer")?;
        match s.shape {
            Shape::Int => match parse_number::<i64>(&s.text) {
                Some(n) => visitor.visit_i64(n),
                None => Err(de::Error::invalid_value(
                    de::Unexpected::Str(&s.text),
                    &"a signed integer that fits in 64 bits",
                )),
            },
            _ => Err(de::Error::invalid_type(describe(self.node), &"an integer")),
        }
    }

    fn unsigned_int<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        let s = self.expect_scalar("an integer")?;
        match s.shape {
            Shape::Int => match parse_number::<u64>(&s.text) {
                Some(n) => visitor.visit_u64(n),
                None => Err(de::Error::invalid_value(
                    de::Unexpected::Str(&s.text),
                    &"an unsigned integer that fits in 64 bits",
                )),
            },
            _ => Err(de::Error::invalid_type(describe(self.node), &"an integer")),
        }
    }
}

/// Deserializer over a bare `&str` (variant names, map keys).
struct StrDe<'de>(&'de str);

impl<'de> Deserializer<'de> for StrDe<'de> {
    type Error = SerdeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        visitor.visit_borrowed_str(self.0)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        visitor.visit_borrowed_str(self.0)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        visitor.visit_borrowed_str(self.0)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, SerdeError> {
        visitor.visit_borrowed_str(self.0)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, SerdeError> {
        Err(de::Error::custom("nested enums are not supported"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char
        option unit unit_struct newtype_struct seq tuple tuple_struct
        map struct bytes byte_buf ignored_any
    }
}

struct SeqDe<'de> {
    iter: std::slice::Iter<'de, Node>,
}

impl<'de> SeqDe<'de> {
    fn new(list: &'de [Node]) -> Self {
        SeqDe { iter: list.iter() }
    }
}

impl<'de> SeqAccess<'de> for SeqDe<'de> {
    type Error = SerdeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, SerdeError> {
        self.iter
            .next()
            .map(|node| seed.deserialize(NodeDe::new(node)))
            .transpose()
    }
}

struct MapDe<'de> {
    entries: &'de [(String, Node)],
    index: usize,
    pending: Option<&'de Node>,
}

impl<'de> MapDe<'de> {
    fn new(map: &'de Map) -> Self {
        MapDe {
            entries: map.entries(),
            index: 0,
            pending: None,
        }
    }
}

impl<'de> MapAccess<'de> for MapDe<'de> {
    type Error = SerdeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, SerdeError> {
        if self.index >= self.entries.len() {
            return Ok(None);
        }
        let (key, value) = &self.entries[self.index];
        self.index += 1;
        self.pending = Some(value);
        seed.deserialize(StrDe(key)).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, SerdeError> {
        let value = self
            .pending
            .take()
            .expect("next_value_seed called before next_key_seed");
        seed.deserialize(NodeDe::new(value))
    }
}

/// EnumAccess for a bare-string unit variant.
struct UnitVariantDe<'de> {
    name: &'de str,
}

impl<'de> EnumAccess<'de> for UnitVariantDe<'de> {
    type Error = SerdeError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self), SerdeError> {
        let name = self.name;
        Ok((seed.deserialize(StrDe(name))?, self))
    }
}

impl<'de> VariantAccess<'de> for UnitVariantDe<'de> {
    type Error = SerdeError;

    fn unit_variant(self) -> Result<(), SerdeError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        _seed: T,
    ) -> Result<T::Value, SerdeError> {
        Err(de::Error::invalid_type(
            de::Unexpected::UnitVariant,
            &"a unit variant (write the payload form instead)",
        ))
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, SerdeError> {
        Err(de::Error::invalid_type(
            de::Unexpected::UnitVariant,
            &"a unit variant",
        ))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, SerdeError> {
        Err(de::Error::invalid_type(
            de::Unexpected::UnitVariant,
            &"a unit variant",
        ))
    }
}

/// EnumAccess for a single-entry map `{Variant: payload}`.
struct PayloadVariantDe<'de> {
    name: &'de str,
    value: &'de Node,
}

impl<'de> EnumAccess<'de> for PayloadVariantDe<'de> {
    type Error = SerdeError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self), SerdeError> {
        let name = self.name;
        Ok((seed.deserialize(StrDe(name))?, self))
    }
}

impl<'de> VariantAccess<'de> for PayloadVariantDe<'de> {
    type Error = SerdeError;

    fn unit_variant(self) -> Result<(), SerdeError> {
        match self.value {
            Node::Scalar(s) if s.shape == Shape::Null => Ok(()),
            _ => Err(de::Error::invalid_type(
                describe(self.value),
                &"null (this variant carries no data)",
            )),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, SerdeError> {
        seed.deserialize(NodeDe::new(self.value))
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        NodeDe::new(self.value).deserialize_seq(visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, SerdeError> {
        NodeDe::new(self.value).deserialize_map(visitor)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

impl<'de> IntoDeserializer<'de, SerdeError> for &'de Node {
    type Deserializer = NodeDe<'de>;

    fn into_deserializer(self) -> Self::Deserializer {
        NodeDe::new(self)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any KVD value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Bool, v.to_string()))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Int, v.to_string()))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Int, v.to_string()))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Node, E> {
        float_text(v)
            .map_err(E::custom)
            .map(|t| Node::scalar(Shape::Float, t))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Str, v))
    }

    fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Str, v))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Str, v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Null, "null"))
    }

    fn visit_none<E: de::Error>(self) -> Result<Node, E> {
        Ok(Node::scalar(Shape::Null, "null"))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        Node::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Node>()? {
            items.push(item);
        }
        Ok(Node::List(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let mut out = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let value = map.next_value::<Node>()?;
            out.insert(key, value);
        }
        Ok(Node::Map(out))
    }
}
