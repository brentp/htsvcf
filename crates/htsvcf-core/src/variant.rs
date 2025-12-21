//! VCF variant record representation and field access.
//!
//! This module provides types and functions for working with VCF/BCF variant
//! records. The [`Variant`] struct wraps a `bcf::Record` and provides convenient
//! access to standard VCF fields (CHROM, POS, REF, ALT, etc.) as well as INFO
//! and FORMAT data.
//!
//! # Value Types
//!
//! - [`InfoValue`]: Represents INFO field values (scalar, array, flag, or absent)
//! - [`FormatValue`]: Represents FORMAT field values (per-sample data)
//!
//! # Standalone Functions
//!
//! For cases where you have a borrowed `bcf::Record` reference (e.g., from a
//! GcCell in V8 bindings), standalone helper functions are provided:
//!
//! - [`record_info`]: Get INFO field from a borrowed record
//! - [`record_format`]: Get FORMAT field from a borrowed record
//! - [`record_sample`]: Get all FORMAT fields for a single sample
//! - [`record_samples`]: Get all FORMAT fields for multiple samples
//! - [`record_to_string`]: Format record as VCF line
//!
//! # Example
//!
//! ```no_run
//! use htsvcf_core::variant::{Variant, InfoValue};
//! use htsvcf_core::header::Header;
//! use rust_htslib::bcf::{self, Read};
//!
//! let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
//! let header = unsafe { Header::new(reader.header().inner) };
//!
//! for result in reader.records() {
//!     let record = result.unwrap();
//!     let variant = Variant::from_record(record);
//!
//!     println!("{}:{}", variant.chrom(), variant.pos());
//!
//!     // Access INFO fields
//!     match variant.info(&header, "DP") {
//!         InfoValue::Int(dp) => println!("DP = {}", dp),
//!         InfoValue::Absent => println!("No DP"),
//!         _ => {}
//!     }
//! }
//! ```

use crate::header::Header;
use rust_htslib::bcf;
use rust_htslib::bcf::header::{TagLength, TagType};
use rust_htslib::bcf::record::Numeric;
use std::ffi::CString;

/// Represents a value from an INFO field in a VCF record.
///
/// INFO fields can hold various types of data (flags, integers, floats, strings)
/// and can be scalar or array-valued. This enum captures all possible states:
///
/// - `Absent`: The tag is not present in the record
/// - `Missing`: The tag is present but has no value (`.` in VCF)
/// - `Bool`: A flag (presence/absence)
/// - `Int`: A single integer value
/// - `Float`: A single float value
/// - `String`: A single string value
/// - `Array`: Multiple values of any type
#[derive(Debug, Clone, PartialEq)]
pub enum InfoValue {
    /// The INFO tag is not present in the record.
    Absent,
    /// The INFO tag is present but has a missing value (`.`).
    Missing,
    /// A boolean flag (true if present).
    Bool(bool),
    /// A single integer value.
    Int(i32),
    /// A single float value.
    Float(f32),
    /// A single string value.
    String(String),
    /// An array of values (for Number != 1 fields).
    Array(Vec<InfoValue>),
}

/// Represents a value from a FORMAT field in a VCF record.
///
/// FORMAT fields contain per-sample data and can hold integers, floats, or strings.
/// Values can be scalar, array-valued, or organized per-sample.
///
/// - `Absent`: The tag is not present in the record
/// - `Missing`: The tag is present but has no value (`.` in VCF)
/// - `Int`: A single integer value
/// - `Float`: A single float value
/// - `String`: A single string value
/// - `Array`: Multiple values (for Number != 1 fields)
/// - `PerSample`: A vector of values, one per sample in the VCF
#[derive(Debug, Clone, PartialEq)]
pub enum FormatValue {
    /// The FORMAT tag is not present in the record.
    Absent,
    /// The FORMAT tag is present but has a missing value (`.`).
    Missing,
    /// A single integer value.
    Int(i32),
    /// A single float value.
    Float(f32),
    /// A single string value.
    String(String),
    /// An array of values (for Number != 1 fields).
    Array(Vec<FormatValue>),
    /// Per-sample values, one entry per sample in the VCF.
    PerSample(Vec<FormatValue>),
    /// A parsed genotype value (for the GT field).
    Genotype(Genotype),
}

/// A parsed genotype for a single sample.
///
/// This struct represents the GT field parsed into structured data:
/// - `alleles`: Allele indices where `None` represents missing (`.`)
/// - `phase`: Phasing information for each allele after the first.
///   `phase[i]` is `true` if there's a `|` separator before `alleles[i+1]`,
///   `false` if there's a `/` separator.
///
/// # Examples
///
/// | GT String | alleles | phase |
/// |-----------|---------|-------|
/// | `0/1` | `[Some(0), Some(1)]` | `[false]` |
/// | `1\|1` | `[Some(1), Some(1)]` | `[true]` |
/// | `./1` | `[None, Some(1)]` | `[false]` |
/// | `1` | `[Some(1)]` | `[]` |
/// | `0/1\|2` | `[Some(0), Some(1), Some(2)]` | `[false, true]` |
#[derive(Debug, Clone, PartialEq)]
pub struct Genotype {
    /// Allele indices. `None` represents a missing allele (`.`).
    pub alleles: Vec<Option<i32>>,
    /// Phase separators. `phase[i]` indicates whether `alleles[i+1]` is phased
    /// with `alleles[i]` (`true` = `|`, `false` = `/`).
    /// Length is always `alleles.len() - 1` (or 0 for haploid).
    pub phase: Vec<bool>,
}

// ============================================================================
// Public helper functions for working with bcf::Record references directly.
// These allow bindings (like v8) that can't own the record to still use the
// core logic.
// ============================================================================

/// Get an INFO field value from a record.
///
/// This is the standalone version of `Variant::info()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_info(record: &bcf::Record, header: &Header, tag: &str) -> InfoValue {
    let (tag_type, tag_length) = match header.info_type(tag.as_bytes()) {
        Some(v) => v,
        None => return InfoValue::Absent,
    };

    match tag_type {
        TagType::Flag => match header_info_flag(header, record, tag.as_bytes()) {
            Ok(v) => InfoValue::Bool(v),
            Err(InfoError::Absent) => InfoValue::Absent,
            Err(InfoError::Other) => InfoValue::Absent,
        },
        TagType::Integer => match header_info_values_i32(header, record, tag.as_bytes()) {
            Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Int),
            Err(InfoError::Absent) => InfoValue::Absent,
            Err(InfoError::Other) => InfoValue::Absent,
        },
        TagType::Float => match header_info_values_f32(header, record, tag.as_bytes()) {
            Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Float),
            Err(InfoError::Absent) => InfoValue::Absent,
            Err(InfoError::Other) => InfoValue::Absent,
        },
        TagType::String => match header_info_values_string(header, record, tag.as_bytes()) {
            Ok(v) => string_to_infovalue(v, tag_length),
            Err(InfoError::Absent) => InfoValue::Absent,
            Err(InfoError::Other) => InfoValue::Absent,
        },
    }
}

