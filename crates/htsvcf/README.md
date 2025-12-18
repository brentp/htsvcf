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

## Library

Add to your `Cargo.toml`:

- `htsvcf = { path = "/path/to/htsvcf" }`

Minimal example:

```rust
use htsvcf::runner::{run_vcf_expr_with, RunOptions};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_vcf_expr_with(
        "tests/t.vcf.gz",
        "variant.chrom + ':' + variant.pos",
        RunOptions::default(),
        |line| {
            // do something with each stringified JS result
            println!("{line}");
            Ok(())
        },
    )
}
```

## JavaScript API

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
  - `header.addInfo(id, number, type, description)`
  - `header.addFormat(id, number, type, description)`
- `variant`: fields/methods
  - `variant.chrom` (string)
  - `variant.pos` (1-based integer)
  - `variant.start` (0-based integer)
  - `variant.stop` (end position)
  - `variant.id` (string; writable)
  - `variant.ref` (string)
  - `variant.alt` (array of strings)
  - `variant.qual` (number or `null`; writable, set `null` to clear)
  - `variant.filter` (array of strings; writable)
    - `variant.filter = ['PASS']` clears filters and reads back as `[]`
    - Named filters must exist in the header (a `##FILTER=<ID=...>` definition) to set successfully
- `variant.info(tag)` (typed `INFO` lookup using `header`)
- `variant.set_info(tag, value)` (mutate INFO; typed by `header`, `null`/`undefined` clears)
- `variant.format(tag)` (typed `FORMAT` lookup using `header`, returns array per sample)


## Notes

- V8 initialization is global-process state; the library uses a global lock to
  serialize access for safety.
- This is currently oriented around evaluating an expression per record.
