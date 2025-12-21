//! V8-based `Variant` object representing a single VCF/BCF record.
//!
//! This module exposes VCF record data to JavaScript, providing typed access
//! to all standard VCF fields (CHROM, POS, REF, ALT, QUAL, FILTER, INFO, FORMAT).
//!
//! # JavaScript Usage
//!
//! The `variant` global represents the current record in expression evaluation:
//!
//! ```js
//! // Read-only fields
//! variant.chrom      // "chr1"
//! variant.pos        // 12345 (1-based)
//! variant.start      // 12344 (0-based)
//! variant.stop       // end position
//! variant.ref        // "A"
//! variant.alt        // ["G", "T"]
//!
//! // Read/write fields
//! variant.id = "rs12345"
//! variant.qual = 30.0
//! variant.filter = ["PASS"]
//!
//! // INFO access (typed by header definition)
//! variant.info('DP')           // => 42
//! variant.set_info('DP', 100)
//! variant.set_info('DP', null) // clear
//!
//! // FORMAT access (per-sample arrays)
//! variant.format('GT')         // => ["0/1", "0/0"]
//! variant.sample('NA12878')    // => { GT: "0/1", DP: 30, ... }
//! variant.samples()            // => [{ GT: "0/1", ... }, ...]
//!
//! // Output
//! variant.toString()           // full VCF line
//! ```

use crate::header;
use htsvcf_core::{FormatValue, Genotype, InfoValue};
use rust_htslib::bcf;
use rust_htslib::bcf::header::TagType;
use rust_htslib::bcf::record::Numeric;

// ============================================================================
// Conversion functions from core types to V8 values
// ============================================================================

/// Convert a Genotype from the core to a V8 object.
///
/// Returns an object with:
/// - `alleles`: Array of numbers (or null for missing)
/// - `phase`: Array of booleans
fn genotype_to_v8<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    gt: &Genotype,
) -> v8::Local<'s, v8::Value> {
    let obj = v8::Object::new(scope);

    // Build alleles array
    let alleles_arr = v8::Array::new(scope, gt.alleles.len() as i32);
    for (i, allele) in gt.alleles.iter().enumerate() {
        let val: v8::Local<v8::Value> = match allele {
            Some(n) => v8::Number::new(scope, *n as f64).into(),
            None => v8::null(scope).into(),
        };
        alleles_arr.set_index(scope, i as u32, val);
    }

    // Build phase array
    let phase_arr = v8::Array::new(scope, gt.phase.len() as i32);
    for (i, p) in gt.phase.iter().enumerate() {
        let val: v8::Local<v8::Value> = v8::Boolean::new(scope, *p).into();
        phase_arr.set_index(scope, i as u32, val);
    }

    let alleles_key = v8::String::new(scope, "alleles").unwrap();
    let phase_key = v8::String::new(scope, "phase").unwrap();
    obj.set(scope, alleles_key.into(), alleles_arr.into());
    obj.set(scope, phase_key.into(), phase_arr.into());

    obj.into()
}

/// Convert an InfoValue from the core to a V8 value.
fn infovalue_to_v8<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    v: &InfoValue,
) -> v8::Local<'s, v8::Value> {
    match v {
        InfoValue::Absent => v8::undefined(scope).into(),
        InfoValue::Missing => v8::null(scope).into(),
        InfoValue::Bool(b) => v8::Boolean::new(scope, *b).into(),
        InfoValue::Int(i) => v8::Number::new(scope, *i as f64).into(),
        InfoValue::Float(f) => v8::Number::new(scope, *f as f64).into(),
        InfoValue::String(s) => v8::String::new(scope, s).unwrap().into(),
        InfoValue::Array(values) => {
            let arr = v8::Array::new(scope, values.len() as i32);
            for (i, v) in values.iter().enumerate() {
                let js_val = infovalue_to_v8(scope, v);
                arr.set_index(scope, i as u32, js_val);
            }
            arr.into()
        }
    }
}

/// Convert a FormatValue from the core to a V8 value.
fn formatvalue_to_v8<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    v: &FormatValue,
) -> v8::Local<'s, v8::Value> {
    match v {
        FormatValue::Absent => v8::undefined(scope).into(),
        FormatValue::Missing => v8::null(scope).into(),
        FormatValue::Int(i) => v8::Number::new(scope, *i as f64).into(),
        FormatValue::Float(f) => v8::Number::new(scope, *f as f64).into(),
        FormatValue::String(s) => v8::String::new(scope, s).unwrap().into(),
        FormatValue::Array(values) => {
            let arr = v8::Array::new(scope, values.len() as i32);
            for (i, v) in values.iter().enumerate() {
                let js_val = formatvalue_to_v8(scope, v);
                arr.set_index(scope, i as u32, js_val);
            }
            arr.into()
        }
        FormatValue::PerSample(values) => {
            let arr = v8::Array::new(scope, values.len() as i32);
            for (i, v) in values.iter().enumerate() {
                let js_val = formatvalue_to_v8(scope, v);
                arr.set_index(scope, i as u32, js_val);
            }
            arr.into()
        }
        FormatValue::Genotype(gt) => genotype_to_v8(scope, gt),
    }
}

