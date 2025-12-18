use crate::header::Header;
use rust_htslib::bcf;
use rust_htslib::bcf::header::{TagLength, TagType};
use rust_htslib::bcf::record::Numeric;
use std::ffi::CString;

#[derive(Debug, Clone, PartialEq)]
pub enum InfoValue {
  Absent,
  Missing,
  Bool(bool),
  Int(i32),
  Float(f32),
  String(String),
  Array(Vec<InfoValue>),
}


#[derive(Debug, Clone, PartialEq)]
pub enum FormatValue {
  Absent,
  Missing,
  Int(i32),
  Float(f32),
  String(String),
  Array(Vec<FormatValue>),
  PerSample(Vec<FormatValue>),
}

// ============================================================================
// Public helper functions for working with bcf::Record references directly.
// These allow bindings (like v8) that can't own the record to still use the
// core logic.
// ============================================================================

/// Get an INFO field value from a record.
///
/// This is the standalone version of `Variant::info()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_info(record: &bcf::Record, header: &Header, tag: &str) -> InfoValue {
  let (tag_type, tag_length) = match header.info_type(tag.as_bytes()) {
    Some(v) => v,
    None => return InfoValue::Absent,
  };

  match tag_type {
    TagType::Flag => match header_info_flag(header, record, tag.as_bytes()) {
      Ok(v) => InfoValue::Bool(v),
      Err(InfoError::Absent) => InfoValue::Absent,
      Err(InfoError::Other) => InfoValue::Absent,
    },
    TagType::Integer => match header_info_values_i32(header, record, tag.as_bytes()) {
      Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Int),
      Err(InfoError::Absent) => InfoValue::Absent,
      Err(InfoError::Other) => InfoValue::Absent,
    },
    TagType::Float => match header_info_values_f32(header, record, tag.as_bytes()) {
      Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Float),
      Err(InfoError::Absent) => InfoValue::Absent,
      Err(InfoError::Other) => InfoValue::Absent,
    },
    TagType::String => match header_info_values_string(header, record, tag.as_bytes()) {
      Ok(v) => string_to_infovalue(v, tag_length),
      Err(InfoError::Absent) => InfoValue::Absent,
      Err(InfoError::Other) => InfoValue::Absent,
    },
  }
}

/// Get a FORMAT field value from a record (per-sample).
///
/// This is the standalone version of `Variant::format()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_format(record: &bcf::Record, header: &Header, tag: &str) -> FormatValue {
  let (tag_type, tag_length) = match header.format_type(tag.as_bytes()) {
    Some(v) => v,
    None => return FormatValue::Absent,
  };

  let sample_count = record.sample_count() as usize;

  match tag_type {
    TagType::Integer => match record.format(tag.as_bytes()).integer() {
      Ok(values) => FormatValue::PerSample(
        values
          .iter()
          .take(sample_count)
          .map(|per_sample| format_numeric_to_value(per_sample, tag_length, FormatValue::Int))
          .collect(),
      ),
      Err(_) => FormatValue::Absent,
    },
    TagType::Float => match record.format(tag.as_bytes()).float() {
      Ok(values) => FormatValue::PerSample(
        values
          .iter()
          .take(sample_count)
          .map(|per_sample| format_numeric_to_value(per_sample, tag_length, FormatValue::Float))
          .collect(),
      ),
      Err(_) => FormatValue::Absent,
    },
    TagType::String => match record.format(tag.as_bytes()).string() {
      Ok(values) => FormatValue::PerSample(
        values
          .iter()
          .take(sample_count)
          .map(|per_sample| format_string_to_value(*per_sample, tag_length))
          .collect(),
      ),
      Err(_) => FormatValue::Absent,
    },
    TagType::Flag => FormatValue::Absent,
  }
}

/// Get sample data from a record for a single sample by name.
///
/// This is the standalone version of `Variant::sample()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_sample(
  record: &bcf::Record,
  header: &Header,
  sample: &str,
) -> Option<Vec<(String, FormatValue)>> {
  let sample_id = header.sample_id(sample.as_bytes())?;
  let sample_count = record.sample_count() as usize;
  if sample_id >= sample_count {
    return None;
  }

  let format_tags = get_format_tag_names(header, record);
  let mut out: Vec<(String, FormatValue)> = Vec::with_capacity(format_tags.len() + 1);

  for (tag_name, tag_bytes) in format_tags {
    let Some(value) = format_value_for_sample(header, record, &tag_bytes, sample_id) else {
      continue;
    };
    out.push((tag_name, value));
  }

  // Include the sample name so JS bindings can expose it.
  // Set it last so it can't be overwritten by a FORMAT tag named "sample_name".
  out.push((
    "sample_name".to_string(),
    FormatValue::String(sample.to_string()),
  ));

  Some(out)
}

