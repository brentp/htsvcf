//! VCF/BCF header representation and manipulation.
//!
//! This module provides the [`Header`] struct, which wraps an htslib `bcf_hdr_t`
//! pointer and provides safe access to header metadata including sample names,
//! INFO/FORMAT field definitions, and header modification.
//!
//! # Ownership
//!
//! [`Header`] owns its underlying `bcf_hdr_t*` via duplication. This ensures
//! correct lifetime management and thread safety - the header can outlive the
//! reader it was created from.
//!
//! # Caching
//!
//! Sample names and tag ID-to-name mappings are cached at construction time
//! for O(1) lookups during variant processing.
//!
//! # Example
//!
//! ```no_run
//! use htsvcf_core::header::Header;
//! use rust_htslib::bcf::{self, Read};
//!
//! let reader = bcf::Reader::from_path("input.vcf.gz").unwrap();
//! let header = unsafe { Header::new(reader.header().inner) };
//!
//! // Access sample information
//! println!("Samples: {:?}", header.sample_names());
//!
//! // Look up INFO field type
//! if let Some((ty, len)) = header.info_type(b"DP") {
//!     println!("DP is {:?} with length {:?}", ty, len);
//! }
//! ```

use rust_htslib::bcf;
use rust_htslib::bcf::header::{HeaderRecord, TagLength, TagType};
use std::collections::HashMap;
use std::ffi::CString;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};

impl Drop for Header {
  fn drop(&mut self) {
    unsafe {
      rust_htslib::htslib::bcf_hdr_destroy(self.inner);
    }
  }
}

/// A VCF/BCF header with metadata and sample information.
///
/// `Header` owns its underlying `bcf_hdr_t*` via duplication. It provides
/// access to sample names, INFO/FORMAT field definitions, and supports
/// header modification (adding INFO/FORMAT fields).
///
/// # Caching
///
/// Sample names and tag ID-to-name mappings are cached at construction time
/// for O(1) lookups during variant processing.
#[derive(Debug)]
pub struct Header {
  inner: *mut rust_htslib::htslib::bcf_hdr_t,
  dirty: AtomicBool,
  /// Cached sample names in header order.
  sample_names: Vec<String>,
  /// Cached map from sample name to index for O(1) lookup.
  sample_name_to_idx: HashMap<String, usize>,
  /// Cached map from tag ID to (name_string, name_bytes) for O(1) lookup.
  /// This covers both INFO and FORMAT tags since they share the ID namespace.
  id_to_name_cache: HashMap<u32, (String, Vec<u8>)>,
}

/// Represents a field definition from the VCF header (INFO, FORMAT, or FILTER).
#[derive(Debug, Clone)]
pub struct HeaderField {
  /// The field ID (e.g., "DP", "GQ").
  pub id: String,
  /// The field type ("Integer", "Float", "String", "Flag").
  pub r#type: String,
  /// The Number field ("1", "A", "R", "G", ".").
  pub number: String,
  /// The description from the header.
  pub description: String,
}

unsafe impl Send for Header {}
unsafe impl Sync for Header {}

