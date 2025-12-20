use crate::Header;
use rust_htslib::bcf;
use rust_htslib::errors::Result;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
  Vcf,
  Bcf,
}

#[derive(Debug, Clone, Default)]
pub struct WriterOptions {
  pub format: Option<OutputFormat>,
  pub uncompressed: bool,
  pub threads: Option<usize>,
}

#[derive(Debug)]
pub struct Writer {
  inner: bcf::Writer,
}

impl Writer {
  pub fn write_record(&mut self, record: &mut bcf::Record) -> Result<()> {
    self.inner.subset(record);
    self.inner.write(record)
  }
}

fn infer_format(path: &str) -> OutputFormat {
  if path == "-" {
    return OutputFormat::Vcf;
  }
  if path.ends_with(".bcf") {
    OutputFormat::Bcf
  } else {
    OutputFormat::Vcf
  }
}

fn to_rust_htslib_format(format: OutputFormat) -> bcf::Format {
  match format {
    OutputFormat::Vcf => bcf::Format::Vcf,
    OutputFormat::Bcf => bcf::Format::Bcf,
  }
}

fn clone_as_rust_htslib_header(header: &Header) -> bcf::header::Header {
  let dup = unsafe { rust_htslib::htslib::bcf_hdr_dup(header.inner_ptr()) };
  // SAFETY: dup is a fresh header owned by this header wrapper and will be freed
  // by rust-htslib's Header Drop impl.
  bcf::header::Header { inner: dup, subset: None }
}

pub fn open_writer(path: &str, header: &Header, opts: WriterOptions) -> Result<Writer> {
  let format = opts.format.unwrap_or_else(|| infer_format(path));
  let rust_header = clone_as_rust_htslib_header(header);

  let inner = if path == "-" {
    bcf::Writer::from_stdout(&rust_header, opts.uncompressed, to_rust_htslib_format(format))?
  } else {
    bcf::Writer::from_path(Path::new(path), &rust_header, opts.uncompressed, to_rust_htslib_format(format))?
  };

  let mut out = Writer { inner };
  if let Some(threads) = opts.threads {
    if threads > 0 {
      out.inner.set_threads(threads)?;
    }
  }

  Ok(out)
}