/// Get sample data from a record for all samples or a subset.
///
/// This is the standalone version of `Variant::samples()` that can be used when
/// you have a borrowed reference to a record (e.g., from a GcCell).
pub fn record_samples(
  record: &bcf::Record,
  header: &Header,
  subset: Option<&[&str]>,
) -> Vec<Vec<(String, FormatValue)>> {
  let sample_count = record.sample_count() as usize;
  if sample_count == 0 {
    return Vec::new();
  }

  let sample_names = header.sample_names();
  let format_tags = get_format_tag_names(header, record);

  // Determine which sample indices to include and in what order
  let sample_indices: Vec<usize> = match subset {
    None => (0..sample_count).collect(),
    Some(names) => {
      let name_to_idx = header.sample_name_to_idx();
      names
        .iter()
        .filter_map(|name| name_to_idx.get(*name).copied())
        .collect()
    }
  };

  if sample_indices.is_empty() {
    return Vec::new();
  }

  // Pre-allocate result vectors for each requested sample
  let mut results: Vec<Vec<(String, FormatValue)>> = sample_indices
    .iter()
    .map(|_| Vec::with_capacity(format_tags.len() + 1))
    .collect();

  // For each FORMAT tag, fetch values for ALL samples at once and distribute to requested ones
  for (tag_name, tag_bytes) in &format_tags {
    let Some((tag_type, tag_length)) = header.format_type(tag_bytes) else {
      continue;
    };

    match tag_type {
      bcf::header::TagType::Integer => {
        let Ok(all_values) = record.format(tag_bytes).integer() else {
          continue;
        };
        for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
          if let Some(per_sample) = all_values.get(sample_idx) {
            let value = format_numeric_to_value(per_sample, tag_length, FormatValue::Int);
            results[result_idx].push((tag_name.clone(), value));
          }
        }
      }
      bcf::header::TagType::Float => {
        let Ok(all_values) = record.format(tag_bytes).float() else {
          continue;
        };
        for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
          if let Some(per_sample) = all_values.get(sample_idx) {
            let value = format_numeric_to_value(per_sample, tag_length, FormatValue::Float);
            results[result_idx].push((tag_name.clone(), value));
          }
        }
      }
      bcf::header::TagType::String => {
        let Ok(all_values) = record.format(tag_bytes).string() else {
          continue;
        };
        for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
          if let Some(per_sample) = all_values.get(sample_idx) {
            let value = format_string_to_value(*per_sample, tag_length);
            results[result_idx].push((tag_name.clone(), value));
          }
        }
      }
      bcf::header::TagType::Flag => {
        // Flags are not valid for FORMAT
      }
    }
  }

  // Add sample_name to each result (last, so it can't be overwritten by a FORMAT tag)
  for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
    let name = sample_names
      .get(sample_idx)
      .map(|s| s.clone())
      .unwrap_or_else(|| format!("sample_{sample_idx}"));
    results[result_idx].push(("sample_name".to_string(), FormatValue::String(name)));
  }

  results
}

/// Get the list of FORMAT tag names present in a record.
pub fn get_format_tag_names(header: &Header, record: &bcf::Record) -> Vec<(String, Vec<u8>)> {
  let record_ptr = record.inner() as *const rust_htslib::htslib::bcf1_t
    as *mut rust_htslib::htslib::bcf1_t;

  let n_fmt = unsafe { (*record_ptr).n_fmt() as usize };
  let fmt_ptr = unsafe { (*record_ptr).d.fmt };

  if fmt_ptr.is_null() || n_fmt == 0 {
    return Vec::new();
  }

  let mut tags = Vec::with_capacity(n_fmt);
  for i in 0..n_fmt {
    let fmt = unsafe { *fmt_ptr.add(i) };
    let (tag_name, tag_bytes) = header.id_to_name_cached(fmt.id as u32);
    tags.push((tag_name, tag_bytes));
  }
  tags
}

