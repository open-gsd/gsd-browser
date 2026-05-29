# gsd-browser MCP — Agent Best Practices (High-Value Patterns)

This guide is for agents (and the humans directing them) who want to get maximum power, reliability, and auditability out of gsd-browser via MCP.

**Primary entry point:** After connecting your MCP client to `gsd-browser mcp`, read this document + the root [SKILL.md](../SKILL.md) (for exact CLI semantics that the MCP tools mirror) and the curated skill pack in `gsd-browser-skill/`.

## Core Philosophy

gsd-browser is strongest when you treat it as a **collaborative, evidence-oriented browser platform**, not just a headless automation engine.

Its superpowers are:
- Stable, versioned element addressing (refs)
- First-class human + agent handoff (live viewer + annotations + recordings + step/abort/goal)
- Built-in reproducibility and evidence (recordings, visual diff, debug bundles, HAR, PDF, traces)
- Semantic + precise control + self-healing (action cache) in one interface
- Rich standardized envelopes with `suggested_next_actions` and `evidence_refs`

## Golden Rules

1. **Snapshot early, snapshot often**
   - Always call `browser_snapshot` after navigation or major state changes before using any `_ref` tools.
   - Old refs (`@v1:...`) become stale. New snapshots give you `@v2:...` etc.
   - The `gsd-browser://latest-snapshot` resource is wired to perform a fresh snapshot for you.

2. **Prefer semantic first, refs second**
   - Start with `browser_act` (or `browser_find_best`) for common intents.
   - Fall back to `browser_snapshot` + `_ref` tools (click_ref, fill_ref, etc.) when you need precision or the intent system doesn't cover the case.
   - Use `browser_find_element` for high-resilience lookup when refs may be stale.

3. **Use the live viewer as a superpower**
   - `browser_view` gives you (and humans) eyes on what the agent is doing (plus goal banners).
   - `browser_takeover` / `browser_release_control` + `browser_annotation_request` + `browser_step` / `browser_abort` lets humans inject judgment and create rich evidence.
   - Recordings started during viewer sessions become extremely high-quality reproduction packages.
   - Use `browser_goal`, `browser_control_state`, pause/resume/step for shared control flows.

4. **Treat recordings and annotations as first-class evidence**
   - Use `browser_record_start` / `stop` + annotations for any flow that might need to be reproduced, audited, or shown to a human later.
   - Export bundles (`browser_recording_export`) for sharing.
   - Combine with `browser_annotation_request` at key decision points.

5. **Assert explicitly**
   - Use `browser_assert` instead of hoping the previous action "worked".
   - Combine with `browser_wait_for` (network_idle, selector_visible, url_contains, region_stable, etc.) for robust long-horizon flows.
   - Prefer `browser_batch` for atomic multi-step sequences with built-in assertions.

6. **Leverage state & vault for speed and reliability**
   - `browser_save_state` / `browser_restore_state` + `browser_vault_login` lets agents skip repetitive login/setup steps.
   - Use named sessions (`--session` / session param) for persistent cache + state across runs.

7. **Use diagnostic tools when stuck**
   - `browser_debug_bundle` is your best friend when an agent (or human) needs to understand "what just happened".
   - Pair with `browser_console`, `browser_network`, `browser_timeline`, and the `debug_stuck_agent_flow` prompt.

8. **Build long-term resilience**
   - Use the `browser_action_cache` (stats / get / put / clear) + named sessions so successful intent → selector mappings survive across agent sessions.
   - Follow the `suggested_next_actions` returned in every tool envelope.

## Recommended Workflow Patterns

### Reliable Login / Auth Flow

```
browser_navigate(login_url)
browser_act("accept_cookies")  // if relevant
browser_act("fill_email") or browser_fill_form
browser_act("fill_password")
browser_act("submit_form")
browser_wait_for network_idle or url_contains dashboard
browser_assert [checks for logged-in state]
browser_save_state("myapp-auth")   // for future reuse
```

Or use `browser_vault_login` if the profile exists (recommended for repeated use).

See the built-in `robust_login_flow` prompt.

### Complex Form / Checkout

- `browser_analyze_form` first
- `browser_fill_form` with the map of labels/names
- `browser_act("submit_form")` or targeted ref
- Strong assertions + wait conditions afterward
- Consider wrapping key sections in `browser_batch`

### Human-in-the-Loop Investigation or Bug Reproduction

- `browser_view` (print_only true or interactive)
- Perform the flow using refs/act (or let human drive via takeover)
- `browser_annotation_request` at key decision points
- `browser_record_start` → do the repro (human or agent) → `browser_record_stop`
- `browser_recording_export` the bundle + annotations
- Use `browser_goal` to communicate intent to the human collaborator

### Full Page / Flow Audit

Use the `full_page_audit` prompt or manually call in parallel where possible:
- `browser_snapshot` (visible_only or interactive)
- `browser_console` + `browser_network`
- `browser_debug_bundle`
- `browser_visual_diff` (against a known good baseline if one exists)
- `browser_check_injection` if untrusted content

Then synthesize with refs and evidence links.

See also the `autonomous_research_task` prompt.

## Response Envelope (Major Differentiator)

Every successful `tools/call` returns a rich, standardized envelope (inside the text content as JSON):

- `summary`: short clear outcome
- `structured_data`: full parsable result (when JSON)
- `suggested_next_actions`: concrete high-signal hints — **use them!**
- `evidence_refs`: pointers to recordings, viewer, annotations, debug bundles
- `raw_fallback`: original output when needed

