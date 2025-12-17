use rust_htslib::bcf;
use rust_htslib::bcf::Read;

use crate::header::{create_header_object, Header};
use crate::variant::{create_object_template as create_variant_template, create_variant_object, Variant};

pub const READER_TAG: u16 = 3;
const READER_TYPE_NAME: &[u8] = b"Reader\0";

const OWNER_PRIVATE_KEY: &str = "htsvcf::Reader#owner";

#[derive(Debug)]
enum InnerReader {
    Unindexed(bcf::Reader),
    Indexed(bcf::IndexedReader),
}

impl InnerReader {
    fn header_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
        match self {
            InnerReader::Unindexed(r) => r.header().inner,
            InnerReader::Indexed(r) => r.header().inner,
        }
    }

    fn empty_record(&self) -> bcf::Record {
        match self {
            InnerReader::Unindexed(r) => r.empty_record(),
            InnerReader::Indexed(r) => r.empty_record(),
        }
    }

    fn read_record(&mut self, record: &mut bcf::Record) -> Option<Result<(), rust_htslib::errors::Error>> {
        match self {
            InnerReader::Unindexed(r) => r.read(record),
            InnerReader::Indexed(r) => r.read(record),
        }
    }

    fn fetch(&mut self, chrom: &str, start: u64, end: Option<u64>) -> Result<(), rust_htslib::errors::Error> {
        let InnerReader::Indexed(r) = self else {
            return Err(rust_htslib::errors::Error::Fetch);
        };

        // `fetch()` takes a numeric reference id.
        let rid = r.header().name2rid(chrom.as_bytes())?;
        r.fetch(rid, start, end)
    }
}

struct ReaderWrapper {
    inner: v8::cppgc::GcCell<InnerReader>,
    has_index: bool,
    header_obj: v8::TracedReference<v8::Object>,
    variant_template: v8::TracedReference<v8::ObjectTemplate>,
}

unsafe impl v8::cppgc::GarbageCollected for ReaderWrapper {
    fn trace(&self, visitor: &mut v8::cppgc::Visitor) {
        visitor.trace(&self.header_obj);
        visitor.trace(&self.variant_template);
    }

    fn get_name(&self) -> &'static std::ffi::CStr {
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(READER_TYPE_NAME) }
    }
}

fn owner_private_key<'s>(scope: &v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, OWNER_PRIVATE_KEY).unwrap();
    v8::Private::for_api(scope, Some(name))
}

fn throw_type_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let msg = v8::String::new(scope, message).unwrap();
    let exc = v8::Exception::type_error(scope, msg);
    scope.throw_exception(exc);
}

fn throw_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let msg = v8::String::new(scope, message).unwrap();
    let exc = v8::Exception::error(scope, msg);
    scope.throw_exception(exc);
}

fn has_index_on_disk(path: &str) -> bool {
    // htslib supports CSI or TBI alongside the main file.
    let candidates = [format!("{path}.csi"), format!("{path}.tbi")];
    candidates.iter().any(|p| std::fs::metadata(p).is_ok())
}

fn reader_ctor(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Reader must be called with new");
        return;
    }

    if args.length() < 1 {
        throw_type_error(scope, "Reader(path) requires a path string");
        return;
    }

    let Ok(path_val) = v8::Local::<v8::String>::try_from(args.get(0)) else {
        throw_type_error(scope, "Reader(path) requires a path string");
        return;
    };
    let path = path_val.to_rust_string_lossy(scope);

    let has_index = has_index_on_disk(&path);

    let inner = if has_index {
        match bcf::IndexedReader::from_path(&path) {
            Ok(r) => InnerReader::Indexed(r),
            Err(_) => match bcf::Reader::from_path(&path) {
                Ok(r) => {
                    // Index exists but couldn\'t be opened; still allow unindexed reads.
                    InnerReader::Unindexed(r)
                }
                Err(e) => {
                    throw_error(scope, &format!("failed to open {path}: {e}"));
                    return;
                }
            },
        }
    } else {
        match bcf::Reader::from_path(&path) {
            Ok(r) => InnerReader::Unindexed(r),
            Err(e) => {
                throw_error(scope, &format!("failed to open {path}: {e}"));
                return;
            }
        }
    };

    let header_obj = create_header_object(scope, Header::new(inner.header_ptr()));

    // tie header lifetime to this Reader instance
    let this = args.this();
    let key = owner_private_key(scope);
    let _ = header_obj.set_private(scope, key, this.into());

    let variant_template_local = create_variant_template(scope);

    let wrapper = ReaderWrapper {
        inner: v8::cppgc::GcCell::new(inner),
        has_index,
        header_obj: v8::TracedReference::new(scope, header_obj),
        variant_template: v8::TracedReference::new(scope, variant_template_local),
    };

    let gc_wrapper =
        unsafe { v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(), wrapper) };

    unsafe {
        v8::Object::wrap::<READER_TAG, ReaderWrapper>(scope, this, &gc_wrapper);
    }

    // Ensure the instance itself is the returned value.
    rv.set(this.into());
}

