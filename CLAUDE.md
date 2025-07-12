# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Rabbit Digger Pro is an all-in-one proxy server written in Rust with hot-reloading capabilities. It supports multiple proxy protocols (Shadowsocks, Trojan, HTTP, Socks5, obfs) and provides both CLI and web API interfaces.

## Architecture

The codebase follows a modular workspace structure:

- **Core**: `rabbit-digger/` - Main proxy engine with hot-reload and configuration management
- **Interface**: `rd-interface/` - Plugin interface definitions and core types
- **Standard Library**: `rd-std/` - Built-in networking utilities, servers, and rules
- **Protocols**: Individual protocol implementations in `protocol/`:
  - `ss/` - Shadowsocks client/server
  - `trojan/` - Trojan protocol implementation
  - `rpc/` - Internal RPC protocol
  - `raw/` - Raw packet forwarding
  - `obfs/` - Obfuscation protocols
- **Frontend**: `ui/` - React-based web interface
- **Documentation**: `docs/` - Docusaurus-based documentation

## Key Components

- **RabbitDigger**: Core proxy engine in `rabbit-digger/src/rabbit_digger/`
- **Registry**: Plugin system for dynamic protocol loading
- **Configuration**: YAML-based config with JSON Schema validation
- **API Server**: RESTful API with WebSocket support for real-time updates
- **Hot Reloading**: Config changes applied without restart

## Common Commands

### Rust Development
```bash
# Build the project
cargo build --release

# Run tests
cargo test

# Run with default config
cargo run --bin rabbit-digger-pro

# Run with custom config
cargo run --bin rabbit-digger-pro -- -c config.yaml

# Generate JSON schema
cargo run --bin rabbit-digger-pro -- generate-schema schema.json

# Run API server only
cargo run --bin rabbit-digger-pro -- server

# Development with console
cargo run --bin rabbit-digger-pro --features console
```

### Frontend Development
```bash
# UI development
cd ui/
pnpm install
pnpm dev

# Build UI
cd ui/
pnpm build

# Documentation development
cd docs/
pnpm install
pnpm start
```

### Testing & Quality
```bash
# Run all tests
cargo test --workspace

# Check formatting
cargo fmt --check

# Run clippy
cargo clippy --workspace

# Generate coverage
./gen_coverage.sh
```

## Configuration

- Main config: `config.yaml` (default) or specified with `-c`
- Environment variables:
  - `RD_CONFIG`: Config file path
  - `RD_BIND`: API server bind address
  - `RD_ACCESS_TOKEN`: API access token
  - `RD_WEB_UI`: Web UI folder path
  - `RUST_LOG`: Logging level

## Development Features

- **Hot reload**: Config changes automatically applied
- **JSON Schema**: Auto-generated schema for config validation
- **Plugin system**: Extensible via registry system
- **Web UI**: React frontend with real-time updates via WebSocket
- **Telemetry**: Optional OpenTelemetry/Jaeger integration
- **Console**: Optional console-subscriber for debugging