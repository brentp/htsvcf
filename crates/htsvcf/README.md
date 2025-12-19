# htsvcf

Expose HTSlib (VCF/BCF) records and header to JavaScript (V8).

This crate builds both:

- A Rust library (`htsvcf`) you can embed in your own program.
- A small CLI (`htsvcf`) that evaluates a JS expression per record.

## CLI

Build and run:

- `cargo run --release -- <input.vcf|input.bcf> [js_expr]`

Examples:

- `cargo run --release -- tests/t.vcf.gz "variant.chrom + ':' + variant.pos"`
- `cargo run --release -- tests/t.vcf.gz "variant.info('DP')"`

The CLI prints the expression result (stringified) once per record.

## Library Example

Add to your `Cargo.toml`:

```toml
[dependencies]
htsvcf = { git = "https://github.com/brentp/htsvcf", package = "htsvcf" }
```

The `Evaluator` struct lets you iterate over VCF records in Rust while applying
user-defined JavaScript expressions. The generic `eval::<T>()` method converts
JavaScript results to Rust types:

```rust
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
    let mut eval = Evaluator::new(reader.header())?;

    for result in reader.records() {
        let record = result?;
        eval.set_record(record);
        let dp: i32 = eval.eval("variant.info('DP')")?;
        println!("DP = {}", dp);
    }
    Ok(())
}
```

### Evaluator API

#### Expression Caching

Expressions are compiled to JavaScript on first use and cached by their exact
string value. Subsequent calls with the same expression string reuse the compiled
script. The cache holds up to 8192 unique expressions; attempting to add more
returns `EvalError::CacheFull`.

For best performance, reuse the same expression strings across records rather
than generating dynamic expression strings per-record.

#### Multiple Expressions

You can evaluate multiple different expressions against the same record:

```rust
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
    let mut eval = Evaluator::new(reader.header())?;

    for result in reader.records() {
        let record = result?;
        eval.set_record(record);

        // Multiple expressions, all evaluated against the same record
        let dp: i32 = eval.eval("variant.info('DP')")?;
        let passes: bool = eval.eval("variant.info('DP') > 20")?;
        let loc: String = eval.eval("variant.chrom + ':' + variant.pos")?;

        if passes {
            let record = eval.take().unwrap();
            // write record...
        }
    }
    Ok(())
}
```

#### Supported Types

The `eval::<T>()` method supports these Rust types:

- `String` - any JS value converted to string
- `bool` - uses JavaScript truthiness (`0`, `""`, `null`, `undefined`, `NaN`, `false` are falsy)
- `i32`, `i64` - integers
- `f32`, `f64` - floating point numbers
- `Vec<T>` - arrays (e.g., `Vec<f64>` for `variant.info('AF')`)
- `Option<T>` - returns `None` for `null`/`undefined`

For complex types (custom structs, `serde_json::Value`, `HashMap`), use `eval_serde::<T>()`:

```rust
use serde::Deserialize;

#[derive(Deserialize)]
struct VariantSummary {
    chrom: String,
    pos: i64,
    depth: Option<i32>,
}

let mut eval = Evaluator::new(reader.header())?;
let record = reader.records().next().unwrap()?;
eval.set_record(record);
let summary: VariantSummary = eval.eval_serde(
    "({ chrom: variant.chrom, pos: variant.pos, depth: variant.info('DP') })"
)?;
```

#### Filtering

Use `eval::<bool>()` to filter variants:

```rust
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
    let header = bcf::Header::from_template(reader.header());
    let mut writer = bcf::Writer::from_path("output.vcf.gz", &header, true, bcf::Format::Vcf)?;
    
    let mut eval = Evaluator::new(reader.header())?;

    for result in reader.records() {
        let record = result?;
        eval.set_record(record);
        if eval.eval::<bool>("variant.info('DP') > 20 && variant.qual > 30")? {
            // Use take() to get ownership of the record for writing
            let record = eval.take().unwrap();
            writer.write(&record)?;
        }
    }
    Ok(())
}
```

#### Extracting Records with `take()`

After calling `eval()`, use `take()` to get ownership of the `bcf::Record`:

```rust
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
    let mut eval = Evaluator::new(reader.header())?;

    for result in reader.records() {
        let record = result?;
        eval.set_record(record);
        let passes: bool = eval.eval("variant.info('DP') > 10")?;
        if passes {
            // take() returns Option<bcf::Record>
            // Returns None if called before set_record() or called twice without set_record()
            let record = eval.take().unwrap();
            // Use the record (write to file, collect, etc.)
        }
    }
    Ok(())
}
```

#### Arrays and Optional Values

```rust
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
    let mut eval = Evaluator::new(reader.header())?;

    // Extract array of allele frequencies
    let record = reader.records().next().unwrap()?;
    eval.set_record(record);
    let afs: Vec<f64> = eval.eval("variant.info('AF')")?;

    // Handle potentially missing values
    let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
    let mut eval = Evaluator::new(reader.header())?;
    let record = reader.records().next().unwrap()?;
    eval.set_record(record);
    let maybe: Option<i32> = eval.eval("variant.info('MAYBE_MISSING')")?;
    
    Ok(())
}
```

#### Complex Expressions

The JS expression can include multi-statement logic:

```rust
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
    let mut eval = Evaluator::new(reader.header())?;

    let expr = r#"
        const gt = variant.format('GT');
        const het_count = gt.filter(g => g && g[0] !== g[1]).length;
        het_count > 0
    "#;

    for result in reader.records() {
        let record = result?;
        eval.set_record(record);
        if eval.eval::<bool>(expr)? {
            println!("Variant has heterozygous samples");
        }
    }
    Ok(())
}
```

### Callback-Based API

For simpler use cases, `run_vcf_expr_with` handles file iteration for you:

```rust
use htsvcf::runner::{run_vcf_expr_with, RunOptions};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_vcf_expr_with(
        "input.vcf.gz",
        "variant.chrom + ':' + variant.pos",
        RunOptions::default(),
        |result| {
            println!("{result}");
            Ok(())
        },
    )
}
```

## JavaScript API

### Variant Attributes

```javascript
// Core fields (read-only)
variant.chrom   // string - chromosome/contig name
variant.pos     // number - 1-based position (VCF POS column)
variant.start   // number - 0-based start coordinate
variant.stop    // number - end position
variant.ref     // string - reference allele
variant.alt     // string[] - array of alternate alleles

// Writable fields
variant.id      // string - variant ID (writable)
variant.qual    // number | null - quality score (writable, set null to clear)
variant.filter  // string[] - array of filter IDs (writable)

// INFO field access
variant.info(tag)              // get typed INFO value (uses header for type info)
variant.set_info(tag, value)   // set INFO value (null/undefined clears the tag)

// FORMAT/genotype field access
variant.format(tag)            // get typed FORMAT values as array (one per sample)
variant.sample(name)           // get all FORMAT fields for a single sample as object
variant.samples()              // get all FORMAT fields for all samples as array of objects
variant.samples(['S1', 'S2'])  // get FORMAT fields for a subset of samples

// Serialization
variant.toString()             // format record as VCF line (without newline)
```

### General Usage

The following globals are available to the expression:

- `Reader`: constructor for reading VCF/BCF
  - `const r = new Reader(path)`
  - Iterable: `for (const v of r) { ... }`
  - `r.hasIndex() -> boolean`
  - `r.header() -> header`
  - `r.query(region)` (requires index)
    - `region` form: `"chr"`, `"chr:100"`, `"chr:100-200"` (1-based inclusive)
  - `r.query(chrom, start, end?)` (requires index)
    - numeric form uses 0-based inclusive coordinates

Example (iterate all records):

```js
const r = new Reader("tests/t.vcf.gz")
let n = 0
for (const v of r) {
  n += 1
}
String(n)
```

Example (query if index present):

```js
const r = new Reader("tests/t.vcf.gz")
if (r.hasIndex()) {
  r.query("chr1:1000-2000")
  for (const v of r) {
    // ...
  }
}
```

- `header`: methods
  - `header.records() -> Array<object>`
    - Each record has a `type` field and additional key/value pairs parsed from the header line.
    - `type` can be: `"INFO"`, `"FORMAT"`, `"FILTER"`, `"contig"`, `"structured"`, `"generic"`.
    - For `type === "FILTER"`, records correspond to `##FILTER=<...>` header lines (e.g. named filters like `q10`, etc.).
      - Example (list filter IDs defined in the header):

        ```js
        const filters = header
          .records()
          .filter(r => r.type === 'FILTER')
          .map(r => r.key)
        filters.join(',')
        ```

  - `header.get(section, id) -> {id, type, number} | undefined` where `section` is `"INFO"` or `"FORMAT"`
  - `header.samples() -> Array<string>` returns the list of sample names from the header
  - `header.addInfo(id, number, type, description)`
  - `header.addFormat(id, number, type, description)`
- `variant`: a VCF/BCF record with fields and methods

### Variant Examples

```javascript
// Basic field access
variant.chrom + ':' + variant.pos        // "chr1:1000"
variant.ref + '>' + variant.alt.join(',') // "A>C,G"

// Modify variant ID and quality
variant.id = 'rs12345'
variant.qual = 30.5
variant.qual = null  // clear quality

// Work with filters
variant.filter = ['PASS']    // set to PASS (reads back as [])
variant.filter = ['q10']     // set named filter (must exist in header)
variant.filter = []          // clear all filters

// INFO field access (typed by header)
variant.info('DP')           // 10 (Integer, Number=1 → scalar)
variant.info('AF')           // [0.1, 0.2] (Float, Number=A → array)
variant.info('SOMATIC')      // true (Flag type)
variant.info('MISSING')      // undefined (tag not present)

// Modify INFO fields
variant.set_info('DP', 42)
variant.set_info('AF', [0.25, 0.75])
variant.set_info('SOMATIC', true)
variant.set_info('DP', null)  // remove the tag

// FORMAT field access (returns array, one value per sample)
variant.format('GT')         // [[0, 1], [1, 1]] (genotypes per sample)
variant.format('DP')         // [20, 15] (depth per sample)
variant.format('AD')         // [[10, 10], [5, 10]] (allele depths per sample)

// Single sample access (returns object with all FORMAT fields)
const s = variant.sample('NA12878')
s.sample_name                // "NA12878"
s.GT                         // [0, 1]
s.DP                         // 20
s.AD                         // [10, 10]

// All samples at once
const all = variant.samples()
all[0].sample_name           // first sample name
all[0].DP                    // first sample's depth

// Subset of samples
const subset = variant.samples(['NA12878', 'NA12879'])

// Output as VCF line
variant.toString()           // "chr1\t1000\t.\tA\tC\t30\tPASS\tDP=10\t..."
```


## Notes

- V8 initialization is global-process state; the library uses a global lock to
  serialize access for safety.
- This is currently oriented around evaluating an expression per record.

## Building a Static Binary

To build a fully static binary (no dynamic library dependencies):

```bash
cargo rustc -p htsvcf --release --features static --bin htsvcf -- -C target-feature=+crt-static
```

## Testing

Run tests with:

```bash
cargo test -p htsvcf
```

**Note:** You may see messages like `<unknown>:5: Uncaught SyntaxError: Unexpected identifier 'is'`
during test runs. This is expected—it comes from V8 printing to stderr when the
`test_compile_error` test intentionally compiles invalid JavaScript to verify error handling.
