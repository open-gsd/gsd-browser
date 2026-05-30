# AGENTS.md

## gsd-browser

Native Rust browser automation CLI for AI agents. Controls Chrome/Chromium via CDP with a persistent background daemon. Supports navigation, element interaction (click, type, fill), snapshots with versioned refs, assertions, form analysis, structured data extraction, visual regression testing, network mocking, device emulation, auth vault, prompt injection scanning, live viewer collaboration, recording/evidence bundles, and self-healing via action cache.

**Primary path for AI agents (2026+):** `gsd-browser mcp` — the stdio MCP server exposing 50+ tools, live resources, and executable prompts. All the unique strengths (refs, viewer + human handoff + annotations + recordings, semantic intents, evidence, batch, self-healing, etc.) become automatically available to Cursor, Claude Desktop, VS Code + Copilot, and other MCP clients.

### Quick Start (CLI)

```bash
gsd-browser navigate https://example.com
gsd-browser snapshot
gsd-browser click-ref @v1:e1
```

### Quick Start (MCP — Recommended for Agents)

```bash
gsd-browser mcp
```

Then point your MCP client at the command. Run `./scripts/mcp-quickstart.sh cursor` (or claude/vscode) for tailored config snippets.

See:
- `docs/mcp.md`
- `docs/AGENT-BEST-PRACTICES.md` (essential — golden rules, workflows, "When to Use What")
- Root `SKILL.md` (full semantics backing every MCP tool)
- `gsd-browser-skill/` (curated pack for coding agents)

### Installation

```bash
curl -fsSL https://raw.githubusercontent.com/open-gsd/gsd-browser/main/install.sh | bash

# install the Codex Plugin integration too
curl -fsSL https://raw.githubusercontent.com/open-gsd/gsd-browser/main/install.sh | bash -s -- --codex-plugin

# or from a repo checkout
cargo install --path cli
```

Requires Chrome or Chromium. The installer reuses a system browser when present and downloads Chromium automatically where Chrome for Testing is available. It also offers to install the `gsd-browser-skill/` pack.

### Full Reference

See [SKILL.md](./SKILL.md) for complete command reference (90+ CLI commands), workflow patterns, configuration, and examples.

The MCP tools (`browser_navigate`, `browser_snapshot`, `browser_act`, `browser_view`, `browser_record_start`, `browser_batch`, `browser_action_cache`, etc.) are a direct, high-fidelity mapping of this surface, augmented with resources, prompts, and agent-optimized response envelopes.
