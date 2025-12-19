//! VCF/BCF file reader with optional index support.
//!
//! This module provides [`Reader`], a unified interface for reading VCF/BCF files
//! that automatically detects and uses tabix (.tbi) or CSI (.csi) indices when
//! available.
//!
//! # Index Detection
//!
//! When opening a file, the reader checks for index files at `{path}.tbi` and
//! `{path}.csi`. If found, region queries are enabled via [`Reader::query`].
//!
//! # Example
//!
//! ```no_run
//! use htsvcf_core::reader::open_reader;
//!
//! let mut reader = open_reader("input.vcf.gz").unwrap();
//!
//! // Check if indexed
//! if reader.has_index() {
//!     // Query a specific region
//!     reader.query("chr1:1000-2000", None, None).unwrap();
//! }
//!
//! // Iterate over records
//! while let Ok(Some(record)) = reader.next_record() {
//!     println!("pos = {}", record.pos());
//! }
//! ```
//!
//! # Types
//!
//! - [`Reader`]: Main reader interface
//! - [`InnerReader`]: Enum wrapping indexed or unindexed htslib readers
//! - [`open_reader`]: Constructor that auto-detects index availability

use rust_htslib::bcf;
use rust_htslib::bcf::Read;

use crate::region::parse_region_1based;

/// Internal enum wrapping indexed or unindexed htslib readers.
///
/// This allows the `Reader` to work with both indexed and unindexed
/// VCF/BCF files through a unified interface.
#[derive(Debug)]
pub enum InnerReader {
  /// An unindexed reader (sequential access only).
  Unindexed(bcf::Reader),
  /// An indexed reader (supports region queries).
  Indexed(bcf::IndexedReader),
}

impl InnerReader {
  /// Get the raw header pointer.
  pub fn header_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
    match self {
      InnerReader::Unindexed(r) => r.header().inner,
      InnerReader::Indexed(r) => r.header().inner,
    }
  }

  /// Create an empty record for reading into.
  pub fn empty_record(&self) -> bcf::Record {
    match self {
      InnerReader::Unindexed(r) => r.empty_record(),
      InnerReader::Indexed(r) => r.empty_record(),
    }
  }

  /// Read the next record into the provided buffer.
  ///
  /// Returns `None` at EOF, `Some(Ok(()))` on success, or `Some(Err(...))` on error.
  pub fn read_record(
    &mut self,
    record: &mut bcf::Record,
  ) -> Option<Result<(), rust_htslib::errors::Error>> {
    match self {
      InnerReader::Unindexed(r) => r.read(record),
      InnerReader::Indexed(r) => r.read(record),
    }
  }

  /// Fetch records from a genomic region (indexed readers only).
  ///
  /// After calling this, subsequent `read_record` calls will return
  /// only records overlapping the specified region.
  pub fn fetch(
    &mut self,
    chrom: &str,
    start0: u64,
    end0: Option<u64>,
  ) -> Result<(), rust_htslib::errors::Error> {
    let InnerReader::Indexed(r) = self else {
      return Err(rust_htslib::errors::Error::Fetch);
    };

    let rid = r.header().name2rid(chrom.as_bytes())?;
    r.fetch(rid, start0, end0)
  }
}

/// A VCF/BCF file reader with automatic index detection.
///
/// `Reader` provides a unified interface for reading VCF/BCF files.
/// When opening a file, it automatically checks for index files
/// (`.tbi` or `.csi`) and enables region queries if found.
#[derive(Debug)]
pub struct Reader {
  inner: InnerReader,
  has_index: bool,
}

/// Check if an index file exists on disk.
fn has_index_on_disk(path: &str) -> bool {
  let candidates = [format!("{path}.csi"), format!("{path}.tbi")];
  candidates.iter().any(|p| std::fs::metadata(p).is_ok())
}

/// Open a VCF/BCF file for reading.
///
/// Automatically detects and uses tabix (`.tbi`) or CSI (`.csi`) indices
/// when available. If an index is found, region queries are enabled.
///
/// # Example
///
/// ```no_run
/// use htsvcf_core::reader::open_reader;
///
/// let mut reader = open_reader("input.vcf.gz").unwrap();
/// while let Ok(Some(record)) = reader.next_record() {
///     // process record
/// }
/// ```
pub fn open_reader(path: &str) -> Result<Reader, rust_htslib::errors::Error> {
  let has_index = has_index_on_disk(path);

  let inner = if has_index {
    match bcf::IndexedReader::from_path(path) {
      Ok(r) => InnerReader::Indexed(r),
      Err(_) => InnerReader::Unindexed(bcf::Reader::from_path(path)?),
    }
  } else {
    InnerReader::Unindexed(bcf::Reader::from_path(path)?)
  };

  let has_index = matches!(inner, InnerReader::Indexed(_)) && has_index;
  Ok(Reader { inner, has_index })
}

impl Reader {
  /// Check if this reader has an index available.
  ///
  /// Returns `true` if region queries are supported.
  pub fn has_index(&self) -> bool {
    self.has_index
  }

  /// Get the raw header pointer.
  pub fn header_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
    self.inner.header_ptr()
  }

  /// Query by a 1-based region string (e.g., "chr1:1000-2000").
  ///
  /// After calling this, subsequent `next_record` calls will return
  /// only records overlapping the specified region.
  pub fn query_region_1based(&mut self, region: &str) -> Result<(), rust_htslib::errors::Error> {
    let (chrom, start0, end0) = parse_region_1based(region).ok_or(rust_htslib::errors::Error::Fetch)?;
    self.query_chrom_start_end(&chrom, start0, end0)
  }

  /// Query by chromosome and 0-based coordinates.
  ///
  /// After calling this, subsequent `next_record` calls will return
  /// only records overlapping the specified region.
  pub fn query_chrom_start_end(
    &mut self,
    chrom: &str,
    start0: u64,
    end0: Option<u64>,
  ) -> Result<(), rust_htslib::errors::Error> {
    self.inner.fetch(chrom, start0, end0)
  }

  /// Query by region string ("chr1:1000-2000") or by coordinates.
  /// If `start0` is None, treat `region_or_chrom` as a region string.
  /// Otherwise, treat it as a chromosome name with numeric coordinates.
  pub fn query(
    &mut self,
    region_or_chrom: &str,
    start0: Option<u64>,
    end0: Option<u64>,
  ) -> Result<(), rust_htslib::errors::Error> {
    match start0 {
      None => self.query_region_1based(region_or_chrom),
      Some(s) => self.query_chrom_start_end(region_or_chrom, s, end0),
    }
  }

  /// Read the next record.
  ///
  /// Returns `Ok(Some(record))` if a record was read, `Ok(None)` at EOF,
  /// or `Err(...)` on error.
  pub fn next_record(&mut self) -> Result<Option<bcf::Record>, rust_htslib::errors::Error> {
    let mut record = self.inner.empty_record();
    match self.inner.read_record(&mut record) {
      None => Ok(None),
      Some(Ok(())) => Ok(Some(record)),
      Some(Err(e)) => Err(e),
    }
  }
}
