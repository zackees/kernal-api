//! Bounded JSON mechanics. Applications own schemas, defaults and field policy.

use std::collections::BTreeMap;
use std::io::Write;

/// Source byte limit, enforced before parsing.
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
/// Encoded byte limit, including escaping and whitespace.
pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum decoded values, counting containers and the root (not object keys).
pub const MAX_NODES: usize = 262_144;
/// Maximum value depth; the root has depth zero.
pub const MAX_DEPTH: usize = 64;

/// Owned JSON values, independent of the private serialization backend.
/// Objects encode in key order. Positive parsed integers use `Signed` when
/// representable, otherwise `Unsigned`; numeric variant identity is not a
/// wire-format guarantee. Floating-point values must be finite when encoded.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

/// Output style; neither style appends a trailing newline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layout {
    Compact,
    /// Two-space indentation.
    Pretty,
}

/// Bounded diagnostics that do not echo document contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("JSON source exceeds byte limit")]
    InputTooLarge,
    #[error("invalid JSON document")]
    InvalidSyntax,
    #[error("JSON exceeds value count limit")]
    TooManyNodes,
    #[error("JSON exceeds depth limit")]
    TooDeep,
    #[error("JSON numbers must be finite")]
    NonFiniteNumber,
    #[error("JSON output exceeds byte limit")]
    OutputTooLarge,
    #[error("JSON encoding failed")]
    EncodingFailed,
}

/// Parse one UTF-8 JSON value; trailing whitespace is allowed, trailing values
/// are not. Duplicate object keys retain their last value.
///
/// The input byte limit is checked before parsing. Value count and depth limits
/// apply to the decoded tree after private parsing, not as independent parser
/// allocation or CPU quotas. The parser also retains its own recursion limit.
/// No partial result is returned. Integers outside i64/u64 may decode as finite
/// f64 values with rounding; this is not an arbitrary-precision number API.
pub fn parse(source: &[u8]) -> Result<Value, Error> {
    if source.len() > MAX_INPUT_BYTES {
        return Err(Error::InputTooLarge);
    }
    let value = serde_json::from_slice(source).map_err(|_| Error::InvalidSyntax)?;
    let mut remaining = MAX_NODES;
    convert(value, 0, &mut remaining)
}

fn visit(depth: usize, remaining: &mut usize) -> Result<(), Error> {
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    *remaining = remaining.checked_sub(1).ok_or(Error::TooManyNodes)?;
    Ok(())
}

fn convert(value: serde_json::Value, depth: usize, remaining: &mut usize) -> Result<Value, Error> {
    visit(depth, remaining)?;
    Ok(match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(value) => Value::Bool(value),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Value::Signed(value)
            } else if let Some(value) = value.as_u64() {
                Value::Unsigned(value)
            } else {
                Value::Float(value.as_f64().ok_or(Error::InvalidSyntax)?)
            }
        }
        serde_json::Value::String(value) => Value::String(value),
        serde_json::Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| convert(value, depth + 1, remaining))
                .collect::<Result<_, _>>()?,
        ),
        serde_json::Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| Ok((key, convert(value, depth + 1, remaining)?)))
                .collect::<Result<_, Error>>()?,
        ),
    })
}

fn validate(value: &Value, depth: usize, remaining: &mut usize) -> Result<(), Error> {
    visit(depth, remaining)?;
    match value {
        Value::Float(value) if !value.is_finite() => return Err(Error::NonFiniteNumber),
        Value::Array(values) => {
            for value in values {
                validate(value, depth + 1, remaining)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate(value, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// Only this private borrowed adapter implements the backend trait. Encoding
// does not clone caller strings or construct a second tree.
struct Borrowed<'a>(&'a Value);

impl serde::Serialize for Borrowed<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self.0 {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::Signed(value) => serializer.serialize_i64(*value),
            Value::Unsigned(value) => serializer.serialize_u64(*value),
            Value::Float(value) => serializer.serialize_f64(*value),
            Value::String(value) => serializer.serialize_str(value),
            Value::Array(values) => {
                let mut output = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    output.serialize_element(&Borrowed(value))?;
                }
                output.end()
            }
            Value::Object(values) => {
                let mut output = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    output.serialize_entry(key, &Borrowed(value))?;
                }
                output.end()
            }
        }
    }
}

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    exceeded: bool,
}

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_OUTPUT_BYTES - self.bytes.len() {
            self.exceeded = true;
            return Err(std::io::Error::other("JSON output limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Encode a caller-owned value, checking depth/count/finite numbers first and
/// enforcing the output byte limit during serialization. No tree/string clone
/// or partial output is returned. These bounds do not limit caller allocations
/// made while constructing a value before this call.
pub fn encode(value: &Value, layout: Layout) -> Result<Vec<u8>, Error> {
    let mut remaining = MAX_NODES;
    validate(value, 0, &mut remaining)?;
    let mut output = Output::default();
    let result = match layout {
        Layout::Compact => serde_json::to_writer(&mut output, &Borrowed(value)),
        Layout::Pretty => serde_json::to_writer_pretty(&mut output, &Borrowed(value)),
    };
    if output.exceeded {
        return Err(Error::OutputTooLarge);
    }
    result.map_err(|_| Error::EncodingFailed)?;
    Ok(output.bytes)
}
