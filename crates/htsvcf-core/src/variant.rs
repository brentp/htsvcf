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