/// Format a record as a VCF line string.
pub fn record_to_string(record: &bcf::Record, header: &Header) -> Option<String> {
  let mut s = rust_htslib::htslib::kstring_t {
    l: 0,
    m: 0,
    s: std::ptr::null_mut(),
  };

  let record_ptr = record.inner() as *const rust_htslib::htslib::bcf1_t
    as *mut rust_htslib::htslib::bcf1_t;

  let _ = unsafe {
    rust_htslib::htslib::bcf_unpack(record_ptr, rust_htslib::htslib::BCF_UN_ALL as i32)
  };

  let ret = unsafe {
    rust_htslib::htslib::vcf_format(
      header.inner_ptr() as *const rust_htslib::htslib::bcf_hdr_t,
      record_ptr as *const rust_htslib::htslib::bcf1_t,
      &mut s,
    )
  };
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

  Some(text.trim_end_matches('\n').to_string())
}

/// Set an INFO flag value on a record.
pub fn record_set_info_flag(
  record: &mut bcf::Record,
  header: &Header,
  tag: &str,
  is_set: bool,
) -> Result<(), rust_htslib::errors::Error> {
  let (tag_type, _) = header
    .info_type(tag.as_bytes())
    .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

  if tag_type != TagType::Flag {
    return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
  }

  if is_set {
    record.push_info_flag(tag.as_bytes())?;
  } else {
    record.clear_info_flag(tag.as_bytes())?;
  }

  record.unpack();
  Ok(())
}

/// Set an INFO integer value on a record.
pub fn record_set_info_integer(
  record: &mut bcf::Record,
  header: &Header,
  tag: &str,
  values: &[i32],
) -> Result<(), rust_htslib::errors::Error> {
  let (tag_type, _) = header
    .info_type(tag.as_bytes())
    .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

  if tag_type != TagType::Integer {
    return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
  }

  record.push_info_integer(tag.as_bytes(), values)?;
  record.unpack();
  Ok(())
}

/// Set an INFO float value on a record.
pub fn record_set_info_float(
  record: &mut bcf::Record,
  header: &Header,
  tag: &str,
  values: &[f32],
) -> Result<(), rust_htslib::errors::Error> {
  let (tag_type, _) = header
    .info_type(tag.as_bytes())
    .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

  if tag_type != TagType::Float {
    return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
  }

  record.push_info_float(tag.as_bytes(), values)?;
  record.unpack();
  Ok(())
}

/// Set an INFO string value on a record.
pub fn record_set_info_string(
  record: &mut bcf::Record,
  header: &Header,
  tag: &str,
  values: &[String],
) -> Result<(), rust_htslib::errors::Error> {
  let (tag_type, _) = header
    .info_type(tag.as_bytes())
    .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

  if tag_type != TagType::String {
    return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
  }

  let refs: Vec<&[u8]> = values.iter().map(|s| s.as_bytes()).collect();
  record.push_info_string(tag.as_bytes(), &refs)?;
  record.unpack();
  Ok(())
}

/// Clear an INFO field from a record.
pub fn record_clear_info(
  record: &mut bcf::Record,
  header: &Header,
  tag: &str,
) -> Result<(), rust_htslib::errors::Error> {
  let (tag_type, _) = header
    .info_type(tag.as_bytes())
    .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

  match tag_type {
    TagType::Flag => record.clear_info_flag(tag.as_bytes())?,
    TagType::Integer => record.clear_info_integer(tag.as_bytes())?,
    TagType::Float => record.clear_info_float(tag.as_bytes())?,
    TagType::String => record.clear_info_string(tag.as_bytes())?,
  }

  record.unpack();
  Ok(())
}

#[derive(Debug)]
pub struct Variant {
  record: bcf::Record,
  chrom: String,
}

impl Variant {
  pub fn from_record(mut record: bcf::Record) -> Self {
    record.unpack();
    let chrom = match record.rid() {
      Some(rid) => record
        .header()
        .rid2name(rid)
        .ok()
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .unwrap_or_else(|| ".".to_string()),
      None => ".".to_string(),
    };
    Self { record, chrom }
  }

  pub fn chrom(&self) -> &str {
    &self.chrom
  }

  pub fn rid(&self) -> Option<u32> {
    self.record.rid()
  }

  pub fn start(&self) -> i64 {
    self.record.pos()
  }

  pub fn pos(&self) -> i64 {
    self.record.pos() + 1
  }

  pub fn end(&self) -> i64 {
    self.record.end()
  }

  pub fn id(&self) -> String {
    String::from_utf8_lossy(&self.record.id()).into_owned()
  }

  pub fn set_id(&mut self, id: &str) -> Result<(), rust_htslib::errors::Error> {
    let id = if id.is_empty() { "." } else { id };
    self.record.set_id(id.as_bytes())?;
    Ok(())
  }

  pub fn reference(&self) -> String {
    self.record
      .alleles()
      .first()
      .map(|a| String::from_utf8_lossy(a).into_owned())
      .unwrap_or_else(|| ".".to_string())
  }