pub const TAG: u16 = 1;
const VARIANT_TYPE_NAME: &[u8] = b"Variant\0";

// Variant instances store a reference to the JS `header` object in an
// internal field so `variant.info()` can always see the latest header.
const HEADER_INTERNAL_FIELD_INDEX: usize = 1;

/// A single VCF/BCF record exposed to JavaScript.
///
/// The embedded `bcf::Record` is kept alive for the duration of evaluation of a
/// single iteration in [`crate::runner::run_vcf_expr_with`].
#[derive(Debug)]
pub struct Variant {
    record: v8::cppgc::GcCell<Option<bcf::Record>>,
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
        Self {
            record: v8::cppgc::GcCell::new(Some(record)),
            chrom,
        }
    }

    /// Get a reference to the record, panicking if taken.
    #[inline]
    fn record<'a>(&'a self, scope: &'a v8::PinScope<'_, '_>) -> &'a bcf::Record {
        self.record
            .get(scope)
            .as_ref()
            .expect("record was taken")
    }

    /// Get a mutable reference to the record, panicking if taken.
    #[inline]
    fn record_mut<'a>(&'a self, scope: &'a mut v8::PinScope<'_, '_>) -> &'a mut bcf::Record {
        self.record
            .get_mut(scope)
            .as_mut()
            .expect("record was taken")
    }

    /// Take ownership of the record, leaving None.
    pub fn take_record(&self, scope: &mut v8::PinScope<'_, '_>) -> Option<bcf::Record> {
        self.record.get_mut(scope).take()
    }

    /// Check if the record is still present (not taken).
    pub fn has_record(&self, scope: &v8::PinScope<'_, '_>) -> bool {
        self.record.get(scope).is_some()
    }

    /// Chromosome/contig name.
    pub fn chrom(&self) -> &str {
        &self.chrom
    }

    /// Zero-based start coordinate.
    pub fn start(&self, scope: &v8::PinScope<'_, '_>) -> i64 {
        self.record(scope).pos()
    }

    /// One-based POS field.
    pub fn pos(&self, scope: &v8::PinScope<'_, '_>) -> i64 {
        self.record(scope).pos() + 1
    }

    /// End coordinate (htslib semantics).
    pub fn end(&self, scope: &v8::PinScope<'_, '_>) -> i64 {
        self.record(scope).end()
    }

    /// `ID` field as a string.
    pub fn id(&self, scope: &v8::PinScope<'_, '_>) -> String {
        String::from_utf8_lossy(&self.record(scope).id()).into_owned()
    }

    /// Set the `ID` field.
    ///
    /// Pass an empty string to clear the ID (sets to ".").
    pub fn set_id(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: &str,
    ) -> Result<(), rust_htslib::errors::Error> {
        let id = if id.is_empty() { "." } else { id };
        let record = self.record_mut(scope);
        record.set_id(id.as_bytes())?;
        record.unpack();
        Ok(())
    }

    /// Reference allele.
    pub fn reference(&self, scope: &v8::PinScope<'_, '_>) -> String {
        self.record(scope)
            .alleles()
            .first()
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .unwrap_or_else(|| ".".to_string())
    }

    /// Alternate alleles.
    pub fn alts(&self, scope: &v8::PinScope<'_, '_>) -> Vec<String> {
        self.record(scope)
            .alleles()
            .into_iter()
            .skip(1)
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    }

    /// QUAL field, or `None` when missing.
    pub fn qual(&self, scope: &v8::PinScope<'_, '_>) -> Option<f32> {
        let qual = self.record(scope).qual();
        if qual.is_missing() {
            None
        } else {
            Some(qual)
        }
    }

    /// Set the QUAL field.
    ///
    /// Pass `None` to set QUAL to missing.
    pub fn set_qual(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        qual: Option<f32>,
    ) {
        let record = self.record_mut(scope);
        match qual {
            Some(v) => record.set_qual(v),
            None => record.set_qual(<f32 as Numeric>::missing()),
        }
    }

    /// Return the FILTER column as a list of filter IDs.
    ///
    /// Records that are '' (or '.') return an empty list.
    pub fn filters(&self, scope: &v8::PinScope<'_, '_>) -> Vec<String> {
        let record = self.record(scope);
        let header = record.header();
        let mut out = Vec::new();
        for id in record.filters() {
            let name = String::from_utf8_lossy(&header.id_to_name(id)).into_owned();
            out.push(name);
        }
        out
    }

    /// Set the FILTER column.
    ///
    /// Pass an empty slice, `[""]`, or `["."]` to clear all filters.
    pub fn set_filters(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        filters: &[String],
    ) -> Result<(), rust_htslib::errors::Error> {
        let record = self.record_mut(scope);
        let want_clear = filters.is_empty() || (filters.len() == 1 && (filters[0].is_empty() || filters[0] == "."));
        if want_clear {
            let refs: Vec<&[u8]> = Vec::new();
            record.set_filters(&refs)?;
            record.unpack();
            return Ok(());
        }

        let refs: Vec<&[u8]> = filters.iter().map(|s| s.as_bytes()).collect();
        record.set_filters(&refs)?;
        record.unpack();
        Ok(())
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

    for key in ["start", "pos", "stop", "chrom", "ref", "alt"] {
        let name = v8::String::new(scope, key).unwrap();
        object_template.set_accessor(name.into(), attr_getter);
    }

    for key in ["id", "qual", "filter"] {
        let name = v8::String::new(scope, key).unwrap();
        object_template.set_accessor_with_setter(name.into(), attr_getter, attr_setter);
    }

    let info_key = v8::String::new(scope, "info").unwrap();
    let info_template = v8::FunctionTemplate::new(scope, info_fn);
    object_template.set(info_key.into(), info_template.into());

    let set_info_key = v8::String::new(scope, "set_info").unwrap();
    let set_info_template = v8::FunctionTemplate::new(scope, set_info_fn);
    object_template.set(set_info_key.into(), set_info_template.into());

    let translate_key = v8::String::new(scope, "translate").unwrap();
    let translate_template = v8::FunctionTemplate::new(scope, translate_fn);
    object_template.set(translate_key.into(), translate_template.into());

    let format_key = v8::String::new(scope, "format").unwrap();
    let format_template = v8::FunctionTemplate::new(scope, format_fn);
    object_template.set(format_key.into(), format_template.into());

    let sample_key = v8::String::new(scope, "sample").unwrap();
    let sample_template = v8::FunctionTemplate::new(scope, sample_fn);
    object_template.set(sample_key.into(), sample_template.into());

    let samples_key = v8::String::new(scope, "samples").unwrap();
    let samples_template = v8::FunctionTemplate::new(scope, samples_fn);
    object_template.set(samples_key.into(), samples_template.into());

    let genotypes_key = v8::String::new(scope, "genotypes").unwrap();
    let genotypes_template = v8::FunctionTemplate::new(scope, genotypes_fn);
    object_template.set(genotypes_key.into(), genotypes_template.into());

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
        b"start" => rv.set(v8::Number::new(scope, variant.start(scope) as f64).into()),
        b"pos" => rv.set(v8::Number::new(scope, variant.pos(scope) as f64).into()),
        b"stop" => rv.set(v8::Number::new(scope, variant.end(scope) as f64).into()),
        b"chrom" => {
            let name_str = v8::String::new(scope, variant.chrom()).unwrap();
            rv.set(name_str.into());
        }
        b"id" => {
            let s = v8::String::new(scope, &variant.id(scope)).unwrap();
            rv.set(s.into());
        }
        b"ref" => {
            let s = v8::String::new(scope, &variant.reference(scope)).unwrap();
            rv.set(s.into());
        }
        b"alt" => {
            let values = variant
                .alts(scope)
                .into_iter()
                .map(|s| v8::String::new(scope, &s).unwrap().into())
                .collect::<Vec<v8::Local<v8::Value>>>();
            let arr = v8::Array::new_with_elements(scope, &values);
            rv.set(arr.into());
        }
        b"qual" => match variant.qual(scope) {
            Some(q) => rv.set(v8::Number::new(scope, q as f64).into()),
            None => rv.set(v8::null(scope).into()),
        },
        b"filter" => {
            let filters = variant
                .filters(scope)
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

fn attr_setter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<v8::Name>,
    value: v8::Local<v8::Value>,
    args: v8::PropertyCallbackArguments,
    mut _rv: v8::ReturnValue<()>,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    match key.to_rust_string_lossy(scope).as_bytes() {
        b"id" => {
            let Ok(v) = v8::Local::<v8::String>::try_from(value) else {
                let msg = v8::String::new(scope, "variant.id must be a string").unwrap();
                scope.throw_exception(v8::Exception::type_error(scope, msg));
                return;
            };
            let id = v.to_rust_string_lossy(scope);
            if let Err(e) = variant.set_id(scope, &id) {
                let msg = v8::String::new(scope, &format!("failed to set id: {e}"))
                    .unwrap();
                scope.throw_exception(v8::Exception::error(scope, msg));
            }
        }
        b"qual" => {
            if value.is_null_or_undefined() {
                variant.set_qual(scope, None);
                return;
            }
            let Ok(v) = v8::Local::<v8::Number>::try_from(value) else {
                let msg = v8::String::new(scope, "variant.qual must be a number or null")
                    .unwrap();
                scope.throw_exception(v8::Exception::type_error(scope, msg));
                return;
            };
            variant.set_qual(scope, Some(v.value() as f32));
        }
        b"filter" => {
            let Ok(arr) = v8::Local::<v8::Array>::try_from(value) else {
                let msg = v8::String::new(scope, "variant.filter must be an array of strings")
                    .unwrap();
                scope.throw_exception(v8::Exception::type_error(scope, msg));
                return;
            };

            let mut filters = Vec::with_capacity(arr.length() as usize);
            for i in 0..arr.length() {
                let Some(v) = arr.get_index(scope, i) else {
                    continue;
                };
                let Ok(s) = v8::Local::<v8::String>::try_from(v) else {
                    let msg = v8::String::new(scope, "variant.filter must be an array of strings")
                        .unwrap();
                    scope.throw_exception(v8::Exception::type_error(scope, msg));
                    return;
                };
                filters.push(s.to_rust_string_lossy(scope));
            }

            if let Err(e) = variant.set_filters(scope, &filters) {
                let msg = v8::String::new(scope, &format!("failed to set filter: {e}"))
                    .unwrap();
                scope.throw_exception(v8::Exception::error(scope, msg));
            }
        }
        _ => {
            let msg = v8::String::new(scope, "Invalid key").unwrap();
            scope.throw_exception(v8::Exception::error(scope, msg));
        }
    }
}

/// V8 callback for `variant.info(tag)`.
///
/// Uses the JS `header` object to resolve the tag's type and cardinality.
fn translate_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let type_error = |scope: &mut v8::PinScope<'_, '_>, msg: &str| {
        let msg = v8::String::new(scope, msg).unwrap();
        scope.throw_exception(v8::Exception::type_error(scope, msg));
    };
    let error = |scope: &mut v8::PinScope<'_, '_>, msg: &str| {
        let msg = v8::String::new(scope, msg).unwrap();
        scope.throw_exception(v8::Exception::error(scope, msg));
    };

    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    if args.length() != 1 {
        type_error(scope, "variant.translate(header) requires 1 argument");
        return;
    }

    let header_arg = args.get(0);
    let Ok(header_obj) = v8::Local::<v8::Object>::try_from(header_arg) else {
        type_error(scope, "variant.translate(header) expects a Header");
        return;
    };

    let header_wrapper =
        unsafe { v8::Object::unwrap::<{ header::HEADER_TAG }, header::Header>(scope, header_obj) };
    let Some(header_wrapper) = header_wrapper else {
        type_error(scope, "variant.translate(header) expects a Header");
        return;
    };
    let header: &header::Header = unsafe { header_wrapper.as_ref() };

    // Use the core Header's translation view to avoid header duplication.
    // This view is intentionally non-dropping to prevent double-free.
    let mut rust_header = header.inner().translate_view();

    let record = variant.record_mut(scope);
    if let Err(e) = record.translate(&mut rust_header) {
        error(scope, &format!("translate failed: {e}"));
        return;
    }

    // Only update JS-visible header after successful translation so future
    // `set_info` uses the new schema. We do this after translate succeeds to
    // keep the variant in a consistent state if translation fails.
    this.set_internal_field(HEADER_INTERNAL_FIELD_INDEX, header_obj.into());
}

