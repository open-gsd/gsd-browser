<overview>
Semantic intents allow interaction by purpose rather than selector. `find-best` returns scored candidates. `act` finds the best match and clicks/focuses it in one call. Intents are predefined categories, not free-form text.

**MCP Note (High Value):** In the MCP server, prefer `browser_act` (and `browser_find_best`) as the first approach for common actions. They are among the most agent-friendly tools because they require no prior snapshot or fragile selectors. Fall back to snapshot + refs only when needed. `browser_find_element` adds further resilience. The action cache (`browser_action_cache`) lets successful intent→selector mappings persist across sessions (use named sessions).
</overview>

<intent_table>

| Intent | Action | Description |
|--------|--------|-------------|
| `submit_form` | click | Submit buttons, form actions |
| `close_dialog` | click | Modal/dialog close buttons |
| `primary_cta` | click | Primary call-to-action elements |
| `search_field` | focus | Search inputs and searchboxes |
| `next_step` | click | Next/continue/proceed buttons |
| `dismiss` | click | Dismiss overlays, banners, toasts |
| `auth_action` | click | Login/signup/register buttons |
| `back_navigation` | click | Back/previous navigation links |
| `fill_email` | focus | Email input fields |
| `fill_password` | focus | Password input fields |
| `fill_username` | focus | Username/login input fields |
| `accept_cookies` | click | Cookie consent accept buttons |
| `main_content` | click | Main content area (semantic markup required) |
| `pagination_next` | click | Next page in pagination |
| `pagination_prev` | click | Previous page in pagination |

</intent_table>

<usage>

**CLI:**
```bash
gsd-browser act --intent accept_cookies
gsd-browser find-best --intent primary_cta --scope "#hero"
gsd-browser act --intent submit_form
```

**MCP (recommended for agents):**
- `browser_act` with `intent` (and optional `scope` + `session`)
- `browser_find_best` (inspect before acting)
- `browser_find_element` (intent + text/role/selector fallbacks — great when refs may be stale)
- After success, optionally `browser_action_cache put` to train the system

See docs/AGENT-BEST-PRACTICES.md "Prefer semantic first, refs second" and the self-healing section. Many of the built-in MCP prompts start with semantic intents.

</usage>

<when_to_fallback>

Use snapshot + refs (or `browser_find_element`) when:
- The intent system does not have a matching category for your target
- You need pixel-precise or context-specific targeting inside a complex widget
- You have already snapshotted and have fresh, reliable refs
- You need the full metadata from `get-ref` (bbox, deepPath, structuralSignature, etc.)

The combination (semantic first → refs for precision → action cache for learning) is one of gsd-browser's strongest advantages for long-running agentic work.

</when_to_fallback>