  pub fn alts(&self) -> Vec<String> {
    self.record
      .alleles()
      .into_iter()
      .skip(1)
      .map(|a| String::from_utf8_lossy(a).into_owned())
      .collect()
  }

  pub fn qual(&self) -> Option<f32> {
    let qual = self.record.qual();
    if qual.is_missing() {
      None
    } else {
      Some(qual)
    }
  }

  pub fn set_qual(&mut self, qual: Option<f32>) {
    match qual {
      Some(v) => self.record.set_qual(v),
      None => self.record.set_qual(<f32 as Numeric>::missing()),
    }
  }

  /// Return the FILTER column as a list of filter IDs.
  ///
  /// Records that are '.' return an empty list.
  pub fn filters(&self) -> Vec<String> {
    let header = self.record.header();
    let mut out = Vec::new();
    for id in self.record.filters() {
      let name = String::from_utf8_lossy(&header.id_to_name(id)).into_owned();
      out.push(name);
    }
    out
  }

  pub fn set_filters(&mut self, filters: &[String]) -> Result<(), rust_htslib::errors::Error> {
    let want_clear = filters.is_empty() || (filters.len() == 1 && (filters[0] == "" || filters[0] == ".")) ;

    if want_clear {
      let refs: Vec<&[u8]> = Vec::new();
      self.record.set_filters(&refs)?;
      return Ok(());
    }

    let refs: Vec<&[u8]> = filters.iter().map(|s| s.as_bytes()).collect();
    self.record.set_filters(&refs)?;
    Ok(())
  }

  pub fn set_info_flag(&mut self, header: &Header, tag: &str, is_set: bool) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header
      .info_type(tag.as_bytes())
      .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

    if tag_type != TagType::Flag {
      return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
    }

    if is_set {
      self.record.push_info_flag(tag.as_bytes())?;
    } else {
      self.record.clear_info_flag(tag.as_bytes())?;
    }