fn set_info_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    if args.length() < 2 {
        let msg = v8::String::new(scope, "variant.set_info(tag, value) requires 2 arguments").unwrap();
        scope.throw_exception(v8::Exception::type_error(scope, msg));
        return;
    }

    let tag = args.get(0);
    let Ok(tag_str) = v8::Local::<v8::String>::try_from(tag) else {
        let msg = v8::String::new(scope, "variant.set_info tag must be a string").unwrap();
        scope.throw_exception(v8::Exception::type_error(scope, msg));
        return;
    };
    let tag = tag_str.to_rust_string_lossy(scope);
    let tag_bytes = tag.as_bytes();

    let Some(header_data) = this.get_internal_field(scope, HEADER_INTERNAL_FIELD_INDEX) else {
        let msg = v8::String::new(scope, "variant has no header").unwrap();
        scope.throw_exception(v8::Exception::error(scope, msg));
        return;
    };
    let Ok(header_obj) = v8::Local::<v8::Object>::try_from(header_data) else {
        let msg = v8::String::new(scope, "variant has invalid header").unwrap();
        scope.throw_exception(v8::Exception::error(scope, msg));
        return;
    };

    let header_wrapper =
        unsafe { v8::Object::unwrap::<{ header::HEADER_TAG }, header::Header>(scope, header_obj) }
            .expect("Failed to unwrap Header");
    let header: &header::Header = unsafe { header_wrapper.as_ref() };

    let (tag_type, _tag_length) = match header.info_type(tag_bytes) {
        Some(v) => v,
        None => {
            let msg = v8::String::new(scope, &format!("undefined INFO tag: {tag}")).unwrap();
            scope.throw_exception(v8::Exception::error(scope, msg));
            return;
        }
    };

    let value = args.get(1);

    enum InfoWriteValue {
        Clear,
        Flag(bool),
        Integers(Vec<i32>),
        Floats(Vec<f32>),
        Strings(Vec<String>),
    }

    let parsed = if value.is_null_or_undefined() {
        InfoWriteValue::Clear
    } else {
        match tag_type {
            TagType::Flag => {
                let Ok(b) = v8::Local::<v8::Boolean>::try_from(value) else {
                    let msg = v8::String::new(scope, "flag INFO requires boolean value").unwrap();
                    scope.throw_exception(v8::Exception::type_error(scope, msg));
                    return;
                };
                InfoWriteValue::Flag(b.is_true())
            }
            TagType::Integer => {
                if let Ok(arr) = v8::Local::<v8::Array>::try_from(value) {
                    let mut out = Vec::with_capacity(arr.length() as usize);
                    for i in 0..arr.length() {
                        let Some(v) = arr.get_index(scope, i) else {
                            continue;
                        };
                        let Ok(n) = v8::Local::<v8::Number>::try_from(v) else {
                            let msg = v8::String::new(scope, "integer INFO requires number array").unwrap();
                            scope.throw_exception(v8::Exception::type_error(scope, msg));
                            return;
                        };
                        let f = n.value();
                        if !f.is_finite() || (f.fract() != 0.0) {
                            let msg = v8::String::new(scope, "integer INFO values must be integers").unwrap();
                            scope.throw_exception(v8::Exception::type_error(scope, msg));
                            return;
                        }
                        out.push(f as i32);
                    }
                    InfoWriteValue::Integers(out)
                } else {
                    let Ok(n) = v8::Local::<v8::Number>::try_from(value) else {
                        let msg = v8::String::new(scope, "integer INFO requires number value").unwrap();
                        scope.throw_exception(v8::Exception::type_error(scope, msg));
                        return;
                    };
                    let f = n.value();
                    if !f.is_finite() || (f.fract() != 0.0) {
                        let msg = v8::String::new(scope, "integer INFO value must be an integer").unwrap();
                        scope.throw_exception(v8::Exception::type_error(scope, msg));
                        return;
                    }
                    InfoWriteValue::Integers(vec![f as i32])
                }
            }
            TagType::Float => {
                if let Ok(arr) = v8::Local::<v8::Array>::try_from(value) {
                    let mut out = Vec::with_capacity(arr.length() as usize);
                    for i in 0..arr.length() {
                        let Some(v) = arr.get_index(scope, i) else {
                            continue;
                        };
                        let Ok(n) = v8::Local::<v8::Number>::try_from(v) else {
                            let msg = v8::String::new(scope, "float INFO requires number array").unwrap();
                            scope.throw_exception(v8::Exception::type_error(scope, msg));
                            return;
                        };
                        let f = n.value();
                        if !f.is_finite() {
                            let msg = v8::String::new(scope, "float INFO values must be finite").unwrap();
                            scope.throw_exception(v8::Exception::type_error(scope, msg));
                            return;
                        }
                        out.push(f as f32);
                    }
                    InfoWriteValue::Floats(out)
                } else {
                    let Ok(n) = v8::Local::<v8::Number>::try_from(value) else {
                        let msg = v8::String::new(scope, "float INFO requires number value").unwrap();
                        scope.throw_exception(v8::Exception::type_error(scope, msg));
                        return;
                    };
                    let f = n.value();
                    if !f.is_finite() {
                        let msg = v8::String::new(scope, "float INFO value must be finite").unwrap();
                        scope.throw_exception(v8::Exception::type_error(scope, msg));
                        return;
                    }
                    InfoWriteValue::Floats(vec![f as f32])
                }
            }
            TagType::String => {
                if let Ok(arr) = v8::Local::<v8::Array>::try_from(value) {
                    let mut strings = Vec::with_capacity(arr.length() as usize);
                    for i in 0..arr.length() {
                        let Some(v) = arr.get_index(scope, i) else {
                            continue;
                        };
                        let Ok(s) = v8::Local::<v8::String>::try_from(v) else {
                            let msg = v8::String::new(scope, "string INFO requires string array").unwrap();
                            scope.throw_exception(v8::Exception::type_error(scope, msg));
                            return;
                        };
                        strings.push(s.to_rust_string_lossy(scope));
                    }
                    InfoWriteValue::Strings(strings)
                } else {
                    let Ok(s) = v8::Local::<v8::String>::try_from(value) else {
                        let msg = v8::String::new(scope, "string INFO requires string value").unwrap();
                        scope.throw_exception(v8::Exception::type_error(scope, msg));
                        return;
                    };
                    InfoWriteValue::Strings(vec![s.to_rust_string_lossy(scope)])
                }
            }
        }
    };

    let res = {
        let record = variant.record_mut(scope);
        let out = match (tag_type, parsed) {
            (_, InfoWriteValue::Clear) => match tag_type {
                TagType::Flag => record.clear_info_flag(tag_bytes),
                TagType::Integer => record.clear_info_integer(tag_bytes),
                TagType::Float => record.clear_info_float(tag_bytes),
                TagType::String => record.clear_info_string(tag_bytes),
            },
            (TagType::Flag, InfoWriteValue::Flag(true)) => record.push_info_flag(tag_bytes),
            (TagType::Flag, InfoWriteValue::Flag(false)) => record.clear_info_flag(tag_bytes),
            (TagType::Integer, InfoWriteValue::Integers(v)) => record.push_info_integer(tag_bytes, &v),
            (TagType::Float, InfoWriteValue::Floats(v)) => record.push_info_float(tag_bytes, &v),
            (TagType::String, InfoWriteValue::Strings(v)) => {
                let refs: Vec<&[u8]> = v.iter().map(|s| s.as_bytes()).collect();
                record.push_info_string(tag_bytes, &refs)
            }
            _ => Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() }),
        };
        record.unpack();
        out
    };

    if let Err(e) = res {
        let msg = v8::String::new(scope, &format!("failed to set info {tag}: {e}")).unwrap();
        scope.throw_exception(v8::Exception::error(scope, msg));
    }
}

