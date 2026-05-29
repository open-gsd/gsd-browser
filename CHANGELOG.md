# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **MCP Server** (`gsd-browser mcp`): Full stdio MCP server exposing 50+ tools, resources, and executable prompts for AI agents.
  - Rich response envelopes with `summary`, `structured_data`, `suggested_next_actions`, and `evidence_refs`.
  - First-class support for versioned refs, live viewer collaboration (takeover, annotations, recordings), semantic intents, batch execution, action cache for self-healing, and more.
  - See [docs/mcp.md](docs/mcp.md) and [docs/AGENT-BEST-PRACTICES.md](docs/AGENT-BEST-PRACTICES.md).

- **Stealth & Alternative Backends**:
  - New `--stealth` flag and `--backend` option for undetectable automation.
  - Optional integration with `chromey`, `chaser-oxide`, and `ferrous-browser` (feature-gated).
  - Anti-detection patches, realistic fingerprints, and human-like input when enabled.

- **BiDi Spike**: Experimental support for WebDriver BiDi via `rustenium` behind the `bidi` feature flag (see `docs/bidi-spike.md`).

- New scripts and examples:
  - `scripts/mcp-quickstart.sh` — client-specific setup helper (Cursor, Claude Desktop, VS Code, etc.).
  - `scripts/test-mcp.py` — end-to-end MCP smoke test.
  - Example client config in `docs/examples/mcp-client-config.json`.

- Extensive documentation updates across `README.md`, `SKILL.md`, the `gsd-browser-skill/` package, and new dedicated guides.

### Changed

- Named session behavior is now stricter (explicit `daemon stop` required for unhealthy sessions) for improved safety and auditability.
- Documentation now positions the MCP server as the primary path for AI agents while preserving full CLI depth.

### Fixed

- Various robustness and cleanup improvements for macOS daemon lifecycle (documented in audit notes).

## [0.1.24] - Previous

Initial public state before the major MCP + stealth expansion work.