    self.record.unpack();
    Ok(())
  }

  pub fn set_info_integer(&mut self, header: &Header, tag: &str, values: &[i32]) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header
      .info_type(tag.as_bytes())
      .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

    if tag_type != TagType::Integer {
      return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
    }

    self.record.push_info_integer(tag.as_bytes(), values)?;
    self.record.unpack();
    Ok(())
  }

  pub fn set_info_float(&mut self, header: &Header, tag: &str, values: &[f32]) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header
      .info_type(tag.as_bytes())
      .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

    if tag_type != TagType::Float {
      return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
    }

    self.record.push_info_float(tag.as_bytes(), values)?;
    self.record.unpack();
    Ok(())
  }

  pub fn set_info_string(&mut self, header: &Header, tag: &str, values: &[String]) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header
      .info_type(tag.as_bytes())
      .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

    if tag_type != TagType::String {
      return Err(rust_htslib::errors::Error::BcfSetTag { tag: tag.to_string() });
    }

    let refs: Vec<&[u8]> = values.iter().map(|s| s.as_bytes()).collect();
    self.record.push_info_string(tag.as_bytes(), &refs)?;
    self.record.unpack();
    Ok(())
  }

  pub fn clear_info(&mut self, header: &Header, tag: &str) -> Result<(), rust_htslib::errors::Error> {
    let (tag_type, _) = header
      .info_type(tag.as_bytes())
      .ok_or_else(|| rust_htslib::errors::Error::BcfUndefinedTag { tag: tag.to_string() })?;

    match tag_type {
      TagType::Flag => self.record.clear_info_flag(tag.as_bytes())?,
      TagType::Integer => self.record.clear_info_integer(tag.as_bytes())?,
      TagType::Float => self.record.clear_info_float(tag.as_bytes())?,
      TagType::String => self.record.clear_info_string(tag.as_bytes())?,
    }

    self.record.unpack();
    Ok(())
  }

  pub fn info(&self, header: &Header, tag: &str) -> InfoValue {

    let (tag_type, tag_length) = match header.info_type(tag.as_bytes()) {
      Some(v) => v,
      None => return InfoValue::Absent,
    };

    match tag_type {
      TagType::Flag => match header_info_flag(header, &self.record, tag.as_bytes()) {
        Ok(v) => InfoValue::Bool(v),
        Err(InfoError::Absent) => InfoValue::Absent,
        Err(InfoError::Other) => InfoValue::Absent,
      },
      TagType::Integer => match header_info_values_i32(header, &self.record, tag.as_bytes()) {
        Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Int),
        Err(InfoError::Absent) => InfoValue::Absent,
        Err(InfoError::Other) => InfoValue::Absent,
      },
      TagType::Float => match header_info_values_f32(header, &self.record, tag.as_bytes()) {
        Ok(v) => numeric_to_infovalue(v, tag_length, InfoValue::Float),
        Err(InfoError::Absent) => InfoValue::Absent,
        Err(InfoError::Other) => InfoValue::Absent,
      },
      TagType::String => match header_info_values_string(header, &self.record, tag.as_bytes()) {
        Ok(v) => string_to_infovalue(v, tag_length),
        Err(InfoError::Absent) => InfoValue::Absent,
        Err(InfoError::Other) => InfoValue::Absent,
      },
    }
  }

  pub fn format(&self, header: &Header, tag: &str) -> FormatValue {
    let (tag_type, tag_length) = match header.format_type(tag.as_bytes()) {
      Some(v) => v,
      None => return FormatValue::Absent,
    };

    let sample_count = self.record.sample_count() as usize;

    match tag_type {
      TagType::Integer => match self.record.format(tag.as_bytes()).integer() {
        Ok(values) => FormatValue::PerSample(
          values
            .iter()
            .take(sample_count)
            .map(|per_sample| format_numeric_to_value(per_sample, tag_length, FormatValue::Int))
            .collect(),
        ),
        Err(_) => FormatValue::Absent,
      },
      TagType::Float => match self.record.format(tag.as_bytes()).float() {
        Ok(values) => FormatValue::PerSample(
          values
            .iter()
            .take(sample_count)
            .map(|per_sample| format_numeric_to_value(per_sample, tag_length, FormatValue::Float))
            .collect(),
        ),
        Err(_) => FormatValue::Absent,
      },
      TagType::String => match self.record.format(tag.as_bytes()).string() {
        Ok(values) => FormatValue::PerSample(
          values
            .iter()
            .take(sample_count)
            .map(|per_sample| format_string_to_value(*per_sample, tag_length))
            .collect(),
        ),
        Err(_) => FormatValue::Absent,
      },
      TagType::Flag => FormatValue::Absent,
    }
  }

  pub fn sample(&self, header: &Header, sample: &str) -> Option<Vec<(String, FormatValue)>> {
    let sample_id = header.sample_id(sample.as_bytes())?;
    let sample_count = self.record.sample_count() as usize;
    if sample_id >= sample_count {
      return None;
    }

    let format_tags = self.get_format_tag_names(header);
    let mut out: Vec<(String, FormatValue)> = Vec::with_capacity(format_tags.len() + 1);

    for (tag_name, tag_bytes) in format_tags {
      let Some(value) = format_value_for_sample(header, &self.record, &tag_bytes, sample_id) else {
        continue;
      };
      out.push((tag_name, value));
    }

    // Include the sample name so JS bindings can expose it.
    // Set it last so it can't be overwritten by a FORMAT tag named "sample_name".
    out.push((
      "sample_name".to_string(),
      FormatValue::String(sample.to_string()),
    ));

    Some(out)
  }

  /// Returns samples' FORMAT data as an array of objects.
  ///
  /// If `subset` is `None`, returns all samples in header order.
  /// If `subset` is `Some(names)`, returns only the specified samples in the
  /// order given. Unknown sample names are silently skipped.
  ///
  /// Each element contains all FORMAT fields plus a `sample_name` key.
  /// Returns an empty Vec if the VCF has no samples or no requested samples exist.
  pub fn samples(
    &self,
    header: &Header,
    subset: Option<&[&str]>,
  ) -> Vec<Vec<(String, FormatValue)>> {
    let sample_count = self.record.sample_count() as usize;
    if sample_count == 0 {
      return Vec::new();
    }

    let sample_names = header.sample_names();
    let format_tags = self.get_format_tag_names(header);

    // Determine which sample indices to include and in what order
    let sample_indices: Vec<usize> = match subset {
      None => (0..sample_count).collect(),
      Some(names) => {
        let name_to_idx = header.sample_name_to_idx();
        names
          .iter()
          .filter_map(|name| name_to_idx.get(*name).copied())
          .collect()
      }
    };

    if sample_indices.is_empty() {
      return Vec::new();
    }

    // Pre-allocate result vectors for each requested sample
    let mut results: Vec<Vec<(String, FormatValue)>> = sample_indices
      .iter()
      .map(|_| Vec::with_capacity(format_tags.len() + 1))
      .collect();

    // For each FORMAT tag, fetch values for ALL samples at once and distribute to requested ones
    for (tag_name, tag_bytes) in &format_tags {
      let Some((tag_type, tag_length)) = header.format_type(tag_bytes) else {
        continue;
      };

      match tag_type {
        bcf::header::TagType::Integer => {
          let Ok(all_values) = self.record.format(tag_bytes).integer() else {
            continue;
          };
          for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
            if let Some(per_sample) = all_values.get(sample_idx) {
              let value = format_numeric_to_value(per_sample, tag_length, FormatValue::Int);
              results[result_idx].push((tag_name.clone(), value));
            }
          }
        }
        bcf::header::TagType::Float => {
          let Ok(all_values) = self.record.format(tag_bytes).float() else {
            continue;
          };
          for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
            if let Some(per_sample) = all_values.get(sample_idx) {
              let value = format_numeric_to_value(per_sample, tag_length, FormatValue::Float);
              results[result_idx].push((tag_name.clone(), value));
            }
          }
        }
        bcf::header::TagType::String => {
          let Ok(all_values) = self.record.format(tag_bytes).string() else {
            continue;
          };
          for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
            if let Some(per_sample) = all_values.get(sample_idx) {
              let value = format_string_to_value(*per_sample, tag_length);
              results[result_idx].push((tag_name.clone(), value));
            }
          }
        }
        bcf::header::TagType::Flag => {
          // Flags are not valid for FORMAT
        }
      }
    }

    // Add sample_name to each result (last, so it can't be overwritten by a FORMAT tag)
    for (result_idx, &sample_idx) in sample_indices.iter().enumerate() {
      let name = sample_names
        .get(sample_idx)
        .map(|s| s.clone())
        .unwrap_or_else(|| format!("sample_{sample_idx}"));
      results[result_idx].push(("sample_name".to_string(), FormatValue::String(name)));
    }

    results
  }

  /// Get the list of FORMAT tag names present in this record.
  fn get_format_tag_names(&self, header: &Header) -> Vec<(String, Vec<u8>)> {
    let record_ptr = self.record.inner() as *const rust_htslib::htslib::bcf1_t
      as *mut rust_htslib::htslib::bcf1_t;

    let n_fmt = unsafe { (*record_ptr).n_fmt() as usize };
    let fmt_ptr = unsafe { (*record_ptr).d.fmt };

    if fmt_ptr.is_null() || n_fmt == 0 {
      return Vec::new();
    }

    let mut tags = Vec::with_capacity(n_fmt);
    for i in 0..n_fmt {
      let fmt = unsafe { *fmt_ptr.add(i) };
      let (tag_name, tag_bytes) = header.id_to_name_cached(fmt.id as u32);
      tags.push((tag_name, tag_bytes));
    }
    tags
  }


  pub fn to_string(&self, header: &Header) -> Option<String> {
    let mut s = rust_htslib::htslib::kstring_t {
      l: 0,
      m: 0,
      s: std::ptr::null_mut(),
    };

    let record_ptr = self.record.inner() as *const rust_htslib::htslib::bcf1_t
      as *mut rust_htslib::htslib::bcf1_t;

    let _ = unsafe {
      rust_htslib::htslib::bcf_unpack(record_ptr, rust_htslib::htslib::BCF_UN_ALL as i32)
    };

    let ret = unsafe {
      rust_htslib::htslib::vcf_format(
        header.inner_ptr() as *const rust_htslib::htslib::bcf_hdr_t,
        record_ptr as *const rust_htslib::htslib::bcf1_t,
        &mut s,
      )
    };
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

    Some(text.trim_end_matches('\n').to_string())
  }
}

