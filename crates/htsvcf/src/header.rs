//! V8-based `Header` object exposed to JavaScript.
//!
//! This module wraps [`htsvcf_core::Header`] for use in the V8 runtime, providing
//! access to VCF header metadata (INFO/FORMAT definitions, sample names, etc.).
//!
//! # JavaScript Usage
//!
//! The `header` global is available in all JS expressions:
//!
//! ```js
//! // List sample names
//! header.samples()  // => ["NA12878", "NA12879", ...]
//!
//! // Get field definition
//! header.get('INFO', 'DP')
//! // => { id: 'DP', type: 'Integer', number: '1', description: 'Read depth' }
//!
//! // Add new fields (for use with variant.set_info)
//! header.addInfo('CUSTOM', '1', 'Integer', 'My annotation')
//! header.addFormat('SCORE', '1', 'Float', 'Per-sample score')
//!
//! // Get all header records
//! header.records()  // => [{ type: 'INFO', ID: 'DP', ... }, ...]
//!
//! // Full header text
//! header.toString()
//! ```

use rust_htslib::bcf::header::{HeaderRecord, TagLength, TagType};
use std::collections::HashMap;

pub const HEADER_TAG: u16 = 2;
const HEADER_TYPE_NAME: &[u8] = b"Header\0";

/// Thin wrapper around `htsvcf_core::Header` for use in V8.
///
/// This delegates all VCF header logic to the core Header and adds only
/// V8-specific functionality (GC integration, JS bindings).
#[derive(Debug)]
pub struct Header {
    inner: htsvcf_core::Header,
}

impl Header {
    /// Create a `Header` from a raw `bcf_hdr_t` pointer.
    ///
    /// # Safety
    /// The pointer must be valid. The core Header will duplicate and own the memory.
    pub unsafe fn new(ptr: *mut rust_htslib::htslib::bcf_hdr_t) -> Self {
        Self {
            inner: htsvcf_core::Header::new(ptr),
        }
    }

    /// Return parsed header records.
    pub fn header_records(&self) -> Vec<HeaderRecord> {
        self.inner.header_records()
    }

    /// Get INFO tag type/length from the header.
    pub fn info_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
        self.inner.info_type(tag)
    }

    /// Get FORMAT tag type/length from the header.
    pub fn format_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
        self.inner.format_type(tag)
    }

    /// Get sample ID (index) from sample name.
    pub fn sample_id(&self, sample: &[u8]) -> Option<usize> {
        self.inner.sample_id(sample)
    }

    /// Get tag name from numeric ID.
    pub fn id_to_name(&self, id: u32) -> Vec<u8> {
        self.inner.id_to_name(id)
    }

    /// Get the cached name for a tag ID, returning both the String and bytes.
    pub fn id_to_name_cached(&self, id: u32) -> (String, Vec<u8>) {
        self.inner.id_to_name_cached(id)
    }

    /// Get the number of samples in the header.
    pub fn sample_count(&self) -> usize {
        self.inner.sample_count()
    }

    /// Get all sample names as strings.
    pub fn sample_names(&self) -> &[String] {
        self.inner.sample_names()
    }

    /// Get the index of a sample by name, or None if not found.
    pub fn sample_idx(&self, name: &str) -> Option<usize> {
        self.inner.sample_idx(name)
    }

    /// Get a reference to the sample name-to-index map.
    pub fn sample_name_to_idx(&self) -> &HashMap<String, usize> {
        self.inner.sample_name_to_idx()
    }

    /// Append a raw header line (e.g. `##INFO=...`).
    pub fn push_record(&self, record: &[u8]) -> bool {
        self.inner.push_record(record)
    }

    /// Add an `##INFO` header line.
    pub fn add_info(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
        self.inner.add_info(id, number, ty, description)
    }

    /// Add a `##FORMAT` header line.
    pub fn add_format(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
        self.inner.add_format(id, number, ty, description)
    }

    /// Format header as string.
    pub fn to_string(&self) -> Option<String> {
        self.inner.to_string()
    }

    /// Get a reference to the inner core Header.
    pub fn inner(&self) -> &htsvcf_core::Header {
        &self.inner
    }
}