/// Get a FORMAT field value from a record (per-sample).
///
/// This is the standalone version of `Variant::format()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_format(record: &bcf::Record, header: &Header, tag: &str) -> FormatValue {
    let (tag_type, tag_length) = match header.format_type(tag.as_bytes()) {
        Some(v) => v,
        None => return FormatValue::Absent,
    };

    let sample_count = record.sample_count() as usize;

    match tag_type {
        TagType::Integer => match record.format(tag.as_bytes()).integer() {
            Ok(values) => FormatValue::PerSample(
                values
                    .iter()
                    .take(sample_count)
                    .map(|per_sample| {
                        format_numeric_to_value(per_sample, tag_length, FormatValue::Int)
                    })
                    .collect(),
            ),
            Err(_) => FormatValue::Absent,
        },
        TagType::Float => match record.format(tag.as_bytes()).float() {
            Ok(values) => FormatValue::PerSample(
                values
                    .iter()
                    .take(sample_count)
                    .map(|per_sample| {
                        format_numeric_to_value(per_sample, tag_length, FormatValue::Float)
                    })
                    .collect(),
            ),
            Err(_) => FormatValue::Absent,
        },
        TagType::String => match record.format(tag.as_bytes()).string() {
            Ok(values) => FormatValue::PerSample(
                values
                    .iter()
                    .take(sample_count)
                    .map(|per_sample| format_string_to_value(per_sample, tag_length))
                    .collect(),
            ),
            Err(_) => FormatValue::Absent,
        },
        TagType::Flag => FormatValue::Absent,
    }
}

/// Get sample data from a record for a single sample by name.
///
/// This is the standalone version of `Variant::sample()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_sample(
    record: &bcf::Record,
    header: &Header,
    sample: &str,
) -> Option<Vec<(String, FormatValue)>> {
    let sample_id = header.sample_id(sample.as_bytes())?;
    let sample_count = record.sample_count() as usize;
    if sample_id >= sample_count {
        return None;
    }

    let format_tags = get_format_tag_names(header, record);
    let mut out: Vec<(String, FormatValue)> = Vec::with_capacity(format_tags.len() + 2);

    for (tag_name, tag_bytes) in format_tags {
        let Some(value) = format_value_for_sample(header, record, &tag_bytes, sample_id) else {
            continue;
        };
        out.push((tag_name, value));
    }

    // Add parsed genotype if GT field exists
    if let Some(gt) = parse_genotype_for_sample(record, sample_id) {
        out.push(("genotype".to_string(), FormatValue::Genotype(gt)));
    }

    // Include the sample name so JS bindings can expose it.
    // Set it last so it can't be overwritten by a FORMAT tag named "sample_name".
    out.push((
        "sample_name".to_string(),
        FormatValue::String(sample.to_string()),
    ));

    Some(out)
}

/// Get sample data from a record for all samples or a subset.
///
/// This is the standalone version of `Variant::samples()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_samples(
    record: &bcf::Record,
    header: &Header,
    subset: Option<&[&str]>,
) -> Vec<Vec<(String, FormatValue)>> {
    let sample_count = record.sample_count() as usize;
    if sample_count == 0 {
        return Vec::new();
    }

    let sample_names = header.sample_names();
    let format_tags = get_format_tag_names(header, record);

    // Determine which sample indices to include and in what order
    let sample_indices: Vec<usize> = match subset {
        None => (0..sample_count).collect(),
        Some(names) => {
            let name_to_idx = header.sample_name_to_idx();
            names
                .iter()
                .filter_map(|name| name_to_idx.get(*name).copied())
                .collect()
        }
    };

    if sample_indices.is_empty() {
        return Vec::new();
    }

    // Pre-allocate result vectors for each requested sample
    let mut results: Vec<Vec<(String, FormatValue)>> = sample_indices
        .iter()
        .map(|_| Vec::with_capacity(format_tags.len() + 1))
        .collect();

    // For each FORMAT tag, fetch values for ALL samples at once and distribute to requested ones
    for (tag_name, tag_bytes) in &format_tags {
        let Some((tag_type, tag_length)) = header.format_type(tag_bytes) else {
            continue;
        };

        match tag_type {
            bcf::header::TagType::Integer => {
                let Ok(all_values) = record.format(tag_bytes).integer() else {
                    continue;
                };
                for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
                    if let Some(per_sample) = all_values.get(sample_idx) {
                        let value =
                            format_numeric_to_value(per_sample, tag_length, FormatValue::Int);
                        results[result_idx].push((tag_name.clone(), value));
                    }
                }
            }
            bcf::header::TagType::Float => {
                let Ok(all_values) = record.format(tag_bytes).float() else {
                    continue;
                };
                for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
                    if let Some(per_sample) = all_values.get(sample_idx) {
                        let value =
                            format_numeric_to_value(per_sample, tag_length, FormatValue::Float);
                        results[result_idx].push((tag_name.clone(), value));
                    }
                }
            }
            bcf::header::TagType::String => {
                let Ok(all_values) = record.format(tag_bytes).string() else {
                    continue;
                };
                for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
                    if let Some(per_sample) = all_values.get(sample_idx) {
                        let value = format_string_to_value(per_sample, tag_length);
                        results[result_idx].push((tag_name.clone(), value));
                    }
                }
            }
            bcf::header::TagType::Flag => {
                // Flags are not valid for FORMAT
            }
        }
    }

    // Add parsed genotypes if GT field exists
    if let Ok(gts) = record.genotypes() {
        for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
            let gt = parse_genotype(&gts.get(sample_idx));
            results[result_idx].push(("genotype".to_string(), FormatValue::Genotype(gt)));
        }
    }

    // Add sample_name to each result (last, so it can't be overwritten by a FORMAT tag)
    for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
        let name = sample_names
            .get(sample_idx)
            .cloned()
            .unwrap_or_else(|| format!("sample_{sample_idx}"));
        results[result_idx].push(("sample_name".to_string(), FormatValue::String(name)));
    }

    results
}

/// Get the list of FORMAT tag names present in a record.
pub fn get_format_tag_names(header: &Header, record: &bcf::Record) -> Vec<(String, Vec<u8>)> {
    let record_ptr =
        record.inner() as *const rust_htslib::htslib::bcf1_t as *mut rust_htslib::htslib::bcf1_t;

    let n_fmt = unsafe { (*record_ptr).n_fmt() as usize };
    let fmt_ptr = unsafe { (*record_ptr).d.fmt };

    if fmt_ptr.is_null() || n_fmt == 0 {
        return Vec::new();
    }

    let mut tags = Vec::with_capacity(n_fmt);
    for i in 0..n_fmt {
        let fmt = unsafe { *fmt_ptr.add(i) };
        let (tag_name, tag_bytes) = header.id_to_name_cached(fmt.id as u32);
        tags.push((tag_name, tag_bytes));
    }
    tags
}