#[derive(Debug)]
enum InfoError {
  Absent,
  Other,
}

fn header_info_flag(header: &Header, record: &bcf::Record, tag: &[u8]) -> Result<bool, InfoError> {
  let Ok(c_str) = CString::new(tag) else {
    return Err(InfoError::Other);
  };

  let record_ptr = record.inner() as *const rust_htslib::htslib::bcf1_t
    as *mut rust_htslib::htslib::bcf1_t;

  let mut dst: *mut std::os::raw::c_void = std::ptr::null_mut();
  let mut ndst: i32 = 0;

  let ret = unsafe {
    rust_htslib::htslib::bcf_get_info_values(
      header.inner_ptr(),
      record_ptr,
      c_str.as_ptr() as *mut std::os::raw::c_char,
      &mut dst,
      &mut ndst,
      rust_htslib::htslib::BCF_HT_FLAG as i32,
    )
  };

  if !dst.is_null() {
    unsafe { rust_htslib::htslib::free(dst) };
  }

  match ret {
    -3 => Err(InfoError::Absent),
    1 => Ok(true),
    0 => Ok(false),
    _ => Err(InfoError::Other),
  }
}

fn header_info_values_i32(
  header: &Header,
  record: &bcf::Record,
  tag: &[u8],
) -> Result<Option<Vec<i32>>, InfoError> {
  header_info_values_numeric::<i32>(header, record, tag, rust_htslib::htslib::BCF_HT_INT as i32)
}

