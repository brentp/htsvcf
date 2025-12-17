use rust_htslib::bcf;
use rust_htslib::bcf::header::{HeaderRecord, TagLength, TagType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::ffi::CString;
use std::mem::ManuallyDrop;

impl Drop for Header {
  fn drop(&mut self) {
    unsafe {
      rust_htslib::htslib::bcf_hdr_destroy(self.inner);
    }
  }
}

#[derive(Debug)]
pub struct Header {
  inner: *mut rust_htslib::htslib::bcf_hdr_t,
  dirty: AtomicBool,
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
    Self {
      inner,
      dirty: AtomicBool::new(false),
    }
  }

  pub fn empty() -> Self {
    let c_str = CString::new(&b"w"[..]).unwrap();
    let inner = unsafe { rust_htslib::htslib::bcf_hdr_init(c_str.as_ptr()) };
    Self {
      inner,
      dirty: AtomicBool::new(false),
    }
  }

  pub fn inner_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
    self.inner
  }

  fn view(&self) -> ManuallyDrop<bcf::header::HeaderView> {
    ManuallyDrop::new(bcf::header::HeaderView::new(self.inner))
  }

  pub fn header_records(&self) -> Vec<HeaderRecord> {
    self.view().header_records()
  }

  pub fn info_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
    self.view().info_type(tag).ok()
  }

  pub fn format_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
    self.view().format_type(tag).ok()
  }

  pub fn sync(&self) {
    if !self.dirty.swap(false, Ordering::AcqRel) {
      return;
    }
    unsafe {
      rust_htslib::htslib::bcf_hdr_sync(self.inner);
    }
  }

  pub fn push_record(&self, record: &[u8]) -> bool {
    let Ok(c_str) = CString::new(record) else {
      return false;
    };
    let r = unsafe { rust_htslib::htslib::bcf_hdr_append(self.inner, c_str.as_ptr()) };
    self.dirty.store(true, Ordering::Release);
    self.sync();
    r == 0
  }

  pub fn add_info(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
    let record =
      format!("##INFO=<ID={id},Number={number},Type={ty},Description=\"{description}\">",
    );
    self.push_record(record.as_bytes())
  }

  pub fn add_format(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
    let record =
      format!("##FORMAT=<ID={id},Number={number},Type={ty},Description=\"{description}\">",
    );
    self.push_record(record.as_bytes())
  }

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
}
