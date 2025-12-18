# htsvcf workspace [![CI](https://github.com/brentp/htsvcf/actions/workflows/ci.yml/badge.svg)](https://github.com/brentp/htsvcf/actions/workflows/ci.yml)

Reading and working with VCF/BCF using HTSlib (via `rust-htslib`), with two JavaScript-related facets:

1) a rust library that facilitates accessing variant properties with javascript expressions.
2) a **Node-API (N-API) addon** intended for normal programmatic use from Node.js/Bun, published via an npm package

## Repo layout

### Rust crates (`crates/`)

- `crates/htsvcf` (library + CLI)
  - Provides a Rust library and a CLI that evaluates a JS expression per record.
  - Docs: `crates/htsvcf/README.md`

- `crates/htsvcf-core` (Rust “core” library)
  - Shared, non-JS-facing functionality for working with readers/headers/variants.
  - Used by the N-API addon.

- `crates/htsvcf-napi` (Node-API addon)
  - Rust `cdylib` built with `napi-rs` v3.
  - Exposes JS classes like `Reader`, `Variant`, and `Header` for Node/Bun.

### JavaScript / npm package (`npm/`)

- `npm/htsvcf`
  - The npm package wrapper (ESM + CJS entrypoints + TypeScript types).
  - Includes examples and `node:test` tests.
  - Entry points/types: `npm/htsvcf/index.js`, `npm/htsvcf/index.mjs`, `npm/htsvcf/index.d.ts`

## API documentation

- JS API spec/proposal: `js-api.md`
  - This is the current reference for the intended Node/Bun API surface.
  - Includes `Variant.info(tag)` and `Variant.format(tag)` (typed INFO/FORMAT lookups).
  - `Variant.id`, `Variant.qual`, and `Variant.filter` are writable (e.g. `variant.qual = null` clears; `variant.filter = ['PASS']` clears and reads back as `[]`).

## Development notes

### Running Rust tests

- `cargo test`

### Building and testing the npm package

The npm package expects a compiled native addon to be present as `npm/htsvcf/htsvcf.node`.

From the repo root:

- Build the addon (release): `cargo build -p htsvcf-napi --release`
- Copy/rename the produced shared library to `npm/htsvcf/htsvcf.node`
  - Linux: `cp -f target/release/libhtsvcf_napi.so npm/htsvcf/htsvcf.node`
  - macOS: `cp -f target/release/libhtsvcf_napi.dylib npm/htsvcf/htsvcf.node`
- Run the smoke example against the rebuilt addon:
  - Node: `node npm/htsvcf/examples/smoke.mjs`
  - Bun: `bun run npm/htsvcf/examples/smoke.mjs`
- Run JS tests (Node): `npm -C npm/htsvcf test`
