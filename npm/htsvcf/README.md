# htsvcf

A fast Node.js library for reading VCF/BCF files, powered by HTSlib.

## Installation

```bash
npm install htsvcf
```

## Quick Start

```javascript
import { Reader } from "htsvcf";

const reader = new Reader("path/to/file.vcf.gz");

// Fast sync iteration
for (const variant of reader) {
  console.log(variant.chrom, variant.pos, variant.ref, variant.alt);
}

reader.close();
```

## API

### Reader

Create a reader from a VCF/BCF file path:

```javascript
import { Reader, openReader } from "htsvcf";

// Synchronous constructor
const reader = new Reader("path/to/file.vcf.gz");

// Async factory (useful if you need to await initialization)
const reader = await openReader("path/to/file.vcf.gz");
```

#### Iterating Records

There are two iteration modes:

- **Fast synchronous iteration** (recommended for max throughput): `for...of`
- **Asynchronous iteration** (doesn’t block the event loop): `for await...of`

##### Fast synchronous iteration (recommended)

This uses `nextSync()` under the hood and avoids per-record Promise/task overhead.

```javascript
for (const variant of reader) {
  console.log(`${variant.chrom}:${variant.pos} ${variant.ref}>${variant.alt.join(",")}`);
}
```

##### Asynchronous iteration

```javascript
for await (const variant of reader) {
  console.log(`${variant.chrom}:${variant.pos} ${variant.ref}>${variant.alt.join(",")}`);
}
```

##### Manual synchronous iteration

```javascript
let result;
while (!(result = reader.nextSync()).done) {
  const variant = result.value;
  console.log(variant.chrom, variant.pos);
}
```

#### Querying Regions (requires index)

```javascript
if (reader.hasIndex()) {
  // String form (1-based, inclusive)
  await reader.query("chr1:1000-2000");

  // Numeric form (0-based)
  await reader.query("chr1", 999, 2000);

  for await (const variant of reader) {
    // variants in region
  }
}
```

#### Closing

Always close the reader when done:

```javascript
reader.close();
```

### Header

Access the VCF header via `reader.header`:

```javascript
const header = reader.header;

// Get sample names
const samples = header.samples();
console.log("Samples:", samples); // ["S1", "S2", "S3"]

// Get INFO/FORMAT field definitions
const dpInfo = header.get("INFO", "DP");
if (dpInfo) {
  console.log(dpInfo.id);          // "DP"
  console.log(dpInfo.type);        // "Integer"
  console.log(dpInfo.number);      // "1"
  console.log(dpInfo.description); // "Depth"
}

// Get all header records
const records = header.records();
for (const rec of records) {
  if (rec.type === "INFO") {
    console.log(`INFO field: ${rec.key}`);
  }
}

// Add new INFO/FORMAT fields
header.addInfo("CUSTOM", "1", "Integer", "My custom field");
header.addFormat("GT", "1", "String", "Genotype");

// Get header as string
console.log(header.toString());
```

### Variant

Each variant record has the following properties and methods:

#### Basic Fields

```javascript
const variant = reader.nextSync().value;

variant.chrom;  // Chromosome (string)
variant.pos;    // Position, 1-based (number)
variant.start;  // Start position, 0-based (number)
variant.stop;   // End position (number)
variant.id;     // Variant ID (string), writable
variant.ref;    // Reference allele (string)
variant.alt;    // Alternate alleles (string[])
variant.qual;   // Quality score (number | null), writable
variant.filter; // Filter status (string[]), writable
```

#### Modifying Fields

```javascript
// Set variant ID
variant.id = "rs12345";

// Set quality
variant.qual = 99.5;
variant.qual = null; // Clear quality

// Set filters (filter IDs must exist in header)
variant.filter = ["PASS"];
variant.filter = ["LowQual", "LowDP"];
```

#### INFO Fields

```javascript
// Read INFO fields (returns typed values based on header)
const dp = variant.info("DP");        // number
const af = variant.info("AF");        // number[] for Number=A/R/G/.
const somatic = variant.info("SOMATIC"); // boolean for Flag type
const missing = variant.info("NOPE"); // undefined if not present

// Modify INFO fields
variant.set_info("DP", 42);
variant.set_info("AF", [0.1, 0.2]);
variant.set_info("SOMATIC", true);
variant.set_info("DP", null); // Clear field
```

#### FORMAT/Sample Fields

```javascript
// Get FORMAT field values for all samples (array per sample)
const dpValues = variant.format("DP"); // [10, 15, null]
const adValues = variant.format("AD"); // [[8, 2], [12, 3], [null, null]]

// Get all FORMAT data for a single sample
const s1 = variant.sample("S1");
if (s1) {
  console.log(s1.sample_name); // "S1"
  console.log(s1.DP);          // 10
  console.log(s1.AD);          // [8, 2]
  console.log(s1.GT);          // "0/1"
}

// Get all samples
const allSamples = variant.samples();
for (const sample of allSamples) {
  console.log(`${sample.sample_name}: DP=${sample.DP}`);
}

// Get a subset of samples
const subset = variant.samples(["S1", "S3"]);
```

#### String Representation

```javascript
// Get VCF line representation
console.log(variant.toString());
// chr1	1000	rs123	A	C	99	PASS	DP=42	GT:DP	0/1:10	0/0:15
```

## Complete Example

```javascript
import { Reader } from "htsvcf";

const reader = new Reader("samples.vcf.gz");

// Print header info
console.log("Samples:", reader.header.samples());
const dpDef = reader.header.get("INFO", "DP");
if (dpDef) {
  console.log(`DP field: ${dpDef.type} (${dpDef.description})`);
}

// Process variants
let count = 0;
for await (const v of reader) {
  // Filter by quality
  if (v.qual !== null && v.qual < 30) continue;

  // Get INFO depth
  const dp = v.info("DP");

  // Get per-sample data
  for (const s of v.samples()) {
    if (s.DP !== null && s.DP > 10) {
      console.log(`${v.chrom}:${v.pos} ${s.sample_name} DP=${s.DP}`);
    }
  }

  if (++count >= 100) break;
}

reader.close();
```

## Query Example

```javascript
import { Reader } from "htsvcf";

const reader = new Reader("indexed.vcf.gz");

if (reader.hasIndex()) {
  // Query a specific region
  await reader.query("chr17:7570000-7580000");

  for await (const v of reader) {
    console.log(`${v.chrom}:${v.pos} ${v.ref}>${v.alt.join(",")}`);
  }
}

reader.close();
```

## License

MIT
