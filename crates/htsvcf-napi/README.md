# htsvcf-napi

This crate builds the native Node-API addon (via `napi-rs`) for the `htsvcf` JavaScript package.

## Build + Smoke Test

The npm wrapper package (`npm/htsvcf`) expects a compiled native addon at `npm/htsvcf/htsvcf.node`.

From the repo root:

- Build the addon (release): `cargo build -p htsvcf-napi --release`
- Copy/rename the produced shared library to the npm package:
  - Linux: `cp -f target/release/libhtsvcf_napi.so npm/htsvcf/htsvcf.node`
  - macOS: `cp -f target/release/libhtsvcf_napi.dylib npm/htsvcf/htsvcf.node`
- Run the smoke example against the rebuilt addon:
  - Node: `node npm/htsvcf/examples/smoke.mjs`
  - Bun: `bun run npm/htsvcf/examples/smoke.mjs`

## Rust Tests

- `cargo test -p htsvcf-napi`
