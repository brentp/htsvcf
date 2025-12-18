use crate::header;
use rust_htslib::bcf;
use rust_htslib::bcf::header::{TagLength, TagType};
use rust_htslib::bcf::record::Numeric;
use std::ffi::CString;

pub const TAG: u16 = 1;
const VARIANT_TYPE_NAME: &[u8] = b"Variant\0";

// Variant instances store a reference to the JS `header` object in an
// internal field so `variant.info()` can always see the latest header.
const HEADER_INTERNAL_FIELD_INDEX: usize = 1;

/// A single VCF/BCF record exposed to JavaScript.
///
/// The embedded `bcf::Record` is kept alive for the duration of evaluation of a
/// single iteration in [`runner::run_vcf_expr_with`].
#[derive(Debug)]
pub struct Variant {
    record: bcf::Record,
    chrom: String,
}

impl Variant {
    /// Build a `Variant` from a decoded `bcf::Record`.
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

    /// Chromosome/contig name.
    pub fn chrom(&self) -> &str {
        &self.chrom
    }

    /// Zero-based start coordinate.
    pub fn start(&self) -> i64 {
        self.record.pos()
    }

    /// One-based POS field.
    pub fn pos(&self) -> i64 {
        self.record.pos() + 1
    }

    /// End coordinate (htslib semantics).
    pub fn end(&self) -> i64 {
        self.record.end()
    }

    /// `ID` field as a string.
    pub fn id(&self) -> String {
        String::from_utf8_lossy(&self.record.id()).into_owned()
    }

    /// Reference allele.
    pub fn reference(&self) -> String {
        self.record
            .alleles()
            .first()
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .unwrap_or_else(|| ".".to_string())
    }

    /// Alternate alleles.
    pub fn alts(&self) -> Vec<String> {
        self.record
            .alleles()
            .into_iter()
            .skip(1)
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    }

    /// QUAL field, or `None` when missing.
    pub fn qual(&self) -> Option<f32> {
        let qual = self.record.qual();
        if qual.is_missing() {
            None
        } else {
            Some(qual)
        }
    }

    /// Return the FILTER column as a list of filter IDs.
    ///
    /// Records that are `PASS` (or '.') return an empty list.
    pub fn filters(&self) -> Vec<String> {
        let header = self.record.header();
        self
            .record
            .filters()
            .map(|id| String::from_utf8_lossy(&header.id_to_name(id)).into_owned())
            .collect()
    }
}

unsafe impl v8::cppgc::GarbageCollected for Variant {
    /// No-op trace because `Variant` does not reference other GC objects.
    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {}

    /// Class name shown in V8 heap snapshots.
    fn get_name(&self) -> &'static std::ffi::CStr {
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(VARIANT_TYPE_NAME) }
    }
}

/// Create the V8 `ObjectTemplate` used for all `variant` objects.
///
/// The template includes accessors for core fields plus an `info(tag)` method.
pub fn create_object_template<'a>(
    scope: &mut v8::PinScope<'a, '_>,
) -> v8::Local<'a, v8::ObjectTemplate> {
    let object_template = v8::ObjectTemplate::new(scope);
    // internal field 0: wrapped `Variant`
    // internal field 1: JS `header` object
    object_template.set_internal_field_count(2);

    for key in ["start", "pos", "stop", "chrom", "id", "ref", "alt", "qual", "filter"] {
        let name = v8::String::new(scope, key).unwrap();
        object_template.set_accessor(name.into(), attr_getter);
    }

    let info_key = v8::String::new(scope, "info").unwrap();
    let info_template = v8::FunctionTemplate::new(scope, info_fn);
    object_template.set(info_key.into(), info_template.into());

    let format_key = v8::String::new(scope, "format").unwrap();
    let format_template = v8::FunctionTemplate::new(scope, format_fn);
    object_template.set(format_key.into(), format_template.into());

    let to_string_key = v8::String::new(scope, "toString").unwrap();
    let to_string_template = v8::FunctionTemplate::new(scope, to_string_fn);
    object_template.set(to_string_key.into(), to_string_template.into());

    object_template
}

