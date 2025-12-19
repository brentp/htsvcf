//! Standalone evaluator for applying JS expressions to VCF records.
//!
//! The [`Evaluator`] struct provides a way to iterate over VCF records in Rust
//! while applying user-defined JavaScript expressions to each record.
//!
//! # Example
//!
//! ```no_run
//! use htsvcf::Evaluator;
//! use rust_htslib::bcf::{self, Read};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
//!     let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP')")?;
//!
//!     for result in reader.records() {
//!         let record = result?;
//!         let dp: i32 = js_eval.eval(record)?;
//!         println!("DP = {}", dp);
//!     }
//!     Ok(())
//! }
//! ```

use rust_htslib::bcf;

use crate::fromjs::FromJsValue;
use crate::header::{create_header_object, Header};
use crate::runtime;
use crate::variant::{create_object_template, create_variant_object, Variant};

/// Errors that can occur during JavaScript evaluation.
#[derive(Debug)]
pub enum EvalError {
    /// V8 initialization or setup failed.
    V8Setup(String),
    /// JavaScript compilation failed (syntax error, etc.).
    CompileError(String),
    /// JavaScript runtime error during evaluation.
    RuntimeError(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::V8Setup(msg) => write!(f, "V8 setup error: {}", msg),
            EvalError::CompileError(msg) => write!(f, "JS compile error: {}", msg),
            EvalError::RuntimeError(msg) => write!(f, "JS runtime error: {}", msg),
        }
    }
}

impl std::error::Error for EvalError {}

/// A reusable evaluator for applying JavaScript expressions to VCF records.
///
/// The evaluator compiles the JS expression once and can then be used to
/// evaluate it against multiple records efficiently.
///
/// # Example
///
/// ```no_run
/// use htsvcf::Evaluator;
/// use rust_htslib::bcf::{self, Read};
///
/// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
/// let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP') > 20").unwrap();
///
/// for result in reader.records() {
///     let record = result.unwrap();
///     if js_eval.eval::<bool>(record).unwrap() {
///         println!("Record passed filter");
///     }
/// }
/// ```
pub struct Evaluator {
    /// The V8 isolate that owns all JS state.
    isolate: v8::OwnedIsolate,
    /// Persistent handle to the JS context.
    context: v8::Global<v8::Context>,
    /// Persistent handle to the compiled script.
    script: v8::Global<v8::Script>,
    /// Persistent handle to the variant object template.
    object_template: v8::Global<v8::ObjectTemplate>,
    /// Persistent handle to the header JS object.
    header_obj: v8::Global<v8::Object>,
}

impl Evaluator {
    /// Create a new evaluator with a compiled JS expression.
    ///
    /// The evaluator compiles the given JavaScript expression and prepares
    /// the V8 runtime for evaluation. The header is used to resolve INFO
    /// and FORMAT field types.
    ///
    /// # Arguments
    ///
    /// * `header` - The VCF header (from `reader.header()`)
    /// * `js_expr` - A JavaScript expression to evaluate per record
    ///
    /// # Errors
    ///
    /// Returns `EvalError::CompileError` if the JavaScript expression has
    /// syntax errors.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use htsvcf::Evaluator;
    /// use rust_htslib::bcf::{self, Read};
    ///
    /// let reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
    /// let eval = Evaluator::new(reader.header(), "variant.chrom + ':' + variant.pos").unwrap();
    /// ```
    pub fn new(header: &bcf::header::HeaderView, js_expr: &str) -> Result<Self, EvalError> {
        let platform = runtime::ensure_v8_initialized().clone();
        let _guard = runtime::v8_lock();

        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let mut isolate = v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

        // Create globals inside a temporary scope
        let (context, script, object_template, header_obj) = {
            v8::scope!(handle_scope, &mut isolate);
            let context = v8::Context::new(handle_scope, Default::default());
            let scope = &mut v8::ContextScope::new(handle_scope, context);

            // Create and store object template
            let tmpl = create_object_template(scope);
            let object_template = v8::Global::new(scope, tmpl);

            // Create header object from the bcf header
            let hdr = unsafe { Header::new(header.inner) };
            let hdr_obj = create_header_object(scope, hdr);

            // Set header on global so variant.info() can find it
            let global = context.global(scope);
            let header_name = v8::String::new(scope, "header")
                .ok_or_else(|| EvalError::V8Setup("failed to create header string".into()))?;
            global.set(scope, header_name.into(), hdr_obj.into());

            // Re-fetch header_obj from global for the Global handle
            let hdr_obj = global
                .get(scope, header_name.into())
                .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
                .ok_or_else(|| EvalError::V8Setup("failed to get header object".into()))?;
            let header_obj = v8::Global::new(scope, hdr_obj);

            // Compile script
            let code = v8::String::new(scope, js_expr)
                .ok_or_else(|| EvalError::V8Setup("failed to create script string".into()))?;

            let compiled = v8::Script::compile(scope, code, None).ok_or_else(|| {
                EvalError::CompileError(format!("failed to compile: {}", js_expr))
            })?;
            let script = v8::Global::new(scope, compiled);

            let context = v8::Global::new(scope, context);

            (context, script, object_template, header_obj)
        };

        Ok(Self {
            isolate,
            context,
            script,
            object_template,
            header_obj,
        })
    }