/// V8 callback for `variant.info(tag)`.
///
/// Uses the core `record_info()` function to get the value, then converts to V8.
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

    let record = variant.record(scope);
    let value = htsvcf_core::record_info(record, header.inner(), &tag);
    rv.set(infovalue_to_v8(scope, &value));
}

/// V8 callback for `variant.format(tag)`.
///
/// Uses the core `record_format()` function to get the value, then converts to V8.
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

    let record = variant.record(scope);
    let value = htsvcf_core::record_format(record, header.inner(), &tag);
    rv.set(formatvalue_to_v8(scope, &value));
}

/// V8 callback for `variant.sample(name)`.
///
/// Uses the core `record_sample()` function to get data for a single sample.
fn sample_fn(
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

    let Ok(sample_str) = v8::Local::<v8::String>::try_from(args.get(0)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let sample_name = sample_str.to_rust_string_lossy(scope);

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

    let record = variant.record(scope);
    let Some(fields) = htsvcf_core::record_sample(record, header.inner(), &sample_name) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let out = v8::Object::new(scope);
    for (tag, value) in fields {
        let key = v8::String::new(scope, &tag).unwrap();
        let v = formatvalue_to_v8(scope, &value);
        out.set(scope, key.into(), v);
    }

    rv.set(out.into());
}

/// V8 callback for `variant.samples(subset?)`.
///
/// Uses the core `record_samples()` function to get data for all/subset of samples.
fn samples_fn(
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

    // Parse optional subset argument
    let subset_names: Option<Vec<String>> = if args.length() > 0 {
        let arg0 = args.get(0);
        if arg0.is_undefined() || arg0.is_null() {
            None
        } else if arg0.is_array() {
            let Some(arr) = v8::Local::<v8::Array>::try_from(arg0).ok() else {
                rv.set(v8::undefined(scope).into());
                return;
            };
            let mut names = Vec::with_capacity(arr.length() as usize);
            for i in 0..arr.length() {
                if let Some(elem) = arr.get_index(scope, i) {
                    if let Ok(s) = v8::Local::<v8::String>::try_from(elem) {
                        names.push(s.to_rust_string_lossy(scope));
                    }
                }
            }
            Some(names)
        } else {
            // Invalid argument type - return undefined
            rv.set(v8::undefined(scope).into());
            return;
        }
    } else {
        None
    };

    let record = variant.record(scope);
    let subset_refs: Option<Vec<&str>> = subset_names.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());
    let results = htsvcf_core::record_samples(record, header.inner(), subset_refs.as_deref());

    // Convert to V8 array of objects
    let arr = v8::Array::new(scope, results.len() as i32);
    for (i, sample_data) in results.into_iter().enumerate() {
        let obj = v8::Object::new(scope);
        for (tag, value) in sample_data {
            let key = v8::String::new(scope, &tag).unwrap();
            let v = formatvalue_to_v8(scope, &value);
            obj.set(scope, key.into(), v);
        }
        arr.set_index(scope, i as u32, obj.into());
    }

    rv.set(arr.into());
}

