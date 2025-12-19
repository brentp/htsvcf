//! VCF/BCF processing with embedded V8 JavaScript.
//!
//! This crate exposes HTSlib VCF/BCF records to JavaScript via the V8 engine,
//! enabling powerful filtering, transformation, and analysis using JS expressions.
//!
//! # Overview
//!
//! The primary entrypoint is [`runner::run_vcf_expr_with`], which iterates
//! records in a VCF/BCF file and evaluates a JavaScript expression for each.
//!
//! # CLI Example
//!
//! ```bash
//! # Print chrom:pos for each variant
//! htsvcf input.vcf.gz "variant.chrom + ':' + variant.pos"
//!
//! # Filter by INFO field
//! htsvcf input.vcf.gz "variant.info('DP') > 20 ? variant.toString() : ''"
//!
//! # Access sample genotypes
//! htsvcf input.vcf.gz "variant.sample('NA12878').GT"
//! ```
//!
//! # Library Example
//!
//! ```no_run
//! use htsvcf::runner::{run_vcf_expr_with, RunOptions};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     run_vcf_expr_with(
//!         "input.vcf.gz",
//!         "variant.chrom + ':' + variant.pos",
//!         RunOptions::default(),
//!         |line| {
//!             println!("{}", line);
//!             Ok(())
//!         },
//!     )
//! }
//! ```
//!
//! # JavaScript API
//!
//! The following globals are available in JS expressions:
//!
//! ## `variant` - The current VCF record
//!
//! **Read-only fields:**
//! - `variant.chrom` - Chromosome name (string)
//! - `variant.pos` - 1-based position (integer)
//! - `variant.start` - 0-based start position
//! - `variant.stop` - End position
//! - `variant.ref` - Reference allele (string)
//! - `variant.alt` - Alternate alleles (array of strings)
//!
//! **Read/write fields:**
//! - `variant.id` - Variant ID (string, e.g., "rs12345")
//! - `variant.qual` - Quality score (number or null)
//! - `variant.filter` - Filter status (array of strings)
//!
//! **INFO field access:**
//! ```js
//! variant.info('DP')           // => 42 (integer)
//! variant.info('AF')           // => [0.25, 0.75] (array)
//! variant.info('SOMATIC')      // => true (flag)
//! variant.info('MISSING')      // => undefined (absent)
//!
//! // Modify INFO (value type must match header definition)
//! variant.set_info('DP', 100)
//! variant.set_info('AF', [0.1, 0.9])
//! variant.set_info('SOMATIC', true)
//! variant.set_info('DP', null) // Clear the field
//! ```
//!
//! **FORMAT field access (per-sample):**
//! ```js
//! variant.format('GT')         // => ["0/1", "0/0", "1/1"] (one per sample)
//! variant.format('DP')         // => [30, 25, null] (null for missing)
//! variant.format('AD')         // => [[10, 20], [25, 0], [0, 30]] (arrays)
//! ```
//!
//! **Sample access:**
//! ```js
//! // Get all FORMAT fields for one sample
//! const s = variant.sample('NA12878')
//! s.GT          // => "0/1"
//! s.DP          // => 30
//! s.AD          // => [10, 20]
//! s.sample_name // => "NA12878"
//!
//! // Get all samples at once (array of objects)
//! const all = variant.samples()
//! all[0].GT     // First sample's genotype
//!
//! // Get a subset of samples
//! const subset = variant.samples(['NA12878', 'NA12879'])
//! ```
//!
//! **Output:**
//! ```js
//! variant.toString()  // => Full VCF line (without newline)
//! ```
//!
//! ## `header` - VCF header metadata
//!
//! ```js
//! // List all samples
//! header.samples()  // => ["NA12878", "NA12879", ...]
//!
//! // Get INFO/FORMAT field definitions
//! header.get('INFO', 'DP')
//!   // => { id: 'DP', type: 'Integer', number: '1', description: 'Read depth' }
//!
//! header.get('FORMAT', 'GT')
//!   // => { id: 'GT', type: 'String', number: '1', description: 'Genotype' }
//!
//! // List all header records
//! header.records()  // => [{ type: 'INFO', ID: 'DP', ... }, ...]
//!
//! // Add new fields (for use with set_info)
//! header.addInfo('CUSTOM', '1', 'Integer', 'My custom field')
//! header.addFormat('CUSTOM', '1', 'Float', 'Per-sample value')
//!
//! // Get full header text
//! header.toString()
//! ```
//!
//! ## `Reader` - Iterate VCF files from JS
//!
//! ```js
//! const r = new Reader('input.vcf.gz')
//!
//! // Iterate all records
//! for (const v of r) {
//!     if (v.info('DP') > 20) {
//!         print(v.toString())
//!     }
//! }
//!
//! // Query a region (requires index)
//! if (r.hasIndex()) {
//!     r.query('chr1:1000-2000')  // Region string
//!     // or: r.query('chr1', 999, 2000)  // 0-based coords
//!     for (const v of r) {
//!         // ... variants in region
//!     }
//! }
//!
//! // Access header
//! const samples = r.header().samples()
//! ```

pub mod evaluator;
pub mod header;
pub mod reader;
pub mod runner;
pub mod runtime;
pub mod variant;

pub use evaluator::{EvalError, Evaluator, FromJsValue};
pub use header::Header;
pub use variant::Variant;