    /// Evaluate the JS expression on a record and convert the result to type `T`.
    ///
    /// Takes ownership of the record. If you need to keep the record, clone
    /// it before calling this method.
    ///
    /// # Type Parameter
    ///
    /// The result type `T` must implement [`FromJsValue`]. Built-in implementations:
    ///
    /// - `String` - converts any JS value to string
    /// - `bool` - uses JavaScript truthiness rules (`0`, `""`, `null`, `undefined`, `NaN`, `false` are falsy)
    /// - `i32`, `i64` - extracts integers
    /// - `f32`, `f64` - extracts floating point numbers
    /// - `Vec<T>` - extracts arrays
    /// - `Option<T>` - returns `None` for `null`/`undefined`
    ///
    /// # Errors
    ///
    /// Returns `EvalError::RuntimeError` if the JavaScript execution fails
    /// or if the result cannot be converted to `T`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use htsvcf::Evaluator;
    /// use rust_htslib::bcf::{self, Read};
    ///
    /// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
    ///
    /// // Extract as string
    /// let mut js_eval = Evaluator::new(reader.header(), "variant.chrom + ':' + variant.pos").unwrap();
    /// let record = reader.records().next().unwrap().unwrap();
    /// let loc: String = js_eval.eval(record).unwrap();
    ///
    /// // Extract as integer
    /// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
    /// let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP')").unwrap();
    /// let record = reader.records().next().unwrap().unwrap();
    /// let dp: i32 = js_eval.eval(record).unwrap();
    ///
    /// // Filter with boolean
    /// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
    /// let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP') > 20").unwrap();
    /// let record = reader.records().next().unwrap().unwrap();
    /// let passed: bool = js_eval.eval(record).unwrap();
    ///
    /// // Extract array of floats
    /// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
    /// let mut js_eval = Evaluator::new(reader.header(), "variant.info('AF')").unwrap();
    /// let record = reader.records().next().unwrap().unwrap();
    /// let afs: Vec<f64> = js_eval.eval(record).unwrap();
    ///
    /// // Handle missing values with Option
    /// let mut reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
    /// let mut js_eval = Evaluator::new(reader.header(), "variant.info('MAYBE_MISSING')").unwrap();
    /// let record = reader.records().next().unwrap().unwrap();
    /// let maybe: Option<i32> = js_eval.eval(record).unwrap();
    /// ```
    pub fn eval<T: FromJsValue>(&mut self, record: bcf::Record) -> Result<T, EvalError> {
        let _guard = runtime::v8_lock();

        v8::scope!(handle_scope, &mut self.isolate);
        let context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        // Get locals from globals
        let object_template = v8::Local::new(scope, &self.object_template);
        let header_obj = v8::Local::new(scope, &self.header_obj);
        let script = v8::Local::new(scope, &self.script);

        // Create variant from record
        let variant = Variant::from_record(record);

        // Create variant JS object
        let variant_object = create_variant_object(scope, object_template, variant, header_obj);

        // Set variant on global
        let global = context.global(scope);
        let variant_name = v8::String::new(scope, "variant")
            .ok_or_else(|| EvalError::V8Setup("failed to create variant string".into()))?;
        global.set(scope, variant_name.into(), variant_object.into());

        // Run script
        let result = script
            .run(scope)
            .ok_or_else(|| EvalError::RuntimeError("script execution failed".into()))?;

        // Convert to requested type
        T::from_js_value(scope, result).map_err(EvalError::RuntimeError)
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

    fn tmp_path(file_name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "htsvcf_eval_{}_{}_{}",
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
    fn test_eval_string() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval =
            Evaluator::new(reader.header(), "variant.chrom + ':' + variant.pos").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: String = js_eval.eval(record).unwrap();
        assert_eq!(result, "chr1:1000");
    }

    #[test]
    fn test_eval_i32() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP')").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: i32 = js_eval.eval(record).unwrap();
        assert_eq!(result, 10);
    }