/// Format a record as a VCF line string.
pub fn record_to_string(record: &bcf::Record, header: &Header) -> Option<String> {
    let mut s = rust_htslib::htslib::kstring_t {
        l: 0,
        m: 0,
        s: std::ptr::null_mut(),
    };

    let record_ptr =
        record.inner() as *const rust_htslib::htslib::bcf1_t as *mut rust_htslib::htslib::bcf1_t;

    let _ = unsafe {
        rust_htslib::htslib::bcf_unpack(record_ptr, rust_htslib::htslib::BCF_UN_ALL as i32)
    };

    let ret = unsafe {
        rust_htslib::htslib::vcf_format(
            header.inner_ptr() as *const rust_htslib::htslib::bcf_hdr_t,
            record_ptr as *const rust_htslib::htslib::bcf1_t,
            &mut s,
        )
    };
    if ret != 0 {
        if !s.s.is_null() {
            unsafe { rust_htslib::htslib::free(s.s as *mut std::os::raw::c_void) };
        }
        return None;
    }

    let bytes = unsafe { std::slice::from_raw_parts(s.s as *const u8, s.l as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();

    if !s.s.is_null() {
        unsafe { rust_htslib::htslib::free(s.s as *mut std::os::raw::c_void) };
    }

    Some(text.trim_end_matches('\n').to_string())
}

/// Parse genotypes for all samples or a subset of samples.
///
/// Returns a vector of [`Genotype`] structs, one per requested sample.
/// If `subset` is `None`, returns genotypes for all samples in header order.
/// If `subset` is `Some(names)`, returns genotypes only for those samples
/// in the order specified (unknown sample names are skipped).
///
/// Returns an empty vector if:
/// - The record has no GT field
/// - The record has no samples
/// - None of the requested samples exist
pub fn record_genotypes(
    record: &bcf::Record,
    header: &Header,
    subset: Option<&[&str]>,
) -> Vec<Genotype> {
    let sample_count = record.sample_count() as usize;
    if sample_count == 0 {
        return Vec::new();
    }

    let gts = match record.genotypes() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };

    // Determine which sample indices to include
    let sample_indices: Vec<usize> = match subset {
        None => (0..sample_count).collect(),
        Some(names) => {
            let name_to_idx = header.sample_name_to_idx();
            names
                .iter()
                .filter_map(|name| name_to_idx.get(*name).copied())
                .collect()
        }
    };

    sample_indices
        .iter()
        .map(|&idx| parse_genotype(&gts.get(idx)))
        .collect()
}

/// Parse a single genotype from rust-htslib's Genotype type.
fn parse_genotype(gt: &rust_htslib::bcf::record::Genotype) -> Genotype {
    use rust_htslib::bcf::record::GenotypeAllele;

    let mut alleles: Vec<Option<i32>> = Vec::with_capacity(gt.len());
    let mut phase: Vec<bool> = Vec::with_capacity(gt.len().saturating_sub(1));

    for (i, allele) in gt.iter().enumerate() {
        match allele {
            GenotypeAllele::Unphased(idx) => {
                alleles.push(Some(*idx));
                // First allele has no preceding separator, subsequent unphased alleles mean '/'
                if i > 0 {
                    phase.push(false);
                }
            }
            GenotypeAllele::Phased(idx) => {
                alleles.push(Some(*idx));
                // Phased means '|' separator before this allele
                if i > 0 {
                    phase.push(true);
                }
            }
            GenotypeAllele::UnphasedMissing => {
                alleles.push(None);
                if i > 0 {
                    phase.push(false);
                }
            }
            GenotypeAllele::PhasedMissing => {
                alleles.push(None);
                if i > 0 {
                    phase.push(true);
                }
            }
        }
    }

    Genotype { alleles, phase }
}

/// Parse a single sample's genotype by index.
fn parse_genotype_for_sample(record: &bcf::Record, sample_idx: usize) -> Option<Genotype> {
    let gts = record.genotypes().ok()?;
    Some(parse_genotype(&gts.get(sample_idx)))
}

/// Set an INFO flag value on a record.
pub fn record_set_info_flag(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
    is_set: bool,
) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    if tag_type != TagType::Flag {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: tag.to_string(),
        });
    }

    if is_set {
        record.push_info_flag(tag.as_bytes())?;
    } else {
        record.clear_info_flag(tag.as_bytes())?;
    }

    record.unpack();
    Ok(())
}

/// Set an INFO integer value on a record.
pub fn record_set_info_integer(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
    values: &[i32],
) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    if tag_type != TagType::Integer {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: tag.to_string(),
        });
    }

    record.push_info_integer(tag.as_bytes(), values)?;
    record.unpack();
    Ok(())
}

/// Set an INFO float value on a record.
pub fn record_set_info_float(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
    values: &[f32],
) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    if tag_type != TagType::Float {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: tag.to_string(),
        });
    }

    record.push_info_float(tag.as_bytes(), values)?;
    record.unpack();
    Ok(())
}

/// Set an INFO string value on a record.
pub fn record_set_info_string(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
    values: &[String],
) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    if tag_type != TagType::String {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: tag.to_string(),
        });
    }

    let refs: Vec<&[u8]> = values.iter().map(|s| s.as_bytes()).collect();
    record.push_info_string(tag.as_bytes(), &refs)?;
    record.unpack();
    Ok(())
}

/// Clear an INFO field from a record.
pub fn record_clear_info(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    match tag_type {
        TagType::Flag => record.clear_info_flag(tag.as_bytes())?,
        TagType::Integer => record.clear_info_integer(tag.as_bytes())?,
        TagType::Float => record.clear_info_float(tag.as_bytes())?,
        TagType::String => record.clear_info_string(tag.as_bytes())?,
    }

    record.unpack();
    Ok(())
}

// ============================================================================
// FORMAT field setters
// ============================================================================

/// Missing value sentinel for i32 FORMAT fields.
/// This matches htslib's bcf_int32_missing.
const FORMAT_MISSING_INT: i32 = i32::MIN;

/// Missing value sentinel for f32 FORMAT fields.
/// This matches htslib's bcf_float_missing (a specific NaN).
fn format_missing_float() -> f32 {
    f32::from_bits(0x7F80_0001)
}

/// Set a FORMAT integer field on a record.
///
/// The `values` slice should be flattened: for a field with `n` values per sample
/// and `s` samples, provide `s * n` values in sample-major order:
/// `[sample0_val0, sample0_val1, ..., sample1_val0, sample1_val1, ...]`
///
/// Use `FORMAT_MISSING_INT` (`i32::MIN`) to represent missing values.
///
/// # Errors
///
/// Returns an error if:
/// - The tag is not defined in the header
/// - The tag is not an Integer type
/// - The tag is "GT" (use dedicated genotype methods instead)
pub fn record_set_format_integer(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
    values: &[i32],
) -> Result<(), rust_htslib::errors::Error> {
    if tag == "GT" {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: "GT cannot be set via set_format; use dedicated genotype methods".to_string(),
        });
    }

    let (tag_type, _) = header.format_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    if tag_type != TagType::Integer {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: tag.to_string(),
        });
    }

    record.push_format_integer(tag.as_bytes(), values)?;
    record.unpack();
    Ok(())
}

/// Set a FORMAT float field on a record.
///
/// The `values` slice should be flattened: for a field with `n` values per sample
/// and `s` samples, provide `s * n` values in sample-major order:
/// `[sample0_val0, sample0_val1, ..., sample1_val0, sample1_val1, ...]`
///
/// Use `format_missing_float()` to represent missing values.
///
/// # Errors
///
/// Returns an error if:
/// - The tag is not defined in the header
/// - The tag is not a Float type
pub fn record_set_format_float(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
    values: &[f32],
) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header.format_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    if tag_type != TagType::Float {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: tag.to_string(),
        });
    }

    record.push_format_float(tag.as_bytes(), values)?;
    record.unpack();
    Ok(())
}

/// Set a FORMAT string field on a record.
///
/// Provide one string per sample. For multi-value string fields, concatenate
/// values with commas within each sample's string.
///
/// # Errors
///
/// Returns an error if:
/// - The tag is not defined in the header
/// - The tag is not a String type
/// - The tag is "GT" (use dedicated genotype methods instead)
pub fn record_set_format_string(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
    values: &[String],
) -> Result<(), rust_htslib::errors::Error> {
    if tag == "GT" {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: "GT cannot be set via set_format; use dedicated genotype methods".to_string(),
        });
    }

    let (tag_type, _) = header.format_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    if tag_type != TagType::String {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: tag.to_string(),
        });
    }

    let refs: Vec<&[u8]> = values.iter().map(|s| s.as_bytes()).collect();
    record.push_format_string(tag.as_bytes(), &refs)?;
    record.unpack();
    Ok(())
}

