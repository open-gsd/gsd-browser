---
name: gsd-browser
description: >
  Native Rust browser automation CLI for AI agents. Use when the user needs to
  interact with websites, navigate pages, fill forms, click buttons, take
  screenshots, share a live browser view, narrate browser actions, extract
  structured data, run assertions, test web apps, mock network requests, or
  automate any browser task. Triggers include "open a website", "fill out a
  form", "take a screenshot", "show me the browser", "share the screen",
  "pause the browser", "step through this", "scrape data", "test this web app",
  "login to a site", "visual regression test", or any task requiring
  programmatic web interaction.
allowed-tools: Bash(gsd-browser:*), Bash(gsd-browser *)
---

<essential_principles>

**Primary path for AI agents (2026+):** Use `gsd-browser mcp` (the stdio MCP server). It exposes 50+ tools, live resources (latest-snapshot, current-state, active-recordings, timeline, etc.), and executable prompts (robust_login_flow, full_page_audit, autonomous_research_task, evidence_creation_workflow, debug_stuck_agent_flow, etc.) with rich agent-optimized envelopes (summary + structured_data + suggested_next_actions + evidence_refs).

**For MCP clients** (Cursor, Claude Desktop, VS Code + Copilot, etc.): After connecting to `gsd-browser mcp`, read the root `docs/AGENT-BEST-PRACTICES.md` (golden rules, workflows, "When to Use What" table) and `docs/mcp.md`. Run `./scripts/mcp-quickstart.sh cursor` (or claude/vscode) for tailored config.

**The daemon auto-starts on browser commands.** `gsd-browser daemon health` reports state and does not start a session. Use `gsd-browser daemon start` only when you want to pre-warm or verify daemon lifecycle explicitly.

**Always re-snapshot after page changes.** Refs are versioned (`@v1:e1`, `@v2:e3`). After navigation, form submission, or dynamic content loading, old refs are stale. Run `gsd-browser snapshot` (or read the `gsd-browser://latest-snapshot` MCP resource) to get fresh refs before interacting with `_ref` tools.

**Use `--json` when parsing output.** Use text mode when reading output yourself. Use `--json` when you need to extract values programmatically. MCP tools return rich structured envelopes.

**Positional args have no flag prefix.** Commands like `click`, `type`, `hover` take positional args. Do NOT add `--selector`:
- `gsd-browser click "button.submit"` (correct)
- `gsd-browser click --selector "button.submit"` (WRONG)

**Core workflow pattern:** Every browser automation follows: navigate -> snapshot (or read latest-snapshot resource) -> interact (prefer semantic `act`/`find_best` first, then refs) -> re-snapshot (after DOM changes). For complex flows, prefer `browser_batch` via MCP.

```bash
gsd-browser navigate https://example.com
gsd-browser snapshot
# Read snapshot output: @v1:e1 [input type="email"], @v1:e2 [button] "Submit"
gsd-browser fill-ref @v1:e1 "user@example.com"
gsd-browser click-ref @v1:e2
gsd-browser wait-for --condition network_idle
gsd-browser snapshot  # REQUIRED - old refs are now stale
```

**Command chaining:** Use `&&` when you don't need intermediate output. Run separately when you need to parse output first (e.g., snapshot to discover refs, then interact). MCP agents: use `browser_batch` for atomic multi-step execution.

**Use the live viewer when the user wants to watch or direct the browser.** `gsd-browser view` opens a localhost viewer with live frames, narrated action history, ref overlays, and pause/step/resume/abort controls + annotation + record capabilities. Keep using CLI/MCP commands for actions; the viewer is the shared screen and control surface. MCP tools: `browser_view`, `browser_takeover`, `browser_annotation_request`, `browser_record_start/stop`, `browser_goal`, `browser_step`, etc. These are among the highest-leverage collaborative features.

**Global options** available on all commands:

| Flag | Purpose |
|------|---------|
| `--json` | Structured JSON output |
| `--browser-path <path>` | Path to Chrome/Chromium |
| `--cdp-url <url>` | Attach to an already-running Chrome instance |
| `--session <name>` | Named session for parallel instances + persistent action cache / state (highly recommended for MCP agents) |
| `--no-narration-delay` | Skip narration lead-time sleeps while keeping history/events |

