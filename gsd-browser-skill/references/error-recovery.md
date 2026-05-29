<overview>
Common errors and their fixes. When an error occurs, match it against these patterns before attempting custom debugging.

**MCP agents:** The same root causes apply. In addition, leverage `browser_debug_bundle`, `browser_console`, `browser_network`, the `debug_stuck_agent_flow` prompt, and the rich `suggested_next_actions` + `evidence_refs` returned in every tool envelope. Read `gsd-browser://current-state` or `latest-snapshot` resources when stuck. See docs/AGENT-BEST-PRACTICES.md "Use diagnostic tools when stuck".
</overview>

<error name="stale_refs">

**Error:** `resolve_ref: JS evaluation failed: ref @v1:e3 not found`

**Cause:** Refs become stale after page changes (navigation, form submission, dynamic content).

**Fix:** Re-snapshot and use the new version:

```bash
gsd-browser snapshot
gsd-browser click-ref @v2:e1       # Use fresh version number
```

**MCP:** Call `browser_snapshot` again (or read the `gsd-browser://latest-snapshot` resource) before any `_ref` tool after a page-changing action. The `browser_find_element` tool and action cache also help here.

</error>

<error name="click_timeout">

**Error:** `click timed out after 10s for: #submit-btn`

**Cause:** Element not visible, behind overlay, or not yet in DOM.

**Fixes:**
- `gsd-browser find --selector "#submit-btn"` to verify existence
- `gsd-browser scroll --direction down` to bring into view
- `gsd-browser wait-for --condition selector_visible --value "#submit-btn"`
- `gsd-browser act --intent dismiss` or `accept_cookies` first if a banner is blocking
- Re-snapshot after waits

**MCP equivalents:** `browser_find`, `browser_wait_for`, `browser_act("dismiss")`, `browser_scroll` (if exposed), etc.

</error>

<error name="daemon_not_healthy">

**Error patterns around "daemon did not start", unhealthy sessions, or blank pages.**

**Fix:** 
```bash
gsd-browser --session foo daemon health
gsd-browser --session foo daemon stop
gsd-browser --session foo navigate https://...
```

Use the **exact same** `--session` value for the entire flow. The MCP server supports the `session` argument on virtually every tool for the same reason.

</error>

<error name="vault_key">

**Error:** Vault operations fail or "key not set".

**Fix:** `GSD_BROWSER_VAULT_KEY` must be set in the environment **before the daemon process starts**. If the daemon is already running, stop it first, export the key, then retry (or restart the MCP server process so it inherits the new env).

For MCP clients, put the key in the server's `env` config and restart the client.

</error>

<more_errors>
See the full error recovery section in the root SKILL.md (and the debug-and-diagnose workflow) for additional patterns (empty logs, cookie banners, etc.).

**MCP-specific resilience tools & patterns:**
- `browser_debug_bundle`
- `browser_find_element` (semantic + fallback)
- `browser_action_cache`
- The `debug_stuck_agent_flow` and `full_page_audit` prompts
- `browser_control_state` + viewer takeover when human judgment is needed
- Always follow the `suggested_next_actions` in tool envelopes
</more_errors>
