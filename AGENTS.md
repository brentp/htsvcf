# Agent Notes (htsvcf v8 + napi)

This repo exposes the same high-level API through **two bindings**:

- **v8 binding**: `crates/htsvcf/` (direct `v8` API)
- **Node-API binding**: `crates/htsvcf-napi/` + JS wrapper `npm/htsvcf/`

## Feature addition checklist (required)

When adding or changing any public API (Reader/Header/Variant/etc.), 
Attempt to **minimize code duplication** by sharing methods written in the core.
do **all** of the following:

1. **Implement in the v8 binding**
   - Update the v8-side implementation in `crates/htsvcf/src/*`.
   - Ensure the JS-visible names match the existing API surface (e.g. `hasIndex`, iterator protocol, etc.).
   - Update the `crates/htsvcf/README.md` with the new fields/methods

2. **Implement in the N-API binding**
   - Update `crates/htsvcf-napi/src/lib.rs` to expose the same API.
   - If it changes the public JS API, update `npm/htsvcf/index.d.ts` accordingly.

3. **Add tests for both bindings**
   - **v8 tests**: add/extend Rust tests under `crates/htsvcf/` (e.g. `#[cfg(test)]` modules in `src/*` or `crates/htsvcf/tests/*.rs`).
   - **napi tests**: add/extend Node tests under `npm/htsvcf/test/*.test.mjs`.

4. **Build + copy the native addon, then run JS tests**
   - Build the N-API addon:
     - `cargo build -p htsvcf-napi --release`
   - Copy the produced shared library into the npm package location:
     - Linux: `cp -f target/release/libhtsvcf_napi.so npm/htsvcf/htsvcf.node`
     - macOS: `cp -f target/release/libhtsvcf_napi.dylib npm/htsvcf/htsvcf.node`
   - Run Node tests:
     - `npm -C npm/htsvcf test`
   - Run Bun smoke test (quick sanity check):
     - `bun run npm/htsvcf/examples/smoke.mjs`

5. **Run Rust tests**
   - `cargo test -p htsvcf`
   - `cargo test -p htsvcf-napi`
