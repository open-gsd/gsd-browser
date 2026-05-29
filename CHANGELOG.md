# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Agent Testing

Comprehensive end-to-end testing by an autonomous agent (Claude Sonnet 4.6 via OpenClaw) across two full rounds surfaced multiple real bugs and UX issues in the MCP + daemon surface. All high-severity issues from the test report have been fixed and independently re-verified.

### Fixed

- **Multi-tab tracking (Bug #1 from agent testing)**: JS-opened tabs (`window.open()`, `target="_blank"`, external clients) were invisible to `list-pages` / `switch-page` even though Chrome/CDP saw them. Root cause: daemon only registered the initial page at launch and never subscribed to `Target.targetCreated` (or related lifecycle events).  
  Fix: Added always-on `spawn_core_target_tracker` (wired at daemon startup) that subscribes to `targetCreated` / `targetDestroyed` / `targetInfoChanged`, performs initial `GetTargets` enumeration (helpful for `--cdp-url` attach scenarios), auto-registers new pages with helpers injected, auto-activates newly discovered tabs, cleans up on external close, and keeps metadata fresh. `list-pages` now sees the full live set of page targets. (See `cli/src/daemon/handlers/pages.rs:240` and `cli/src/daemon/mod.rs:580`.)

- **Switch-page / list-pages consistency (Bug #1 verification polish)**: Immediately after `switch-page`, a subsequent `list-pages` could briefly show stale title/url (and appear to have the wrong active entry) because metadata update happened after async helper injection. Reordered the fresh url/title read + registry `update_metadata` to occur synchronously right after `set_active` (before the heavier inject work). The very next `list-pages` now reflects the correct active page + current url/title.

- **Critical metadata corruption race (final root cause of B1)**: Even after the above, when a new tab was opened via `window.open()` (triggering the core target tracker), the async `sync_session_manifest` call scheduled from the `targetCreated` handler could run *after* a subsequent `switch_page`, read the new `active_page_id`, probe the page object it was originally given (the new tab), and blindly write that tab's title/url into the *currently active* registry entry. This corrupted `list-pages` for all tabs. Root cause: `sync_session_manifest` used the live `current_active_page()` id for the registry write-back instead of resolving the entry belonging to the concrete `&Page` it was passed. Fixed by making `sync_session_manifest` resolve the target registry entry via the passed page's `target_id` (Option C). The manifest's "active page" concept still reflects the session's declared active tab. This protects the tracker, the viewer target follower, and all other call sites. (See `cli/src/daemon/handlers/session.rs:258` and the call site in `pages.rs:396`.)

- **switch-page / close-page --id UX (Bug #1 verification polish)**: Only positional `<ID>` was accepted. Added `--id <N>` support (alongside the positional form) for both commands. Old `--no-clear`-style confusion and bad "did you mean --identity-key" suggestions are resolved for these commands. Help text updated.

- **Log buffer draining (Bug #2 from agent testing)**: `gsd-browser network` (and `console`/`dialog`) defaulted to `clear:true`, silently calling `drain()` on the shared buffer. Any subsequent `har-export` (which uses `snapshot()`) would see zero entries and fail with an unhelpful error, even though the data had just been visible. Same anti-pattern affected console logs.
  Fix: Flipped the default for all three (`clear` now defaults to `false` = snapshot/preserve). Added explicit `--clear` flag (and kept hidden `--no-clear` compat alias during transition). Updated CLI commands, daemon handlers, and the full MCP surface (`browser_network`, `browser_console`; descriptions now call out the safe snapshot default for har-export / replay workflows). `har-export` now works reliably after a `network` inspection call. (See `cli/src/daemon/handlers/inspect.rs`, `main.rs`, and `mcp.rs`.)

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