use std::sync::{Arc, Mutex};

use htsvcf_core as core;
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

  #[napi(getter)]
  pub fn filter(&self) -> Vec<String> {
    self.inner.filters()
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
    use rust_htslib::bcf::header::{HeaderRecord, TagLength, TagType};

    let tag_info = match section.as_str() {
      "INFO" => self.inner.info_type(id.as_bytes()),
      "FORMAT" => self.inner.format_type(id.as_bytes()),
      _ => None,
    };

    let Some((tag_type, tag_length)) = tag_info else {
      return unsafe { ToNapiValue::to_napi_value(env.raw(), ()) };
    };

    let mut description: Option<String> = None;
    let want_section = section.as_str();

    for record in self.inner.header_records() {
      match record {
        HeaderRecord::Info { values, .. } if want_section == "INFO" => {
          let mut record_id: Option<String> = None;
          let mut record_description: Option<String> = None;

          for (k, v) in values.into_iter() {
            if k == "ID" {
              record_id = Some(v);
            } else if k == "Description" {
              record_description = Some(unquote_string(v));
            }
          }

          if record_id.as_deref() == Some(id.as_str()) {
            description = record_description;
            break;
          }
        }
        HeaderRecord::Format { values, .. } if want_section == "FORMAT" => {
          let mut record_id: Option<String> = None;
          let mut record_description: Option<String> = None;

          for (k, v) in values.into_iter() {
            if k == "ID" {
              record_id = Some(v);
            } else if k == "Description" {
              record_description = Some(unquote_string(v));
            }
          }

          if record_id.as_deref() == Some(id.as_str()) {
            description = record_description;
            break;
          }
        }
        _ => {}
      }
    }

    let type_str = match tag_type {
      TagType::Flag => "Flag",
      TagType::Integer => "Integer",
      TagType::Float => "Float",
      TagType::String => "String",
    }
    .to_string();

    let number = match tag_length {
      TagLength::Fixed(n) => n.to_string(),
      TagLength::AltAlleles => "A".to_string(),
      TagLength::Alleles => "R".to_string(),
      TagLength::Genotypes => "G".to_string(),
      TagLength::Variable => ".".to_string(),
    };

    let mut out = Object::new(&env)?;
    out.set_named_property("id", id)?;
    out.set_named_property("type", type_str)?;
    out.set_named_property("number", number)?;
    out.set_named_property("description", description.unwrap_or_default())?;

    Ok(out.raw())
  }

  #[napi]
  pub fn records(&self, env: Env) -> napi::Result<Vec<Object<'static>>> {
    use rust_htslib::bcf::header::HeaderRecord;

    let mut out = Vec::new();
    for record in self.inner.header_records() {
      match record {
        HeaderRecord::Info { key, values } => {
          out.push(record_kv(env, "INFO", key, values)?);
        }
        HeaderRecord::Format { key, values } => {
          out.push(record_kv(env, "FORMAT", key, values)?);
        }
        HeaderRecord::Filter { key, values } => {
          out.push(record_kv(env, "FILTER", key, values)?);
        }
        HeaderRecord::Contig { key, values } => {
          out.push(record_kv(env, "contig", key, values)?);
        }
        HeaderRecord::Structured { key, values } => {
          out.push(record_kv(env, "structured", key, values)?);
        }
         HeaderRecord::Generic { key, value } => {
          let mut o: Object<'static> = Object::new(&env)?;
          o.set_named_property("type", "generic")?;
          o.set_named_property("key", key)?;
          o.set_named_property("value", value)?;
          out.push(o);
        }
      }
    }

    Ok(out)
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

fn unquote_string(s: String) -> String {
  let bytes = s.as_bytes();
  if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
    s[1..bytes.len() - 1].to_string()
  } else {
    s
  }
}

fn record_kv(
  env: Env,
  record_type: &str,
  key: String,
  values: impl IntoIterator<Item = (String, String)>,
) -> napi::Result<Object<'static>> {
  let mut o: Object<'static> = Object::new(&env)?;
  o.set_named_property("type", record_type)?;
  o.set_named_property("key", key)?;
  for (k, v) in values.into_iter() {
    o.set_named_property(k.as_str(), v)?;
  }
  Ok(o)
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