    #[test]
    fn test_eval_i64() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.pos").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: i64 = js_eval.eval(record).unwrap();
        assert_eq!(result, 1000);
    }

    #[test]
    fn test_eval_f64() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP') * 1.5").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: f64 = js_eval.eval(record).unwrap();
        assert!((result - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_eval_f32() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP') / 3.0").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: f32 = js_eval.eval(record).unwrap();
        assert!((result - 3.333).abs() < 0.01);
    }

    #[test]
    fn test_eval_bool_true() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP') >= 10").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: bool = js_eval.eval(record).unwrap();
        assert!(result);
    }

    #[test]
    fn test_eval_bool_false() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP') > 100").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: bool = js_eval.eval(record).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_eval_bool_truthy_falsy() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();

        // Truthy: non-zero number
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP')").unwrap();
        let record = reader.records().next().unwrap().unwrap();
        assert!(js_eval.eval::<bool>(record).unwrap());

        // Falsy: zero
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "0").unwrap();
        let record = reader.records().next().unwrap().unwrap();
        assert!(!js_eval.eval::<bool>(record).unwrap());

        // Falsy: empty string
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "''").unwrap();
        let record = reader.records().next().unwrap().unwrap();
        assert!(!js_eval.eval::<bool>(record).unwrap());

        // Falsy: undefined
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('NONEXISTENT')").unwrap();
        let record = reader.records().next().unwrap().unwrap();
        assert!(!js_eval.eval::<bool>(record).unwrap());
    }

    #[test]
    fn test_eval_vec_i32() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "[1, 2, 3]").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Vec<i32> = js_eval.eval(record).unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn test_eval_vec_f64() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "[0.1, 0.2, 0.7]").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Vec<f64> = js_eval.eval(record).unwrap();
        assert_eq!(result.len(), 3);
        assert!((result[0] - 0.1).abs() < 0.001);
        assert!((result[1] - 0.2).abs() < 0.001);
        assert!((result[2] - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_eval_vec_string() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.alt").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Vec<String> = js_eval.eval(record).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_eval_option_some() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP')").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Option<i32> = js_eval.eval(record).unwrap();
        assert_eq!(result, Some(10));
    }

    #[test]
    fn test_eval_option_none() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('NONEXISTENT')").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Option<i32> = js_eval.eval(record).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_eval_option_null() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "null").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Option<String> = js_eval.eval(record).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_eval_type_mismatch_int() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "'hello'").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Result<i32, EvalError> = js_eval.eval(record);
        assert!(result.is_err());
        match result {
            Err(EvalError::RuntimeError(msg)) => {
                assert!(msg.contains("expected i32"));
                assert!(msg.contains("hello"));
            }
            _ => panic!("expected RuntimeError"),
        }
    }

    #[test]
    fn test_eval_type_mismatch_array() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "42").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Result<Vec<i32>, EvalError> = js_eval.eval(record);
        assert!(result.is_err());
        match result {
            Err(EvalError::RuntimeError(msg)) => {
                assert!(msg.contains("expected array"));
                assert!(msg.contains("42"));
            }
            _ => panic!("expected RuntimeError"),
        }
    }

    #[test]
    fn test_eval_type_mismatch_truncates_long_value() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        // Create a long string that should be truncated in error message
        let mut js_eval = Evaluator::new(
            reader.header(),
            "'this is a very long string that should be truncated in the error message to avoid excessive output'",
        )
        .unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Result<i32, EvalError> = js_eval.eval(record);
        assert!(result.is_err());
        match result {
            Err(EvalError::RuntimeError(msg)) => {
                assert!(msg.contains("..."));
                assert!(msg.len() < 100); // Should be truncated
            }
            _ => panic!("expected RuntimeError"),
        }
    }

    #[test]
    fn test_multiple_evals() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.pos").unwrap();

        let mut positions = Vec::new();
        for result in reader.records() {
            let record = result.unwrap();
            let pos: i64 = js_eval.eval(record).unwrap();
            positions.push(pos);
        }

        assert!(!positions.is_empty());
        assert_eq!(positions[0], 1000);
    }

    #[test]
    fn test_compile_error() {
        let path = fixture_vcf();
        let reader = bcf::Reader::from_path(&path).unwrap();

        let result = Evaluator::new(reader.header(), "this is not valid javascript {{{{");
        assert!(result.is_err());
        match result {
            Err(EvalError::CompileError(_)) => {}
            _ => panic!("expected CompileError"),
        }
    }

    #[test]
    fn test_runtime_error() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval =
            Evaluator::new(reader.header(), "throw new Error('test error')").unwrap();

        let record = reader.records().next().unwrap().unwrap();
        let result: Result<String, EvalError> = js_eval.eval(record);
        assert!(result.is_err());
        match result {
            Err(EvalError::RuntimeError(_)) => {}
            _ => panic!("expected RuntimeError"),
        }
    }

    #[test]
    fn test_many_records() {
        // Create a VCF with many records
        let path = tmp_path("many_records.vcf");
        let mut vcf = String::from(
            "##fileformat=VCFv4.2\n\
             ##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
             ##contig=<ID=chr1>\n\
             #CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n",
        );
        for i in 1..=1000 {
            vcf.push_str(&format!("chr1\t{}\t.\tA\tC\t.\t.\tDP={}\n", i, i % 100));
        }
        fs::write(&path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut js_eval = Evaluator::new(reader.header(), "variant.info('DP')").unwrap();

        let mut count = 0;
        for result in reader.records() {
            let record = result.unwrap();
            let _: i32 = js_eval.eval(record).unwrap();
            count += 1;
        }

        assert_eq!(count, 1000);
        let _ = fs::remove_file(&path);
    }
}