/// Instantiate a new V8 `variant` object for a particular record.
///
/// The `header_obj` is stored in an internal field so `variant.info()` can
/// resolve tag types/numbers dynamically.
pub fn create_variant_object<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    object_template: v8::Local<'a, v8::ObjectTemplate>,
    variant: Variant,
    header_obj: v8::Local<'a, v8::Object>,
) -> v8::Local<'a, v8::Object> {
    let object = object_template
        .new_instance(scope)
        .expect("failed to create Variant instance");

    let wrapper =
        unsafe { v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(), variant) };
    unsafe {
        v8::Object::wrap::<TAG, Variant>(scope, object, &wrapper);
    }

    // Store the JS header object directly so it remains GC-traced.
    object.set_internal_field(HEADER_INTERNAL_FIELD_INDEX, header_obj.into());

    object
}

/// V8 property accessor for the `variant.*` core fields.
fn attr_getter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    match key.to_rust_string_lossy(scope).as_bytes() {
        b"start" => {
            rv.set(v8::Number::new(scope, variant.start() as f64).into());
        }
        b"pos" => {
            rv.set(v8::Number::new(scope, variant.pos() as f64).into());
        }
        b"stop" => {
            rv.set(v8::Number::new(scope, variant.end() as f64).into());
        }
        b"chrom" => {
            let name_str = v8::String::new(scope, variant.chrom()).unwrap();
            rv.set(name_str.into());
        }
        b"id" => {
            let s = v8::String::new(scope, &variant.id()).unwrap();
            rv.set(s.into());
        }
        b"ref" => {
            let s = v8::String::new(scope, &variant.reference()).unwrap();
            rv.set(s.into());
        }
        b"alt" => {
            let values = variant
                .alts()
                .into_iter()
                .map(|s| v8::String::new(scope, &s).unwrap().into())
                .collect::<Vec<v8::Local<v8::Value>>>();
            let arr = v8::Array::new_with_elements(scope, &values);
            rv.set(arr.into());
        }
        b"qual" => match variant.qual() {
            Some(q) => rv.set(v8::Number::new(scope, q as f64).into()),
            None => rv.set(v8::null(scope).into()),
        },
        b"filter" => {
            let filters = variant
                .filters()
                .into_iter()
                .map(|s| v8::String::new(scope, &s).unwrap().into())
                .collect::<Vec<v8::Local<v8::Value>>>();
            let arr = v8::Array::new_with_elements(scope, &filters);
            rv.set(arr.into());
        }
        _ => {
            let message = v8::String::new(scope, "Invalid key").unwrap();
            let error = v8::Exception::error(scope, message);
            rv.set(error);
        }
    }
}

/// V8 callback for `variant.info(tag)`.
///
/// Uses the JS `header` object to resolve the tag's type and cardinality.
fn info_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    if args.length() < 1 {
        rv.set(v8::undefined(scope).into());
        return;
    }

    let tag = args.get(0);
    let Ok(tag_str) = v8::Local::<v8::String>::try_from(tag) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let tag = tag_str.to_rust_string_lossy(scope);
    let tag_bytes = tag.as_bytes();

    let Some(header_data) = this.get_internal_field(scope, HEADER_INTERNAL_FIELD_INDEX) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let Ok(header_obj) = v8::Local::<v8::Object>::try_from(header_data) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let header_wrapper =
        unsafe { v8::Object::unwrap::<{ header::HEADER_TAG }, header::Header>(scope, header_obj) }
            .expect("Failed to unwrap Header");
    let header: &header::Header = unsafe { header_wrapper.as_ref() };

    let (tag_type, tag_length) = match header.info_type(tag_bytes) {
        Some(v) => v,
        None => {
            rv.set(v8::undefined(scope).into());
            return;
        }
    };

    match tag_type {
        TagType::Flag => {
            let is_set = match header_info_flag(header, &variant.record, tag_bytes) {
                Ok(v) => v,
                Err(_) => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };
            rv.set(v8::Boolean::new(scope, is_set).into());
        }
        TagType::Integer => {
            let values = match header_info_values_i32(header, &variant.record, tag_bytes) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
                Err(_) => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };

            info_numeric_to_value(scope, &values, tag_length, &mut rv, |scope, v| {
                v8::Number::new(scope, v as f64).into()
            });
        }
        TagType::Float => {
            let values = match header_info_values_f32(header, &variant.record, tag_bytes) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
                Err(_) => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };

            info_numeric_to_value(scope, &values, tag_length, &mut rv, |scope, v| {
                v8::Number::new(scope, v as f64).into()
            });
        }
        TagType::String => {
            let values = match header_info_values_string(header, &variant.record, tag_bytes) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
                Err(_) => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };

            match tag_length {
                TagLength::Fixed(1) => {
                    let value = values
                        .first()
                        .map(|s| String::from_utf8_lossy(s).into_owned());
                    match value {
                        Some(v) => rv.set(v8::String::new(scope, &v).unwrap().into()),
                        None => rv.set(v8::null(scope).into()),
                    }
                }
                _ => {
                    let arr = v8::Array::new(scope, values.len() as i32);
                    for (i, v) in values.iter().enumerate() {
                        let v = v8::String::new(scope, &String::from_utf8_lossy(v)).unwrap();
                        arr.set_index(scope, i as u32, v.into());
                    }
                    rv.set(arr.into());
                }
            }
        }
    }
}