/// Clear (remove) a FORMAT field from a record.
///
/// # Errors
///
/// Returns an error if the tag is not defined in the header.
pub fn record_clear_format(
    record: &mut bcf::Record,
    header: &Header,
    tag: &str,
) -> Result<(), rust_htslib::errors::Error> {
    if tag == "GT" {
        return Err(rust_htslib::errors::Error::BcfSetTag {
            tag: "GT cannot be cleared via clear_format".to_string(),
        });
    }

    let (tag_type, _) = header.format_type(tag.as_bytes()).ok_or_else(|| {
        rust_htslib::errors::Error::BcfUndefinedTag {
            tag: tag.to_string(),
        }
    })?;

    // To clear a FORMAT field, we call the appropriate push method with an empty slice.
    // This is how htslib handles clearing FORMAT fields.
    match tag_type {
        TagType::Integer => record.push_format_integer(tag.as_bytes(), &[])?,
        TagType::Float => record.push_format_float(tag.as_bytes(), &[])?,
        TagType::String => record.push_format_string::<&[u8]>(tag.as_bytes(), &[])?,
        TagType::Flag => {
            // FORMAT flags are rare but handle them
            return Err(rust_htslib::errors::Error::BcfSetTag {
                tag: format!("FORMAT/{tag} is a Flag type which is not supported"),
            });
        }
    }

    record.unpack();
    Ok(())
}

/// Get the missing value sentinel for FORMAT integer fields.
pub fn format_int_missing() -> i32 {
    FORMAT_MISSING_INT
}

/// Get the missing value sentinel for FORMAT float fields.
pub fn format_float_missing() -> f32 {
    format_missing_float()
}

/// A VCF/BCF variant record with convenient field accessors.
///
/// `Variant` wraps a `rust_htslib::bcf::Record` and provides methods for
/// accessing standard VCF fields (CHROM, POS, REF, ALT, etc.) as well as
/// INFO and FORMAT data.
///
/// # Example
///
/// ```no_run
/// use htsvcf_core::variant::Variant;
/// use rust_htslib::bcf::{self, Read};
///
/// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
/// for result in reader.records() {
///     let record = result.unwrap();
///     let variant = Variant::from_record(record);
///     println!("{}:{} {}", variant.chrom(), variant.pos(), variant.reference());
/// }
/// ```
#[derive(Debug)]
pub struct Variant {
    record: bcf::Record,
    chrom: String,
}

impl Variant {
    /// Create a `Variant` from a `bcf::Record`.
    ///
    /// The record is unpacked and the chromosome name is cached for efficient access.
    pub fn from_record(mut record: bcf::Record) -> Self {
        record.unpack();
        let chrom = match record.rid() {
            Some(rid) => record
                .header()
                .rid2name(rid)
                .ok()
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .unwrap_or_else(|| ".".to_string()),
            None => ".".to_string(),
        };
        Self { record, chrom }
    }

    /// Consume this Variant and return the underlying `bcf::Record`.
    ///
    /// This is primarily used by writer bindings so `write(variant)` can consume
    /// a JS `Variant` without cloning.
    pub fn into_record(self) -> bcf::Record {
        self.record
    }

    /// Get a mutable reference to the underlying `bcf::Record`.
    ///
    /// This is useful for passing the record to functions that need `&mut bcf::Record`,
    /// such as [`Writer::write_record`](crate::Writer::write_record).
    pub fn record_mut(&mut self) -> &mut bcf::Record {
        &mut self.record
    }

    /// Get the chromosome/contig name (CHROM column).
    pub fn chrom(&self) -> &str {
        &self.chrom
    }

    /// Get the reference sequence ID (rid) from the header, if present.
    pub fn rid(&self) -> Option<u32> {
        self.record.rid()
    }

    /// Get the zero-based start position.
    ///
    /// This is the internal representation used by htslib. For 1-based VCF
    /// coordinates, use [`pos()`](Self::pos).
    pub fn start(&self) -> i64 {
        self.record.pos()
    }

    /// Get the 1-based position (POS column).
    ///
    /// This matches the coordinate shown in VCF files.
    pub fn pos(&self) -> i64 {
        self.record.pos() + 1
    }

    /// Get the end coordinate (htslib semantics).
    ///
    /// For SNPs this equals `start + 1`. For indels and other variants,
    /// this reflects the span of the reference allele.
    pub fn end(&self) -> i64 {
        self.record.end()
    }

    /// Get the variant ID (ID column).
    ///
    /// Returns "." if no ID is set.
    pub fn id(&self) -> String {
        String::from_utf8_lossy(&self.record.id()).into_owned()
    }

    /// Set the variant ID (ID column).
    ///
    /// Pass an empty string or "." to clear the ID.
    pub fn set_id(&mut self, id: &str) -> Result<(), rust_htslib::errors::Error> {
        let id = if id.is_empty() { "." } else { id };
        self.record.set_id(id.as_bytes())?;
        Ok(())
    }

    /// Get the reference allele (REF column).
    pub fn reference(&self) -> String {
        self.record
            .alleles()
            .first()
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .unwrap_or_else(|| ".".to_string())
    }

    /// Get the alternate alleles (ALT column).
    ///
    /// Returns a vector of alternate allele strings. May be empty if there
    /// are no alternates.
    pub fn alts(&self) -> Vec<String> {
        self.record
            .alleles()
            .into_iter()
            .skip(1)
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    }

    /// Get the quality score (QUAL column).
    ///
    /// Returns `None` if QUAL is missing (`.` in VCF).
    pub fn qual(&self) -> Option<f32> {
        let qual = self.record.qual();
        if qual.is_missing() {
            None
        } else {
            Some(qual)
        }
    }

    /// Set the quality score (QUAL column).
    ///
    /// Pass `None` to set QUAL to missing (`.`).
    pub fn set_qual(&mut self, qual: Option<f32>) {
        match qual {
            Some(v) => self.record.set_qual(v),
            None => self.record.set_qual(<f32 as Numeric>::missing()),
        }
    }

    /// Return the FILTER column as a list of filter IDs.
    ///
    /// Records that are '.' return an empty list.
    pub fn filters(&self) -> Vec<String> {
        let header = self.record.header();
        let mut out = Vec::new();
        for id in self.record.filters() {
            let name = String::from_utf8_lossy(&header.id_to_name(id)).into_owned();
            out.push(name);
        }
        out
    }

    /// Set the FILTER column.
    ///
    /// Pass an empty slice, `[""]`, or `["."]` to clear all filters.
    /// Otherwise, provide an array of filter names to set.
    pub fn set_filters(&mut self, filters: &[String]) -> Result<(), rust_htslib::errors::Error> {
        let want_clear = filters.is_empty()
            || (filters.len() == 1 && (filters[0].is_empty() || filters[0] == "."));

        if want_clear {
            let refs: Vec<&[u8]> = Vec::new();
            self.record.set_filters(&refs)?;
            return Ok(());
        }

        let refs: Vec<&[u8]> = filters.iter().map(|s| s.as_bytes()).collect();
        self.record.set_filters(&refs)?;
        Ok(())
    }

