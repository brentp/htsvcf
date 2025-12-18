use std::sync::{Arc, Mutex};

use htsvcf_core as core;
use htsvcf_core::variant::FormatValue;
use napi::bindgen_prelude::*;
use napi::{sys, Env};
use napi_derive::napi;

#[napi(object)]
pub struct ReaderOptions {}

#[napi]
pub struct Reader {
  inner: Arc<Mutex<Option<core::Reader>>>,
  header: Arc<core::Header>,
  header_ref: Reference<Header>,
}

#[napi]
impl Reader {
  #[napi(constructor)]
  pub fn new(env: Env, path: String, _opts: Option<ReaderOptions>) -> napi::Result<Self> {
    let reader = core::open_reader(&path)
      .map_err(|e| Error::new(Status::GenericFailure, format!("failed to open {path}: {e}")))?;

    let header = Arc::new(unsafe { core::Header::new(reader.header_ptr()) });
    let header_ref = Header::into_reference(Header { inner: header.clone() }, env)?;

    Ok(Self {
      inner: Arc::new(Mutex::new(Some(reader))),
      header,
      header_ref,
    })
  }

  #[napi(getter)]
  pub fn header(&self, env: Env) -> napi::Result<Reference<Header>> {
    self.header_ref.clone(env)
  }

  #[napi(js_name = "hasIndex")]
  pub fn has_index(&self) -> bool {
    self
      .inner
      .lock()
      .ok()
      .and_then(|g| g.as_ref().map(|r| r.has_index()))
      .unwrap_or(false)
  }

  #[napi]
  pub fn query(
    &self,
    region_or_chrom: String,
    start0: Option<u32>,
    end0: Option<u32>,
  ) -> AsyncTask<QueryTask> {
    AsyncTask::new(QueryTask {
      inner: self.inner.clone(),
      region_or_chrom,
      start0,
      end0,
    })
  }

  #[napi]
  pub fn next(&self) -> AsyncTask<NextTask> {
    AsyncTask::new(NextTask {
      inner: self.inner.clone(),
      header: self.header.clone(),
    })
  }

  #[napi(js_name = "nextSync")]
  pub fn next_sync(&self, env: Env) -> napi::Result<Object<'static>> {
    let mut guard = self
      .inner
      .lock()
      .map_err(|_| Error::new(Status::GenericFailure, "reader lock poisoned"))?;
    let reader = guard
      .as_mut()
      .ok_or_else(|| Error::new(Status::GenericFailure, "reader is closed"))?;

    let rec = reader
      .next_record()
      .map_err(|e| Error::new(Status::GenericFailure, format!("read failed: {e}")))?;

    let mut out: Object<'static> = Object::new(&env)?;

    match rec.map(core::Variant::from_record) {
      None => {
        out.set_named_property("done", true)?;
        out.set_named_property("value", ())?;
      }
      Some(variant) => {
        out.set_named_property("done", false)?;
        out.set_named_property(
          "value",
          Variant {
            inner: variant,
            header: self.header.clone(),
          },
        )?;
      }
    }

    Ok(out)
  }

  #[napi]
  pub fn close(&self) {
    if let Ok(mut guard) = self.inner.lock() {
      let _ = guard.take();
    }
  }
}

#[napi]
pub fn open_reader(path: String, opts: Option<ReaderOptions>) -> AsyncTask<OpenReaderTask> {
  AsyncTask::new(OpenReaderTask { path, opts })
}

pub struct OpenReaderTask {
  path: String,
  #[allow(dead_code)]
  opts: Option<ReaderOptions>,
}

impl Task for OpenReaderTask {
  type Output = core::Reader;
  type JsValue = Reader;

  fn compute(&mut self) -> napi::Result<Self::Output> {
    core::open_reader(&self.path)
      .map_err(|e| Error::new(Status::GenericFailure, format!("failed to open {}: {e}", self.path)))
  }

  fn resolve(&mut self, env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
    let header = Arc::new(unsafe { core::Header::new(output.header_ptr()) });
    let header_ref = Header::into_reference(Header { inner: header.clone() }, env)?;

    Ok(Reader {
      inner: Arc::new(Mutex::new(Some(output))),
      header,
      header_ref,
    })
  }
}