This structure, combined with the prompts and resources, makes gsd-browser MCP feel like a true agent operating system for the browser.

## Using Resources Effectively

Resources give agents passive/live context without always issuing a tool call:

- `gsd-browser://latest-snapshot` — triggers fresh snapshot + returns versioned refs + structure (extremely useful after navigate)
- `gsd-browser://current-state` — rich debug bundle (screenshot + console + network + timeline + a11y)
- `gsd-browser://active-recordings`, `gsd-browser://timeline`, `gsd-browser://current-refs`

Read them in your agent loop for up-to-date context. Example pattern: After navigation, read latest-snapshot resource, then decide next tool using the fresh refs.

## Using Prompts

The built-in prompts are executable multi-step guides that encode these best practices:

- `robust_login_flow`
- `full_page_audit`
- `create_evidence_bundle`
- `autonomous_research_task`
- `evidence_creation_workflow`
- `debug_stuck_agent_flow`

Ask your MCP client: "Use the gsd-browser prompt `autonomous_research_task` with start_url X and goal Y".

## Self-Healing & Resilience Patterns

- Prefer `browser_find_element` or `browser_act` + `browser_find_best` when exact refs may be stale.
- After successful interactions, call `browser_action_cache put` (with intent + selector + score) to train the system.
- Combine with named sessions for persistent cache/state across long-running agent projects.
- The envelope's `suggested_next_actions` frequently contains resilience hints — follow them.
- `browser_batch` for atomicity on complex flows reduces partial-state errors.

**Pro tip**: Build agent loops around "snapshot or read latest-snapshot resource → find_element or act → (on success) action_cache put → assert/wait → re-snapshot".

## When to Use What

| Situation                              | Best Starting Tool(s) / Pattern                                      |
|----------------------------------------|----------------------------------------------------------------------|
| Need to click/fill something           | `browser_act` first (semantic), then refs for precision             |
| Complex or dynamic page                | `browser_snapshot` + refs (or `browser_find_element`)                |
| Want human to see / help / judge       | `browser_view` + `annotation_request` + recordings + `browser_goal` + takeover/step |
| Need reproducibility / evidence        | `browser_record_start/stop` + annotations + `recording_export`       |
| Form filling                           | `browser_analyze_form` + `browser_fill_form`                         |
| Visual / regression concerns           | `browser_visual_diff` + screenshots + `browser_debug_bundle`         |
| Agent is lost or needs evidence        | `browser_debug_bundle`, console, network, timeline, `debug_stuck_agent_flow` prompt |
| Preserve auth across runs              | `browser_save_state` / `restore_state` + `browser_vault_login`       |
| Stale refs or uncertain element        | `browser_find_element` (semantic + text/role/selector fallback)      |
| Mobile / device testing                | `browser_emulate_device`                                             |
| Performance or deep debugging          | `browser_trace_start/stop` + HAR export + network                    |
| Long-term self-healing across runs     | `browser_action_cache` (stats/get/put) + persistent named sessions   |
| Archiving evidence                     | `browser_save_pdf`, `browser_recording_export`                       |
| Complex multi-step atomic flows        | `browser_batch` (highly recommended for reliability + fewer roundtrips) |
| Tab/frame management                   | `browser_list_pages` / `switch_page` / `list_frames` / `select_frame` |
| Human goal setting & fine control      | `browser_goal`, `browser_step`, `browser_abort`, `browser_control_state` |

## Cross-References to Core Documentation

- **Root [SKILL.md](../SKILL.md)**: Authoritative CLI syntax, all 90+ commands with examples, error recovery patterns, common workflows, config precedence, and exact semantics. MCP tools (`browser_*`) are a direct mapping of this surface. Read it for the "what does this tool actually do?" details.
- **gsd-browser-skill/** (the installable agent skill pack): Lightweight workflow router + curated references/workflows for coding agents. The MCP best practices here complement the CLI-focused skill pack. Update the skill pack via the installer or by copying refreshed files.
- **[docs/mcp.md](./mcp.md)**: MCP server architecture, quickstart script usage, client config examples, how the adapter works.
- **scripts/mcp-quickstart.sh**: Run it for your client (cursor/claude/vscode/...) to get tailored instructions and config snippets.
- **Underlying implementation**: `cli/src/mcp.rs` (build_tool_list + prompt definitions + resource wiring + envelope construction).

## Current Status & Directions

**Current (as of latest):**
- 50+ MCP tools with full coverage of the rich daemon surface.
- Real resources (latest-snapshot, current-state via debug bundle, active-recordings, timeline, etc.).
- Executable prompts encoding best-practice workflows.
- Powerful response envelopes with suggested_next_actions and evidence pointers on every call.
- Strong coverage of refs, viewer collaboration, recordings, annotations, evidence, safety, self-healing cache, batch, multi-page/frame, and more.

**Active / Future (high leverage areas):**
- Continued hardening of self-healing (richer snapshot context, fuzzy ref resolution).
- Even deeper resource implementations and caching.
- More high-value prompts and reusable agent workflow templates.
- Packaging / distribution improvements and one-command MCP onboarding.
- Integration with broader evidence platforms and multi-agent orchestration.

This is deliberately being built as one of the most powerful browser surfaces for serious agentic work.

Use it boldly. The combination of stable refs + human-agent collaboration via the live viewer + first-class evidence bundles + semantic tools + self-healing + rich envelopes is currently unmatched.

Welcome to the high end of agent browser automation. Feedback and contributions welcome.

See the root SKILL.md and gsd-browser-skill/ for the complete underlying command and workflow reference.