unsafe impl v8::cppgc::GarbageCollected for Header {
    /// No-op trace because `Header` does not reference other GC objects.
    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {}

    /// Class name shown in V8 heap snapshots.
    fn get_name(&self) -> &'static std::ffi::CStr {
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(HEADER_TYPE_NAME) }
    }
}

/// Create a V8 object backing the `header` global.
///
/// The returned object wraps (via internal field) a gc-managed [`Header`].
pub fn create_header_object<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    header: Header,
) -> v8::Local<'a, v8::Object> {
    let object_template = v8::ObjectTemplate::new(scope);
    object_template.set_internal_field_count(1);

    let records_key = v8::String::new(scope, "records").unwrap();
    let records_template = v8::FunctionTemplate::new(scope, records_fn);
    object_template.set(records_key.into(), records_template.into());

    let get_key = v8::String::new(scope, "get").unwrap();
    let get_template = v8::FunctionTemplate::new(scope, get_fn);
    object_template.set(get_key.into(), get_template.into());

    let add_info_key = v8::String::new(scope, "addInfo").unwrap();
    let add_info_template = v8::FunctionTemplate::new(scope, add_info_fn);
    object_template.set(add_info_key.into(), add_info_template.into());

    let add_format_key = v8::String::new(scope, "addFormat").unwrap();
    let add_format_template = v8::FunctionTemplate::new(scope, add_format_fn);
    object_template.set(add_format_key.into(), add_format_template.into());

    let to_string_key = v8::String::new(scope, "toString").unwrap();
    let to_string_template = v8::FunctionTemplate::new(scope, to_string_fn);
    object_template.set(to_string_key.into(), to_string_template.into());

    let samples_key = v8::String::new(scope, "samples").unwrap();
    let samples_template = v8::FunctionTemplate::new(scope, samples_fn);
    object_template.set(samples_key.into(), samples_template.into());

    let object = object_template
        .new_instance(scope)
        .expect("failed to create Header instance");

    let wrapper =
        unsafe { v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(), header) };
    unsafe {
        v8::Object::wrap::<HEADER_TAG, Header>(scope, object, &wrapper);
    }

    object
}

/// V8 callback for `header.records()`.
fn records_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header = unsafe { wrapper.as_ref() };

    let fields = header.inner.all_fields();
    let arr = v8::Array::new(scope, fields.len() as i32);
    for (i, (section, field)) in fields.into_iter().enumerate() {
        let o = v8::Object::new(scope);
        set_str(scope, &o, "type", &section);
        set_str(scope, &o, "ID", &field.id);
        set_str(scope, &o, "Number", &field.number);
        set_str(scope, &o, "Type", &field.r#type);
        set_str(scope, &o, "Description", &field.description);

        arr.set_index(scope, i as u32, o.into());
    }

    rv.set(arr.into());
}

/// V8 callback for `header.get(section, id)`.
fn get_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header = unsafe { wrapper.as_ref() };

    if args.length() < 2 {
        rv.set(v8::undefined(scope).into());
        return;
    }

    let Ok(section_str) = v8::Local::<v8::String>::try_from(args.get(0)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(id_str) = v8::Local::<v8::String>::try_from(args.get(1)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let section = section_str.to_rust_string_lossy(scope);
    let id = id_str.to_rust_string_lossy(scope);

    let Some(field) = header.inner.get_field(&section, &id) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let o = v8::Object::new(scope);
    set_str(scope, &o, "id", &field.id);
    set_str(scope, &o, "type", &field.r#type);
    set_str(scope, &o, "number", &field.number);
    set_str(scope, &o, "description", &field.description);

    rv.set(o.into());
}

/// V8 callback for `header.addInfo(id, number, type, description)`.
fn add_info_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    add_field_impl(scope, args, &mut rv, |h, id, number, ty, desc| {
        h.add_info(id, number, ty, desc);
    });
}

/// V8 callback for `header.addFormat(id, number, type, description)`.
fn add_format_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    add_field_impl(scope, args, &mut rv, |h, id, number, ty, desc| {
        h.add_format(id, number, ty, desc);
    });
}