pub struct QueryTask {
  inner: Arc<Mutex<Option<core::Reader>>>,
  region_or_chrom: String,
  start0: Option<u32>,
  end0: Option<u32>,
}

impl Task for QueryTask {
  type Output = ();
  type JsValue = ();

  fn compute(&mut self) -> napi::Result<Self::Output> {
    let mut guard = self
      .inner
      .lock()
      .map_err(|_| Error::new(Status::GenericFailure, "reader lock poisoned"))?;
    let reader = guard
      .as_mut()
      .ok_or_else(|| Error::new(Status::GenericFailure, "reader is closed"))?;

    if !reader.has_index() {
      return Err(Error::new(
        Status::GenericFailure,
        "query() requires an indexed file",
      ));
    }

    match self.start0 {
      None => reader
        .query_region_1based(&self.region_or_chrom)
        .map_err(|e| Error::new(Status::GenericFailure, format!("query failed: {e}")))?,
      Some(start0) => reader
        .query(
          &self.region_or_chrom,
          start0 as u64,
          self.end0.map(|v| v as u64),
        )
        .map_err(|e| Error::new(Status::GenericFailure, format!("query failed: {e}")))?,
    };

    Ok(())
  }

  fn resolve(&mut self, _env: Env, _output: Self::Output) -> napi::Result<Self::JsValue> {
    Ok(())
  }
}

pub struct NextTask {
  inner: Arc<Mutex<Option<core::Reader>>>,
  header: Arc<core::Header>,
}

impl Task for NextTask {
  type Output = Option<core::Variant>;
  type JsValue = Object<'static>;

  fn compute(&mut self) -> napi::Result<Self::Output> {
    let mut guard = self
      .inner
      .lock()
      .map_err(|_| Error::new(Status::GenericFailure, "reader lock poisoned"))?;
    let reader = guard
      .as_mut()
      .ok_or_else(|| Error::new(Status::GenericFailure, "reader is closed"))?;

    let rec = reader
      .next_record()
      .map_err(|e| Error::new(Status::GenericFailure, format!("read failed: {e}")))?;

    Ok(rec.map(core::Variant::from_record))
  }

  fn resolve(&mut self, env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
    let mut out: Object<'static> = Object::new(&env)?;

    match output {
      None => {
        out.set_named_property("done", true)?;
        out.set_named_property("value", ())?;
      }
      Some(variant) => {
        out.set_named_property("done", false)?;
        out.set_named_property(
          "value",
          Variant {
            inner: variant,
            header: self.header.clone(),
          },
        )?;
      }
    }

    Ok(out)
  }
}

#[napi]
pub struct Variant {
  inner: core::Variant,
  header: Arc<core::Header>,
}

#[napi]
impl Variant {
  #[napi(getter)]
  pub fn chrom(&self) -> String {
    self.inner.chrom().to_string()
  }

  #[napi(getter)]
  pub fn rid(&self) -> Option<u32> {
    self.inner.rid()
  }

  #[napi(getter)]
  pub fn pos(&self) -> i64 {
    self.inner.pos()
  }

  #[napi(getter)]
  pub fn start(&self) -> i64 {
    self.inner.start()
  }

  #[napi(getter, js_name = "stop")]
  pub fn stop(&self) -> i64 {
    self.inner.end()
  }

  #[napi(getter)]
  pub fn id(&self) -> String {
    self.inner.id()
  }

  #[napi(setter)]
  pub fn set_id(&mut self, id: String) -> napi::Result<()> {
    self
      .inner
      .set_id(&id)
      .map_err(|e| Error::new(Status::GenericFailure, format!("failed to set id: {e}")))
  }

  #[napi(getter, js_name = "ref")]
  pub fn reference(&self) -> String {
    self.inner.reference()
  }

  #[napi(getter)]
  pub fn alt(&self) -> Vec<String> {
    self.inner.alts()
  }

  #[napi(getter)]
  pub fn qual(&self) -> Option<f64> {
    self.inner.qual().map(|v| v as f64)
  }

