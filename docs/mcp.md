# MCP Server for gsd-browser

**gsd-browser mcp** is the most powerful browser automation surface available for AI agents via the Model Context Protocol.

It exposes gsd-browser’s unique strengths — versioned refs, live authenticated viewer with human takeover + annotations + recordings, semantic intents, encrypted vault, visual regression, evidence bundles, self-healing action cache, batch execution, and strong observability — as first-class MCP tools, resources, and prompts.

This is not "just another browser tool". It is designed for serious, auditable, human-collaborative agentic web work at scale.

## Current Capabilities (Massively Expanded)

- **50+ tools** covering: core navigation & state, snapshot & versioned refs (core differentiator), precise ref-based interaction, semantic/intent-based actions (`browser_act`, `browser_act_instruction`, and `find_best`), advanced forms, assertions & robust waits, visual & evidence (screenshots, visual-diff, PDF), live viewer + full human collaboration (takeover, goal, step/abort, sensitive), state/auth/vault persistence, rich diagnostics (debug_bundle, console, network, timeline), structured extraction, injection scanning, full recording & evidence bundles, annotations, network mocking/blocking, device emulation, action cache for self-healing, batch/diff for complex flows, multi-page/tab & frame management, and more.
- **Resources** (gsd-browser://current-state via debug bundle, latest-snapshot [real data + refs], current-refs, active-recordings, timeline) — wired to query the live daemon.
- **Prompts** (robust_login_flow, full_page_audit, create_evidence_bundle, autonomous_research_task, evidence_creation_workflow, debug_stuck_agent_flow) — rich executable multi-step workflows with best-practice guidance built in.
- Powerful standardized response envelopes on every tool call: `summary`, `structured_data`, `suggested_next_actions` (high-signal hints), `evidence_refs`, plus raw fallback.
- Full reuse of the battle-tested daemon client: auto-start, named sessions, error handling, JSON fidelity.

See the full current surface by connecting your MCP client to `gsd-browser mcp` and calling `tools/list`, `resources/list`, `prompts/list`.

## Why This Matters

gsd-browser already has one of the richest browser automation surfaces available for agents:
- Versioned refs (`@v1:e1`) for reliable, deterministic interaction
- Semantic intents via `browser_act`, plus generic natural-language page planning via `browser_act_instruction`
- Live authenticated viewer with human takeover, annotations, recordings, and step-through control
- First-class evidence & audit (recordings, visual regression, traces, HAR, debug bundles)
- Self-healing via action cache + find_element resilience patterns
- Assertions (including new `title_contains`), batch (accepts both native `{action, ...}` and legacy `{tool, params}` formats — auto-normalized), forms, vault, network control, safety scanning — all in one cohesive tool

Exposing it via MCP (stdio) makes the entire surface automatically discoverable and usable by any MCP-compatible agent (Cursor, Claude Desktop, VS Code + Copilot, Windsurf, etc.).

## Quickstart (Recommended)

```bash
# Run the MCP server (JSON-RPC over stdio — most clients manage the process)
gsd-browser mcp
```

## Public / Remote Hosting

For hosted clients, run the same MCP surface over Streamable HTTP:

```bash
export GSD_BROWSER_MCP_AUTH_TOKEN="$(openssl rand -hex 32)"
gsd-browser mcp --http --host 0.0.0.0 --port 8788
```

Expose the service over HTTPS with your proxy or platform, then point remote MCP clients at:

```text
https://your-domain.example/mcp
Authorization: Bearer <GSD_BROWSER_MCP_AUTH_TOKEN>
```

For OpenGSD-hosted tokens, point the HTTP server at the console verifier:

```bash
export GSD_BROWSER_MCP_AUTH_VERIFY_URL="https://mcp.opengsd.dev/api/mcp/tokens/verify"
gsd-browser mcp --http --host 0.0.0.0 --port 8788
```

The HTTP server calls the verifier for each request. `tools/call` requests are
recorded as billable MCP usage with `runtime: "gsd-browser"` and the MCP tool
name; metadata/list/read requests are authenticated without incrementing quota.
If the console returns `429`, the MCP server forwards the throttle response.

Local development can bind without auth on loopback:

```bash
gsd-browser mcp --http --host 127.0.0.1 --port 8788
```

Non-loopback hosts refuse unauthenticated startup by default. To override intentionally, pass `--no-auth`.

For tailored setup instructions and copy-paste config snippets for your client:

```bash
./scripts/mcp-quickstart.sh cursor     # or: claude, vscode, generic
```

### Example Client Configuration

See `docs/examples/mcp-client-config.json` and the output of the quickstart script.

**Cursor / VS Code + Copilot** (add to mcp.json or settings):

```json
{
  "mcpServers": {
    "gsd-browser": {
      "command": "gsd-browser",
      "args": ["mcp"],
      "env": {
        "GSD_BROWSER_BROWSER_PATH": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "GSD_BROWSER_VAULT_KEY": "your-strong-key-here"
      }
    }
  }
}
```

**Claude Desktop**:

```json
{
  "mcpServers": {
    "gsd-browser": {
      "command": "gsd-browser",
      "args": ["mcp"]
    }
  }
}
```

Pass `--session my-project` (via args or env) for isolated browser instances + persistent intent cache/state per workspace.

**Pro tips:**
- Set `GSD_BROWSER_VAULT_KEY` (and browser path) in the MCP client's env config **before** first daemon start.
- Use named sessions per project for isolation + self-healing cache reuse.
- After connecting, ask your agent to explore `tools/list`, read resources like `gsd-browser://latest-snapshot`, and try the built-in prompts.

## How It Works

The MCP server is a thin, high-fidelity adapter:
1. Implements stdio MCP transport (initialize, tools/list + call, resources/list + read, prompts/list + get).
2. On `tools/call` it translates directly to gsd-browser's internal daemon JSON-RPC using the exact same `daemon_client` used by the CLI.
3. You get automatic daemon lifecycle, session management, robust error formatting, and all prior CLI reliability work for free.
4. Response envelopes add agent-optimized structure (`suggested_next_actions`, evidence pointers) on top of raw results.

This is why the surface was able to expand rapidly to 50+ tools + resources + prompts.

## Development / Testing

```bash
# Build
cargo build -p gsd-browser

# Manual protocol smoke
printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}\n' | ./target/debug/gsd-browser mcp

# End-to-end with real browser
python3 scripts/test-mcp.py
```

## Packaging & Production Use

- Install: `cargo install --path cli` or GitHub release binaries.
- Configure env vars (VAULT_KEY, BROWSER_PATH) in your MCP client definition.
- Use `--session` / named sessions for isolation and persistent cache/state.
- The `./scripts/mcp-quickstart.sh` helper gives client-specific guidance.

This makes `gsd-browser mcp` a drop-in, extremely powerful browser platform for any serious MCP-capable agent environment.

## Current Status

The MCP layer is production-ready for agent use and has been massively expanded beyond the initial prototype:
- Broad tool coverage of the rich daemon surface (navigation, refs, semantic actions, viewer/collaboration, recordings/evidence, diagnostics, batch (supports both {action, ...} and legacy {tool, params} formats), self-healing, etc.). `tools/list` is the source of truth for the currently wired MCP contract.
- Live resources and executable prompts.
- Rich envelopes + best-practice guidance in responses.

See [docs/AGENT-BEST-PRACTICES.md](./AGENT-BEST-PRACTICES.md) for high-value agent patterns, golden rules, recommended workflows (login, audit, human-in-loop, evidence, self-healing), the "When to Use What" table, and response envelope usage.

The underlying full CLI command surface (and exact semantics) lives in the root [SKILL.md](../SKILL.md) and the `gsd-browser-skill/` curated agent skill pack.

## See Also

- [docs/AGENT-BEST-PRACTICES.md](./AGENT-BEST-PRACTICES.md) — The primary guide for agents using gsd-browser MCP (strongly recommended).
- [scripts/mcp-quickstart.sh](../scripts/mcp-quickstart.sh) — One-command tailored setup for Cursor/Claude/VS Code/etc.
- Root [SKILL.md](../SKILL.md) — Complete CLI command reference, workflow patterns, error recovery, and examples (MCP tools are a direct 1:1 mapping of this surface).
- `gsd-browser --help` and per-command `--help`.
- `docs/examples/mcp-client-config.json` for a ready-to-use example.