/// V8 callback for `variant.format(tag)`.
///
/// Uses the JS `header` object to resolve the tag's type and cardinality.
/// Returns an array with one entry per sample.
fn format_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    if args.length() < 1 {
        rv.set(v8::undefined(scope).into());
        return;
    }

    let tag = args.get(0);
    let Ok(tag_str) = v8::Local::<v8::String>::try_from(tag) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let tag = tag_str.to_rust_string_lossy(scope);
    let tag_bytes = tag.as_bytes();

    let Some(header_data) = this.get_internal_field(scope, HEADER_INTERNAL_FIELD_INDEX) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(header_obj) = v8::Local::<v8::Object>::try_from(header_data) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let header_wrapper =
        unsafe { v8::Object::unwrap::<{ header::HEADER_TAG }, header::Header>(scope, header_obj) }
            .expect("Failed to unwrap Header");
    let header: &header::Header = unsafe { header_wrapper.as_ref() };

    let (tag_type, tag_length) = match header.format_type(tag_bytes) {
        Some(v) => v,
        None => {
            rv.set(v8::undefined(scope).into());
            return;
        }
    };

    let sample_count = variant.record.sample_count() as usize;

    match tag_type {
        TagType::Integer => {
            let Ok(values) = variant.record.format(tag_bytes).integer() else {
                rv.set(v8::undefined(scope).into());
                return;
            };
            let arr = v8::Array::new(scope, sample_count as i32);
            for (i, per_sample) in values.iter().take(sample_count).enumerate() {
                match tag_length {
                    TagLength::Fixed(1) => {
                        let v = per_sample.first().copied();
                        let out = match v {
                            Some(v) if v.is_missing() => v8::null(scope).into(),
                            Some(v) => v8::Number::new(scope, v as f64).into(),
                            None => v8::null(scope).into(),
                        };
                        arr.set_index(scope, i as u32, out);
                    }
                    _ => {
                        let inner = v8::Array::new(scope, per_sample.len() as i32);
                        for (j, v) in per_sample.iter().copied().enumerate() {
                            let out = if v.is_missing() {
                                v8::null(scope).into()
                            } else {
                                v8::Number::new(scope, v as f64).into()
                            };
                            inner.set_index(scope, j as u32, out);
                        }
                        arr.set_index(scope, i as u32, inner.into());
                    }
                }
            }
            rv.set(arr.into());
        }
        TagType::Float => {
            let Ok(values) = variant.record.format(tag_bytes).float() else {
                rv.set(v8::undefined(scope).into());
                return;
            };
            let arr = v8::Array::new(scope, sample_count as i32);
            for (i, per_sample) in values.iter().take(sample_count).enumerate() {
                match tag_length {
                    TagLength::Fixed(1) => {
                        let v = per_sample.first().copied();
                        let out = match v {
                            Some(v) if v.is_missing() => v8::null(scope).into(),
                            Some(v) => v8::Number::new(scope, v as f64).into(),
                            None => v8::null(scope).into(),
                        };
                        arr.set_index(scope, i as u32, out);
                    }
                    _ => {
                        let inner = v8::Array::new(scope, per_sample.len() as i32);
                        for (j, v) in per_sample.iter().copied().enumerate() {
                            let out = if v.is_missing() {
                                v8::null(scope).into()
                            } else {
                                v8::Number::new(scope, v as f64).into()
                            };
                            inner.set_index(scope, j as u32, out);
                        }
                        arr.set_index(scope, i as u32, inner.into());
                    }
                }
            }
            rv.set(arr.into());
        }
        TagType::String => {
            let Ok(values) = variant.record.format(tag_bytes).string() else {
                rv.set(v8::undefined(scope).into());
                return;
            };
            let arr = v8::Array::new(scope, sample_count as i32);
            for (i, per_sample) in values.iter().take(sample_count).enumerate() {
                match tag_length {
                    TagLength::Fixed(1) => {
                        let s = String::from_utf8_lossy(per_sample).into_owned();
                        let out = if s.is_empty() || s == "." {
                            v8::null(scope).into()
                        } else {
                            v8::String::new(scope, &s).unwrap().into()
                        };
                        arr.set_index(scope, i as u32, out);
                    }
                    _ => {
                        let parts: Vec<_> = per_sample.split(|c| *c == b',').collect();
                        let inner = v8::Array::new(scope, parts.len() as i32);
                        for (j, part) in parts.iter().enumerate() {
                            let s = String::from_utf8_lossy(part).into_owned();
                            let out = if s.is_empty() || s == "." {
                                v8::null(scope).into()
                            } else {
                                v8::String::new(scope, &s).unwrap().into()
                            };
                            inner.set_index(scope, j as u32, out);
                        }
                        arr.set_index(scope, i as u32, inner.into());
                    }
                }
            }
            rv.set(arr.into());
        }
        TagType::Flag => {
            // Flag isn't valid for FORMAT in practice; expose undefined.
            rv.set(v8::undefined(scope).into());
        }
    }
}