  #[napi(setter)]
  pub fn set_qual(&mut self, qual: Option<f64>) {
    self.inner.set_qual(qual.map(|v| v as f32))
  }

  #[napi(getter)]
  pub fn filter(&self) -> Vec<String> {
    self.inner.filters()
  }

  #[napi(setter)]
  pub fn set_filter(&mut self, filter: Vec<String>) -> napi::Result<()> {
    self
      .inner
      .set_filters(&filter)
      .map_err(|e| Error::new(Status::GenericFailure, format!("failed to set filter: {e}")))
  }

  #[napi(js_name = "toString")]
  pub fn to_string(&self) -> napi::Result<String> {
    self
      .inner
      .to_string(&self.header)
      .ok_or_else(|| Error::new(Status::GenericFailure, "failed to format record"))
  }

  #[napi]
  pub fn info(&self, env: Env, tag: String) -> napi::Result<sys::napi_value> {
    let v = self.inner.info(&self.header, &tag);
    infovalue_to_napi_value(&env, &v)
  }

  #[napi]
  pub fn format(&self, env: Env, tag: String) -> napi::Result<sys::napi_value> {
    let v = self.inner.format(&self.header, &tag);
    formatvalue_to_napi_value(&env, &v)
  }

  #[napi]
  pub fn sample(&self, env: Env, name: String) -> napi::Result<sys::napi_value> {
    let Some(fields) = self.inner.sample(&self.header, &name) else {
      return unsafe { ToNapiValue::to_napi_value(env.raw(), ()) };
    };

    let mut out: Object<'static> = Object::new(&env)?;
    for (tag, value) in fields {
      let js_value = formatvalue_to_napi_value(&env, &value)?;
      out.set_named_property(tag.as_str(), unsafe { Unknown::from_raw_unchecked(env.raw(), js_value) })?;
    }

    Ok(out.raw())
  }

  #[napi]
  pub fn samples(&self, env: Env, subset: Option<Vec<String>>) -> napi::Result<sys::napi_value> {
    let subset_refs: Option<Vec<&str>> = subset.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect());
    let all_samples = self.inner.samples(&self.header, subset_refs.as_deref());

    let mut arr_items: Vec<sys::napi_value> = Vec::with_capacity(all_samples.len());

    for fields in all_samples {
      let mut out: Object<'static> = Object::new(&env)?;
      for (tag, value) in fields {
        let js_value = formatvalue_to_napi_value(&env, &value)?;
        out.set_named_property(tag.as_str(), unsafe { Unknown::from_raw_unchecked(env.raw(), js_value) })?;
      }
      arr_items.push(out.raw());
    }