/// V8 callback for `variant.genotypes(subset?)`.
///
/// Uses the core `record_genotypes()` function to get parsed genotypes.
/// Returns an array of Genotype objects: { alleles: number[]|null[], phase: boolean[] }
fn genotypes_fn(
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

    // Parse optional subset argument
    let subset_names: Option<Vec<String>> = if args.length() > 0 {
        let arg0 = args.get(0);
        if arg0.is_undefined() || arg0.is_null() {
            None
        } else if arg0.is_array() {
            let Some(arr) = v8::Local::<v8::Array>::try_from(arg0).ok() else {
                rv.set(v8::undefined(scope).into());
                return;
            };
            let mut names = Vec::with_capacity(arr.length() as usize);
            for i in 0..arr.length() {
                if let Some(elem) = arr.get_index(scope, i) {
                    if let Ok(s) = v8::Local::<v8::String>::try_from(elem) {
                        names.push(s.to_rust_string_lossy(scope));
                    }
                }
            }
            Some(names)
        } else {
            // Invalid argument type - return undefined
            rv.set(v8::undefined(scope).into());
            return;
        }
    } else {
        None
    };

    let record = variant.record(scope);
    let subset_refs: Option<Vec<&str>> = subset_names.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());
    let genotypes = htsvcf_core::record_genotypes(record, header.inner(), subset_refs.as_deref());

    // Convert to V8 array of genotype objects
    let arr = v8::Array::new(scope, genotypes.len() as i32);
    for (i, gt) in genotypes.iter().enumerate() {
        let obj = genotype_to_v8(scope, gt);
        arr.set_index(scope, i as u32, obj);
    }

    rv.set(arr.into());
}