/// V8 callback for `variant.toString()`. 
fn to_string_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    let Some(header_data) = this.get_internal_field(scope, HEADER_INTERNAL_FIELD_INDEX) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(header_obj) = v8::Local::<v8::Object>::try_from(header_data) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let header_wrapper =
        unsafe { v8::Object::unwrap::<{ header::HEADER_TAG }, header::Header>(scope, header_obj) }
            .expect("Failed to unwrap Header");
    let header: &header::Header = unsafe { header_wrapper.as_ref() };

    let mut s = rust_htslib::htslib::kstring_t {
        l: 0,
        m: 0,
        s: std::ptr::null_mut(),
    };

    // bcf_unpack wants a mutable `bcf1_t*` (it mutates in-place).
    let record_ptr = variant.record.inner() as *const rust_htslib::htslib::bcf1_t
        as *mut rust_htslib::htslib::bcf1_t;

    // vcf_format expects an unpacked record.
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
        rv.set(v8::undefined(scope).into());
        return;
    }

    let bytes = unsafe { std::slice::from_raw_parts(s.s as *const u8, s.l as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();

    if !s.s.is_null() {
        unsafe { rust_htslib::htslib::free(s.s as *mut std::os::raw::c_void) };
    }

    let out = v8::String::new(scope, text.trim_end_matches('\n')).unwrap();
    rv.set(out.into());
}

/// Read an INFO/Flag tag from a record.
fn header_info_flag(header: &header::Header, record: &bcf::Record, tag: &[u8]) -> Result<bool, ()> {
    let Ok(c_str) = CString::new(tag) else {
        return Err(());
    };

    // bcf_get_info_values wants a mutable bcf1_t*, but does not mutate the record.
    // rust-htslib does not expose a mutable pointer from an immutable borrow, so
    // we cast here.
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
        -3 => Ok(false),
        1 => Ok(true),
        _ => Err(()),
    }
}

/// Read an INFO/Integer tag as i32 values.
fn header_info_values_i32(
    header: &header::Header,
    record: &bcf::Record,
    tag: &[u8],
) -> Result<Option<Vec<i32>>, ()> {
    header_info_values_numeric::<i32>(header, record, tag, rust_htslib::htslib::BCF_HT_INT as i32)
}

/// Read an INFO/Float tag as f32 values.
fn header_info_values_f32(
    header: &header::Header,
    record: &bcf::Record,
    tag: &[u8],
) -> Result<Option<Vec<f32>>, ()> {
    header_info_values_numeric::<f32>(header, record, tag, rust_htslib::htslib::BCF_HT_REAL as i32)
}

/// Shared implementation for numeric INFO values.
fn header_info_values_numeric<T: Copy + Numeric>(
    header: &header::Header,
    record: &bcf::Record,
    tag: &[u8],
    data_type: i32,
) -> Result<Option<Vec<T>>, ()> {
    let Ok(c_str) = CString::new(tag) else {
        return Err(());
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
            Err(())
        }
    }
}

/// Read an INFO/String tag as a list of byte strings.
fn header_info_values_string(
    header: &header::Header,
    record: &bcf::Record,
    tag: &[u8],
) -> Result<Option<Vec<Vec<u8>>>, ()> {
    let Ok(c_str) = CString::new(tag) else {
        return Err(());
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
                // stop at zero character
                let part = part.split(|c| *c == 0u8).next().ok_or(())?;
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
            Err(())
        }
    }
}

