<required_reading>
**Read these reference files NOW:**
1. references/configuration.md
2. references/command-reference.md (Daemon Management section)
3. (For MCP agents) Root docs/mcp.md and docs/AGENT-BEST-PRACTICES.md + run scripts/mcp-quickstart.sh
</required_reading>

<process>

**Step 1: Install gsd-browser**

```bash
# One-liner (macOS / Linux)
curl -fsSL https://raw.githubusercontent.com/open-gsd/gsd-browser/main/install.sh | bash

# Or from a repo checkout
git clone https://github.com/open-gsd/gsd-browser.git
cd gsd-browser
cargo install --path cli

# Verify
gsd-browser daemon start
gsd-browser daemon health
gsd-browser daemon stop
```

The installer downloads the binary and reuses a system Chrome/Chromium when present. Otherwise it downloads Chromium automatically when Chrome for Testing is available for the platform.
Run `gsd-browser update` to install the current release.

**MCP agents:** The installer header and post-install message explicitly document the `gsd-browser mcp` path and quickstart helper.

**Step 2: Configure browser path (if needed)**

If Chrome/Chromium is not in the default location:

```bash
# Via config file
mkdir -p ~/.gsd-browser
cat > ~/.gsd-browser/config.toml << 'TOML'
[browser]
path = "/path/to/chrome"
TOML

# Or via environment variable
export GSD_BROWSER_BROWSER_PATH="/path/to/chrome"

# Or via CLI flag (per-command)
gsd-browser --browser-path "/path/to/chrome" navigate https://example.com
```

**For MCP clients:** Set `GSD_BROWSER_BROWSER_PATH` (and especially `GSD_BROWSER_VAULT_KEY`) in the `env` block of your mcpServer definition in the client's config (Cursor mcp.json, Claude Desktop, etc.). The daemon will inherit them when the MCP server starts it.

**Step 3: Project-level configuration**

Create `gsd-browser.toml` in your project root when a project needs browser settings that should travel with the checkout:

```toml
[browser]
path = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
headless = false

[sessions]
default = "project"
```

**Step 4: Set up the encrypted auth vault**

The vault key must be set **before the daemon starts**:

```bash
export GSD_BROWSER_VAULT_KEY="your-encryption-key"
gsd-browser daemon stop    # Stop existing daemon if running
gsd-browser vault-save --profile github \
  --url https://github.com/login \
  --username user --password "secret"
```

**MCP note:** Configure `GSD_BROWSER_VAULT_KEY` in the MCP client's env for the gsd-browser server entry. Then use `browser_vault_login` (and `browser_vault_save` / `browser_vault_list`) from your agent.

**Step 5: Parallel sessions**

Run multiple independent browser instances (named sessions are also the foundation for persistent MCP action cache + state):

```bash
gsd-browser --session site1 navigate https://site-a.com
gsd-browser --session site2 navigate https://site-b.com

# Each session has its own daemon, socket, and Chrome instance
gsd-browser --session site1 snapshot
gsd-browser --session site2 snapshot

# Clean up both
gsd-browser --session site1 daemon stop
gsd-browser --session site2 daemon stop
```

**MCP agents:** Pass `session` to virtually every `browser_*` tool for isolation + cache reuse. This is strongly recommended in AGENT-BEST-PRACTICES.md.

**Step 6: Daemon management**

The daemon auto-starts on browser commands. `daemon health` reports state and does not start a session. Manual management is rarely needed:

```bash
gsd-browser daemon health     # Check status of the current session
gsd-browser daemon stop       # Stop daemon and Chrome
gsd-browser daemon start      # Explicit start (rarely needed)
```

**Step 7: Live viewer setup**

Open the narrated shared-screen viewer for a session (one of the most powerful MCP collaboration features):

```bash
gsd-browser --session demo navigate https://example.com
gsd-browser --session demo view
```

Use `--print-only` when another tool or person will open the URL.

The viewer runs on localhost and attaches to the session daemon. It shows live browser frames, action history, refs overlay, and pause/step/resume/abort + annotate + record controls.

**MCP tools:** `browser_view`, `browser_takeover`, `browser_annotation_request`, `browser_record_start/stop`, `browser_goal`, `browser_step`, `browser_control_state`, etc. See the live-viewer workflow and best-practices guide for human-in-the-loop and evidence patterns.

**Step 8: CI/CD usage**

For CI pipelines, ensure headless mode and configure paths...

**Step 9: MCP client setup (primary for agents)**

1. Install gsd-browser (via installer or cargo).
2. Run the quickstart helper for your client:
   ```bash
   ./scripts/mcp-quickstart.sh cursor   # or claude / vscode / generic
   ```
3. Add the provided JSON snippet to your MCP settings (mcp.json, Claude Desktop config, etc.), filling in `GSD_BROWSER_BROWSER_PATH` and `GSD_BROWSER_VAULT_KEY` as needed.
4. Start/restart your MCP client.
5. In the agent chat: ask it to call tools from gsd-browser, read resources like `gsd-browser://latest-snapshot`, or invoke prompts like `autonomous_research_task`.

See root `docs/mcp.md`, `docs/examples/mcp-client-config.json`, and `docs/AGENT-BEST-PRACTICES.md` for details and high-value patterns.

**Step 10: Install/refresh the gsd-browser-skill pack (for coding agents)**

The main installer offers to copy the curated `gsd-browser-skill/` files into Claude / Codex / Gemini skill directories (global or per-project). Re-run the installer with `--skip-chromium` or use `gsd-browser skill install` (if exposed) to refresh.

</process>

<verification>
After setup:
- `gsd-browser daemon health` reports healthy (or the MCP server starts the daemon on first tool call).
- For MCP: your client shows gsd-browser tools/resources/prompts after restart.
- Named sessions + vault work as expected.
- Viewer opens and can control/annotate/record.
</verification>