/// V8 callback for `variant.toString()`.
///
/// Uses the core `record_to_string()` function to format the record as VCF.
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

    let record = variant.record(scope);
    match htsvcf_core::record_to_string(record, header.inner()) {
        Some(text) => {
            let out = v8::String::new(scope, &text).unwrap();
            rv.set(out.into());
        }
        None => {
            rv.set(v8::undefined(scope).into());
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
            unsafe { crate::header::Header::new(reader.header().inner) },
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

        let platform = crate::runtime::ensure_v8_initialized().clone();
        let _guard = crate::runtime::v8_lock();
        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));
        v8::scope!(handle_scope, isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        assert_eq!(variant.chrom(), "chr1");
        assert_eq!(variant.pos(scope), 1000);
        assert_eq!(variant.start(scope), 999);
        assert_eq!(variant.end(scope), 1000);
        assert_eq!(variant.id(scope), ".");
        assert_eq!(variant.reference(scope), "A");
        assert_eq!(variant.alts(scope), vec!["C".to_string()]);
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

        assert_eq!(eval_js(&path, "variant.id = 'rs1'; variant.id"), "rs1");
        assert_eq!(eval_js(&path, "variant.qual = 42; variant.qual"), "42");
        assert_eq!(eval_js(&path, "variant.qual = null; variant.qual === null"), "true");

        assert_eq!(eval_js(&path, "variant.filter = ['PASS']; variant.filter.length"), "1");
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
    /// `variant.set_info()` should mutate INFO by header type.
    fn test_js_set_info() {
        let path = tmp_path("set_info.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
  ##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
  ##INFO=<ID=AD,Number=2,Type=Integer,Description=\"Allele depths\">\n\
  ##INFO=<ID=AF,Number=2,Type=Float,Description=\"Allele frequencies\">\n\
  ##INFO=<ID=NOTE,Number=1,Type=String,Description=\"Note\">\n\
  ##INFO=<ID=SOMATIC,Number=0,Type=Flag,Description=\"Somatic\">\n\
  ##contig=<ID=chr1>\n\
  #CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n\
  chr1\t1\t.\tA\tC,G\t.\t.\t.\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_js(path, "variant.set_info('DP', 32); variant.info('DP')"), "32");
        assert_eq!(
            eval_js(path, "variant.set_info('AD', [1, 2]); variant.info('AD')[1]"),
            "2"
        );
        assert_eq!(
            eval_js(path, "variant.set_info('AF', [0.1, 0.2]); Math.abs(variant.info('AF')[0] - 0.1) < 1e-6"),
            "true"
        );
        assert_eq!(
            eval_js(path, "variant.set_info('NOTE', 'hi'); variant.info('NOTE')"),
            "hi"
        );
        assert_eq!(
            eval_js(path, "variant.set_info('SOMATIC', true); variant.info('SOMATIC')"),
            "true"
        );

        assert_eq!(
            eval_js(path, "variant.set_info('DP', null); variant.info('DP')"),
            "undefined"
        );

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

        // sample(name) returns a keyed object with only that sample's values.
        assert_eq!(eval_js(path, "variant.sample('S1').sample_name"), "S1");
        assert_eq!(eval_js(path, "variant.sample('S1').DP"), "7");
        assert_eq!(eval_js(path, "variant.sample('S1').AD[1]"), "2");
        assert_eq!(
            eval_js(path, "Math.abs(variant.sample('S1').AF[0] - 0.1) < 1e-6"),
            "true"
        );
        assert_eq!(eval_js(path, "variant.sample('S2').DP === null"), "true");
        assert_eq!(eval_js(path, "variant.sample('S2').AD[0] === null"), "true");
        assert_eq!(eval_js(path, "variant.sample('NOPE')"), "undefined");

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

    #[test]
    /// `variant.genotypes()` should return parsed genotype objects.
    fn test_variant_genotypes() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/genotypes.vcf")
            .to_string_lossy()
            .into_owned();

        // Test genotypes() returns array
        assert_eq!(eval_js(&path, "Array.isArray(variant.genotypes())"), "true");
        assert_eq!(eval_js(&path, "variant.genotypes().length"), "5");

        // diploid_unphased: 0/1
        assert_eq!(eval_js(&path, "variant.genotypes()[0].alleles[0]"), "0");
        assert_eq!(eval_js(&path, "variant.genotypes()[0].alleles[1]"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes()[0].phase[0]"), "false");

        // diploid_phased: 1|1
        assert_eq!(eval_js(&path, "variant.genotypes()[1].alleles[0]"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes()[1].alleles[1]"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes()[1].phase[0]"), "true");

        // diploid_missing: ./1
        assert_eq!(eval_js(&path, "variant.genotypes()[2].alleles[0]"), "null");
        assert_eq!(eval_js(&path, "variant.genotypes()[2].alleles[1]"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes()[2].phase[0]"), "false");

        // haploid: 1
        assert_eq!(eval_js(&path, "variant.genotypes()[3].alleles.length"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes()[3].alleles[0]"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes()[3].phase.length"), "0");

        // triploid: 0/1|2
        assert_eq!(eval_js(&path, "variant.genotypes()[4].alleles.length"), "3");
        assert_eq!(eval_js(&path, "variant.genotypes()[4].alleles[0]"), "0");
        assert_eq!(eval_js(&path, "variant.genotypes()[4].alleles[1]"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes()[4].alleles[2]"), "2");
        assert_eq!(eval_js(&path, "variant.genotypes()[4].phase[0]"), "false");
        assert_eq!(eval_js(&path, "variant.genotypes()[4].phase[1]"), "true");

        // Test genotypes(subset)
        assert_eq!(eval_js(&path, "variant.genotypes(['haploid']).length"), "1");
        assert_eq!(eval_js(&path, "variant.genotypes(['haploid'])[0].alleles[0]"), "1");

        // Test sample().genotype
        assert_eq!(eval_js(&path, "variant.sample('diploid_phased').genotype.alleles[0]"), "1");
        assert_eq!(eval_js(&path, "variant.sample('diploid_phased').genotype.phase[0]"), "true");
    }
}

