//! Traits and implementations for converting between Rust types and V8 JavaScript values.
//!
//! This module provides two symmetric traits:
//!
//! - [`FromJsValue`]: Convert V8 values to Rust types (used by [`Evaluator::eval`](crate::Evaluator::eval))
//! - [`ToJsValue`]: Convert Rust types to V8 values (used by [`Evaluator::set`](crate::Evaluator::set))
//!
//! # Built-in Implementations
//!
//! Both traits are implemented for:
//!
//! - **Primitives**: `String`, `bool`, `i32`, `i64`, `f32`, `f64`
//! - **Collections**: `Vec<T>` for JavaScript arrays
//! - **Optional**: `Option<T>` (returns `None` for `null`/`undefined` when reading)
//!
//! Additionally, `ToJsValue` is implemented for `&str`.
//!
//! # Example
//!
//! ```no_run
//! use htsvcf::{Evaluator, FromJsValue, ToJsValue};
//! use rust_htslib::bcf::{self, Read};
//!
//! let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
//! let mut eval = Evaluator::new(reader.header()).unwrap();
//!
//! // Set a global variable efficiently (no JS compilation)
//! eval.set("threshold", 10i32).unwrap();
//!
//! for result in reader.records() {
//!     let record = result.unwrap();
//!     eval.set_record(record);
//!
//!     // Use the variable in expressions
//!     let passes: bool = eval.eval("variant.info('DP') >= threshold").unwrap();
//!     if passes {
//!         println!("DP = {}", eval.eval::<i32>("variant.info('DP')").unwrap());
//!     }
//! }
//! ```
//!
//! For complex types like `serde_json::Value` or custom structs, use
//! [`Evaluator::eval_serde`](crate::Evaluator::eval_serde) instead.

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
///
/// For complex types like `serde_json::Value`, `HashMap<String, T>`, or custom
/// structs with `#[derive(Deserialize)]`, use [`crate::Evaluator::eval_serde`] instead.
///
/// # Example
///
/// ```no_run
/// use htsvcf::{Evaluator, FromJsValue};
/// use rust_htslib::bcf::{self, Read};
///
/// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
/// let mut eval = Evaluator::new(reader.header()).unwrap();
///
/// for result in reader.records() {
///     let record = result.unwrap();
///     eval.set_record(record);
///     let dp: i32 = eval.eval("variant.info('DP')").unwrap();
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

impl FromJsValue for usize {
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
            return Err(format!("expected usize, got '{}'", truncate_for_error(&repr)));
        }
        let n = value.integer_value(scope).ok_or_else(|| {
            let repr = value
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "<unknown>".into());
            format!("expected usize, got '{}'", truncate_for_error(&repr))
        })?;
        if n < 0 {
            return Err(format!("expected usize, got negative value '{}'", n));
        }
        Ok(n as usize)
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

// ============================================================================
// ToJsValue trait and implementations
// ============================================================================

/// Trait for types that can be converted to a JavaScript value.
///
/// This trait is the symmetric counterpart to [`FromJsValue`]. It is used by
/// [`Evaluator::set`](crate::Evaluator::set) to efficiently set JavaScript
/// global variables from Rust values without JS compilation overhead.
///
/// # Built-in Implementations
///
/// - `i32`, `i64` - converts to JS `Number`
/// - `f32`, `f64` - converts to JS `Number`
/// - `bool` - converts to JS `Boolean`
/// - `String`, `&str` - converts to JS `String`
/// - `Vec<T>` - converts to JS `Array` where each element is converted via `T::to_js_value`
///
/// # Example
///
/// ```no_run
/// use htsvcf::{Evaluator, ToJsValue};
/// use rust_htslib::bcf::{self, Read};
///
/// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
/// let mut eval = Evaluator::new(reader.header()).unwrap();
///
/// // Set global variables efficiently
/// eval.set("min_dp", 10i32).unwrap();
/// eval.set("threshold", 0.05f64).unwrap();
/// eval.set("sample_name", "NA12878").unwrap();
/// eval.set("allowed_chroms", vec!["chr1".to_string(), "chr2".to_string()]).unwrap();
///
/// // Use in expressions
/// for result in reader.records() {
///     let record = result.unwrap();
///     eval.set_record(record);
///     let passes: bool = eval.eval("variant.info('DP') >= min_dp").unwrap();
/// }
/// ```
pub trait ToJsValue {
    /// Convert this value to a V8 JavaScript value.
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value>;
}

impl ToJsValue for i32 {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Integer::new(scope, *self).into()
    }
}

impl ToJsValue for i64 {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        // Note: JS numbers are f64, so large i64 values may lose precision
        v8::Number::new(scope, *self as f64).into()
    }
}

impl ToJsValue for usize {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        // Note: JS numbers are f64, so large usize values may lose precision
        v8::Number::new(scope, *self as f64).into()
    }
}

impl ToJsValue for f32 {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Number::new(scope, *self as f64).into()
    }
}

impl ToJsValue for f64 {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Number::new(scope, *self).into()
    }
}

impl ToJsValue for bool {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Boolean::new(scope, *self).into()
    }
}

impl ToJsValue for String {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::String::new(scope, self)
            .map(|s| s.into())
            .unwrap_or_else(|| v8::undefined(scope).into())
    }
}

impl ToJsValue for &str {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::String::new(scope, self)
            .map(|s| s.into())
            .unwrap_or_else(|| v8::undefined(scope).into())
    }
}

impl<T: ToJsValue> ToJsValue for Vec<T> {
    fn to_js_value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        let arr = v8::Array::new(scope, self.len() as i32);
        for (i, item) in self.iter().enumerate() {
            let js_val = item.to_js_value(scope);
            arr.set_index(scope, i as u32, js_val);
        }
        arr.into()
    }
}
