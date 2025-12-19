//! Core VCF/BCF parsing library built on HTSlib.
//!
//! This crate provides the shared implementation for reading VCF/BCF files
//! and accessing variant data. It is used by both the V8 binding (`htsvcf`)
//! and the Node-API binding (`htsvcf-napi`). It has some nice additions to rust-htslib, but
//! it's not likely you'd need to use it directly.
//!
//! # Overview
//!
//! The main types are:
//! - [`Reader`] - Opens and iterates VCF/BCF files (with optional index-based queries)
//! - [`Header`] - Access VCF header metadata (INFO/FORMAT definitions, samples)
//! - [`Variant`] - A single VCF record with typed accessors for all fields
//!
//! # Example: Reading a VCF file
//!
//! ```no_run
//! use htsvcf_core::{open_reader, Header, Variant};
//!
//! let mut reader = open_reader("input.vcf.gz").expect("failed to open");
//! let header = unsafe { Header::new(reader.header_ptr()) };
//!
//! while let Ok(Some(record)) = reader.next_record() {
//!     let variant = Variant::from_record(record);
//!
//!     // Basic fields
//!     println!("{}:{} {} -> {:?}",
//!         variant.chrom(),
//!         variant.pos(),      // 1-based position
//!         variant.reference(),
//!         variant.alts()
//!     );
//!
//!     // INFO fields (typed by header)
//!     match variant.info(&header, "DP") {
//!         htsvcf_core::InfoValue::Int(dp) => println!("  DP={}", dp),
//!         htsvcf_core::InfoValue::Array(vals) => println!("  DP={:?}", vals),
//!         _ => {}
//!     }
//!
//!     // FORMAT fields (per-sample)
//!     // variant.format(&header, "GT") returns FormatValue::PerSample(...)
//!     if let htsvcf_core::FormatValue::PerSample(gts) = variant.format(&header, "GT") {
//!         println!("  Genotypes: {:?}", gts);
//!     }
//! }
//! ```
//!
//! # Example: Accessing sample data
//!
//! ```no_run
//! use htsvcf_core::{open_reader, Header, Variant, FormatValue};
//!
//! let mut reader = open_reader("input.vcf.gz").unwrap();
//! let header = unsafe { Header::new(reader.header_ptr()) };
//!
//! if let Ok(Some(record)) = reader.next_record() {
//!     let variant = Variant::from_record(record);
//!
//!     // Get data for a specific sample by name
//!     if let Some(fields) = variant.sample(&header, "SAMPLE1") {
//!         for (tag, value) in fields {
//!             match value {
//!                 FormatValue::Int(v) => println!("  {}: {}", tag, v),
//!                 FormatValue::Float(v) => println!("  {}: {}", tag, v),
//!                 FormatValue::String(v) => println!("  {}: {}", tag, v),
//!                 FormatValue::Array(vals) => println!("  {}: {:?}", tag, vals),
//!                 _ => {}
//!             }
//!         }
//!     }
//!
//!     // Get data for all samples at once (more efficient)
//!     for sample_data in variant.samples(&header, None) {
//!         // sample_data is Vec<(String, FormatValue)> with a "sample_name" key
//!         println!("{:?}", sample_data);
//!     }
//!
//!     // Get data for a subset of samples
//!     let subset = variant.samples(&header, Some(&["SAMPLE1", "SAMPLE3"]));
//! }
//! ```
//!
//! # Example: Modifying variant data
//!
//! ```no_run
//! use htsvcf_core::{open_reader, Header, Variant};
//!
//! let mut reader = open_reader("input.vcf.gz").unwrap();
//! let header = unsafe { Header::new(reader.header_ptr()) };
//!
//! if let Ok(Some(record)) = reader.next_record() {
//!     let mut variant = Variant::from_record(record);
//!
//!     // Modify basic fields
//!     variant.set_id("rs12345").unwrap();
//!     variant.set_qual(Some(30.0));
//!     variant.set_filters(&["PASS".to_string()]).unwrap();
//!
//!     // Modify INFO fields (must match header type)
//!     variant.set_info_integer(&header, "DP", &[42]).unwrap();
//!     variant.set_info_float(&header, "AF", &[0.25, 0.75]).unwrap();
//!     variant.set_info_flag(&header, "SOMATIC", true).unwrap();
//!
//!     // Clear an INFO field
//!     variant.clear_info(&header, "DP").unwrap();
//!
//!     // Output as VCF line
//!     if let Some(line) = variant.to_string(&header) {
//!         println!("{}", line);
//!     }
//! }
//! ```
//!
//! # Value types
//!
//! [`InfoValue`] represents INFO field values:
//! - `Absent` - Tag not present in record
//! - `Missing` - Tag present but value is `.`
//! - `Bool(bool)` - Flag type
//! - `Int(i32)` - Single integer
//! - `Float(f32)` - Single float
//! - `String(String)` - Single string
//! - `Array(Vec<InfoValue>)` - Multiple values
//!
//! [`FormatValue`] represents FORMAT field values:
//! - `Absent` - Tag not present
//! - `Missing` - Value is `.`
//! - `Int(i32)`, `Float(f32)`, `String(String)` - Scalar values
//! - `Array(Vec<FormatValue>)` - Multi-value field for one sample
//! - `PerSample(Vec<FormatValue>)` - Array of values, one per sample

pub mod header;
pub mod reader;
pub mod region;
pub mod variant;

pub use header::Header;
pub use reader::{open_reader, InnerReader, Reader};
pub use variant::{
  get_format_tag_names, record_clear_info, record_format, record_info, record_sample,
  record_samples, record_set_info_flag, record_set_info_float, record_set_info_integer,
  record_set_info_string, record_to_string, FormatValue, InfoValue, Variant,
};