/// V8 callback for `header.toString()`.
fn to_string_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header = unsafe { wrapper.as_ref() };

    match header.to_string() {
        Some(text) => {
            let out = v8::String::new(scope, &text).unwrap();
            rv.set(out.into());
        }
        None => {
            rv.set(v8::undefined(scope).into());
        }
    }
}

/// V8 callback for `header.samples()`.
fn samples_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header = unsafe { wrapper.as_ref() };

    let samples = header.sample_names();
    let arr = v8::Array::new(scope, samples.len() as i32);
    for (i, name) in samples.iter().enumerate() {
        let v = v8::String::new(scope, name).unwrap();
        arr.set_index(scope, i as u32, v.into());
    }

    rv.set(arr.into());
}

/// Shared implementation for header field mutations from JS.
fn add_field_impl(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    rv: &mut v8::ReturnValue,
    mut f: impl FnMut(&Header, &str, &str, &str, &str),
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header: &Header = unsafe { wrapper.as_ref() };

    if args.length() < 4 {
        rv.set(v8::undefined(scope).into());
        return;
    }

    let Ok(id) = v8::Local::<v8::String>::try_from(args.get(0)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(number) = v8::Local::<v8::String>::try_from(args.get(1)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(ty) = v8::Local::<v8::String>::try_from(args.get(2)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(desc) = v8::Local::<v8::String>::try_from(args.get(3)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let id = id.to_rust_string_lossy(scope);
    let number = number.to_rust_string_lossy(scope);
    let ty = ty.to_rust_string_lossy(scope);
    let desc = desc.to_rust_string_lossy(scope);

    f(header, &id, &number, &ty, &desc);
    rv.set(v8::undefined(scope).into());
}

/// Convenience for setting a string-valued property.
fn set_str(scope: &mut v8::PinScope<'_, '_>, obj: &v8::Local<v8::Object>, key: &str, value: &str) {
    let k = v8::String::new(scope, key).unwrap();
    let v = v8::String::new(scope, value).unwrap();
    obj.set(scope, k.into(), v.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;
    use std::fs;
    use std::path::PathBuf;

    /// Create a unique temp file path for a test.
    fn tmp_path(file_name: &str) -> PathBuf {
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

    /// Evaluate JS against the `header` global and return its stringified result.
    fn eval_header_js(path: &str, js_expr: &str) -> String {
        let platform = crate::runtime::ensure_v8_initialized().clone();
        let _guard = crate::runtime::v8_lock();

        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

        v8::scope!(handle_scope, isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        let reader = rust_htslib::bcf::Reader::from_path(path).unwrap();
        let header_obj = create_header_object(scope, unsafe { Header::new(reader.header().inner) });

        let code = v8::String::new(scope, js_expr).unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();

        let global = context.global(scope);
        let header_name = v8::String::new(scope, "header").unwrap();
        global.set(scope, header_name.into(), header_obj.into());

        let result = script.run(scope).unwrap();
        result.to_string(scope).unwrap().to_rust_string_lossy(scope)
    }

    #[test]
    /// Basic `header.records()` and `header.get()` behavior.
    fn test_header_records_and_get() {
        let path = tmp_path("header_api.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\n\
chr1\t1\t.\tA\tC\t.\t.\tDP=7\tGT\t0/1\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(
            eval_header_js(path, "header.get('INFO','DP').type"),
            "Integer"
        );
        assert_eq!(eval_header_js(path, "header.get('INFO','DP').number"), "1");
        assert_eq!(
            eval_header_js(path, "header.get('FORMAT','GT').type"),
            "String"
        );
        assert_eq!(
            eval_header_js(path, "header.get('FORMAT','GT').number"),
            "1"
        );

        assert_eq!(
            eval_header_js(
                path,
                "header.records().filter(r => r.type === 'INFO').length"
            ),
            "1"
        );
        assert_eq!(
            eval_header_js(
                path,
                "header.records().filter(r => r.type === 'FORMAT').length"
            ),
            "1"
        );
        assert_eq!(
            eval_header_js(
                path,
                "(() => { const r = header.records().filter(r => r.type === 'INFO')[0]; return r ? (r.ID + ':' + r.Description.replaceAll('\\\"','')) : 'missing'; })()",
            ),
            "DP:Depth"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    /// `header.addInfo/addFormat` should affect subsequent lookups.
    fn test_add_info_and_format_update_header_lookup() {
        let path = tmp_path("add_header_fields.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
 ##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
 ##contig=<ID=chr1>\n\
 #CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n\
 chr1\t1\t.\tA\tC\t.\t.\tDP=7\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_header_js(path, "header.get('INFO','ZZ')"), "undefined");
        assert_eq!(
            eval_header_js(
                path,
                "header.addInfo('ZZ','1','Integer','Zed'); header.get('INFO','ZZ').type",
            ),
            "Integer"
        );
        assert_eq!(
            eval_header_js(
                path,
                "header.addInfo('ZZ','1','Integer','Zed'); header.get('INFO','ZZ').number",
            ),
            "1"
        );

        assert_eq!(
            eval_header_js(path, "header.get('FORMAT','ZZ')"),
            "undefined"
        );
        assert_eq!(
            eval_header_js(
                path,
                "header.addFormat('ZZ','1','Integer','Zed'); header.get('FORMAT','ZZ').type",
            ),
            "Integer"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    /// `header.toString()` should return formatted header text.
    fn test_header_to_string() {
        let path = tmp_path("header_to_string.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n\
chr1\t1\t.\tA\tC\t.\t.\tDP=7\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(
            eval_header_js(path, "header.toString().includes('##fileformat=VCFv4.2')"),
            "true"
        );
        assert_eq!(
            eval_header_js(path, "header.toString().includes('##INFO=<ID=DP')"),
            "true"
        );
        assert_eq!(
            eval_header_js(path, "header.toString().includes('#CHROM\\tPOS')"),
            "true"
        );

        assert_eq!(
            eval_header_js(
                path,
                "header.addInfo('ZZ','1','Integer','Zed'); header.toString().includes('##INFO=<ID=ZZ')",
            ),
            "true"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    /// `header.samples()` should return an array of sample names.
    fn test_header_samples() {
        let path = tmp_path("header_samples.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\tS2\tS3\n\
chr1\t1\t.\tA\tC\t.\t.\tDP=7\tGT\t0/1\t0/0\t1/1\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_header_js(path, "header.samples().length"), "3");
        assert_eq!(eval_header_js(path, "header.samples()[0]"), "S1");
        assert_eq!(eval_header_js(path, "header.samples()[1]"), "S2");
        assert_eq!(eval_header_js(path, "header.samples()[2]"), "S3");
        assert_eq!(
            eval_header_js(path, "header.samples().join(',')"),
            "S1,S2,S3"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    /// `header.samples()` should return empty array for VCF without samples.
    fn test_header_samples_empty() {
        let path = tmp_path("header_samples_empty.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n\
chr1\t1\t.\tA\tC\t.\t.\tDP=7\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_header_js(path, "header.samples().length"), "0");
        assert_eq!(
            eval_header_js(path, "Array.isArray(header.samples())"),
            "true"
        );

        let _ = fs::remove_file(path);
    }
}
