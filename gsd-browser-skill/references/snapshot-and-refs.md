<overview>
Snapshots assign versioned refs to interactive page elements. Refs are the primary mechanism for deterministic element interaction — they eliminate fragile CSS selectors by giving each element a stable, versioned identifier.

**MCP Critical:** `browser_snapshot` (and the `gsd-browser://latest-snapshot` resource) + `_ref` tools (`browser_click_ref`, `browser_fill_ref`, `browser_get_ref`, `browser_hover_ref`) are core to reliable agent flows. The MCP best-practices guide and all built-in prompts stress "snapshot early, snapshot often". After navigation or major DOM changes, always re-snapshot (or re-read the latest-snapshot resource) before using any ref-based tool.
</overview>

<how_refs_work>

Running `gsd-browser snapshot` scans the page and assigns refs like `@v1:e1`, `@v1:e2`, etc.

- The **version** (`v1`, `v2`, ...) increments with each snapshot call
- The **element** (`e1`, `e2`, ...) is a unique ID within that version
- Refs map to specific DOM elements at snapshot time

```
@v1:e1  [input type="email"] placeholder="Email"
@v1:e2  [input type="password"] placeholder="Password"
@v1:e3  [button] "Sign In"
@v1:e4  [a] "Forgot password?"
```

**MCP agents:** The `browser_snapshot` tool returns the same structured ref data. Several MCP prompts and the response envelopes explicitly remind you to re-snapshot.

</how_refs_work>

<staleness_rule>

**Refs become stale when the page changes.** This includes:
- Navigation to a new URL
- Form submission
- Dynamic content loading (AJAX, SPA transitions)
- Modal/dialog open or close

After any of these, **always re-snapshot before interacting**:

```bash
gsd-browser navigate https://example.com
gsd-browser snapshot          # @v1:*
# ... interact with @v1:* refs ...
gsd-browser click-ref @v1:e3
gsd-browser wait-for --condition network_idle
gsd-browser snapshot          # now @v2:* — old refs are invalid
```

**MCP equivalent pattern:** `browser_navigate` → (read `gsd-browser://latest-snapshot` or call `browser_snapshot`) → use refs → after action that changes page → repeat.

</staleness_rule>

<snapshot_modes>

Use `--mode` (or the `mode` param in MCP `browser_snapshot`) to focus the snapshot:

| Mode | Captures | Typical MCP/Agent Use |
|------|----------|-----------------------|
| interactive (default) | Buttons, inputs, links, selects | General navigation & forms |
| form | Form fields + current values + labels | `browser_analyze_form` + `fill_form` flows |
| dialog | Content inside open modals | Handling dialogs |
| navigation | Nav links and menus | Site exploration |
| errors | Error messages / validation | Assertion + debug |
| headings | h1-h6 | Page structure / scraping |
| visible_only | Everything visible | Broad visual audit |

</snapshot_modes>

<best_practices>

- Snapshot with higher `--limit` on complex/SPA pages.
- Scope with `--selector` when you only care about a region (e.g. a specific form or table).
- After `browser_act` (semantic) that may have caused navigation, still snapshot before the next ref-based step.
- Combine snapshot with `browser_get_ref <ref>` (MCP) when you need bbox, ARIA, selector hints, or structural signature for debugging.
- The action cache + `browser_find_element` provide resilience when exact refs are uncertain.

See root SKILL.md (Snapshot & Refs section) and docs/AGENT-BEST-PRACTICES.md (Golden Rule #1 and the self-healing section) for full patterns.

</best_practices>
