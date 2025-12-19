# htsvcf workspace [![CI](https://github.com/brentp/htsvcf/actions/workflows/ci.yml/badge.svg)](https://github.com/brentp/htsvcf/actions/workflows/ci.yml) [![Documentation](https://img.shields.io/badge/docs-latest-blue)](https://brentp.github.io/htsvcf/latest/htsvcf_napi/index.html)

Reading and working with VCF/BCF using HTSlib (via `rust-htslib`), with two JavaScript-related facets:

1. A Rust library + CLI that evaluates JavaScript expressions per VCF record
2. A **Node-API addon** for programmatic use from Node.js/Bun

## crates/htsvcf (CLI + V8 Library)

Evaluate JavaScript expressions per VCF/BCF record:

```bash
# Print chrom:pos for each variant
cargo run --release -- tests/t.vcf.gz "variant.chrom + ':' + variant.pos"

# Extract INFO field
cargo run --release -- tests/t.vcf.gz "variant.info('DP')"

# Query a region (requires index)
cargo run --release -- tests/t.vcf.gz "variant.pos" --region chr1:1000-2000
```

Use as a Rust library with the `Evaluator` API:

```rust
use htsvcf::Evaluator;
use rust_htslib::bcf::{self, Read};

let mut reader = bcf::Reader::from_path("input.vcf.gz")?;
let mut eval = Evaluator::new(reader.header())?;

// Optionally define custom functions
eval.run("function passes(v) { return v.info('DP') > 20 }")?;

for result in reader.records() {
    let record = result?;
    eval.set_record(record);
    
    if eval.eval::<bool>("passes(variant)")? {
        // Use take() to get ownership of the record
        let record = eval.take().unwrap();
        // write record to output, collect it, etc.
    }
}
```

## crates/htsvcf-napi + npm/htsvcf (Node.js/Bun)

Native addon for reading and manipulating VCF files from JavaScript.
See [npm/htsvcf/examples/smoke.mjs](npm/htsvcf/examples/smoke.mjs) for a complete example.

```javascript
import { Reader } from "htsvcf";

const reader = new Reader("input.vcf.gz");

// Async iteration
for await (const v of reader) {
  console.log(v.chrom, v.pos, v.ref, v.alt);
  console.log("DP:", v.info("DP"));
}

// Sync iteration
let result;
while (!(result = reader.nextSync()).done) {
  const v = result.value;
  console.log(v.chrom, v.pos, v.ref, v.alt);
}

// Query a region (requires index)
if (reader.hasIndex()) {
  await reader.query("chr1:1000-2000");
  for await (const v of reader) {
    console.log(v.toString());
  }
}

// Access sample data
for await (const v of reader) {
  const s1 = v.sample("SAMPLE1");
  console.log(s1.DP, s1.AD);  // { DP: 30, AD: [20, 10], sample_name: "SAMPLE1" }
}

// Modify variants
for await (const v of reader) {
  v.id = "rs12345";
  v.qual = 30;
  v.filter = ["PASS"];
  v.set_info("DP", 100);
}

reader.close();
```

Header inspection:

```javascript
const reader = new Reader("input.vcf.gz");
const hdr = reader.header;

// Get field definitions
hdr.get("INFO", "DP");   // { id: "DP", type: "Integer", number: "1", description: "..." }
hdr.samples();           // ["SAMPLE1", "SAMPLE2", ...]

// Add new fields
hdr.addInfo("CUSTOM", "1", "Integer", "My custom field");
hdr.addFormat("GT2", "1", "String", "Secondary genotype");
```

## Development

### Testing

```bash
# Rust tests
cargo test

# Build and test Node-API addon
cargo build -p htsvcf-napi --release
cp -f target/release/libhtsvcf_napi.so npm/htsvcf/htsvcf.node   # Linux
# cp -f target/release/libhtsvcf_napi.dylib npm/htsvcf/htsvcf.node  # macOS
npm -C npm/htsvcf test
bun npm/htsvcf/examples/smoke.mjs
```

### API Documentation

See `js-api.md` for the full JS API specification, including:
- `Variant.info(tag)` / `Variant.format(tag)` for typed INFO/FORMAT lookups
- `Variant.set_info(tag, value)` for mutating INFO fields (`null` clears)
- Writable properties: `id`, `qual`, `filter`

### Releasing

See [npm/htsvcf/RELEASING.md](npm/htsvcf/RELEASING.md) for instructions on publishing new npm releases.