fn reader_header_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let Some(wrapper) = (unsafe { v8::Object::unwrap::<READER_TAG, ReaderWrapper>(scope, this) }) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let reader = unsafe { wrapper.as_ref() };
    let Some(header_obj) = reader.header_obj.get(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    rv.set(header_obj.into());
}

fn reader_has_index_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let Some(wrapper) = (unsafe { v8::Object::unwrap::<READER_TAG, ReaderWrapper>(scope, this) }) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };

    let reader = unsafe { wrapper.as_ref() };
    rv.set(v8::Boolean::new(scope, reader.has_index).into());
}

fn reader_iterator_fn(
    _scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    // Make Reader its own iterator.
    rv.set(args.this().into());
}

fn reader_next_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let Some(wrapper) = (unsafe { v8::Object::unwrap::<READER_TAG, ReaderWrapper>(scope, this) }) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let reader = unsafe { wrapper.as_ref() };
    let result_obj = v8::Object::new(scope);

    let Some(header_obj) = reader.header_obj.get(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let Some(variant_template) = reader.variant_template.get(scope) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let inner = reader.inner.get_mut(scope);
    let mut record = inner.empty_record();
    match inner.read_record(&mut record) {
        None => {
            set_kv(scope, &result_obj, "done", v8::Boolean::new(scope, true).into());
            set_kv(scope, &result_obj, "value", v8::undefined(scope).into());
            rv.set(result_obj.into());
        }
        Some(Ok(())) => {
            let variant = Variant::from_record(record);
            let variant_obj = create_variant_object(scope, variant_template, variant, header_obj);

            set_kv(scope, &result_obj, "done", v8::Boolean::new(scope, false).into());
            set_kv(scope, &result_obj, "value", variant_obj.into());
            rv.set(result_obj.into());
        }
        Some(Err(e)) => {
            throw_error(scope, &format!("read failed: {e}"));
        }
    }
}

fn reader_query_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let Some(wrapper) = (unsafe { v8::Object::unwrap::<READER_TAG, ReaderWrapper>(scope, this) }) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let reader = unsafe { wrapper.as_ref() };

    if !reader.has_index {
        throw_type_error(scope, "query() requires an indexed file");
        return;
    }

    if args.length() < 1 {
        throw_type_error(scope, "query(region) or query(chrom, start, end?)");
        return;
    }

    let (chrom, start, end) = if args.length() == 1 {
        let Ok(region_str) = v8::Local::<v8::String>::try_from(args.get(0)) else {
            throw_type_error(scope, "query(region) requires a string");
            return;
        };
        let Some(tup) = parse_region_1based(&region_str.to_rust_string_lossy(scope)) else {
            throw_type_error(scope, "invalid region");
            return;
        };
        tup
    } else {
        let Ok(chrom_str) = v8::Local::<v8::String>::try_from(args.get(0)) else {
            throw_type_error(scope, "query(chrom, start, end?) requires chrom string");
            return;
        };
        let chrom = chrom_str.to_rust_string_lossy(scope);

        let Ok(start_num) = v8::Local::<v8::Number>::try_from(args.get(1)) else {
            throw_type_error(scope, "query(chrom, start, end?) requires numeric start");
            return;
        };
        let start = start_num.value() as i64;
        if start < 0 {
            throw_type_error(scope, "query start must be >= 0");
            return;
        }

        let end = if args.length() >= 3 {
            if args.get(2).is_undefined() || args.get(2).is_null() {
                None
            } else {
                let Ok(end_num) = v8::Local::<v8::Number>::try_from(args.get(2)) else {
                    throw_type_error(scope, "query(chrom, start, end?) requires numeric end");
                    return;
                };
                let end = end_num.value() as i64;
                if end < 0 {
                    throw_type_error(scope, "query end must be >= 0");
                    return;
                }
                Some(end as u64)
            }
        } else {
            None
        };

        (chrom, start as u64, end)
    };

    if chrom.is_empty() {
        throw_type_error(scope, "invalid region");
        return;
    }

    let inner = reader.inner.get_mut(scope);
    match inner.fetch(&chrom, start, end) {
        Ok(()) => rv.set(v8::undefined(scope).into()),
        Err(e) => throw_error(scope, &format!("query failed: {e}")),
    }
}