impl Header {
  /// Duplicate the underlying header and take ownership.
  ///
  /// This is important for thread safety and correct lifetime management: the
  /// returned `Header` owns its internal `bcf_hdr_t*` and frees it on drop.
  /// # Safety
  ///
  /// `inner` must be a valid pointer to a `bcf_hdr_t`.
  pub unsafe fn new(inner: *mut rust_htslib::htslib::bcf_hdr_t) -> Self {
    let inner = rust_htslib::htslib::bcf_hdr_dup(inner);
    let view = ManuallyDrop::new(bcf::header::HeaderView::new(inner));
    let sample_count = view.sample_count();
    let (sample_names, sample_name_to_idx) = if sample_count > 0 {
      let names: Vec<String> = view
        .samples()
        .iter()
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
      let name_to_idx: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), i))
        .collect();
      (names, name_to_idx)
    } else {
      (Vec::new(), HashMap::new())
    };

    // Build id_to_name cache from INFO and FORMAT records
    let mut id_to_name_cache: HashMap<u32, (String, Vec<u8>)> = HashMap::new();
    for record in view.header_records() {
      let tag_id = match &record {
        HeaderRecord::Info { values, .. } | HeaderRecord::Format { values, .. } => {
          values.iter().find(|(k, _)| k.as_str() == "ID").map(|(_, v)| v.as_str())
        }
        _ => None,
      };
      if let Some(tag_name) = tag_id {
        let tag_bytes = tag_name.as_bytes();
        if let Ok(id) = view.name_to_id(tag_bytes) {
          let name = tag_name.to_string();
          let bytes = tag_bytes.to_vec();
          id_to_name_cache.insert(id.0, (name, bytes));
        }
      }
    }

    Self {
      inner,
      dirty: AtomicBool::new(false),
      sample_names,
      sample_name_to_idx,
      id_to_name_cache,
    }
  }

  /// Create an empty header (for writing new VCFs).
  pub fn empty() -> Self {
    let c_str = CString::new(&b"w"[..]).unwrap();
    let inner = unsafe { rust_htslib::htslib::bcf_hdr_init(c_str.as_ptr()) };
    Self {
      inner,
      dirty: AtomicBool::new(false),
      sample_names: Vec::new(),
      sample_name_to_idx: HashMap::new(),
      id_to_name_cache: HashMap::new(),
    }
  }

  /// Get the raw `bcf_hdr_t` pointer.
  ///
  /// # Safety
  ///
  /// The returned pointer is valid for the lifetime of this `Header`.
  pub fn inner_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
    self.inner
  }

  /// Get a temporary header view (internal use).
  fn view(&self) -> ManuallyDrop<bcf::header::HeaderView> {
    ManuallyDrop::new(bcf::header::HeaderView::new(self.inner))
  }

  /// Get all header records (INFO, FORMAT, FILTER, contig, etc.).
  pub fn header_records(&self) -> Vec<HeaderRecord> {
    self.view().header_records()
  }

  /// Get the sample index for a sample name.
  ///
  /// Returns `None` if the sample is not found.
  pub fn sample_id(&self, sample: &[u8]) -> Option<usize> {
    match self.view().sample_to_id(sample) {
      Ok(id) => Some(id.0 as usize),
      Err(_) => None,
    }
  }

  /// Get the tag name for a numeric ID (INFO/FORMAT).
  ///
  /// This performs a fresh lookup; prefer [`id_to_name_cached`](Self::id_to_name_cached)
  /// for repeated calls.
  pub fn id_to_name(&self, id: u32) -> Vec<u8> {
    self.view().id_to_name(bcf::header::Id(id))
  }

  /// Get the cached name for a tag ID, returning both String and bytes.
  ///
  /// Falls back to [`id_to_name`](Self::id_to_name) if not in cache
  /// (e.g., for dynamically added tags).
  pub fn id_to_name_cached(&self, id: u32) -> (String, Vec<u8>) {
    if let Some(cached) = self.id_to_name_cache.get(&id) {
      return cached.clone();
    }
    // Fallback for tags added after construction
    let bytes = self.view().id_to_name(bcf::header::Id(id));
    let name = String::from_utf8_lossy(&bytes).into_owned();
    (name, bytes)
  }

  /// Get the number of samples in the VCF.
  pub fn sample_count(&self) -> usize {
    self.sample_names.len()
  }

  /// Get all sample names in header order.
  pub fn sample_names(&self) -> &[String] {
    &self.sample_names
  }

  /// Get the index of a sample by name.
  ///
  /// Returns `None` if the sample is not found.
  pub fn sample_idx(&self, name: &str) -> Option<usize> {
    self.sample_name_to_idx.get(name).copied()
  }

  /// Get a reference to the sample name-to-index map.
  pub fn sample_name_to_idx(&self) -> &HashMap<String, usize> {
    &self.sample_name_to_idx
  }

  /// Get the type and length of an INFO field.
  ///
  /// Returns `None` if the tag is not defined in the header.
  pub fn info_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
    self.view().info_type(tag).ok()
  }

  /// Get the type and length of a FORMAT field.
  ///
  /// Returns `None` if the tag is not defined in the header.
  pub fn format_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
    self.view().format_type(tag).ok()
  }

  /// Synchronize header changes to the underlying htslib structure.
  ///
  /// Called automatically after modifications; usually not needed directly.
  pub fn sync(&self) {
    if !self.dirty.swap(false, Ordering::AcqRel) {
      return;
    }
    unsafe {
      rust_htslib::htslib::bcf_hdr_sync(self.inner);
    }
  }

  /// Append a raw header line (e.g., `##INFO=<...>`).
  ///
  /// Returns `true` on success, `false` on failure.
  pub fn push_record(&self, record: &[u8]) -> bool {
    let Ok(c_str) = CString::new(record) else {
      return false;
    };
    let r = unsafe { rust_htslib::htslib::bcf_hdr_append(self.inner, c_str.as_ptr()) };
    self.dirty.store(true, Ordering::Release);
    self.sync();
    r == 0
  }

  /// Add an INFO field definition to the header.
  ///
  /// # Arguments
  ///
  /// * `id` - Field ID (e.g., "DP")
  /// * `number` - Number field ("1", "A", "R", "G", ".")
  /// * `ty` - Type ("Integer", "Float", "String", "Flag")
  /// * `description` - Human-readable description
  ///
  /// Returns `true` on success.
  pub fn add_info(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
    let record =
      format!("##INFO=<ID={id},Number={number},Type={ty},Description=\"{description}\">",
    );
    self.push_record(record.as_bytes())
  }

  /// Add a FORMAT field definition to the header.
  ///
  /// # Arguments
  ///
  /// * `id` - Field ID (e.g., "GQ")
  /// * `number` - Number field ("1", "A", "R", "G", ".")
  /// * `ty` - Type ("Integer", "Float", "String")
  /// * `description` - Human-readable description
  ///
  /// Returns `true` on success.
  pub fn add_format(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
    let record =
      format!("##FORMAT=<ID={id},Number={number},Type={ty},Description=\"{description}\">",
    );
    self.push_record(record.as_bytes())
  }

  /// Format the header as a VCF header string.
  ///
  /// Returns `None` if formatting fails.
  pub fn to_string(&self) -> Option<String> {
    self.sync();

    let mut s = rust_htslib::htslib::kstring_t {
      l: 0,
      m: 0,
      s: std::ptr::null_mut(),
    };

    let ret = unsafe { rust_htslib::htslib::bcf_hdr_format(self.inner_ptr(), 0, &mut s) };
    if ret != 0 {
      if !s.s.is_null() {
        unsafe { rust_htslib::htslib::free(s.s as *mut std::os::raw::c_void) };
      }
      return None;
    }

    let bytes = unsafe { std::slice::from_raw_parts(s.s as *const u8, s.l as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();

    if !s.s.is_null() {
      unsafe { rust_htslib::htslib::free(s.s as *mut std::os::raw::c_void) };
    }

    Some(text)
  }

  /// Get a specific field definition by section and ID.
  ///
  /// # Arguments
  ///
  /// * `section` - "INFO" or "FORMAT"
  /// * `id` - Field ID (e.g., "DP")
  ///
  /// Returns `None` if the field is not found.
  pub fn get_field(&self, section: &str, id: &str) -> Option<HeaderField> {
    let tag_info = match section {
      "INFO" => self.info_type(id.as_bytes()),
      "FORMAT" => self.format_type(id.as_bytes()),
      _ => return None,
    };

    let (tag_type, tag_length) = tag_info?;

    let mut description = String::new();
    for record in self.header_records() {
      match record {
        HeaderRecord::Info { values, .. } if section == "INFO" => {
          if values.iter().any(|(k, v)| k.as_str() == "ID" && v == id) {
            description = values
              .iter()
              .find(|(k, _)| k.as_str() == "Description")
              .map(|(_, v)| unquote(v))
              .unwrap_or_default();
            break;
          }
        }
        HeaderRecord::Format { values, .. } if section == "FORMAT" => {
          if values.iter().any(|(k, v)| k.as_str() == "ID" && v == id) {
            description = values
              .iter()
              .find(|(k, _)| k.as_str() == "Description")
              .map(|(_, v)| unquote(v))
              .unwrap_or_default();
            break;
          }
        }
        _ => {}
      }
    }

    Some(HeaderField {
      id: id.to_string(),
      r#type: tag_type_to_str(tag_type).to_string(),
      number: tag_length_to_str(tag_length),
      description,
    })
  }

  /// Get all INFO, FORMAT, and FILTER field definitions.
  ///
  /// Returns a vector of (section, field) tuples.
  pub fn all_fields(&self) -> Vec<(String, HeaderField)> {
    let mut fields = Vec::new();
    for record in self.header_records() {
      match record {
        HeaderRecord::Info { values, .. } => {
          if let Some(field) = self.parse_record_to_field("INFO", values.into_iter().collect()) {
            fields.push(("INFO".to_string(), field));
          }
        }
        HeaderRecord::Format { values, .. } => {
          if let Some(field) = self.parse_record_to_field("FORMAT", values.into_iter().collect()) {
            fields.push(("FORMAT".to_string(), field));
          }
        }
        HeaderRecord::Filter { values, .. } => {
          if let Some(field) = self.parse_record_to_field("FILTER", values.into_iter().collect()) {
            fields.push(("FILTER".to_string(), field));
          }
        }
        _ => {}
      }
    }
    fields
  }

  /// Parse a header record into a HeaderField (internal use).
  fn parse_record_to_field(
    &self,
    section: &str,
    values: Vec<(String, String)>,
  ) -> Option<HeaderField> {
    let id = values.iter().find(|(k, _)| k.as_str() == "ID").map(|(_, v)| v.as_str())?;

    let (tag_type, tag_length) = match section {
      "INFO" => self.info_type(id.as_bytes())?,
      "FORMAT" => self.format_type(id.as_bytes())?,
      "FILTER" => (TagType::Flag, TagLength::Fixed(0)), // FILTER is implicitly a flag-like type
      _ => return None,
    };

    let description = values
      .iter()
      .find(|(k, _)| k.as_str() == "Description")
      .map(|(_, v)| unquote(v))
      .unwrap_or_default();

    Some(HeaderField {
      id: id.to_string(),
      r#type: tag_type_to_str(tag_type).to_string(),
      number: tag_length_to_str(tag_length),
      description,
    })
  }
}

fn unquote(s: &str) -> String {
  if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
    s[1..s.len() - 1].to_string()
  } else {
    s.to_string()
  }
}

fn tag_type_to_str(t: TagType) -> &'static str {
  match t {
    TagType::Flag => "Flag",
    TagType::Integer => "Integer",
    TagType::Float => "Float",
    TagType::String => "String",
  }
}

fn tag_length_to_str(l: TagLength) -> String {
  match l {
    TagLength::Fixed(n) => n.to_string(),
    TagLength::AltAlleles => "A".to_string(),
    TagLength::Alleles => "R".to_string(),
    TagLength::Genotypes => "G".to_string(),
    TagLength::Variable => ".".to_string(),
  }
}
