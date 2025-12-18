# Documentation Workflow

The `docs.yml` workflow automatically builds and publishes Rust API documentation for all workspace crates to GitHub Pages.

## Triggers

- **Push to `main`**: Updates the `latest` documentation
- **Push a tag `v*`**: Adds documentation for that version
- **Manual**: Can be triggered via workflow_dispatch

## URL Structure

Assuming the repository is `https://github.com/OWNER/htsvcf`, the docs are available at:

| URL | Description |
|-----|-------------|
| `https://OWNER.github.io/htsvcf/` | Redirects to latest |
| `https://OWNER.github.io/htsvcf/latest/htsvcf/` | Latest docs (main branch) |
| `https://OWNER.github.io/htsvcf/v0.1.0/htsvcf/` | Docs for tag v0.1.0 |
| `https://OWNER.github.io/htsvcf/versions.html` | Index of all versions |

### Crate-specific URLs

Each version directory contains docs for all workspace crates:

- `.../latest/htsvcf/` - Main crate (v8 binding)
- `.../latest/htsvcf_core/` - Core library
- `.../latest/htsvcf_napi/` - Node-API binding

Replace `latest` with a version tag (e.g., `v0.1.0`) for versioned docs.

## How It Works

On every run, the workflow:

1. Checks out the repository with full history
2. Builds docs for every existing `v*` tag
3. Builds docs for the current commit as `latest`
4. Generates a `versions.html` index page
5. Deploys everything to GitHub Pages

This approach rebuilds all versions each time, which ensures reliability without managing cached state.

## Setup Requirements

To enable this workflow, configure GitHub Pages in repository settings:

1. Go to **Settings > Pages**
2. Set **Source** to **GitHub Actions**