fn header_info_values_f32(
  header: &Header,
  record: &bcf::Record,
  tag: &[u8],
) -> Result<Option<Vec<f32>>, InfoError> {
  header_info_values_numeric::<f32>(header, record, tag, rust_htslib::htslib::BCF_HT_REAL as i32)
}

fn header_info_values_numeric<T: Copy + Numeric>(
  header: &Header,
  record: &bcf::Record,
  tag: &[u8],
  data_type: i32,
) -> Result<Option<Vec<T>>, InfoError> {
  let Ok(c_str) = CString::new(tag) else {
    return Err(InfoError::Other);
  };

  let record_ptr = record.inner() as *const rust_htslib::htslib::bcf1_t
    as *mut rust_htslib::htslib::bcf1_t;

  let mut dst: *mut std::os::raw::c_void = std::ptr::null_mut();
  let mut ndst: i32 = 0;

  let ret = unsafe {
    rust_htslib::htslib::bcf_get_info_values(
      header.inner_ptr(),
      record_ptr,
      c_str.as_ptr() as *mut std::os::raw::c_char,
      &mut dst,
      &mut ndst,
      data_type,
    )
  };

  match ret {
    -3 => Ok(None),
    0 => {
      if !dst.is_null() {
        unsafe { rust_htslib::htslib::free(dst) };
      }
      Ok(Some(Vec::new()))
    }
    ret if ret > 0 => {
      let slice = unsafe { std::slice::from_raw_parts(dst as *const T, ret as usize) };
      let vec = slice.to_vec();
      if !dst.is_null() {
        unsafe { rust_htslib::htslib::free(dst) };
      }
      Ok(Some(vec))
    }
    _ => {
      if !dst.is_null() {
        unsafe { rust_htslib::htslib::free(dst) };
      }
      Err(InfoError::Other)
    }
  }
}

fn header_info_values_string(
  header: &Header,
  record: &bcf::Record,
  tag: &[u8],
) -> Result<Option<Vec<Vec<u8>>>, InfoError> {
  let Ok(c_str) = CString::new(tag) else {
    return Err(InfoError::Other);
  };

  let record_ptr = record.inner() as *const rust_htslib::htslib::bcf1_t
    as *mut rust_htslib::htslib::bcf1_t;

  let mut dst: *mut std::os::raw::c_void = std::ptr::null_mut();
  let mut ndst: i32 = 0;

  let ret = unsafe {
    rust_htslib::htslib::bcf_get_info_values(
      header.inner_ptr(),
      record_ptr,
      c_str.as_ptr() as *mut std::os::raw::c_char,
      &mut dst,
      &mut ndst,
      rust_htslib::htslib::BCF_HT_STR as i32,
    )
  };

  match ret {
    -3 => Ok(None),
    0 => {
      if !dst.is_null() {
        unsafe { rust_htslib::htslib::free(dst) };
      }
      Ok(Some(Vec::new()))
    }
    ret if ret > 0 => {
      let bytes = unsafe { std::slice::from_raw_parts(dst as *const u8, ret as usize) };
      let mut out = Vec::new();
      for part in bytes.split(|c| *c == b',') {
        let part = part.split(|c| *c == 0u8).next().ok_or(InfoError::Other)?;
        out.push(part.to_vec());
      }
      if !dst.is_null() {
        unsafe { rust_htslib::htslib::free(dst) };
      }
      Ok(Some(out))
    }
    _ => {
      if !dst.is_null() {
        unsafe { rust_htslib::htslib::free(dst) };
      }
      Err(InfoError::Other)
    }
  }
}


fn numeric_to_infovalue<T: Numeric + Copy>(
  values: Option<Vec<T>>,
  tag_length: TagLength,
  scalar: impl FnOnce(T) -> InfoValue + Copy,
) -> InfoValue {
  let Some(values) = values else {
    return InfoValue::Absent;
  };

  match tag_length {
    TagLength::Fixed(1) => {
      let v = values.first().copied();
      match v {
        Some(v) if v.is_missing() => InfoValue::Missing,
        Some(v) => scalar(v),
        None => InfoValue::Missing,
      }
    }
    _ => InfoValue::Array(
      values
        .into_iter()
        .map(|v| if v.is_missing() { InfoValue::Missing } else { scalar(v) })
        .collect(),
    ),
  }
}

