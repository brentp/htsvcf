use crate::header::Header;
use crate::variant::Variant;
use htsvcf_core::{open_writer, OutputFormat, Writer as CoreWriter, WriterOptions};

pub const WRITER_TAG: u16 = 4;
const WRITER_TYPE_NAME: &[u8] = b"Writer\0";

const OWNER_PRIVATE_KEY: &str = "htsvcf::Writer#owner";

struct WriterWrapper {
    inner: v8::cppgc::GcCell<Option<CoreWriter>>,
}

unsafe impl v8::cppgc::GarbageCollected for WriterWrapper {
    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {}

    fn get_name(&self) -> &'static std::ffi::CStr {
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(WRITER_TYPE_NAME) }
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

fn parse_options(
    scope: &mut v8::PinScope<'_, '_>,
    v: v8::Local<v8::Value>,
) -> Option<WriterOptions> {
    if v.is_null_or_undefined() {
        return Some(WriterOptions::default());
    }
    let Ok(obj) = v8::Local::<v8::Object>::try_from(v) else {
        return None;
    };

    let mut opts = WriterOptions::default();

    // opts.format: 'vcf' | 'bcf'
    let format_key = v8::String::new(scope, "format").unwrap();
    if let Some(fmt_val) = obj.get(scope, format_key.into()) {
        if !fmt_val.is_null_or_undefined() {
            let Ok(s) = v8::Local::<v8::String>::try_from(fmt_val) else {
                return None;
            };
            let s = s.to_rust_string_lossy(scope);
            match s.as_str() {
                "vcf" => opts.format = Some(OutputFormat::Vcf),
                "bcf" => opts.format = Some(OutputFormat::Bcf),
                _ => return None,
            }
        }
    }

    let uncompressed_key = v8::String::new(scope, "uncompressed").unwrap();
    if let Some(uc_val) = obj.get(scope, uncompressed_key.into()) {
        if !uc_val.is_null_or_undefined() {
            if !uc_val.is_boolean() {
                return None;
            }
            opts.uncompressed = uc_val.boolean_value(scope);
        }
    }

    let threads_key = v8::String::new(scope, "threads").unwrap();
    if let Some(t_val) = obj.get(scope, threads_key.into()) {
        if !t_val.is_null_or_undefined() {
            if !t_val.is_number() {
                return None;
            }
            let n = t_val.number_value(scope).unwrap_or(0.0);
            if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
                return None;
            }
            let n = n as i64;
            if n > 0 {
                opts.threads = Some(n as usize);
            }
        }
    }

    Some(opts)
}

fn writer_ctor(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Writer must be called with new");
        return;
    }

    if args.length() < 2 {
        throw_type_error(scope, "Writer(path, header) requires path and header");
        return;
    }

    let Ok(path_val) = v8::Local::<v8::String>::try_from(args.get(0)) else {
        throw_type_error(scope, "Writer(path, header): path must be string");
        return;
    };
    let path = path_val.to_rust_string_lossy(scope);

    let header_val = args.get(1);
    let Ok(header_obj) = v8::Local::<v8::Object>::try_from(header_val) else {
        throw_type_error(scope, "Writer(path, header): header must be Header");
        return;
    };

    let Some(header_wrapper) =
        (unsafe { v8::Object::unwrap::<{ crate::header::HEADER_TAG }, Header>(scope, header_obj) })
    else {
        throw_type_error(scope, "Writer(path, header): header must be Header");
        return;
    };
    let header = unsafe { header_wrapper.as_ref() };

    let opts = if args.length() >= 3 {
        match parse_options(scope, args.get(2)) {
            Some(o) => o,
            None => {
                throw_type_error(
                    scope,
                    "Writer options must be {format?, uncompressed?, threads?}",
                );
                return;
            }
        }
    } else {
        WriterOptions::default()
    };

    let inner = match open_writer(&path, header.inner(), opts) {
        Ok(w) => w,
        Err(e) => {
            throw_error(scope, &format!("failed to open writer {path}: {e}"));
            return;
        }
    };

    let wrapper = WriterWrapper {
        inner: v8::cppgc::GcCell::new(Some(inner)),
    };

    let this = args.this();

    // tie header lifetime to this Writer instance
    let key = owner_private_key(scope);
    let _ = header_obj.set_private(scope, key, this.into());

    let gc_wrapper =
        unsafe { v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(), wrapper) };
    unsafe {
        v8::Object::wrap::<WRITER_TAG, WriterWrapper>(scope, this, &gc_wrapper);
    }
    rv.set(this.into());
}

fn writer_close_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let Some(wrapper_ptr) =
        (unsafe { v8::Object::unwrap::<WRITER_TAG, WriterWrapper>(scope, this) })
    else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let wrapper = unsafe { wrapper_ptr.as_ref() };
    let _ = wrapper.inner.get_mut(scope).take();
    rv.set(v8::undefined(scope).into());
}

fn writer_write_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let Some(wrapper_ptr) =
        (unsafe { v8::Object::unwrap::<WRITER_TAG, WriterWrapper>(scope, this) })
    else {
        throw_type_error(scope, "invalid Writer receiver");
        return;
    };

    let wrapper = unsafe { wrapper_ptr.as_ref() };

    if args.length() < 1 {
        throw_type_error(scope, "Writer.write(variant) requires a Variant");
        return;
    }

    let Ok(variant_obj) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(scope, "Writer.write(variant) requires a Variant");
        return;
    };

    let Some(variant_wrapper) =
        (unsafe { v8::Object::unwrap::<{ crate::variant::TAG }, Variant>(scope, variant_obj) })
    else {
        throw_type_error(scope, "Writer.write(variant) requires a Variant");
        return;
    };
    let variant = unsafe { variant_wrapper.as_ref() };

    let Some(mut record) = variant.take_record(scope) else {
        throw_error(scope, "variant was consumed");
        return;
    };

    let Some(writer) = wrapper.inner.get_mut(scope).as_mut() else {
        throw_error(scope, "writer is closed");
        return;
    };

    match writer.write_record(&mut record) {
        Ok(()) => rv.set(v8::undefined(scope).into()),
        Err(e) => throw_error(scope, &format!("write failed: {e}")),
    }
}

pub(crate) fn create_writer_constructor<'a>(
    scope: &mut v8::PinScope<'a, '_>,
) -> v8::Local<'a, v8::Function> {
    let function_template = v8::FunctionTemplate::new(scope, writer_ctor);
    function_template.set_class_name(v8::String::new(scope, "Writer").unwrap());

    let instance_template = function_template.instance_template(scope);
    instance_template.set_internal_field_count(1);

    let proto = function_template.prototype_template(scope);

    let write_key = v8::String::new(scope, "write").unwrap();
    proto.set(
        write_key.into(),
        v8::FunctionTemplate::new(scope, writer_write_fn).into(),
    );

    let close_key = v8::String::new(scope, "close").unwrap();
    proto.set(
        close_key.into(),
        v8::FunctionTemplate::new(scope, writer_close_fn).into(),
    );

    function_template
        .get_function(scope)
        .expect("failed to create Writer constructor")
}
