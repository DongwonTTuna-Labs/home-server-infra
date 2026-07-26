use std::collections::BTreeMap;
use std::fmt;

use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde::Serialize;
use serde_json::Value;

pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    reject_non_integer_numbers(&value)?;
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn parse_strict_value(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    if bytes.first() == Some(&0xef) || bytes.contains(&0) {
        return Err(<serde_json::Error as serde::de::Error>::custom(
            "BOM/NUL is forbidden",
        ));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictSeed.deserialize(&mut deserializer)?;
    deserializer.end()?;
    reject_non_integer_numbers(&value)?;
    Ok(value)
}

pub fn parse_canonical(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let value = parse_strict_value(bytes)?;
    if canonical_bytes(&value)? != bytes {
        return Err(<serde_json::Error as serde::de::Error>::custom(
            "JSON is not canonical",
        ));
    }
    Ok(value)
}

fn reject_non_integer_numbers(value: &Value) -> Result<(), serde_json::Error> {
    match value {
        Value::Number(number) if !(number.is_i64() || number.is_u64()) => Err(
            <serde_json::Error as serde::de::Error>::custom("non-integer number is forbidden"),
        ),
        Value::Array(values) => values.iter().try_for_each(reject_non_integer_numbers),
        Value::Object(values) => values.values().try_for_each(reject_non_integer_numbers),
        _ => Ok(()),
    }
}

struct StrictSeed;

impl<'de> DeserializeSeed<'de> for StrictSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("strict JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E: serde::de::Error>(self, _value: f64) -> Result<Self::Value, E> {
        Err(E::custom("non-integer number is forbidden"))
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        if value.contains('\0') {
            return Err(E::custom("NUL is forbidden"));
        }
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
        if value.contains('\0') {
            return Err(E::custom("NUL is forbidden"));
        }
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictSeed.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if key.contains('\0') {
                return Err(A::Error::custom("NUL is forbidden"));
            }
            if values.contains_key(&key) {
                return Err(A::Error::custom(format!("duplicate key: {key}")));
            }
            values.insert(key, map.next_value_seed(StrictSeed)?);
        }
        Ok(Value::Object(values.into_iter().collect()))
    }
}
