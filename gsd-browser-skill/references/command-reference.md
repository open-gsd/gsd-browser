<overview>
Complete syntax reference for gsd-browser commands. Argument syntax: `<arg>` = required positional, `[arg]` = optional positional, `--flag` = named option. Do NOT add `--` prefix to positional args.

**MCP Integration Note:** The MCP server (`gsd-browser mcp`) exposes the vast majority of these commands as discoverable `browser_*` tools (e.g. `browser_navigate`, `browser_snapshot`, `browser_click_ref`, `browser_act`, `browser_fill_form`, `browser_batch`, `browser_view`, `browser_record_start`, `browser_debug_bundle`, `browser_action_cache`, etc.). Tool schemas, descriptions, and rich response envelopes (with suggested_next_actions) are served live by the MCP server. See root `docs/mcp.md`, `docs/AGENT-BEST-PRACTICES.md`, and the `gsd-browser-skill/SKILL.md` MCP highlights section. The CLI syntax here is the authoritative semantics for every MCP tool.
</overview>

<navigation>

```bash
gsd-browser navigate <url>
gsd-browser back
gsd-browser forward
gsd-browser reload
```

</navigation>

<interaction>

All selectors are **positional** — do NOT use `--selector`.

```bash
gsd-browser click <selector>
gsd-browser click --x 100 --y 200                         # Click by coordinates

gsd-browser type <selector> <text>
gsd-browser type <selector> <text> --slowly                # Character-by-character
gsd-browser type <selector> <text> --clear-first           # Clear before typing
gsd-browser type <selector> <text> --submit                # Press Enter after

gsd-browser press <key>                                    # Enter, Escape, Tab, Meta+A
gsd-browser hover <selector>
gsd-browser scroll --direction down                        # Default 300px
gsd-browser scroll --direction up --amount 500
gsd-browser select-option <selector> <option>              # Dropdown by label/value
gsd-browser set-checked <selector> --checked               # Omit --checked to uncheck
gsd-browser drag <source-selector> <target-selector>
gsd-browser upload-file <selector> <file>...
gsd-browser set-viewport --preset mobile                   # mobile, tablet, desktop, wide
gsd-browser set-viewport --width 1920 --height 1080
```

Natural-language instruction actions:

```bash
gsd-browser act-instruction <instruction>
gsd-browser act-instruction --dry-run <instruction>
gsd-browser act-instruction --scope <selector> <instruction>
gsd-browser act-instruction --min-confidence 0.7 <instruction>
gsd-browser act-instruction --max-steps 3 <instruction>
```

`act-instruction` plans against a live instruction page model and executes existing primitives such as click, type, select-option, set-checked, drag, scroll, typed control values (number/spinner, range/slider, date/time/month/week, color), reveal-and-click discovery, simple repeated-row choices, DOM-visible counts, and short bounded sequences. The page model captures visible DOM/accessibility elements, labels, bounds, grouping context, and inferred affordances, and executed actions include before/after verification signals. Use it for short instructions like "enter 'Alice' into Name and click Save", "choose California from State", "select 42 with the slider", "use the spinner to select 7", "check Red, Green, and Blue", "expand sections to find and click the link", or "buy the shortest duration". Use `--dry-run` to inspect the selected plan and page model, `--scope` to constrain repeated controls to a form/dialog/panel, `--min-confidence` to block weak matches, and `--max-steps` to cap compound actions.

**MCP note:** `browser_act_instruction` exposes the same controls (`instruction`, `dry_run`, `scope`, `min_confidence`, `max_steps`). Prefer this for concise natural-language actions; prefer refs, `browser_fill_form`, or `browser_batch` when exact element identity, full form semantics, or longer workflows matter.

</interaction>

<snapshot_and_refs>

```bash
gsd-browser snapshot
gsd-browser snapshot --selector "form"
gsd-browser snapshot --mode <mode>                         # See snapshot modes below
gsd-browser snapshot --limit 80                            # Default: 40

gsd-browser get-ref <ref>
gsd-browser click-ref <ref>
gsd-browser hover-ref <ref>
gsd-browser fill-ref <ref> <text>
```

**Snapshot modes:** `interactive` (default), `form`, `dialog`, `navigation`, `errors`, `headings`, `visible_only`

**MCP note:** `browser_snapshot` (with mode/limit/selector/session) + `browser_get_ref`, `browser_click_ref`, `browser_fill_ref`, etc. are core. Always re-snapshot (or read `gsd-browser://latest-snapshot` resource) after page changes. Prefer semantic `browser_act`/`browser_find_best`/`browser_find_element` first when possible.