</essential_principles>

<routing>

Based on what the user needs, read the appropriate workflow:

| User intent | Workflow |
|-------------|----------|
| Navigate, click, type, fill forms, interact with pages (MCP or CLI) | `workflows/navigate-and-interact.md` |
| Share the browser screen, narrate actions, pause/step/resume/abort, human collaboration + evidence | `workflows/live-viewer-and-narration.md` |
| Scrape data, extract content, read page structure | `workflows/scrape-and-extract.md` |
| Test pages, run assertions, visual regression, mock network | `workflows/test-and-assert.md` |
| Debug issues, check logs, diagnose problems (use debug_bundle + MCP prompts) | `workflows/debug-and-diagnose.md` |
| Install, configure, set up sessions, MCP client setup | `workflows/setup-and-configure.md` |

**MCP-first agents:** Start with the root `docs/AGENT-BEST-PRACTICES.md` and `docs/mcp.md` (plus the quickstart script) before diving into these CLI-oriented workflows. The workflows here remain useful for direct CLI scripting or when the agent needs the exact underlying command semantics.

**After reading the workflow, follow it. Load references only when the workflow directs you to.**

</routing>

<reference_index>

All domain knowledge in `references/`:

**Commands:** command-reference.md (complete command syntax; MCP tools are a direct mapping — see `browser_*` names in the MCP server)
**Snapshots:** snapshot-and-refs.md (versioned refs, snapshot modes — critical for reliable MCP ref tools)
**Intents:** semantic-intents.md (15 predefined intents for find-best/act — prefer these first in MCP flows)
**Errors:** error-recovery.md (common errors and fixes — applies to both CLI and MCP)
**Config:** configuration.md (TOML config, env vars, 5-layer merge — especially relevant for MCP client env blocks)

**MCP-specific (read from repo root):** docs/mcp.md, docs/AGENT-BEST-PRACTICES.md, scripts/mcp-quickstart.sh, and the response envelope + prompt/resource patterns documented there.

</reference_index>

<workflows_index>

| Workflow | Purpose |
|----------|---------|
| navigate-and-interact.md | Page navigation, clicking, typing, forms, intents (MCP: browser_navigate, browser_act, browser_snapshot + refs, browser_fill_form, etc.) |
| live-viewer-and-narration.md | Live shared viewer, narrated history, refs overlay, controls, annotations, recordings (highest-leverage MCP collaboration surface) |
| scrape-and-extract.md | Data extraction, accessibility tree, page source |
| test-and-assert.md | Assertions, visual regression, network mocking, test generation |
| debug-and-diagnose.md | Console/network logs, timeline, debug bundles (pair with MCP `debug_stuck_agent_flow` prompt) |
| setup-and-configure.md | Installation, configuration, sessions, daemon management, MCP client setup |

</workflows_index>

<success_criteria>

Browser automation task is complete when:
- Target page state is achieved and verified (via assertions or visual confirmation)
- Daemon is stopped if no further browser work is needed (`gsd-browser daemon stop`)
- Extracted data is returned in the expected format
- Any saved state (auth, cookies) is persisted for reuse if appropriate
- For evidence/audit flows: recordings exported + annotations captured where relevant

For MCP agents, also ensure the rich response envelopes and suggested_next_actions were leveraged, and human collaboration via the viewer was used when judgment was required.

</success_criteria>

<mcp_highlights>
gsd-browser via MCP is now a first-class, extremely powerful browser platform for agents. Key differentiators available as MCP tools/resources/prompts:
- Versioned refs + snapshot modes for deterministic interaction
- Semantic intents (`browser_act`) + resilient finders + action cache for self-healing
- Full live viewer collaboration (takeover, annotations, recordings, goal/step/abort)
- First-class evidence bundles and visual regression
- `browser_batch` for atomic complex flows
- Named sessions for isolation + persistent learning
- Rich envelopes with concrete next-action hints on every call

See the main repo docs/mcp.md + AGENT-BEST-PRACTICES.md (and run the quickstart script) for the complete modern agent experience. The content in this skill pack provides the deep CLI semantics that back every MCP tool.
</mcp_highlights>
