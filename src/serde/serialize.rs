//! serde support (feature `serde`): serialize any `T: Serialize` to a KVD
//! node tree, and serialize [`Node`] itself.
//!
//! Emission stays canonical because the resulting tree goes through the
//! same emitter as the DOM API. Note that `HashMap` keys have unstable
//! order — use structs (field order), `BTreeMap`, or `IndexMap` when
//! deterministic output matters.

use crate::serde::error::{SerdeError, float_text};
use crate::value::{Map, Node, Shape};
use serde::ser::{self, SerializeMap as _, SerializeSeq as _};
use serde::{Serialize, Serializer};

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Node::Scalar(s) => match s.shape {
                Shape::Str => serializer.serialize_str(&s.text),
                Shape::Int => match s.text.replace('_', "").parse::<i64>() {
                    Ok(n) => serializer.serialize_i64(n),
                    Err(_) => match s.text.replace('_', "").parse::<u64>() {
                        Ok(n) => serializer.serialize_u64(n),
                        Err(_) => Err(ser::Error::custom(format!(
                            "integer literal out of range: {}",
                            s.text
                        ))),
                    },
                },
                Shape::Float => match s.text.replace('_', "").parse::<f64>() {
                    Ok(x) => serializer.serialize_f64(x),
                    Err(_) => Err(ser::Error::custom(format!(
                        "invalid float literal: {}",
                        s.text
                    ))),
                },
                Shape::Bool => serializer.serialize_bool(s.text == "true"),
                Shape::Null => serializer.serialize_none(),
            },
            Node::Map(m) => {
                let mut map = serializer.serialize_map(Some(m.len()))?;
                for (k, v) in m.iter() {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
            Node::List(l) => {
                let mut seq = serializer.serialize_seq(Some(l.len()))?;
                for item in l {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
        }
    }
}

/// serde serializer producing KVD [`Node`] trees.
#[derive(Debug, Clone, Copy, Default)]
pub struct NodeSerializer;

fn scalar(shape: Shape, text: String) -> Result<Node, SerdeError> {
    Ok(Node::scalar(shape, text))
}

impl Serializer for NodeSerializer {
    type Ok = Node;
    type Error = SerdeError;
    type SerializeSeq = SeqSer;
    type SerializeTuple = SeqSer;
    type SerializeTupleStruct = SeqSer;
    type SerializeTupleVariant = TupleVariantSer;
    type SerializeMap = MapSer;
    type SerializeStruct = MapSer;
    type SerializeStructVariant = StructVariantSer;

    fn serialize_bool(self, v: bool) -> Result<Node, SerdeError> {
        scalar(Shape::Bool, v.to_string())
    }

    fn serialize_i8(self, v: i8) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_i16(self, v: i16) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_i32(self, v: i32) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_i64(self, v: i64) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_u8(self, v: u8) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_u16(self, v: u16) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_u32(self, v: u32) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_u64(self, v: u64) -> Result<Node, SerdeError> {
        scalar(Shape::Int, v.to_string())
    }

    fn serialize_f32(self, v: f32) -> Result<Node, SerdeError> {
        self.serialize_f64(f64::from(v))
    }

    fn serialize_f64(self, v: f64) -> Result<Node, SerdeError> {
        float_text(v).map(|t| Node::scalar(Shape::Float, t))
    }

    fn serialize_char(self, v: char) -> Result<Node, SerdeError> {
        scalar(Shape::Str, v.to_string())
    }

    fn serialize_str(self, v: &str) -> Result<Node, SerdeError> {
        scalar(Shape::Str, v.to_string())
    }

    /// Bytes become a list of integer nodes; KVD has no byte-string type.
    fn serialize_bytes(self, v: &[u8]) -> Result<Node, SerdeError> {
        let items = v
            .iter()
            .map(|b| Node::scalar(Shape::Int, b.to_string()))
            .collect();
        Ok(Node::List(items))
    }

    fn serialize_none(self) -> Result<Node, SerdeError> {
        scalar(Shape::Null, "null".to_string())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Node, SerdeError> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Node, SerdeError> {
        self.serialize_none()
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Node, SerdeError> {
        self.serialize_none()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Node, SerdeError> {
        scalar(Shape::Str, variant.to_string())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Node, SerdeError> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Node, SerdeError> {
        let mut map = Map::new();
        map.insert(variant.to_string(), value.serialize(self)?);
        Ok(Node::Map(map))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<SeqSer, SerdeError> {
        Ok(SeqSer {
            items: Vec::with_capacity(len.unwrap_or(0)),
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<SeqSer, SerdeError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SeqSer, SerdeError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<TupleVariantSer, SerdeError> {
        Ok(TupleVariantSer {
            variant,
            items: Vec::with_capacity(len),
        })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<MapSer, SerdeError> {
        Ok(MapSer {
            entries: Map::new(),
            pending_key: None,
            capacity_hint: len,
        })
    }

    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<MapSer, SerdeError> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<StructVariantSer, SerdeError> {
        Ok(StructVariantSer {
            variant,
            entries: Map::new(),
        })
    }
}

/// Accumulates sequence elements.
pub struct SeqSer {
    items: Vec<Node>,
}

impl ser::SerializeSeq for SeqSer {
    type Ok = Node;
    type Error = SerdeError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), SerdeError> {
        self.items.push(value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Node, SerdeError> {
        Ok(Node::List(self.items))
    }
}

impl ser::SerializeTuple for SeqSer {
    type Ok = Node;
    type Error = SerdeError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), SerdeError> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Node, SerdeError> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SeqSer {
    type Ok = Node;
    type Error = SerdeError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), SerdeError> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Node, SerdeError> {
        ser::SerializeSeq::end(self)
    }
}

/// `{Variant: [a, b, ...]}` for externally tagged tuple variants.
pub struct TupleVariantSer {
    variant: &'static str,
    items: Vec<Node>,
}

impl ser::SerializeTupleVariant for TupleVariantSer {
    type Ok = Node;
    type Error = SerdeError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), SerdeError> {
        self.items.push(value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Node, SerdeError> {
        let mut map = Map::new();
        map.insert(self.variant.to_string(), Node::List(self.items));
        Ok(Node::Map(map))
    }
}

/// Accumulates map/struct entries; keys must serialize to strings.
pub struct MapSer {
    entries: Map,
    pending_key: Option<String>,
    #[allow(dead_code)]
    capacity_hint: Option<usize>,
}

impl ser::SerializeMap for MapSer {
    type Ok = Node;
    type Error = SerdeError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), SerdeError> {
        let node = key.serialize(NodeSerializer)?;
        self.pending_key = Some(match node {
            Node::Scalar(s) if s.shape == Shape::Str => s.text,
            other => {
                return Err(SerdeError::new(format!(
                    "map keys must be strings, got {other:?}"
                )));
            }
        });
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), SerdeError> {
        let key = self
            .pending_key
            .take()
            .expect("serialize_value called before serialize_key");
        self.entries.insert(key, value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn serialize_entry<K: Serialize + ?Sized, V: Serialize + ?Sized>(
        &mut self,
        key: &K,
        value: &V,
    ) -> Result<(), SerdeError> {
        self.serialize_key(key)?;
        self.serialize_value(value)
    }

    fn end(self) -> Result<Node, SerdeError> {
        Ok(Node::Map(self.entries))
    }
}

impl ser::SerializeStruct for MapSer {
    type Ok = Node;
    type Error = SerdeError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), SerdeError> {
        self.entries
            .insert(key.to_string(), value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Node, SerdeError> {
        Ok(Node::Map(self.entries))
    }
}

/// `{Variant: {field: value, ...}}` for externally tagged struct variants.
pub struct StructVariantSer {
    variant: &'static str,
    entries: Map,
}

impl ser::SerializeStructVariant for StructVariantSer {
    type Ok = Node;
    type Error = SerdeError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), SerdeError> {
        self.entries
            .insert(key.to_string(), value.serialize(NodeSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Node, SerdeError> {
        let mut map = Map::new();
        map.insert(self.variant.to_string(), Node::Map(self.entries));
        Ok(Node::Map(map))
    }
}