    let arr = Array::from_vec(&env, arr_items)?;
    Ok(arr.raw())
  }

  #[napi(js_name = "set_info")]
  pub fn set_info(&mut self, tag: String, value: Unknown) -> napi::Result<()> {
    use napi::ValueType;
    use rust_htslib::bcf::header::TagType;

    let Some((tag_type, _tag_length)) = self.header.info_type(tag.as_bytes()) else {
      return Err(Error::new(
        Status::InvalidArg,
        format!("undefined INFO tag: {tag}"),
      ));
    };

    match value.get_type()? {
      ValueType::Null | ValueType::Undefined => {
        self.inner
          .clear_info(&self.header, &tag)
          .map_err(|e| Error::new(Status::GenericFailure, format!("failed to clear info {tag}: {e}")))?;
        return Ok(());
      }
      _ => {}
    }

    match tag_type {
      TagType::Flag => {
        if value.is_array()? {
          return Err(Error::new(
            Status::InvalidArg,
            format!("INFO/{tag} is Flag; expected boolean"),
          ));
        }

        if value.get_type()? != ValueType::Boolean {
          return Err(Error::new(
            Status::InvalidArg,
            format!("INFO/{tag} is Flag; expected boolean"),
          ));
        }

        let is_set: bool = unsafe { value.cast()? };
        self.inner
          .set_info_flag(&self.header, &tag, is_set)
          .map_err(|e| Error::new(Status::GenericFailure, format!("failed to set info {tag}: {e}")))?;
      }
      TagType::Integer => {
        let values = unknown_to_numbers(&tag, value)?;
        let mut out: Vec<i32> = Vec::with_capacity(values.len());
        for n in values {
          if !n.is_finite() {
            return Err(Error::new(Status::InvalidArg, "number must be finite"));
          }
          if n.fract() != 0.0 {
            return Err(Error::new(
              Status::InvalidArg,
              format!("INFO/{tag} is Integer; got non-integer value"),
            ));
          }
          if n < (i32::MIN as f64) || n > (i32::MAX as f64) {
            return Err(Error::new(
              Status::InvalidArg,
              format!("INFO/{tag} integer out of range"),
            ));
          }
          out.push(n as i32);
        }

        self.inner
          .set_info_integer(&self.header, &tag, &out)
          .map_err(|e| Error::new(Status::GenericFailure, format!("failed to set info {tag}: {e}")))?;
      }
      TagType::Float => {
        let values = unknown_to_numbers(&tag, value)?;
        let mut out: Vec<f32> = Vec::with_capacity(values.len());
        for n in values {
          if !n.is_finite() {
            return Err(Error::new(Status::InvalidArg, "number must be finite"));
          }
          out.push(n as f32);
        }

        self.inner
          .set_info_float(&self.header, &tag, &out)
          .map_err(|e| Error::new(Status::GenericFailure, format!("failed to set info {tag}: {e}")))?;
      }
      TagType::String => {
        let values: Vec<String> = unknown_to_strings(&tag, value)?;
        self.inner
          .set_info_string(&self.header, &tag, &values)
          .map_err(|e| Error::new(Status::GenericFailure, format!("failed to set info {tag}: {e}")))?;
      }
    }

    Ok(())
  }
}

fn unknown_to_numbers(tag: &str, value: Unknown) -> napi::Result<Vec<f64>> {
  if value.is_array()? {
    let arr: Array = unsafe { value.cast()? };
    let len = arr.len();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
      let v: Unknown = arr.get_element(i)?;
      let n = unknown_to_number(tag, v)?;
      out.push(n);
    }
    return Ok(out);
  }

  Ok(vec![unknown_to_number(tag, value)?])
}

fn unknown_to_number(tag: &str, value: Unknown) -> napi::Result<f64> {
  use napi::ValueType;

  if value.get_type()? != ValueType::Number {
    return Err(Error::new(
      Status::InvalidArg,
      format!("INFO/{tag} expected number"),
    ));
  }

  let n: f64 = unsafe { value.cast()? };
  Ok(n)
}

fn unknown_to_strings(tag: &str, value: Unknown) -> napi::Result<Vec<String>> {
  if value.is_array()? {
    let arr: Array = unsafe { value.cast()? };
    let len = arr.len();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
      let v: Unknown = arr.get_element(i)?;
      let s = unknown_to_string(tag, v)?;
      out.push(s);
    }
    return Ok(out);
  }

  Ok(vec![unknown_to_string(tag, value)?])
}

fn unknown_to_string(tag: &str, value: Unknown) -> napi::Result<String> {
  use napi::ValueType;

  if value.get_type()? != ValueType::String {
    return Err(Error::new(
      Status::InvalidArg,
      format!("INFO/{tag} expected string"),
    ));
  }

  let s: String = unsafe { value.cast()? };
  Ok(s)
}

#[napi]
pub struct Header {
  inner: Arc<core::Header>,
}

#[napi]
impl Header {
  #[napi(js_name = "toString")]
  pub fn to_string(&self) -> napi::Result<String> {
    self
      .inner
      .to_string()
      .ok_or_else(|| Error::new(Status::GenericFailure, "failed to format header"))
  }

  #[napi(js_name = "addInfo")]
  pub fn add_info(&self, id: String, number: String, ty: String, description: String) {
    let _ = self.inner.add_info(&id, &number, &ty, &description);
  }

  #[napi(js_name = "addFormat")]
  pub fn add_format(&self, id: String, number: String, ty: String, description: String) {
    let _ = self.inner.add_format(&id, &number, &ty, &description);
  }