    /// Set an INFO flag value.
    ///
    /// Pass `true` to set the flag, `false` to clear it.
    /// Returns an error if the tag is not defined in the header or is not a Flag type.
    pub fn set_info_flag(
        &mut self,
        header: &Header,
        tag: &str,
        is_set: bool,
    ) -> Result<(), rust_htslib::errors::Error> {
        let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
            rust_htslib::errors::Error::BcfUndefinedTag {
                tag: tag.to_string(),
            }
        })?;

        if tag_type != TagType::Flag {
            return Err(rust_htslib::errors::Error::BcfSetTag {
                tag: tag.to_string(),
            });
        }

        if is_set {
            self.record.push_info_flag(tag.as_bytes())?;
        } else {
            self.record.clear_info_flag(tag.as_bytes())?;
        }

        self.record.unpack();
        Ok(())
    }

    /// Set an INFO integer value.
    ///
    /// Pass a slice of integers to set. For scalar fields (Number=1), pass a single-element slice.
    /// Returns an error if the tag is not defined in the header or is not an Integer type.
    pub fn set_info_integer(
        &mut self,
        header: &Header,
        tag: &str,
        values: &[i32],
    ) -> Result<(), rust_htslib::errors::Error> {
        let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
            rust_htslib::errors::Error::BcfUndefinedTag {
                tag: tag.to_string(),
            }
        })?;

        if tag_type != TagType::Integer {
            return Err(rust_htslib::errors::Error::BcfSetTag {
                tag: tag.to_string(),
            });
        }

        self.record.push_info_integer(tag.as_bytes(), values)?;
        self.record.unpack();
        Ok(())
    }

    /// Set an INFO float value.
    ///
    /// Pass a slice of floats to set. For scalar fields (Number=1), pass a single-element slice.
    /// Returns an error if the tag is not defined in the header or is not a Float type.
    pub fn set_info_float(
        &mut self,
        header: &Header,
        tag: &str,
        values: &[f32],
    ) -> Result<(), rust_htslib::errors::Error> {
        let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
            rust_htslib::errors::Error::BcfUndefinedTag {
                tag: tag.to_string(),
            }
        })?;

        if tag_type != TagType::Float {
            return Err(rust_htslib::errors::Error::BcfSetTag {
                tag: tag.to_string(),
            });
        }

        self.record.push_info_float(tag.as_bytes(), values)?;
        self.record.unpack();
        Ok(())
    }

    /// Set an INFO string value.
    ///
    /// Pass a slice of strings to set. For scalar fields (Number=1), pass a single-element slice.
    /// Returns an error if the tag is not defined in the header or is not a String type.
    pub fn set_info_string(
        &mut self,
        header: &Header,
        tag: &str,
        values: &[String],
    ) -> Result<(), rust_htslib::errors::Error> {
        let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
            rust_htslib::errors::Error::BcfUndefinedTag {
                tag: tag.to_string(),
            }
        })?;

        if tag_type != TagType::String {
            return Err(rust_htslib::errors::Error::BcfSetTag {
                tag: tag.to_string(),
            });
        }

        let refs: Vec<&[u8]> = values.iter().map(|s| s.as_bytes()).collect();
        self.record.push_info_string(tag.as_bytes(), &refs)?;
        self.record.unpack();
        Ok(())
    }

    /// Translate this record to a new header.
    ///
    /// This is required when you mutate the header (e.g. add a new INFO field)
    /// and then want to set values for those new tags.
    ///
    /// IMPORTANT: this does not duplicate/copy the header.
    pub fn translate(&mut self, header: &Header) -> Result<(), rust_htslib::errors::Error> {
        let mut view = header.translate_view();
        self.record.translate(&mut view)
    }

    /// Clear (remove) an INFO field from the record.
    ///
    /// Returns an error if the tag is not defined in the header.
    pub fn clear_info(
        &mut self,
        header: &Header,
        tag: &str,
    ) -> Result<(), rust_htslib::errors::Error> {
        let (tag_type, _) = header.info_type(tag.as_bytes()).ok_or_else(|| {
            rust_htslib::errors::Error::BcfUndefinedTag {
                tag: tag.to_string(),
            }
        })?;

        match tag_type {
            TagType::Flag => self.record.clear_info_flag(tag.as_bytes())?,
            TagType::Integer => self.record.clear_info_integer(tag.as_bytes())?,
            TagType::Float => self.record.clear_info_float(tag.as_bytes())?,
            TagType::String => self.record.clear_info_string(tag.as_bytes())?,
        }

        self.record.unpack();
        Ok(())
    }

    /// Set a FORMAT integer field.
    ///
    /// The `values` slice should be flattened: for a field with `n` values per sample
    /// and `s` samples, provide `s * n` values in sample-major order.
    ///
    /// Use [`format_int_missing()`] to represent missing values.
    ///
    /// # Errors
    ///
    /// Returns an error if the tag is not defined, is not Integer type, or is "GT".
    pub fn set_format_integer(
        &mut self,
        header: &Header,
        tag: &str,
        values: &[i32],
    ) -> Result<(), rust_htslib::errors::Error> {
        record_set_format_integer(&mut self.record, header, tag, values)
    }

    /// Set a FORMAT float field.
    ///
    /// The `values` slice should be flattened: for a field with `n` values per sample
    /// and `s` samples, provide `s * n` values in sample-major order.
    ///
    /// Use [`format_float_missing()`] to represent missing values.
    ///
    /// # Errors
    ///
    /// Returns an error if the tag is not defined or is not Float type.
    pub fn set_format_float(
        &mut self,
        header: &Header,
        tag: &str,
        values: &[f32],
    ) -> Result<(), rust_htslib::errors::Error> {
        record_set_format_float(&mut self.record, header, tag, values)
    }

    /// Set a FORMAT string field.
    ///
    /// Provide one string per sample.
    ///
    /// # Errors
    ///
    /// Returns an error if the tag is not defined, is not String type, or is "GT".
    pub fn set_format_string(
        &mut self,
        header: &Header,
        tag: &str,
        values: &[String],
    ) -> Result<(), rust_htslib::errors::Error> {
        record_set_format_string(&mut self.record, header, tag, values)
    }

    /// Clear (remove) a FORMAT field from this record.
    ///
    /// # Errors
    ///
    /// Returns an error if the tag is not defined in the header or is "GT".
    pub fn clear_format(
        &mut self,
        header: &Header,
        tag: &str,
    ) -> Result<(), rust_htslib::errors::Error> {
        record_clear_format(&mut self.record, header, tag)
    }

    /// Get an INFO field value by tag name.
    ///
    /// Returns the appropriate [`InfoValue`] variant based on the tag's type
    /// as defined in the header. Returns [`InfoValue::Absent`] if the tag
    /// is not present in this record.
    pub fn info(&self, header: &Header, tag: &str) -> InfoValue {
        let (tag_type, tag_length) = match header.info_type(tag.as_bytes()) {
            Some(v) => v,
            None => return InfoValue::Absent,
        };

        match tag_type {
            TagType::Flag => match header_info_flag(header, &self.record, tag.as_bytes()) {
                Ok(v) => InfoValue::Bool(v),
                Err(InfoError::Absent) => InfoValue::Absent,
                Err(InfoError::Other) => InfoValue::Absent,
            },
            TagType::Integer => {
                match header_info_values_i32(header, &self.record, tag.as_bytes()) {
                    Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Int),
                    Err(InfoError::Absent) => InfoValue::Absent,
                    Err(InfoError::Other) => InfoValue::Absent,
                }
            }
            TagType::Float => match header_info_values_f32(header, &self.record, tag.as_bytes()) {
                Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Float),
                Err(InfoError::Absent) => InfoValue::Absent,
                Err(InfoError::Other) => InfoValue::Absent,
            },
            TagType::String => {
                match header_info_values_string(header, &self.record, tag.as_bytes()) {
                    Ok(v) => string_to_infovalue(v, tag_length),
                    Err(InfoError::Absent) => InfoValue::Absent,
                    Err(InfoError::Other) => InfoValue::Absent,
                }
            }
        }
    }

    /// Get a FORMAT field value by tag name.
    ///
    /// Returns a [`FormatValue::PerSample`] containing values for all samples,
    /// or [`FormatValue::Absent`] if the tag is not present in this record.
    pub fn format(&self, header: &Header, tag: &str) -> FormatValue {
        let (tag_type, tag_length) = match header.format_type(tag.as_bytes()) {
            Some(v) => v,
            None => return FormatValue::Absent,
        };

        let sample_count = self.record.sample_count() as usize;

        match tag_type {
            TagType::Integer => match self.record.format(tag.as_bytes()).integer() {
                Ok(values) => FormatValue::PerSample(
                    values
                        .iter()
                        .take(sample_count)
                        .map(|per_sample| {
                            format_numeric_to_value(per_sample, tag_length, FormatValue::Int)
                        })
                        .collect(),
                ),
                Err(_) => FormatValue::Absent,
            },
            TagType::Float => match self.record.format(tag.as_bytes()).float() {
                Ok(values) => FormatValue::PerSample(
                    values
                        .iter()
                        .take(sample_count)
                        .map(|per_sample| {
                            format_numeric_to_value(per_sample, tag_length, FormatValue::Float)
                        })
                        .collect(),
                ),
                Err(_) => FormatValue::Absent,
            },
            TagType::String => match self.record.format(tag.as_bytes()).string() {
                Ok(values) => FormatValue::PerSample(
                    values
                        .iter()
                        .take(sample_count)
                        .map(|per_sample| format_string_to_value(per_sample, tag_length))
                        .collect(),
                ),
                Err(_) => FormatValue::Absent,
            },
            TagType::Flag => FormatValue::Absent,
        }
    }

    /// Get all FORMAT field values for a single sample by name.
    ///
    /// Returns a vector of (tag_name, value) pairs for all FORMAT fields present
    /// in this record, plus a `genotype` entry with the parsed GT and a
    /// `sample_name` entry with the sample's name.
    /// Returns `None` if the sample is not found.
    pub fn sample(&self, header: &Header, sample: &str) -> Option<Vec<(String, FormatValue)>> {
        let sample_id = header.sample_id(sample.as_bytes())?;
        let sample_count = self.record.sample_count() as usize;
        if sample_id >= sample_count {
            return None;
        }

        let format_tags = self.get_format_tag_names(header);
        let mut out: Vec<(String, FormatValue)> = Vec::with_capacity(format_tags.len() + 2);

        for (tag_name, tag_bytes) in format_tags {
            let Some(value) = format_value_for_sample(header, &self.record, &tag_bytes, sample_id)
            else {
                continue;
            };
            out.push((tag_name, value));
        }

        // Add parsed genotype if GT field exists
        if let Some(gt) = parse_genotype_for_sample(&self.record, sample_id) {
            out.push(("genotype".to_string(), FormatValue::Genotype(gt)));
        }

        // Include the sample name so JS bindings can expose it.
        // Set it last so it can't be overwritten by a FORMAT tag named "sample_name".
        out.push((
            "sample_name".to_string(),
            FormatValue::String(sample.to_string()),
        ));

        Some(out)
    }

    /// Returns samples' FORMAT data as an array of objects.
    ///
    /// If `subset` is `None`, returns all samples in header order.
    /// If `subset` is `Some(names)`, returns only the specified samples in the
    /// order given. Unknown sample names are silently skipped.
    ///
    /// Each element contains all FORMAT fields plus a `sample_name` key.
    /// Returns an empty Vec if the VCF has no samples or no requested samples exist.
    pub fn samples(
        &self,
        header: &Header,
        subset: Option<&[&str]>,
    ) -> Vec<Vec<(String, FormatValue)>> {
        let sample_count = self.record.sample_count() as usize;
        if sample_count == 0 {
            return Vec::new();
        }

        let sample_names = header.sample_names();
        let format_tags = self.get_format_tag_names(header);

        // Determine which sample indices to include and in what order
        let sample_indices: Vec<usize> = match subset {
            None => (0..sample_count).collect(),
            Some(names) => {
                let name_to_idx = header.sample_name_to_idx();
                names
                    .iter()
                    .filter_map(|name| name_to_idx.get(*name).copied())
                    .collect()
            }
        };

        if sample_indices.is_empty() {
            return Vec::new();
        }

        // Pre-allocate result vectors for each requested sample
        let mut results: Vec<Vec<(String, FormatValue)>> = sample_indices
            .iter()
            .map(|_| Vec::with_capacity(format_tags.len() + 1))
            .collect();

        // For each FORMAT tag, fetch values for ALL samples at once and distribute to requested ones
        for (tag_name, tag_bytes) in &format_tags {
            let Some((tag_type, tag_length)) = header.format_type(tag_bytes) else {
                continue;
            };

            match tag_type {
                bcf::header::TagType::Integer => {
                    let Ok(all_values) = self.record.format(tag_bytes).integer() else {
                        continue;
                    };
                    for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
                        if let Some(per_sample) = all_values.get(sample_idx) {
                            let value =
                                format_numeric_to_value(per_sample, tag_length, FormatValue::Int);
                            results[result_idx].push((tag_name.clone(), value));
                        }
                    }
                }
                bcf::header::TagType::Float => {
                    let Ok(all_values) = self.record.format(tag_bytes).float() else {
                        continue;
                    };
                    for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
                        if let Some(per_sample) = all_values.get(sample_idx) {
                            let value =
                                format_numeric_to_value(per_sample, tag_length, FormatValue::Float);
                            results[result_idx].push((tag_name.clone(), value));
                        }
                    }
                }
                bcf::header::TagType::String => {
                    let Ok(all_values) = self.record.format(tag_bytes).string() else {
                        continue;
                    };
                    for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
                        if let Some(per_sample) = all_values.get(sample_idx) {
                            let value = format_string_to_value(per_sample, tag_length);
                            results[result_idx].push((tag_name.clone(), value));
                        }
                    }
                }
                bcf::header::TagType::Flag => {
                    // Flags are not valid for FORMAT
                }
            }
        }

        // Add parsed genotypes if GT field exists
        if let Ok(gts) = self.record.genotypes() {
            for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
                let gt = parse_genotype(&gts.get(sample_idx));
                results[result_idx].push(("genotype".to_string(), FormatValue::Genotype(gt)));
            }
        }

        // Add sample_name to each result (last, so it can't be overwritten by a FORMAT tag)
        for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
            let name = sample_names
                .get(sample_idx)
                .cloned()
                .unwrap_or_else(|| format!("sample_{sample_idx}"));
            results[result_idx].push(("sample_name".to_string(), FormatValue::String(name)));
        }

        results
    }

    /// Get parsed genotypes for all samples or a subset.
    ///
    /// Returns a vector of [`Genotype`] structs, one per requested sample.
    /// If `subset` is `None`, returns genotypes for all samples in header order.
    /// If `subset` is `Some(names)`, returns genotypes only for those samples
    /// in the order specified (unknown sample names are skipped).
    ///
    /// Returns an empty vector if the record has no GT field or no samples.
    pub fn genotypes(&self, header: &Header, subset: Option<&[&str]>) -> Vec<Genotype> {
        record_genotypes(&self.record, header, subset)
    }

    /// Get the list of FORMAT tag names present in this record.
    ///
    /// Returns a vector of (name_string, name_bytes) tuples for efficient
    /// subsequent lookups.
    fn get_format_tag_names(&self, header: &Header) -> Vec<(String, Vec<u8>)> {
        let record_ptr = self.record.inner() as *const rust_htslib::htslib::bcf1_t
            as *mut rust_htslib::htslib::bcf1_t;

        let n_fmt = unsafe { (*record_ptr).n_fmt() as usize };
        let fmt_ptr = unsafe { (*record_ptr).d.fmt };

        if fmt_ptr.is_null() || n_fmt == 0 {
            return Vec::new();
        }

        let mut tags = Vec::with_capacity(n_fmt);
        for i in 0..n_fmt {
            let fmt = unsafe { *fmt_ptr.add(i) };
            let (tag_name, tag_bytes) = header.id_to_name_cached(fmt.id as u32);
            tags.push((tag_name, tag_bytes));
        }
        tags
    }

    /// Format the record as a VCF line string.
    ///
    /// Returns the record formatted as a tab-separated VCF line (without newline),
    /// or `None` if formatting fails.
    pub fn to_string(&self, header: &Header) -> Option<String> {
        let mut s = rust_htslib::htslib::kstring_t {
            l: 0,
            m: 0,
            s: std::ptr::null_mut(),
        };

        let record_ptr = self.record.inner() as *const rust_htslib::htslib::bcf1_t
            as *mut rust_htslib::htslib::bcf1_t;

        let _ = unsafe {
            rust_htslib::htslib::bcf_unpack(record_ptr, rust_htslib::htslib::BCF_UN_ALL as i32)
        };

        let ret = unsafe {
            rust_htslib::htslib::vcf_format(
                header.inner_ptr() as *const rust_htslib::htslib::bcf_hdr_t,
                record_ptr as *const rust_htslib::htslib::bcf1_t,
                &mut s,
            )
        };
        if ret != 0 {
            if !s.s.is_null() {
                unsafe { rust_htslib::htslib::free(s.s as *mut std::os::raw::c_void) };
            }
            return None;
        }

        let bytes = unsafe { std::slice::from_raw_parts(s.s as *const u8, s.l as usize) };
        let text = String::from_utf8_lossy(bytes).into_owned();

        if !s.s.is_null() {
            unsafe { rust_htslib::htslib::free(s.s as *mut std::os::raw::c_void) };
        }

        Some(text.trim_end_matches('\n').to_string())
    }
}

