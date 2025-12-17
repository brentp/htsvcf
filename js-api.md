# JS API (Bun/Node) proposal

This document sketches a proposed JavaScript API for `htsvcf` that can be installed from npm and used as a normal library in **Node.js** and **Bun**, while keeping the existing Rust CLI.

The implementation would be done as a **Node-API (N-API) native addon** built from Rust using [`napi-rs`](https://napi.rs/). Node-API provides a stable C ABI for native addons; Bun supports loading many Node-API addons (`.node` binaries) as well. The addon would expose classes like `Reader`, `Variant`, and `Header` directly to JS.

High-level approach:

- Keep the current CLI (`src/main.rs`) as-is.
- Add a separate Rust crate/target (or feature) producing a `.node` shared library using `napi-rs`.
- Implement a small “core” Rust layer (no `v8::*` types) that wraps `rust-htslib` readers/records/headers.
- Implement a thin Node-API layer that exposes JS classes and methods backed by those core Rust structs.
- Publish an npm package containing:
  - prebuilt binaries for common platforms (and/or build-from-source fallback)
  - a tiny JS loader that picks the correct `.node` binary at runtime
  - the CLI entry (either shipped as a separate binary or via `bin` wrapper that spawns it)

## Package usage

### ESM

```js
import { Reader, openReader } from "htsvcf";

const reader = new Reader("tests/t.vcf.gz");

for await (const v of reader) {
  console.log(v.chrom, v.pos, v.ref, v.alt);
  console.log(v.info("DP"));
}

// region query (indexed input only)
await reader.query("chr1:1000-2000");
for await (const v of reader) {
  console.log(v.toString());
}

// header access
console.log(reader.header.toString());
console.log(reader.header.get("INFO", "DP"));
```

### CommonJS

```js
const { Reader } = require("htsvcf");

const reader = new Reader("tests/t.vcf.gz");
```

## API surface (proposal)

### `class Reader`

A `Reader` opens a VCF/BCF file (optionally indexed) and yields `Variant` objects.

```ts
export type ReaderOptions = {
  // room for future tuning: threads, caching, etc.
};

export class Reader {
  constructor(path: string, opts?: ReaderOptions);

  // A stable header object tied to the reader lifetime.
  get header(): Header;

  hasIndex(): boolean;

  // Set the iteration window.
  // Throws if the file is not indexed.
  query(region: string): Promise<void>; // e.g. "chr1:1000-2000" (1-based, inclusive)
  query(chrom: string, start0: number, end0Inclusive0?: number): Promise<void>; // 0-based, inclusive end if provided

  // Iteration
  [Symbol.asyncIterator](): AsyncIterator<Variant>;
  next(): Promise<IteratorResult<Variant>>;

  // Explicit resource cleanup.
  close(): void;
}

export function openReader(path: string, opts?: ReaderOptions): Promise<Reader>;
```

Notes:

- `Reader` is an async iterator so it can naturally integrate with JS streaming loops.
  - In the current npm wrapper, `[Symbol.asyncIterator]` is shimmed in JS to return `this` (the native addon only provides `next()`).
- `close()` is explicit so JS consumers don’t rely entirely on GC finalizers to close file handles.
- `query(...)` updates the reader’s internal iterator state; a subsequent `for await` continues in that region.
  - For the numeric overload, `start0` is 0-based and `end0Inclusive0` is 0-based inclusive when provided.

### `class Variant`

A `Variant` is an immutable view of one record.

```ts
export class Variant {
  get chrom(): string;
  get rid(): number | null;

  // Coordinates
  get pos(): number;   // 1-based
  get start(): number; // 0-based
  get stop(): number;  // htslib semantics

  // Core VCF fields
  get id(): string;
  get ref(): string;
  get alt(): string[];
  get qual(): number | null;

  // INFO lookup (typed)
  info(tag: string):
    | boolean
    | number
    | string
    | Array<number | string | null>
    | null
    | undefined;

  // Format using the associated header
  toString(): string;
}
```

Notes:

- `Variant.info(tag)` uses the header to determine the tag type/cardinality and returns:
  - `undefined` when the tag is absent or unknown
  - `null` when present but missing
  - `boolean | number | string | Array<...>` when present
  - When the INFO field is an array, individual missing elements are returned as `null`.
- `Variant.toString()` returns the formatted VCF line without a trailing newline.

### `class Header`

The header object is exposed as `reader.header`. It should be a stable JS object that references the underlying header owned by the reader (i.e. no deep-copy per access).

```ts
export type HeaderGetResult = {
  id: string;
  type: "Flag" | "Integer" | "Float" | "String";
  number: string; // e.g. "1", "A", "R", "G", "."
  description: string;
};

export type HeaderRecord =
  | { type: "INFO"; key: string; [k: string]: string }
  | { type: "FORMAT"; key: string; [k: string]: string }
  | { type: "FILTER"; key: string; [k: string]: string }
  | { type: "contig"; key: string; [k: string]: string }
  | { type: "structured"; key: string; [k: string]: string }
  | { type: "generic"; key: string; value: string };

export class Header {
  records(): HeaderRecord[];

  get(section: "INFO" | "FORMAT", id: string): HeaderGetResult | undefined;

  addInfo(
    id: string,
    number: string,
    type: "Flag" | "Integer" | "Float" | "String",
    description: string
  ): void;

  addFormat(
    id: string,
    number: string,
    type: "Flag" | "Integer" | "Float" | "String",
    description: string
  ): void;

  toString(): string;
}
```

Notes:

- `Header.addInfo/addFormat` mutate the underlying header and should affect subsequent lookups and formatting.
- `Header.records()` and `Header.toString()` are intentionally “materializing” APIs and may allocate; callers should avoid calling them in tight loops.

## Runtime / threading notes

- The addon should avoid blocking the JS event loop for large files; `napi-rs` makes it straightforward to run reading/decoding work on a native thread and resolve a Promise.
- Iteration can be implemented in two viable ways:
  - `next(): Promise<IteratorResult<Variant>>` that performs the read in a background thread per call.
  - A background worker that streams records through a native queue.

A conservative initial implementation is the per-`next()` Promise approach; it’s simpler and works well with backpressure (the consumer controls pace).

## Relationship to the existing CLI

This JS API is intended to coexist with the current CLI:

- The CLI remains the best tool for “evaluate expression per variant” workflows.
- The JS addon focuses on a normal programmatic API (`Reader`/`Variant`/`Header`) that integrates with the host runtime (Node/Bun) without embedding V8.
