use rust_htslib::bcf;
use rust_htslib::bcf::Read;

use crate::region::parse_region_1based;

#[derive(Debug)]
pub enum InnerReader {
  Unindexed(bcf::Reader),
  Indexed(bcf::IndexedReader),
}

impl InnerReader {
  pub fn header_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
    match self {
      InnerReader::Unindexed(r) => r.header().inner,
      InnerReader::Indexed(r) => r.header().inner,
    }
  }

  pub fn empty_record(&self) -> bcf::Record {
    match self {
      InnerReader::Unindexed(r) => r.empty_record(),
      InnerReader::Indexed(r) => r.empty_record(),
    }
  }

  pub fn read_record(
    &mut self,
    record: &mut bcf::Record,
  ) -> Option<Result<(), rust_htslib::errors::Error>> {
    match self {
      InnerReader::Unindexed(r) => r.read(record),
      InnerReader::Indexed(r) => r.read(record),
    }
  }

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

#[derive(Debug)]
pub struct Reader {
  inner: InnerReader,
  has_index: bool,
}

fn has_index_on_disk(path: &str) -> bool {
  let candidates = [format!("{path}.csi"), format!("{path}.tbi")];
  candidates.iter().any(|p| std::fs::metadata(p).is_ok())
}

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
  pub fn has_index(&self) -> bool {
    self.has_index
  }

  pub fn header_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
    self.inner.header_ptr()
  }

  pub fn query_region_1based(&mut self, region: &str) -> Result<(), rust_htslib::errors::Error> {
    let (chrom, start0, end0) = parse_region_1based(region).ok_or(rust_htslib::errors::Error::Fetch)?;
    self.query_chrom_start_end(&chrom, start0, end0)
  }

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

  pub fn next_record(&mut self) -> Result<Option<bcf::Record>, rust_htslib::errors::Error> {
    let mut record = self.inner.empty_record();
    match self.inner.read_record(&mut record) {
      None => Ok(None),
      Some(Ok(())) => Ok(Some(record)),
      Some(Err(e)) => Err(e),
    }
  }
}