#[derive(Debug)]
enum InfoError {
    Absent,
    Other,
}

fn header_info_flag(header: &Header, record: &bcf::Record, tag: &[u8]) -> Result<bool, InfoError> {
    let Ok(c_str) = CString::new(tag) else {
        return Err(InfoError::Other);
    };

    let record_ptr =
        record.inner() as *const rust_htslib::htslib::bcf1_t as *mut rust_htslib::htslib::bcf1_t;

    let mut dst: *mut std::os::raw::c_void = std::ptr::null_mut();
    let mut ndst: i32 = 0;

    let ret = unsafe {
        rust_htslib::htslib::bcf_get_info_values(
            header.inner_ptr(),
            record_ptr,
            c_str.as_ptr() as *mut std::os::raw::c_char,
            &mut dst,
            &mut ndst,
            rust_htslib::htslib::BCF_HT_FLAG as i32,
        )
    };

    if !dst.is_null() {
        unsafe { rust_htslib::htslib::free(dst) };
    }

    match ret {
        -3 => Err(InfoError::Absent),
        1 => Ok(true),
        0 => Ok(false),
        _ => Err(InfoError::Other),
    }
}

fn header_info_values_i32(
    header: &Header,
    record: &bcf::Record,
    tag: &[u8],
) -> Result<Option<Vec<i32>>, InfoError> {
    header_info_values_numeric::<i32>(header, record, tag, rust_htslib::htslib::BCF_HT_INT as i32)
}

