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

type AnyError = Box<dyn std::error::Error + Send + Sync>;

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

    /// number of reader threads (default: 3)
    #[argh(option, short = 't', default = "3")]
    threads: usize,
}

fn main() -> Result<(), AnyError> {
    let args: Args = argh::from_env();

    let mut reader = bcf::Reader::from_path(&args.input)?;
    reader.set_threads(args.threads)?;
    let mut evaluator = Evaluator::new(reader.header(), &args.js_expr)?;

    let stdout = std::io::stdout().lock();
    let mut writer = BufWriter::new(stdout);

    let mut record = reader.empty_record();
    while let Some(result) = reader.read(&mut record) {
        result?;
        let output: String = evaluator.eval(record)?;
        writeln!(writer, "{:}", output)?;
        record = reader.empty_record();
    }

    writer.flush()?;
    Ok(())
}