</snapshot_and_refs>

<inspection>

```bash
gsd-browser accessibility-tree
gsd-browser find --text "Sign In"
gsd-browser find --role button
gsd-browser find --selector ".my-class"
gsd-browser find --role link --limit 50                    # Default: 20
gsd-browser page-source
gsd-browser page-source --selector "main"
gsd-browser eval '<js-expression>'
```

</inspection>

<assertions>

```bash
gsd-browser assert --checks '[
  {"kind": "url_contains", "text": "/dashboard"},
  {"kind": "text_visible", "text": "Welcome"},
  {"kind": "selector_visible", "selector": "#user-menu"},
  {"kind": "value_equals", "selector": "input[name=email]", "value": "user@test.com"},
  {"kind": "no_console_errors"},
  {"kind": "no_failed_requests"}
]'
```

**Assertion kinds (18+):** url_contains, title_contains, text_visible, text_hidden, selector_visible, selector_hidden, value_equals, checked, no_console_errors, no_failed_requests, request_url_seen, response_status, console_message_matches, network_count, console_count, element_count, and the _since variants.

**MCP:** `browser_assert` (checks array) and `browser_wait_for` (condition + value + timeout + threshold) are heavily used in agent flows and in the built-in prompts.

</assertions>

<batch>

```bash
gsd-browser batch --steps '[ ... array of step objects ... ]' --stop-on-failure --summary-only
```

**Supported batch actions:** navigate, reload, click, type, select_option, key_press, press, wait_for, assert, click_ref, fill_ref, hover, hover_ref, scroll, snapshot, diff.

**MCP:** `browser_batch` is one of the highest-value tools for reliable long-horizon agent workflows (atomicity + fewer roundtrips). See best-practices guide.

</batch>

<waits>

```bash
gsd-browser wait-for --condition network_idle
gsd-browser wait-for --condition selector_visible --value "#content" --timeout 30000
gsd-browser wait-for --condition url_contains --value "/dashboard"
gsd-browser wait-for --condition text_visible --value "Success"
gsd-browser wait-for --condition element_count --value ".item" --threshold ">=5"
gsd-browser wait-for --condition region_stable --value "#content"
# ... many more conditions (see root SKILL.md)
```

</waits>

<forms>

```bash
gsd-browser analyze-form
gsd-browser analyze-form --selector "#checkout-form"

gsd-browser fill-form --values '{"Email": "a@b.com", "Password": "secret"}' --submit
gsd-browser fill-form --values '{"Full Name": "Jane"}' --selector "#signup" 
```

**MCP equivalents:** `browser_analyze_form`, `browser_fill_form` (values object + submit + selector + session). Extremely ergonomic for agents.

</forms>

<intent_based>

```bash
gsd-browser find-best --intent submit_form --scope "#modal"
gsd-browser act --intent accept_cookies
gsd-browser act --intent primary_cta
```

**Built-in intents (15):** submit_form, close_dialog, primary_cta, search_field, next_step, dismiss, auth_action, back_navigation, fill_email, fill_password, fill_username, accept_cookies, main_content, pagination_next, pagination_prev.

**MCP:** `browser_act` (high value first choice) and `browser_find_best`. Combine with `browser_find_element` for resilience when refs may be stale.

</intent_based>

<pages_frames>

```bash
gsd-browser list-pages
gsd-browser switch-page <id>     # positional id
gsd-browser close-page <id>

gsd-browser list-frames
gsd-browser select-frame --name "main" | --index 0 | --url-pattern "embed"
```

**MCP:** `browser_list_pages`, `browser_switch_page`, `browser_close_page`, `browser_list_frames`, `browser_select_frame`.

</pages_frames>

<diagnostics>

```bash
gsd-browser console
gsd-browser console --no-clear
gsd-browser network --filter errors
gsd-browser dialog
gsd-browser timeline
gsd-browser session-summary
gsd-browser debug-bundle --name "stuck-flow"
```

**MCP:** `browser_console`, `browser_network`, `browser_debug_bundle`, `browser_timeline`, etc. Use `debug_stuck_agent_flow` prompt + these when an agent is lost.

</diagnostics>

<live_viewer_workbench>