fn header_info_values_f32(
    header: &Header,
    record: &bcf::Record,
    tag: &[u8],
) -> Result<Option<Vec<f32>>, InfoError> {
    header_info_values_numeric::<f32>(header, record, tag, rust_htslib::htslib::BCF_HT_REAL as i32)
}

fn header_info_values_numeric<T: Copy + Numeric>(
    header: &Header,
    record: &bcf::Record,
    tag: &[u8],
    data_type: i32,
) -> Result<Option<Vec<T>>, InfoError> {
    let Ok(c_str) = CString::new(tag) else {
        return Err(InfoError::Other);
    };

    let record_ptr =
        record.inner() as *const rust_htslib::htslib::bcf1_t as *mut rust_htslib::htslib::bcf1_t;

    let mut dst: *mut std::os::raw::c_void = std::ptr::null_mut();
    let mut ndst: i32 = 0;

    let ret = unsafe {
        rust_htslib::htslib::bcf_get_info_values(
            header.inner_ptr(),
            record_ptr,
            c_str.as_ptr() as *mut std::os::raw::c_char,
            &mut dst,
            &mut ndst,
            data_type,
        )
    };

    match ret {
        -3 => Ok(None),
        0 => {
            if !dst.is_null() {
                unsafe { rust_htslib::htslib::free(dst) };
            }
            Ok(Some(Vec::new()))
        }
        ret if ret > 0 => {
            let slice = unsafe { std::slice::from_raw_parts(dst as *const T, ret as usize) };
            let vec = slice.to_vec();
            if !dst.is_null() {
                unsafe { rust_htslib::htslib::free(dst) };
            }
            Ok(Some(vec))
        }
        _ => {
            if !dst.is_null() {
                unsafe { rust_htslib::htslib::free(dst) };
            }
            Err(InfoError::Other)
        }
    }
}

fn header_info_values_string(
    header: &Header,
    record: &bcf::Record,
    tag: &[u8],
) -> Result<Option<Vec<Vec<u8>>>, InfoError> {
    let Ok(c_str) = CString::new(tag) else {
        return Err(InfoError::Other);
    };

    let record_ptr =
        record.inner() as *const rust_htslib::htslib::bcf1_t as *mut rust_htslib::htslib::bcf1_t;

    let mut dst: *mut std::os::raw::c_void = std::ptr::null_mut();
    let mut ndst: i32 = 0;

    let ret = unsafe {
        rust_htslib::htslib::bcf_get_info_values(
            header.inner_ptr(),
            record_ptr,
            c_str.as_ptr() as *mut std::os::raw::c_char,
            &mut dst,
            &mut ndst,
            rust_htslib::htslib::BCF_HT_STR as i32,
        )
    };

    match ret {
        -3 => Ok(None),
        0 => {
            if !dst.is_null() {
                unsafe { rust_htslib::htslib::free(dst) };
            }
            Ok(Some(Vec::new()))
        }
        ret if ret > 0 => {
            let bytes = unsafe { std::slice::from_raw_parts(dst as *const u8, ret as usize) };
            let mut out = Vec::new();
            for part in bytes.split(|c| *c == b',') {
                let part = part.split(|c| *c == 0u8).next().ok_or(InfoError::Other)?;
                out.push(part.to_vec());
            }
            if !dst.is_null() {
                unsafe { rust_htslib::htslib::free(dst) };
            }
            Ok(Some(out))
        }
        _ => {
            if !dst.is_null() {
                unsafe { rust_htslib::htslib::free(dst) };
            }
            Err(InfoError::Other)
        }
    }
}

fn numeric_to_infovalue<T: Numeric + Copy>(
    values: Option<Vec<T>>,
    tag_length: TagLength,
    scalar: impl FnOnce(T) -> InfoValue + Copy,
) -> InfoValue {
    let Some(values) = values else {
        return InfoValue::Absent;
    };

    match tag_length {
        TagLength::Fixed(1) => {
            let v = values.first().copied();
            match v {
                Some(v) if v.is_missing() => InfoValue::Missing,
                Some(v) => scalar(v),
                None => InfoValue::Missing,
            }
        }
        _ => InfoValue::Array(
            values
                .into_iter()
                .map(|v| {
                    if v.is_missing() {
                        InfoValue::Missing
                    } else {
                        scalar(v)
                    }
                })
                .collect(),
        ),
    }
}

