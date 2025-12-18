# Releasing htsvcf to npm

This document describes how to publish a new version of the `htsvcf` npm package.

## Prerequisites

1. **NPM_TOKEN secret**: Ensure the `NPM_TOKEN` secret is configured in GitHub repository settings:
   - Go to repository Settings > Secrets and variables > Actions
   - Add a secret named `NPM_TOKEN` with a valid npm access token
   - The token must have publish permissions for the `htsvcf` package

2. **npm account**: The token owner must have publish access to the `htsvcf` package on npmjs.com

## Release Process

### 1. Update version numbers

Update the version in both places to keep them in sync:

- `Cargo.toml` (workspace version)
- `npm/htsvcf/package.json`

```bash
# Example: bump to 0.2.0
# Edit Cargo.toml: version = "0.2.0"
# Edit npm/htsvcf/package.json: "version": "0.2.0"
```

### 2. Commit the version bump

```bash
git add Cargo.toml npm/htsvcf/package.json
git commit -m "chore: bump version to 0.2.0"
```

### 3. Create and push a tag

The tag name must start with `v` followed by the version number:

```bash
git tag v0.2.0
git push origin main
git push origin v0.2.0
```

### 4. Monitor the release

The GitHub Action will automatically:

1. Build the N-API addon for Linux (x86_64) and macOS (ARM64)
2. Update the package.json version from the tag
3. Publish the package to npm with provenance

Monitor progress at: `https://github.com/<owner>/<repo>/actions/workflows/publish.yml`

## What Gets Published

The npm package includes:
- `index.js` - CommonJS entry point
- `index.mjs` - ESM entry point  
- `index.d.ts` - TypeScript definitions
- `htsvcf.node` - Native addon (Linux x64)
- `examples/` - Example scripts

## Troubleshooting

### Build failures
- Check that all Rust tests pass: `cargo test`
- Verify the N-API addon builds: `cargo build -p htsvcf-napi --release`

### Publish failures
- Verify the `NPM_TOKEN` secret is set and valid
- Check npm account has publish permissions
- Ensure the version doesn't already exist on npm

### Version mismatch
If you forget to update version numbers before tagging, the workflow will update `package.json` automatically from the tag. However, keeping versions in sync is recommended for consistency.

## Platform Support

Currently, the automated release builds for:
- Linux x86_64 (ubuntu-latest)
- macOS ARM64 (macos-latest, aarch64-apple-darwin)

The published package includes the Linux binary. For other platforms, users need to build from source.
