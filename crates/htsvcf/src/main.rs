use std::io::{BufWriter, Write};

use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};

type AnyError = Box<dyn std::error::Error + Send + Sync>;

/// CLI entrypoint.
///
/// Usage: `htsvcf <input.vcf|input.bcf> [js_expr]`.
fn main() -> Result<(), AnyError> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "htsvcf".to_string());

    let Some(path) = args.next() else {
        eprintln!("usage: {program} <input.vcf|input.bcf> [js_expr]");
        eprintln!("example: {program} input.vcf.gz 'variant.chrom + \":\" + variant.start'");
        return Ok(());
    };

    let js_expr = args.next().unwrap_or_else(|| "variant.start".to_string());

    let mut reader = bcf::Reader::from_path(&path)?;
    let mut evaluator = Evaluator::new(reader.header(), &js_expr)?;

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