fn string_to_infovalue(values: Option<Vec<Vec<u8>>>, tag_length: TagLength) -> InfoValue {
    let Some(values) = values else {
        return InfoValue::Absent;
    };

    match tag_length {
        TagLength::Fixed(1) => {
            let v = values
                .first()
                .map(|s| String::from_utf8_lossy(s).into_owned());
            match v {
                Some(v) if v.is_empty() => InfoValue::Missing,
                Some(v) => InfoValue::String(v),
                None => InfoValue::Missing,
            }
        }
        _ => InfoValue::Array(
            values
                .into_iter()
                .map(|v| {
                    let s = String::from_utf8_lossy(&v).into_owned();
                    if s.is_empty() {
                        InfoValue::Missing
                    } else {
                        InfoValue::String(s)
                    }
                })
                .collect(),
        ),
    }
}

fn format_numeric_to_value<T: Numeric + Copy>(
    values: &[T],
    tag_length: TagLength,
    scalar: impl FnOnce(T) -> FormatValue + Copy,
) -> FormatValue {
    match tag_length {
        TagLength::Fixed(1) => {
            let v = values.first().copied();
            match v {
                Some(v) if v.is_missing() => FormatValue::Missing,
                Some(v) => scalar(v),
                None => FormatValue::Missing,
            }
        }
        _ => FormatValue::Array(
            values
                .iter()
                .copied()
                .map(|v| {
                    if v.is_missing() {
                        FormatValue::Missing
                    } else {
                        scalar(v)
                    }
                })
                .collect(),
        ),
    }
}

fn format_string_to_value(value: &[u8], tag_length: TagLength) -> FormatValue {
    match tag_length {
        TagLength::Fixed(1) => {
            let out = String::from_utf8_lossy(value).into_owned();
            if out.is_empty() || out == "." {
                FormatValue::Missing
            } else {
                FormatValue::String(out)
            }
        }
        _ => {
            let mut parts = Vec::new();
            for part in value.split(|c| *c == b',') {
                let out = String::from_utf8_lossy(part).into_owned();
                if out.is_empty() || out == "." {
                    parts.push(FormatValue::Missing);
                } else {
                    parts.push(FormatValue::String(out));
                }
            }
            FormatValue::Array(parts)
        }
    }
}

fn format_value_for_sample(
    header: &Header,
    record: &bcf::Record,
    tag: &[u8],
    sample_id: usize,
) -> Option<FormatValue> {
    let (tag_type, tag_length) = header.format_type(tag)?;

    match tag_type {
        TagType::Integer => {
            let values = record.format(tag).integer().ok()?;
            let per_sample = values.get(sample_id)?;
            Some(format_numeric_to_value(
                per_sample,
                tag_length,
                FormatValue::Int,
            ))
        }
        TagType::Float => {
            let values = record.format(tag).float().ok()?;
            let per_sample = values.get(sample_id)?;
            Some(format_numeric_to_value(
                per_sample,
                tag_length,
                FormatValue::Float,
            ))
        }
        TagType::String => {
            let values = record.format(tag).string().ok()?;
            let per_sample = values.get(sample_id)?;
            Some(format_string_to_value(per_sample, tag_length))
        }
        TagType::Flag => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;

    #[test]
    fn sample_includes_sample_name_and_overrides_format_tag() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##FORMAT=<ID=sample_name,Number=1,Type=String,Description=\"Should not override\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tDP:sample_name\t7:EVIL\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("sample-name.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();
        let variant = Variant::from_record(rec);

        let fields = variant.sample(&header, "S1").expect("sample exists");
        let mut map = std::collections::HashMap::new();
        for (k, v) in fields {
            map.insert(k, v);
        }

        assert_eq!(map.get("DP"), Some(&FormatValue::Int(7)));
        assert_eq!(
            map.get("sample_name"),
            Some(&FormatValue::String("S1".to_string()))
        );

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_format_integer() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##FORMAT=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\tS3\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tGT:DP\t0/1:10\t1/1:20\t0/0:30\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("set-format-int.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();
        let mut variant = Variant::from_record(rec);

        // Set new DP values
        variant
            .set_format_integer(&header, "DP", &[100, 200, 300])
            .unwrap();

        // Read back and verify
        let dp = variant.format(&header, "DP");
        match dp {
            FormatValue::PerSample(vals) => {
                assert_eq!(vals.len(), 3);
                assert_eq!(vals[0], FormatValue::Int(100));
                assert_eq!(vals[1], FormatValue::Int(200));
                assert_eq!(vals[2], FormatValue::Int(300));
            }
            _ => panic!("Expected PerSample, got {:?}", dp),
        }

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_format_integer_with_missing() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tDP\t10\t20\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("set-format-int-missing.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();
        let mut variant = Variant::from_record(rec);

        // Set DP with a missing value (using sentinel)
        let missing = format_int_missing();
        variant
            .set_format_integer(&header, "DP", &[100, missing])
            .unwrap();

        let dp = variant.format(&header, "DP");
        match dp {
            FormatValue::PerSample(vals) => {
                assert_eq!(vals.len(), 2);
                assert_eq!(vals[0], FormatValue::Int(100));
                assert_eq!(vals[1], FormatValue::Missing);
            }
            _ => panic!("Expected PerSample, got {:?}", dp),
        }

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_format_float() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=AF,Number=1,Type=Float,Description=\"Allele Freq\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tAF\t0.1\t0.2\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("set-format-float.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();
        let mut variant = Variant::from_record(rec);

        variant
            .set_format_float(&header, "AF", &[0.5, 0.75])
            .unwrap();

        let af = variant.format(&header, "AF");
        match af {
            FormatValue::PerSample(vals) => {
                assert_eq!(vals.len(), 2);
                match &vals[0] {
                    FormatValue::Float(f) => assert!((f - 0.5).abs() < 0.001),
                    _ => panic!("Expected Float"),
                }
                match &vals[1] {
                    FormatValue::Float(f) => assert!((f - 0.75).abs() < 0.001),
                    _ => panic!("Expected Float"),
                }
            }
            _ => panic!("Expected PerSample, got {:?}", af),
        }

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_format_string() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=NOTE,Number=1,Type=String,Description=\"Note\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tNOTE\ta\tb\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("set-format-string.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();
        let mut variant = Variant::from_record(rec);

        variant
            .set_format_string(&header, "NOTE", &["hello".to_string(), "world".to_string()])
            .unwrap();

        let note = variant.format(&header, "NOTE");
        match note {
            FormatValue::PerSample(vals) => {
                assert_eq!(vals.len(), 2);
                assert_eq!(vals[0], FormatValue::String("hello".to_string()));
                assert_eq!(vals[1], FormatValue::String("world".to_string()));
            }
            _ => panic!("Expected PerSample, got {:?}", note),
        }

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_set_format_rejects_gt() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tGT\t0/1\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("set-format-gt.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();
        let mut variant = Variant::from_record(rec);

        // Should fail when trying to set GT
        let result = variant.set_format_string(&header, "GT", &["0/1".to_string()]);
        assert!(result.is_err());

        let _ = std::fs::remove_file(&vcf_path);
    }

    #[test]
    fn test_clear_format() {
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tDP\t10\t20\n";

        let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let vcf_path = tmp_dir.join("clear-format.vcf");
        std::fs::write(&vcf_path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
        let header = unsafe { Header::new(reader.header().inner) };

        let mut rec = reader.empty_record();
        let _ = reader.read(&mut rec).unwrap();
        let mut variant = Variant::from_record(rec);

        // Verify DP exists
        assert!(!matches!(
            variant.format(&header, "DP"),
            FormatValue::Absent
        ));

        // Clear it
        variant.clear_format(&header, "DP").unwrap();

        // Should now be absent
        assert!(matches!(variant.format(&header, "DP"), FormatValue::Absent));

        let _ = std::fs::remove_file(&vcf_path);
    }
}