  #[napi]
  pub fn get(&self, env: Env, section: String, id: String) -> napi::Result<sys::napi_value> {
    let Some(field) = self.inner.get_field(&section, &id) else {
      return unsafe { ToNapiValue::to_napi_value(env.raw(), ()) };
    };

    let mut out = Object::new(&env)?;
    out.set_named_property("id", field.id)?;
    out.set_named_property("type", field.r#type)?;
    out.set_named_property("number", field.number)?;
    out.set_named_property("description", field.description)?;

    Ok(out.raw())
  }

  #[napi]
  pub fn records(&self, env: Env) -> napi::Result<Vec<Object<'static>>> {
    let mut out = Vec::new();
    for (section, field) in self.inner.all_fields() {
      let mut o: Object<'static> = Object::new(&env)?;
      o.set_named_property("type", section)?;
      o.set_named_property("id", field.id)?;
      o.set_named_property("number", field.number)?;
      o.set_named_property("type", field.r#type)?;
      o.set_named_property("description", field.description)?;
      out.push(o);
    }

    Ok(out)
  }

  #[napi]
  pub fn samples(&self) -> Vec<String> {
    self.inner.sample_names().to_vec()
  }
}

#[napi(object)]
pub struct HeaderGetResult {
  pub id: String,
  #[napi(js_name = "type")]
  pub r#type: String,
  pub number: String,
  pub description: String,
}

fn infovalue_to_napi_value(env: &Env, v: &core::InfoValue) -> napi::Result<sys::napi_value> {
  match v {
    core::InfoValue::Absent => unsafe { ToNapiValue::to_napi_value(env.raw(), ()) },
    core::InfoValue::Missing => unsafe { ToNapiValue::to_napi_value(env.raw(), Null) },
    core::InfoValue::Bool(b) => unsafe { ToNapiValue::to_napi_value(env.raw(), *b) },
    core::InfoValue::Int(i) => unsafe { ToNapiValue::to_napi_value(env.raw(), env.create_int32(*i)?) },
    core::InfoValue::Float(f) => unsafe { ToNapiValue::to_napi_value(env.raw(), env.create_double(*f as f64)?) },
    core::InfoValue::String(s) => unsafe { ToNapiValue::to_napi_value(env.raw(), env.create_string(s)?) },
    core::InfoValue::Array(values) => {
      let inner_values = values
        .iter()
        .map(|item| infovalue_to_napi_value(env, item))
        .collect::<napi::Result<Vec<sys::napi_value>>>()?;
      let arr = Array::from_vec(env, inner_values)?;
      Ok(arr.raw())
    }
  }
}

fn formatvalue_to_napi_value(env: &Env, v: &FormatValue) -> napi::Result<sys::napi_value> {
  match v {
    FormatValue::Absent => unsafe { ToNapiValue::to_napi_value(env.raw(), ()) },
    FormatValue::Missing => unsafe { ToNapiValue::to_napi_value(env.raw(), Null) },
    FormatValue::Int(i) => unsafe { ToNapiValue::to_napi_value(env.raw(), env.create_int32(*i)?) },
    FormatValue::Float(f) => unsafe { ToNapiValue::to_napi_value(env.raw(), env.create_double(*f as f64)?) },
    FormatValue::String(s) => unsafe { ToNapiValue::to_napi_value(env.raw(), env.create_string(s)?) },
    FormatValue::Array(values) => {
      let inner_values = values
        .iter()
        .map(|item| formatvalue_to_napi_value(env, item))
        .collect::<napi::Result<Vec<sys::napi_value>>>()?;
      let arr = Array::from_vec(env, inner_values)?;
      Ok(arr.raw())
    }
    FormatValue::PerSample(values) => {
      let inner_values = values
        .iter()
        .map(|item| formatvalue_to_napi_value(env, item))
        .collect::<napi::Result<Vec<sys::napi_value>>>()?;
      let arr = Array::from_vec(env, inner_values)?;
      Ok(arr.raw())
    }
  }
}