fn string_to_infovalue(values: Option<Vec<Vec<u8>>>, tag_length: TagLength) -> InfoValue {
  let Some(values) = values else {
    return InfoValue::Absent;
  };

  match tag_length {
    TagLength::Fixed(1) => {
      let v = values
        .first()
        .map(|s| String::from_utf8_lossy(s).into_owned());
      match v {
        Some(v) if v.is_empty() => InfoValue::Missing,
        Some(v) => InfoValue::String(v),
        None => InfoValue::Missing,
      }
    }
    _ => InfoValue::Array(
      values
        .into_iter()
        .map(|v| {
          let s = String::from_utf8_lossy(&v).into_owned();
          if s.is_empty() {
            InfoValue::Missing
          } else {
            InfoValue::String(s)
          }
        })
        .collect(),
    ),
  }
}

fn format_numeric_to_value<T: Numeric + Copy>(
  values: &[T],
  tag_length: TagLength,
  scalar: impl FnOnce(T) -> FormatValue + Copy,
) -> FormatValue {
  match tag_length {
    TagLength::Fixed(1) => {
      let v = values.first().copied();
      match v {
        Some(v) if v.is_missing() => FormatValue::Missing,
        Some(v) => scalar(v),
        None => FormatValue::Missing,
      }
    }
    _ => FormatValue::Array(
      values
        .iter()
        .copied()
        .map(|v| if v.is_missing() { FormatValue::Missing } else { scalar(v) })
        .collect(),
    ),
  }
}

fn format_string_to_value(value: &[u8], tag_length: TagLength) -> FormatValue {
  match tag_length {
    TagLength::Fixed(1) => {
      let out = String::from_utf8_lossy(value).into_owned();
      if out.is_empty() || out == "." {
        FormatValue::Missing
      } else {
        FormatValue::String(out)
      }
    }
    _ => {
      let mut parts = Vec::new();
      for part in value.split(|c| *c == b',') {
        let out = String::from_utf8_lossy(part).into_owned();
        if out.is_empty() || out == "." {
          parts.push(FormatValue::Missing);
        } else {
          parts.push(FormatValue::String(out));
        }
      }
      FormatValue::Array(parts)
    }
  }
}

fn format_value_for_sample(
  header: &Header,
  record: &bcf::Record,
  tag: &[u8],
  sample_id: usize,
) -> Option<FormatValue> {
  let (tag_type, tag_length) = header.format_type(tag)?;

  match tag_type {
    TagType::Integer => {
      let values = record.format(tag).integer().ok()?;
      let per_sample = values.get(sample_id)?;
      Some(format_numeric_to_value(per_sample, tag_length, FormatValue::Int))
    }
    TagType::Float => {
      let values = record.format(tag).float().ok()?;
      let per_sample = values.get(sample_id)?;
      Some(format_numeric_to_value(per_sample, tag_length, FormatValue::Float))
    }
    TagType::String => {
      let values = record.format(tag).string().ok()?;
      let per_sample = values.get(sample_id)?;
      Some(format_string_to_value(*per_sample, tag_length))
    }
    TagType::Flag => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use rust_htslib::bcf::Read;

  #[test]
  fn sample_includes_sample_name_and_overrides_format_tag() {
    let vcf = "##fileformat=VCFv4.2\n\
##FORMAT=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##FORMAT=<ID=sample_name,Number=1,Type=String,Description=\"Should not override\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\n\
chr1\t1\t.\tA\tC\t.\t.\t.\tDP:sample_name\t7:EVIL\n";

    let tmp_dir = std::env::temp_dir().join("htsvcf-core-test");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let vcf_path = tmp_dir.join("sample-name.vcf");
    std::fs::write(&vcf_path, vcf).unwrap();

    let mut reader = bcf::Reader::from_path(&vcf_path).unwrap();
    let header = unsafe { Header::new(reader.header().inner) };

    let mut rec = reader.empty_record();
    let _ = reader.read(&mut rec).unwrap();
    let variant = Variant::from_record(rec);

    let fields = variant.sample(&header, "S1").expect("sample exists");
    let mut map = std::collections::HashMap::new();
    for (k, v) in fields {
      map.insert(k, v);
    }

    assert_eq!(map.get("DP"), Some(&FormatValue::Int(7)));
    assert_eq!(
      map.get("sample_name"),
      Some(&FormatValue::String("S1".to_string()))
    );

    let _ = std::fs::remove_file(&vcf_path);
  }
}
