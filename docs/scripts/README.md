# Schema Documentation Generation Script

This directory contains the script to automatically generate JSON schema documentation for rabbit-digger-pro configuration files.

## Files

- `generate-schema-docs.js` - Main script that generates the Config Reference documentation page

## How it works

1. **Schema Generation**: The script calls the `rabbit-digger-pro generate-schema` command to get the latest JSON schema
2. **Markdown Conversion**: It converts the JSON schema into a well-formatted markdown documentation page
3. **Type Extraction**: It automatically extracts and lists all supported `net` and `server` types from the schema
4. **Integration**: The generated page is written to `docs/config/reference.md` and automatically included in the documentation site

## Usage

### Manual Generation

```bash
# Generate schema documentation manually
npm run generate-schema-docs
```

### Automatic Generation

The script is automatically run during the documentation build process:

```bash
# Build docs (includes schema generation)
npm run build
```

## Prerequisites

- The `rabbit-digger-pro` binary must be built first: `cargo build` from the project root
- Node.js environment for running the script

## Generated Content

The script generates a comprehensive Config Reference page that includes:

- Overview of configuration structure
- Complete JSON schema in formatted code blocks
- Extracted lists of supported net types (shadowsocks, http, socks5, etc.)
- Extracted lists of supported server types (http, socks5, http+socks5, etc.)
- Usage instructions for JSON schema in editors like VS Code
- Links to related documentation

## Maintenance

The schema documentation is automatically regenerated from the current codebase, so it stays up-to-date with any changes to the configuration schema without manual maintenance.