fn parse_region_1based(region: &str) -> Option<(String, u64, Option<u64>)> {
    // Accept: "chr", "chr:100", "chr:100-200". Coordinates are 1-based inclusive.
    let (chrom, rest) = region.split_once(':').unwrap_or((region, ""));
    if chrom.is_empty() {
        return None;
    }
    if rest.is_empty() {
        return Some((chrom.to_string(), 0, None));
    }

    let (start_str, end_str) = rest.split_once('-').unwrap_or((rest, ""));
    let start_1 = start_str.replace(',', "").parse::<u64>().ok()?;
    let start_0 = start_1.saturating_sub(1);

    if end_str.is_empty() {
        return Some((chrom.to_string(), start_0, None));
    }

    let end_1 = end_str.replace(',', "").parse::<u64>().ok()?;
    let end_0 = end_1.saturating_sub(1);
    Some((chrom.to_string(), start_0, Some(end_0)))
}

fn set_kv(scope: &mut v8::PinScope<'_, '_>, obj: &v8::Local<v8::Object>, key: &str, value: v8::Local<v8::Value>) {
    let k = v8::String::new(scope, key).unwrap();
    obj.set(scope, k.into(), value);
}

/// Create the `Reader` constructor and install it on the global.
///
/// Returns the created constructor function.
pub fn create_reader_constructor<'a>(
    scope: &mut v8::PinScope<'a, '_>,
) -> v8::Local<'a, v8::Function> {
    let function_template = v8::FunctionTemplate::new(scope, reader_ctor);
    function_template.set_class_name(v8::String::new(scope, "Reader").unwrap());

    let instance_template = function_template.instance_template(scope);
    instance_template.set_internal_field_count(1);

    let proto = function_template.prototype_template(scope);

    // Reader.prototype.next()
    let next_key = v8::String::new(scope, "next").unwrap();
    proto.set(next_key.into(), v8::FunctionTemplate::new(scope, reader_next_fn).into());

    // Reader.prototype[Symbol.iterator] = function() { return this; }
    let iterator = v8::Symbol::get_iterator(scope);
    proto.set(iterator.into(), v8::FunctionTemplate::new(scope, reader_iterator_fn).into());

    // Reader.prototype.query(...)
    let query_key = v8::String::new(scope, "query").unwrap();
    proto.set(query_key.into(), v8::FunctionTemplate::new(scope, reader_query_fn).into());

    // Reader.prototype.hasIndex()
    let has_index_key = v8::String::new(scope, "hasIndex").unwrap();
    proto.set(
        has_index_key.into(),
        v8::FunctionTemplate::new(scope, reader_has_index_fn).into(),
    );

    // Reader.prototype.header()
    let header_key = v8::String::new(scope, "header").unwrap();
    proto.set(header_key.into(), v8::FunctionTemplate::new(scope, reader_header_fn).into());

    function_template
        .get_function(scope)
        .expect("failed to create Reader constructor")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;
    use std::path::PathBuf;

    fn fixture_vcf() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/t.vcf.gz")
            .to_string_lossy()
            .into_owned()
    }

    fn eval_js(js_expr: &str) -> String {
        let platform = crate::runtime::ensure_v8_initialized().clone();
        let _guard = crate::runtime::v8_lock();

        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

        v8::scope!(handle_scope, isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        let global = context.global(scope);
        let reader_ctor = create_reader_constructor(scope);
        let name = v8::String::new(scope, "Reader").unwrap();
        global.set(scope, name.into(), reader_ctor.into());

        let code = v8::String::new(scope, js_expr).unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        result.to_string(scope).unwrap().to_rust_string_lossy(scope)
    }

    #[test]
    fn test_reader_iterates_all_records() {
        let path = fixture_vcf();
        let mut rust_reader = bcf::Reader::from_path(&path).unwrap();
        let expected = rust_reader.records().count();

        let js = format!(
            "(() => {{ const r = new Reader('{}'); let n = 0; for (const v of r) {{ n += 1; }} return n; }})()",
            path.replace('\\', "\\\\")
        );
        assert_eq!(eval_js(&js), expected.to_string());
    }

    #[test]
    fn test_reader_query_region_string() {
        let path = fixture_vcf().replace('\\', "\\\\");
        // The test VCF has first record at chr1:1000 (1-based).
        let js = format!(
            "(() => {{ const r = new Reader('{}'); if (!r.hasIndex()) return 'noindex'; r.query('chr1:1000-1000'); let n = 0; for (const v of r) n++; return n; }})()",
            path
        );
        assert_eq!(eval_js(&js), "1");
    }

    #[test]
    fn test_reader_query_numeric_0based() {
        let path = fixture_vcf().replace('\\', "\\\\");
        let js = format!(
            "(() => {{ const r = new Reader('{}'); if (!r.hasIndex()) return 'noindex'; r.query('chr1', 999, 999); let n = 0; for (const v of r) n++; return n; }})()",
            path
        );
        assert_eq!(eval_js(&js), "1");
    }
}
