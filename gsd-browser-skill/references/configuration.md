<overview>
gsd-browser uses a 5-layer configuration merge. Higher layers override lower ones.

**MCP note:** For agents using `gsd-browser mcp`, the most important configuration happens in the **MCP client's server definition** (the `env` block in mcp.json or equivalent). Set `GSD_BROWSER_BROWSER_PATH`, `GSD_BROWSER_VAULT_KEY`, and any other `GSD_BROWSER_*` vars there so the MCP server process (and the daemons it starts) inherit them. Named `session` params on tools provide per-project isolation and persistent cache/state on top of the config layers.
</overview>

<merge_precedence>

1. **Compiled defaults** — sensible values for all settings
2. **User config** — `~/.gsd-browser/config.toml`
3. **Project config** — `./gsd-browser.toml` in project root
4. **Environment variables** — `GSD_BROWSER_*` prefix
5. **CLI flags** — highest priority, override everything

</merge_precedence>

<config_file_format>

```toml
[browser]
path = "/usr/bin/chromium"
headless = true
# cdp_url = "http://localhost:9222"   # attach instead of launching

[daemon]
port = 9222
host = "127.0.0.1"

[screenshot]
quality = 90
format = "png"
full_page = false

[settle]
timeout_ms = 500
poll_ms = 40
quiet_window_ms = 100

[logs]
max_buffer_size = 1000

[artifacts]
dir = "./browser-artifacts"

[timeline]
enabled = true
max_entries = 500

[viewer]
# Additional viewer tuning if needed
```

</config_file_format>

<environment_variables>

Supported variables (use in shell, project .env, or — most importantly for MCP — in your MCP client's `env` block for the gsd-browser server):

- `GSD_BROWSER_BROWSER_PATH`
- `GSD_BROWSER_CDP_URL`
- `GSD_BROWSER_DAEMON_PORT`
- `GSD_BROWSER_VAULT_KEY` (required for auth vault features; set before daemon/MCP server start)
- `GSD_BROWSER_SCREENSHOT_*`, `GSD_BROWSER_SETTLE_*`, etc.

Example for MCP client config:

```json
"env": {
  "GSD_BROWSER_BROWSER_PATH": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "GSD_BROWSER_VAULT_KEY": "strong-secret-for-encrypted-creds"
}
```

</environment_variables>

<named_sessions>

`--session my-project` (or the `session` argument to MCP tools) creates isolated daemon/browser pairs. This is the foundation for:
- Parallel agent workers
- Persistent action cache (self-healing intent mappings survive across runs)
- Per-workspace saved state / vault profiles
- Independent viewer + recording bundles

Strongly recommended for serious MCP agent use (see AGENT-BEST-PRACTICES.md).

</named_sessions>

<see_also>
- Root SKILL.md (Configuration section)
- docs/mcp.md (client config examples + quickstart script)
- docs/AGENT-BEST-PRACTICES.md (MCP-specific env and session advice)
</see_also>
