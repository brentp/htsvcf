# htsvcf-core

Core VCF/BCF parsing library built on HTSlib.

This crate provides the shared implementation for reading and writing VCF/BCF files
and accessing variant data. It is used by both the V8 binding (`htsvcf`)
and the Node-API binding (`htsvcf-napi`).

## Overview

The main types are:

- `Reader` - Opens and iterates VCF/BCF files (with optional index-based queries)
- `Header` - Access and modify VCF header metadata (INFO/FORMAT definitions, samples)
- `Variant` - A single VCF record with typed accessors for all fields
- `Writer` - Write VCF/BCF files

## Example: Reading, modifying, and writing a VCF

```rust
use htsvcf_core::{open_reader, open_writer, Header, Variant, WriterOptions};

// Open input VCF and get a copy of its header
let mut reader = open_reader("input.vcf.gz").expect("failed to open");
let header = unsafe { Header::new(reader.header_ptr()) };

// Add a new INFO field to the header
header.add_info("VARIANT_LENGTH", "1", "Integer", "Length of variant (REF - ALT)");

// Open writer with the modified header
let mut writer = open_writer("output.vcf.gz", &header, WriterOptions::default())
    .expect("failed to create writer");

while let Ok(Some(record)) = reader.next_record() {
    let mut variant = Variant::from_record(record);

    // Translate the record to the new header (required after adding INFO fields)
    variant.translate(&header).expect("translate failed");

    // Access basic fields
    println!("{}:{} {} -> {:?}",
        variant.chrom(),
        variant.pos(),      // 1-based position
        variant.reference(),
        variant.alts()
    );

    // Compute and set the new INFO field
    let ref_len = variant.reference().len() as i32;
    let alt_len = variant.alts().first().map(|a| a.len() as i32).unwrap_or(0);
    variant.set_info_integer(&header, "VARIANT_LENGTH", &[ref_len - alt_len]).unwrap();

    // Write the modified record
    writer.write_record(variant.record_mut()).expect("write failed");
}
```

## Value Types

`InfoValue` represents INFO field values:

- `Absent` - Tag not present in record
- `Missing` - Tag present but value is `.`
- `Bool(bool)` - Flag type
- `Int(i32)` - Single integer
- `Float(f32)` - Single float
- `String(String)` - Single string
- `Array(Vec<InfoValue>)` - Multiple values

`FormatValue` represents FORMAT field values:

- `Absent` - Tag not present
- `Missing` - Value is `.`
- `Int(i32)`, `Float(f32)`, `String(String)` - Scalar values
- `Array(Vec<FormatValue>)` - Multi-value field for one sample
- `PerSample(Vec<FormatValue>)` - Array of values, one per sample
