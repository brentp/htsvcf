use std::collections::HashMap;
use v8;

/// Helper to truncate a string for error messages (max 50 chars).
pub(crate) fn truncate_for_error(s: &str) -> String {
    if s.len() <= 50 {
        s.to_string()
    } else {
        format!("{}...", &s[..47])
    }
}

/// Trait for types that can be extracted from a JavaScript value.
///
/// This trait is public so users can implement it for custom types.
///
/// # Built-in Implementations
///
/// - `String` - converts any JS value to string
/// - `bool` - uses JavaScript truthiness rules
/// - `i32`, `i64` - extracts integers (errors on non-numeric values)
/// - `f32`, `f64` - extracts floating point numbers
/// - `Vec<T>` - extracts arrays where each element is converted to `T`
/// - `Option<T>` - returns `None` for `null`/`undefined`, otherwise `Some(T)`
/// - `serde_json::Value` - converts to JSON-compatible values (null, bool, number, string, array, object)
/// - `HashMap<String, serde_json::Value>` - extracts JS objects as string-keyed maps
///
/// # Example
///
/// ```no_run
/// use htsvcf::{Evaluator, FromJsValue};
/// use rust_htslib::bcf::{self, Read};
///
/// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
/// let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP')").unwrap();
///
/// for result in reader.records() {
///     let record = result.unwrap();
///     let dp: i32 = js_eval.eval(record).unwrap();
///     println!("DP = {}", dp);
/// }
/// ```
pub trait FromJsValue: Sized {
    /// Convert a V8 value to this type.
    ///
    /// Returns an error message if the conversion fails.
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String>;
}

impl FromJsValue for String {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        value
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .ok_or_else(|| "failed to convert to string".into())
    }
}

impl FromJsValue for bool {
    fn from_js_value(
        _scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        Ok(value.boolean_value(_scope))
    }
}

impl FromJsValue for i32 {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        // Check if value is actually a number to avoid V8's silent coercion
        if !value.is_number() {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            return Err(format!("expected i32, got '{}'", truncate_for_error(&repr)));
        }
        value.int32_value(scope).ok_or_else(|| {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            format!("expected i32, got '{}'", truncate_for_error(&repr))
        })
    }
}

impl FromJsValue for i64 {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        // Check if value is actually a number to avoid V8's silent coercion
        if !value.is_number() {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            return Err(format!("expected i64, got '{}'", truncate_for_error(&repr)));
        }
        value.integer_value(scope).ok_or_else(|| {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            format!("expected i64, got '{}'", truncate_for_error(&repr))
        })
    }
}

impl FromJsValue for f32 {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        // Check if value is actually a number to avoid V8's silent coercion
        if !value.is_number() {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            return Err(format!("expected f32, got '{}'", truncate_for_error(&repr)));
        }
        value.number_value(scope).map(|n| n as f32).ok_or_else(|| {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            format!("expected f32, got '{}'", truncate_for_error(&repr))
        })
    }
}

impl FromJsValue for f64 {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        // Check if value is actually a number to avoid V8's silent coercion
        if !value.is_number() {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            return Err(format!("expected f64, got '{}'", truncate_for_error(&repr)));
        }
        value.number_value(scope).ok_or_else(|| {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            format!("expected f64, got '{}'", truncate_for_error(&repr))
        })
    }
}

impl<T: FromJsValue> FromJsValue for Vec<T> {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        let arr = v8::Local::<v8::Array>::try_from(value).map_err(|_| {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            format!("expected array, got '{}'", truncate_for_error(&repr))
        })?;

        let mut result = Vec::with_capacity(arr.length() as usize);
        for i in 0..arr.length() {
            let elem = arr
                .get_index(scope, i)
                .ok_or_else(|| format!("failed to get array element at index {}", i))?;
            result.push(T::from_js_value(scope, elem)?);
        }
        Ok(result)
    }
}

impl<T: FromJsValue> FromJsValue for Option<T> {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        if value.is_null_or_undefined() {
            Ok(None)
        } else {
            Ok(Some(T::from_js_value(scope, value)?))
        }
    }
}

impl FromJsValue for serde_json::Value {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        if value.is_null_or_undefined() {
            Ok(serde_json::Value::Null)
        } else if value.is_boolean() {
            Ok(serde_json::Value::Bool(value.boolean_value(scope)))
        } else if value.is_number() {
            let n = value
                .number_value(scope)
                .ok_or_else(|| "failed to get number value".to_string())?;
            if let Some(i) = serde_json::Number::from_f64(n) {
                Ok(serde_json::Value::Number(i))
            } else {
                Ok(serde_json::Value::Null)
            }
        } else if value.is_string() {
            let s = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .ok_or_else(|| "failed to convert to string".to_string())?;
            Ok(serde_json::Value::String(s))
        } else if value.is_array() {
            let arr = v8::Local::<v8::Array>::try_from(value).map_err(|_| "expected array")?;
            let mut result = Vec::with_capacity(arr.length() as usize);
            for i in 0..arr.length() {
                let elem = arr
                    .get_index(scope, i)
                    .ok_or_else(|| format!("failed to get array element at index {}", i))?;
                result.push(Self::from_js_value(scope, elem)?);
            }
            Ok(serde_json::Value::Array(result))
        } else if value.is_object() {
            let obj = v8::Local::<v8::Object>::try_from(value).map_err(|_| "expected object")?;
            let props = obj
                .get_own_property_names(
                    scope,
                    v8::GetPropertyNamesArgsBuilder::new()
                        .key_conversion(v8::KeyConversionMode::ConvertToString)
                        .build(),
                )
                .ok_or_else(|| "failed to get property names".to_string())?;

            let mut map = serde_json::Map::new();
            for i in 0..props.length() {
                let key = props
                    .get_index(scope, i)
                    .ok_or_else(|| format!("failed to get property at index {}", i))?;
                let key_str = key
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .ok_or_else(|| "failed to convert key to string".to_string())?;
                let val = obj
                    .get(scope, key)
                    .ok_or_else(|| format!("failed to get value for key {}", key_str))?;
                map.insert(key_str, Self::from_js_value(scope, val)?);
            }
            Ok(serde_json::Value::Object(map))
        } else {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            Err(format!(
                "unsupported JS value for serde_json: {}",
                truncate_for_error(&repr)
            ))
        }
    }
}

impl FromJsValue for HashMap<String, serde_json::Value> {
    fn from_js_value(
        scope: &mut v8::PinScope<'_, '_>,
        value: v8::Local<v8::Value>,
    ) -> Result<Self, String> {
        let obj = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            format!("expected object, got '{}'", truncate_for_error(&repr))
        })?;

        let props = obj
            .get_own_property_names(
                scope,
                v8::GetPropertyNamesArgsBuilder::new()
                    .key_conversion(v8::KeyConversionMode::ConvertToString)
                    .build(),
            )
            .ok_or_else(|| "failed to get property names".to_string())?;

        let mut map = HashMap::with_capacity(props.length() as usize);
        for i in 0..props.length() {
            let key = props
                .get_index(scope, i)
                .ok_or_else(|| format!("failed to get property at index {}", i))?;
            let key_str = key
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .ok_or_else(|| "failed to convert key to string".to_string())?;
            let val = obj
                .get(scope, key)
                .ok_or_else(|| format!("failed to get value for key {}", key_str))?;
            map.insert(key_str, serde_json::Value::from_js_value(scope, val)?);
        }
        Ok(map)
    }
}
