//! Command-line interface for htsvcf.
//!
//! This binary evaluates JavaScript expressions on VCF/BCF records, streaming
//! one output line per input variant.
//!
//! # Usage
//!
//! ```bash
//! htsvcf input.vcf.gz "variant.chrom + ':' + variant.pos"
//! ```
//!
//! The expression has access to `variant` (the current record) and `header`
//! (VCF header metadata). See the library documentation for the full API.

use std::io::{BufWriter, Write};

use argh::FromArgs;
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};
use std::sync::Arc;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

fn load_and_run_prelude(
    evaluator: &mut Evaluator,
    prelude_path: Option<&str>,
) -> Result<Option<Arc<bcf::header::HeaderView>>, AnyError> {
    let Some(path) = prelude_path else {
        return Ok(None);
    };

    let prelude = std::fs::read_to_string(path)?;
    evaluator.run(&prelude)?;
    Ok(Some(evaluator.header()?))
}

/// Evaluate JavaScript expressions on VCF/BCF records.
#[derive(FromArgs)]
#[argh(help_triggers("-h", "--help", ""))]
struct Args {
    /// input VCF or BCF file
    #[argh(positional)]
    input: String,

    /// javaScript expression to evaluate (default: "variant.start")
    #[argh(positional, default = "String::from(\"variant.start\")")]
    js_expr: String,

    /// optional JavaScript file to run once before processing records
    ///
    /// May define helper functions and/or modify the header (e.g. `header.addInfo(...)`).
    #[argh(option)]
    prelude: Option<String>,

    /// number of reader threads (default: 3)
    #[argh(option, short = 't', default = "3")]
    threads: usize,
}

fn main() -> Result<(), AnyError> {
    let args: Args = argh::from_env();

    let mut reader = bcf::Reader::from_path(&args.input)?;
    reader.set_threads(args.threads)?;
    let mut evaluator = Evaluator::new(reader.header())?;

    let mut updated_header = load_and_run_prelude(&mut evaluator, args.prelude.as_deref())?;

    let stdout = std::io::stdout().lock();
    let mut writer = BufWriter::new(stdout);

    let mut record = reader.empty_record();
    while let Some(result) = reader.read(&mut record) {
        result?;

        if let Some(ref mut header) = updated_header {
            record.translate(header)?;
        }

        evaluator.set_record(record);
        let output: String = evaluator.eval(&args.js_expr)?;
        writeln!(writer, "{output}")?;

        record = evaluator.take().unwrap_or_else(|| reader.empty_record());
    }

    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;

    fn fixture_vcf() -> String {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/t.vcf.gz")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn test_prelude_mutates_header_translate_allows_set_and_to_string() {
        let path = fixture_vcf();
        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let mut evaluator = Evaluator::new(reader.header()).unwrap();

        // what the CLI would load/run from --prelude
        let prelude = "header.addInfo('NEW_FIELD','1','Integer','test field')";
        evaluator.run(prelude).unwrap();
        let mut header = evaluator.header().unwrap();

        let mut record = reader.records().next().unwrap().unwrap();
        record.translate(&mut header).unwrap();

        evaluator.set_record(record);
        evaluator.run("variant.set_info('NEW_FIELD', 32)").unwrap();

        let line: String = evaluator.eval("variant.toString()").unwrap();
        assert!(line.contains("NEW_FIELD=32"));
    }
}
