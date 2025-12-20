//! High-level API for running JavaScript expressions over VCF/BCF files.
//!
//! This module provides a simple interface for evaluating a JavaScript expression
//! once per VCF record and collecting the results. It handles V8 initialization,
//! isolate creation, and memory management automatically.
//!
//! # Example
//!
//! ```no_run
//! use htsvcf::runner::{run_vcf_expr_to_stdout, RunOptions};
//!
//! // Print each variant's chromosome and position
//! run_vcf_expr_to_stdout(
//!     "input.vcf.gz",
//!     "`${variant.CHROM}:${variant.POS}`",
//!     RunOptions::default(),
//! ).unwrap();
//! ```
//!
//! # Garbage Collection
//!
//! By default, a full GC is triggered every 100,000 records to maintain
//! steady-state memory usage. This can be tuned via [`RunOptions::gc_every`].
//!
//! # Functions
//!
//! - [`run_vcf_expr_with`]: Evaluate expression with custom callback
//! - [`run_vcf_expr_to_stdout`]: Convenience wrapper that prints results

use rust_htslib::bcf::{self, Read};

use crate::header::{create_header_object, Header};
use crate::runtime;
use crate::variant::{create_object_template, create_variant_object, Variant};
use crate::reader;
use crate::writer;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

pub const DEFAULT_GC_EVERY: usize = 100_000;

#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// Force a full GC every N records.
    ///
    /// This exists primarily for benchmarking / steady-state observation.
    pub gc_every: Option<usize>,
}

impl Default for RunOptions {
    /// Create default run options.
    fn default() -> Self {
        Self {
            gc_every: Some(DEFAULT_GC_EVERY),
        }
    }
}

/// Force a full V8 + cppgc collection.
///
/// This uses V8's testing APIs and is intended for steady-state benchmarking.
fn maybe_force_gc(scope: &mut v8::PinScope<'_, '_>) {
    scope.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
    unsafe {
        scope
            .get_cpp_heap()
            .expect("missing cpp heap")
            .collect_garbage_for_testing(v8::cppgc::EmbedderStackState::MayContainHeapPointers);
    }
}

/// Evaluate a JavaScript expression once per VCF/BCF record.
///
/// The callback receives the stringified JS result for each record.
pub fn run_vcf_expr_with<F>(
    path: &str,
    js_expr: &str,
    opts: RunOptions,
    mut on_result: F,
) -> Result<(), AnyError>
where
    F: FnMut(String) -> Result<(), AnyError>,
{
    let platform = runtime::ensure_v8_initialized().clone();
    let _guard = runtime::v8_lock();

    let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
    let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

    v8::scope!(handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(handle_scope, context);

    let object_template_local = create_object_template(scope);
    let object_template = v8::Global::new(scope, object_template_local);

    let mut reader = bcf::Reader::from_path(path)?;

    let header_obj = create_header_object(scope, unsafe { Header::new(reader.header().inner) });
    let global = context.global(scope);
    let header_name = v8::String::new(scope, "header").expect("failed to allocate v8 string");
    global.set(scope, header_name.into(), header_obj.into());

    // Expose Reader(path) constructor.
    let reader_ctor = reader::create_reader_constructor(scope);
    let reader_name = v8::String::new(scope, "Reader").expect("failed to allocate v8 string");
    global.set(scope, reader_name.into(), reader_ctor.into());

    // Expose Writer(path, header) constructor.
    let writer_ctor = writer::create_writer_constructor(scope);
    let writer_name = v8::String::new(scope, "Writer").expect("failed to allocate v8 string");
    global.set(scope, writer_name.into(), writer_ctor.into());

    let header_obj = global
        .get(scope, header_name.into())
        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        .expect("header object missing");

    let code = v8::String::new(scope, js_expr).expect("failed to allocate v8 string");
    let script = v8::Script::compile(scope, code, None).expect("script compile failed");

    let variant_name = v8::String::new(scope, "variant").expect("failed to allocate v8 string");

    for (i, result) in reader.records().enumerate() {
        let record = result?;
        let record = Variant::from_record(record);

        v8::scope!(loop_scope, scope);
        let object_template = v8::Local::new(loop_scope, &object_template);

        let variant_object = create_variant_object(loop_scope, object_template, record, header_obj);
        global.set(loop_scope, variant_name.into(), variant_object.into());

        let result = script.run(loop_scope).expect("script run failed");
        let result_str = result
            .to_string(loop_scope)
            .expect("result to_string failed");
        on_result(result_str.to_rust_string_lossy(loop_scope))?;

        if let Some(gc_every) = opts.gc_every {
            if i != 0 && i % gc_every == 0 {
                maybe_force_gc(loop_scope);
            }
        }
    }

    Ok(())
}

/// Convenience wrapper that prints each expression result to stdout.
pub fn run_vcf_expr_to_stdout(path: &str, js_expr: &str, opts: RunOptions) -> Result<(), AnyError> {
    run_vcf_expr_with(path, js_expr, opts, |line| {
        println!("{line}");
        Ok(())
    })
}
