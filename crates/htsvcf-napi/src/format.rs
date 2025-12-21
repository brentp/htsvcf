//! Helper functions for converting JavaScript FORMAT field values to Rust.
//!
//! These functions handle flattening nested arrays (per-sample values) into
//! the flat arrays expected by htslib's FORMAT field setters.

use napi::{bindgen_prelude::*, Error, Status};

/// Flatten FORMAT integer values from nested array [[s0v0, s0v1], [s1v0, s1v1], ...] to [s0v0, s0v1, s1v0, s1v1, ...]
///
/// Handles:
/// - Scalar values: `[10, 20, 30]` → `[10, 20, 30]`
/// - Nested arrays: `[[1, 2], [3, 4]]` → `[1, 2, 3, 4]`
/// - Null/undefined: converted to the missing sentinel value
pub fn flatten_format_integers(tag: &str, arr: &Array, missing: i32) -> napi::Result<Vec<i32>> {
    use napi::ValueType;

    let mut flattened: Vec<i32> = Vec::new();
    let len = arr.len();

    for i in 0..len {
        let sample_val: Unknown = arr.get_element(i)?;
        match sample_val.get_type()? {
            ValueType::Null | ValueType::Undefined => {
                flattened.push(missing);
            }
            ValueType::Object => {
                // Check if it's an array
                if sample_val.is_array()? {
                    let inner_arr: Array = unsafe { sample_val.cast()? };
                    for j in 0..inner_arr.len() {
                        let v: Unknown = inner_arr.get_element(j)?;
                        match v.get_type()? {
                            ValueType::Null | ValueType::Undefined => {
                                flattened.push(missing);
                            }
                            ValueType::Number => {
                                let n: f64 = unsafe { v.cast()? };
                                if !n.is_finite() || n.fract() != 0.0 {
                                    return Err(Error::new(
                                        Status::InvalidArg,
                                        format!("FORMAT/{tag} integer values must be integers"),
                                    ));
                                }
                                flattened.push(n as i32);
                            }
                            _ => {
                                return Err(Error::new(
                                    Status::InvalidArg,
                                    format!("FORMAT/{tag} integer values must be numbers or null"),
                                ));
                            }
                        }
                    }
                } else {
                    return Err(Error::new(
                        Status::InvalidArg,
                        format!("FORMAT/{tag} values must be numbers, arrays, or null"),
                    ));
                }
            }
            ValueType::Number => {
                let n: f64 = unsafe { sample_val.cast()? };
                if !n.is_finite() || n.fract() != 0.0 {
                    return Err(Error::new(
                        Status::InvalidArg,
                        format!("FORMAT/{tag} integer values must be integers"),
                    ));
                }
                flattened.push(n as i32);
            }
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    format!("FORMAT/{tag} integer values must be numbers or null"),
                ));
            }
        }
    }

    Ok(flattened)
}

/// Flatten FORMAT float values from nested array to flat array.
///
/// Handles:
/// - Scalar values: `[0.1, 0.2, 0.3]` → `[0.1, 0.2, 0.3]`
/// - Nested arrays: `[[0.1, 0.2], [0.3, 0.4]]` → `[0.1, 0.2, 0.3, 0.4]`
/// - Null/undefined: converted to the missing sentinel value
pub fn flatten_format_floats(tag: &str, arr: &Array, missing: f32) -> napi::Result<Vec<f32>> {
    use napi::ValueType;

    let mut flattened: Vec<f32> = Vec::new();
    let len = arr.len();

    for i in 0..len {
        let sample_val: Unknown = arr.get_element(i)?;
        match sample_val.get_type()? {
            ValueType::Null | ValueType::Undefined => {
                flattened.push(missing);
            }
            ValueType::Object => {
                if sample_val.is_array()? {
                    let inner_arr: Array = unsafe { sample_val.cast()? };
                    for j in 0..inner_arr.len() {
                        let v: Unknown = inner_arr.get_element(j)?;
                        match v.get_type()? {
                            ValueType::Null | ValueType::Undefined => {
                                flattened.push(missing);
                            }
                            ValueType::Number => {
                                let n: f64 = unsafe { v.cast()? };
                                if !n.is_finite() {
                                    return Err(Error::new(
                                        Status::InvalidArg,
                                        format!("FORMAT/{tag} float values must be finite"),
                                    ));
                                }
                                flattened.push(n as f32);
                            }
                            _ => {
                                return Err(Error::new(
                                    Status::InvalidArg,
                                    format!("FORMAT/{tag} float values must be numbers or null"),
                                ));
                            }
                        }
                    }
                } else {
                    return Err(Error::new(
                        Status::InvalidArg,
                        format!("FORMAT/{tag} values must be numbers, arrays, or null"),
                    ));
                }
            }
            ValueType::Number => {
                let n: f64 = unsafe { sample_val.cast()? };
                if !n.is_finite() {
                    return Err(Error::new(
                        Status::InvalidArg,
                        format!("FORMAT/{tag} float values must be finite"),
                    ));
                }
                flattened.push(n as f32);
            }
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    format!("FORMAT/{tag} float values must be numbers or null"),
                ));
            }
        }
    }

    Ok(flattened)
}

/// Flatten FORMAT string values (one string per sample).
///
/// Null/undefined values are converted to "." (VCF missing string).
pub fn flatten_format_strings(tag: &str, arr: &Array) -> napi::Result<Vec<String>> {
    use napi::ValueType;

    let mut strings: Vec<String> = Vec::new();
    let len = arr.len();

    for i in 0..len {
        let sample_val: Unknown = arr.get_element(i)?;
        match sample_val.get_type()? {
            ValueType::Null | ValueType::Undefined => {
                strings.push(".".to_string());
            }
            ValueType::String => {
                let s: String = unsafe { sample_val.cast()? };
                strings.push(s);
            }
            _ => {
                return Err(Error::new(
                    Status::InvalidArg,
                    format!("FORMAT/{tag} string values must be strings or null"),
                ));
            }
        }
    }

    Ok(strings)
}
