# Architecture Refactor Plan (Clean/Hex, Long-term)

Date: 2026-01-25

This document turns the proposed Clean/Hex refactor into a commit-friendly checklist. Each checkbox is intended to become a small, reviewable PR/commit.

## 0) Goals

- Keep dependencies **one-way**: Domain → Core → Adapters → App (composition only)
- Make plugin registration a **single composition point** (bundle crate)
- Remove “app contains adapters” leakage (e.g. `select` net currently lives in the app crate)
- Maintain behavior + keep the project building between steps

## 1) Target Layers and Responsibilities

- **Domain** (`rd-interface`, `rd-derive`): traits/types/errors/config derive; no concrete implementations.
- **Core** (`rabbit-digger`): runtime/use-cases/orchestration; depends only on Domain.
- **Adapters** (`rd-std`, `protocol/*`): concrete implementations (nets/servers); depend on Domain (and optionally minimal shared libs).
- **App** (`rabbit-digger-pro`): CLI/API/config/storage/schema/telemetry; wires Core + Adapters via a bundle.

## 2) Dependency Rules (must-haves)

- [ ] Domain crates (`rd-interface`, `rd-derive`) must not depend on Core, Adapters, or App crates.
- [ ] Core crate (`rabbit-digger`) must not depend on App crates; avoid depending on Adapters long-term.
- [ ] Adapters (`rd-std`, `protocol/*`) must not depend on Core or App crates.
- [ ] App (`rabbit-digger-pro`) may depend on Core and a _bundle_ crate; avoid direct dependencies on individual protocol crates.

## 3) Commit Plan (step-by-step)

### Milestone A — Stop leaking adapters into App (low risk)

- [ ] Move the `select` net implementation from the app crate to adapters (prefer `rd-std`).
  - Acceptance: `select` still appears in registry schema; tests compile.
- [ ] Remove `select` registration from the app’s registry wiring.

### Milestone B — Introduce a single composition point (`rdp-bundle`) (medium risk, high payoff)

- [ ] Create a new workspace crate `rdp-bundle`.
  - Purpose: build a `rabbit_digger::Registry` by registering optional plugins.
- [ ] Move protocol plugin registration (`ss`, `trojan`, `rpc`, `raw`, `obfs`) into `rdp-bundle` behind features.
- [ ] Make `rabbit-digger-pro` depend on `rdp-bundle` and call `rdp_bundle::build_registry()`.
  - Acceptance: app no longer imports protocol crates directly.
- [ ] Re-map existing features in `rabbit-digger-pro` to enable corresponding features in `rdp-bundle`.
  - Acceptance: `cargo build --features ss` etc still works.

### Milestone C — Reduce Core ↔ Adapters coupling (optional, later)

- [ ] Audit `rabbit-digger` builtin loading (`Registry::new_with_builtin`) and plan to move builtin implementations into adapters.
- [ ] Introduce a “core-only registry” constructor (no builtin implementations) and let `rdp-bundle` assemble everything.
  - Acceptance: core can be built without `rd-std`.

### Milestone D — Optional App split into infra crates (only if needed)

- [ ] Extract `rdp-storage` from `src/storage/*`.
- [ ] Extract `rdp-config` from `src/config*`.
- [ ] Extract `rdp-api` from `src/api_server/*`.
- [ ] Keep root bin crate thin: CLI + composition.

## 4) Guardrails (recommended)

- [ ] Add CI checks for dependency direction (e.g. `cargo tree -i` patterns or `cargo deny`).
- [ ] Add a short “layer rules” section to README.
- [ ] Prefer `pub(crate)` for app-internal modules; keep Domain/Core public APIs minimal.

## 5) Execution Order (what I will do next)

1. Milestone A: move `select` into `rd-std` and fix registration.
2. Milestone B: add `rdp-bundle`, migrate registration there, and update app features.
3. Run `cargo test` to ensure everything still compiles.