```bash
gsd-browser view
gsd-browser view --print-only
gsd-browser view --interactive

gsd-browser goal "Complete checkout" 
gsd-browser goal --clear

gsd-browser control-state
gsd-browser takeover
gsd-browser release-control
gsd-browser pause / resume / step / abort
gsd-browser sensitive-on / sensitive-off
```

**Annotations:**
```bash
gsd-browser annotations
gsd-browser annotation-request "Please note the price shown here"
gsd-browser annotation-clear --all
```

**Recordings (evidence bundles):**
```bash
gsd-browser record-start --name "checkout-bug-2026-05"
gsd-browser record-stop
gsd-browser recordings
gsd-browser recording-get <id>
gsd-browser recording-export <id> --output ./evidence/
```

**MCP equivalents are first-class superpowers for human+agent collaboration and auditability:** `browser_view`, `browser_takeover`, `browser_annotation_request`, `browser_record_*`, `browser_goal`, `browser_step`, etc. See AGENT-BEST-PRACTICES.md for patterns.

</live_viewer_workbench>

<visual>

```bash
gsd-browser screenshot --output page.png --full-page
gsd-browser screenshot --selector "#hero" --format png
gsd-browser save-pdf --output report.pdf --format A4
gsd-browser zoom-region ...
```

</visual>

<visual_regression>

```bash
gsd-browser visual-diff --name "homepage"
gsd-browser visual-diff --name "homepage" --update-baseline --threshold 0.05
gsd-browser visual-diff --selector "#main" --name "main-content"
```

**MCP:** `browser_visual_diff`, `browser_screenshot`, `browser_save_pdf`.

</visual_regression>

<structured_extraction>

```bash
gsd-browser extract --schema '{ "type":"object", "properties": { "price": {"_selector":".price", "_attribute":"textContent"} } }'
gsd-browser extract --selector ".product" --multiple --schema '{ ... }'
```

**MCP:** `browser_extract`.

</structured_extraction>

<network_control>

```bash
gsd-browser mock-route --url "**/api/*" --body '{"ok":true}' --status 200 --delay 500
gsd-browser block-urls "**/analytics*" "**/ads*"
gsd-browser clear-routes
```

**MCP:** `browser_mock_route`, `browser_block_urls`, `browser_clear_routes`. Powerful for deterministic testing.

</network_control>

<device_emulation>

```bash
gsd-browser emulate-device "iPhone 15"
gsd-browser emulate-device "Pixel 7"
gsd-browser emulate-device list
```

**Warning:** Recreates browser context (cookies/state lost).

**MCP:** `browser_emulate_device`.

</device_emulation>

<state_auth>

```bash
gsd-browser save-state --name "logged-in"
gsd-browser restore-state --name "logged-in"

gsd-browser vault-save --profile myapp --url https://.../login --username u --password p
gsd-browser vault-login --profile myapp
gsd-browser vault-list
```

**Vault requires GSD_BROWSER_VAULT_KEY env var set before daemon start.**

**MCP:** `browser_save_state`, `browser_restore_state`, `browser_vault_login`, `browser_vault_save`, `browser_vault_list`.

</state_auth>

<tracing_recording>

```bash
gsd-browser trace-start --name "perf"
gsd-browser trace-stop --name "perf.json"
gsd-browser har-export --filename "session.har"
gsd-browser generate-test --name "flow" --output tests/flow.spec.ts --include-assertions
```

**MCP:** `browser_trace_*`, `browser_har_export`, `browser_generate_test`.

</tracing_recording>

<security>

```bash
gsd-browser check-injection --include-hidden
```

**MCP:** `browser_check_injection`.

</security>

<action_cache>

```bash
gsd-browser action-cache --action stats
gsd-browser action-cache --action get --intent submit_form
gsd-browser action-cache --action put --intent submit_form --selector "#submit" --score 0.97
gsd-browser action-cache --action clear
```

**MCP:** `browser_action_cache`. Critical for long-term self-healing across MCP agent sessions (use with named --session).

</action_cache>

<daemon>

```bash
gsd-browser daemon health
gsd-browser daemon start
gsd-browser daemon stop
gsd-browser update
```

</daemon>

<full_reference>
For the complete up-to-date surface (including all MCP tool names, inputSchemas, prompt definitions, and resource URIs), connect an MCP client to `gsd-browser mcp` and inspect `tools/list`, `resources/list`, and `prompts/list`, or read the implementation in `cli/src/mcp.rs`.

Cross-reference the root SKILL.md and docs/AGENT-BEST-PRACTICES.md for agent workflow patterns that combine these commands/tools.
</full_reference>