/// Convert scalar/array numeric INFO values into a V8 value.
fn info_numeric_to_value<'s, 'i, T: Numeric + Copy>(
    scope: &mut v8::PinScope<'s, 'i>,
    values: &[T],
    tag_length: TagLength,
    rv: &mut v8::ReturnValue,
    mut to_value: impl FnMut(&mut v8::PinScope<'s, 'i>, T) -> v8::Local<'s, v8::Value>,
) {
    match tag_length {
        TagLength::Fixed(1) => {
            let value = values.iter().next().copied();
            match value {
                Some(v) if v.is_missing() => rv.set(v8::null(scope).into()),
                Some(v) => rv.set(to_value(scope, v)),
                None => rv.set(v8::null(scope).into()),
            }
        }
        _ => {
            let arr = v8::Array::new(scope, values.len() as i32);
            for (i, v) in values.iter().copied().enumerate() {
                let v = if v.is_missing() {
                    v8::null(scope).into()
                } else {
                    to_value(scope, v)
                };
                arr.set_index(scope, i as u32, v);
            }
            rv.set(arr.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;
    use std::fs;
    use std::path::PathBuf;

    fn fixture_vcf() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/t.vcf.gz")
            .to_string_lossy()
            .into_owned()
    }

    /// Evaluate JS against the first record and return stringified result.
    fn eval_js(path: &str, js_expr: &str) -> String {
        let platform = crate::runtime::ensure_v8_initialized().clone();
        let _guard = crate::runtime::v8_lock();

        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

        v8::scope!(handle_scope, isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        let object_template_local = create_object_template(scope);
        let object_template = v8::Global::new(scope, object_template_local);

        let mut reader = bcf::Reader::from_path(path).unwrap();
        let record = reader.records().next().unwrap().unwrap();
        let record = Variant::from_record(record);

        let header_obj = crate::header::create_header_object(
            scope,
            crate::header::Header::new(reader.header().inner),
        );

        let code = v8::String::new(scope, js_expr).unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();

        let global = context.global(scope);

        let header_name = v8::String::new(scope, "header").unwrap();
        global.set(scope, header_name.into(), header_obj.into());
        let header_obj = global
            .get(scope, header_name.into())
            .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
            .expect("header object missing");

        let variant_name = v8::String::new(scope, "variant").unwrap();
        let object_template = v8::Local::new(scope, &object_template);
        let variant_object = create_variant_object(scope, object_template, record, header_obj);
        global.set(scope, variant_name.into(), variant_object.into());

        let result = script.run(scope).unwrap();
        result.to_string(scope).unwrap().to_rust_string_lossy(scope)
    }

    /// Create a unique temp path for tests.
    fn tmp_path(file_name: &str) -> PathBuf {
        // Keep temp files isolated per-test.
        let mut path = std::env::temp_dir();
        path.push(format!(
            "htsvcf_{}_{}_{}",
            std::process::id(),
            file_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }

    #[test]
    /// Validate basic Rust-side `Variant` accessors.
    fn test_variant_basic_fields() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let record = reader.records().next().unwrap().unwrap();
        let variant = Variant::from_record(record);

        assert_eq!(variant.chrom(), "chr1");
        assert_eq!(variant.pos(), 1000);
        assert_eq!(variant.start(), 999);
        assert_eq!(variant.end(), 1000);
        assert_eq!(variant.id(), ".");
        assert_eq!(variant.reference(), "A");
        assert_eq!(variant.alts(), vec!["C".to_string()]);
    }

    #[test]
    /// `variant.info()` should return scalar values where appropriate.
    fn test_js_info_scalar_and_array() {
        let path = fixture_vcf();
        assert_eq!(eval_js(&path, "variant.info('DP')"), "10");
        assert_eq!(eval_js(&path, "variant.info('NOPE')"), "undefined");
    }

    #[test]
    /// V8 accessors expose core VCF fields.
    fn test_js_variant_attributes() {
        let path = fixture_vcf();
        assert_eq!(eval_js(&path, "variant.chrom"), "chr1");
        assert_eq!(eval_js(&path, "variant.pos"), "1000");
        assert_eq!(eval_js(&path, "variant.start"), "999");
        assert_eq!(eval_js(&path, "variant.stop"), "1000");
        assert_eq!(eval_js(&path, "variant.ref"), "A");
        assert_eq!(eval_js(&path, "variant.alt.length"), "1");
        assert_eq!(eval_js(&path, "variant.alt[0]"), "C");
        assert_eq!(eval_js(&path, "variant.id"), ".");
        assert_eq!(eval_js(&path, "variant.qual === null"), "true");
        assert_eq!(eval_js(&path, "Array.isArray(variant.filter)"), "true");
        assert_eq!(eval_js(&path, "variant.filter.length"), "0");
    }

    #[test]
    /// `variant.info()` uses header type/number semantics.
    fn test_variant_info_uses_header_type_and_number() {
        let path = tmp_path("info.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
  ##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
  ##INFO=<ID=AF,Number=2,Type=Float,Description=\"Allele frequencies\">\n\
  ##INFO=<ID=NOTE,Number=1,Type=String,Description=\"Note\">\n\
  ##INFO=<ID=FLAGS,Number=.,Type=String,Description=\"Flags\">\n\
  ##INFO=<ID=SOMATIC,Number=0,Type=Flag,Description=\"Somatic\">\n\
  ##contig=<ID=chr1>\n\
  #CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n\
  chr1\t1\t.\tA\tC,G\t.\t.\tDP=7;AF=0.1,0.2;NOTE=hi;FLAGS=a,b,c;SOMATIC\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_js(path, "variant.info('DP')"), "7");
        assert_eq!(eval_js(path, "variant.info('AF').length"), "2");
        assert_eq!(
            eval_js(path, "Math.abs(variant.info('AF')[0] - 0.1) < 1e-6",),
            "true"
        );
        assert_eq!(
            eval_js(path, "Math.abs(variant.info('AF')[1] - 0.2) < 1e-6",),
            "true"
        );
        assert_eq!(eval_js(path, "variant.info('NOTE')"), "hi");
        assert_eq!(eval_js(path, "variant.info('FLAGS').length"), "3");
        assert_eq!(eval_js(path, "variant.info('FLAGS')[2]"), "c");
        assert_eq!(eval_js(path, "variant.info('SOMATIC')"), "true");

        let _ = fs::remove_file(path);
    }

    #[test]
    /// `variant.format()` should return per-sample typed values.
    fn test_variant_format_per_sample() {
        let path = tmp_path("format.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##FORMAT=<ID=AD,Number=2,Type=Integer,Description=\"Allele Depths\">\n\
##FORMAT=<ID=AF,Number=2,Type=Float,Description=\"Allele Frequencies\">\n\
##FORMAT=<ID=NOTE,Number=1,Type=String,Description=\"Note\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\n\
chr1\t1\t.\tA\tC,G\t.\t.\t.\tDP:AD:AF:NOTE\t7:1,2:0.1,0.2:hi\t.:.,.:.,.:.\n";

        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_js(path, "variant.format('DP').length"), "2");
        assert_eq!(eval_js(path, "variant.format('DP')[0]"), "7");
        assert_eq!(eval_js(path, "variant.format('DP')[1] === null"), "true");

        assert_eq!(eval_js(path, "variant.format('AD')[0].length"), "2");
        assert_eq!(eval_js(path, "variant.format('AD')[0][1]"), "2");
        assert_eq!(eval_js(path, "variant.format('AD')[1][0] === null"), "true");

        assert_eq!(
            eval_js(path, "Math.abs(variant.format('AF')[0][0] - 0.1) < 1e-6"),
            "true"
        );
        assert_eq!(
            eval_js(path, "Math.abs(variant.format('AF')[0][1] - 0.2) < 1e-6"),
            "true"
        );

        assert_eq!(eval_js(path, "variant.format('NOTE')[0]"), "hi");
        assert_eq!(eval_js(path, "variant.format('NOTE')[1] === null"), "true");
        assert_eq!(eval_js(path, "variant.format('NOPE')"), "undefined");

        let _ = fs::remove_file(path);
    }

    #[test]
    /// `variant.toString()` should return the formatted VCF line.
    fn test_variant_to_string() {
        let path = fixture_vcf();

        assert_eq!(
            eval_js(&path, "variant.toString().startsWith('chr1\t1000')"),
            "true"
        );
        assert_eq!(
            eval_js(&path, "variant.toString().includes('\tA\tC')"),
            "true"
        );
        assert_eq!(
            eval_js(&path, "variant.toString().includes('DP=10')"),
            "true"
        );
        assert_eq!(
            eval_js(&path, "variant.toString().endsWith('\\n')"),
            "false"
        );
    }
}

