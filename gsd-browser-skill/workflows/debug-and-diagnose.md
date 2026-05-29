<required_reading>
**Read these reference files NOW:**
1. references/command-reference.md (Diagnostics section)
2. references/error-recovery.md
3. (MCP agents) Root docs/AGENT-BEST-PRACTICES.md section on "Use diagnostic tools when stuck" and the `debug_stuck_agent_flow` prompt
</required_reading>

<process>

**Step 1: Get a debug bundle (MCP & CLI)**

The fastest way to diagnose any issue — captures screenshot, console logs, network logs, timeline, and accessibility tree in one call:

```bash
gsd-browser debug-bundle
```

**MCP:** `browser_debug_bundle` (optionally with `name` + `session`). This is the #1 tool when an agent is lost. Many built-in prompts and response envelopes recommend it immediately.

**Step 2: Check console logs**

```bash
gsd-browser console                    # Read and clear buffer
gsd-browser console --no-clear         # Read without clearing
```

Console buffer starts fresh on each navigation. Check logs **before** navigating away from the page where the issue occurred.

**MCP:** `browser_console` (with `no_clear` option).

**Step 3: Check network logs**

```bash
gsd-browser network
gsd-browser network --filter errors    # or fetch-xhr
```

**MCP:** `browser_network`.

**Step 4: Timeline & session summary**

```bash
gsd-browser timeline
gsd-browser session-summary
```

**MCP:** `browser_timeline` (or read the `gsd-browser://timeline` resource), plus `browser_control_state` when using the viewer.

**Step 5: MCP-specific stuck-agent flow**

When using the MCP server:
1. Immediately call `browser_debug_bundle`.
2. `browser_console` + `browser_network` + `browser_snapshot` (or read `gsd-browser://latest-snapshot`) + `browser_timeline`.
3. Check for stale refs, console errors, or failed requests.
4. If human input/judgment is needed: `browser_view` + `browser_annotation_request` or `browser_takeover`.
5. Invoke the built-in `debug_stuck_agent_flow` prompt.
6. Use `suggested_next_actions` from prior tool envelopes.

**Step 6: Viewer + annotations + recordings for collaborative diagnosis**

```bash
gsd-browser --session demo view --print-only
# Human watches / takes over / annotates
gsd-browser --session demo annotation-request "What is the exact error message visible?"
gsd-browser --session demo record-start --name "bug-repro"
# ... reproduce ...
gsd-browser --session demo record-stop
gsd-browser --session demo recording-export <id> --output ./evidence/
```

**MCP tools:** `browser_view`, `browser_annotation_request`, `browser_record_*`, etc. This produces high-fidelity, shareable evidence bundles.

**Step 7: Injection scanning & other safety checks (agent-specific)**

```bash
gsd-browser check-injection --include-hidden
```

**MCP:** `browser_check_injection`.

**Step 8: Self-healing & resilience diagnostics**

```bash
gsd-browser action-cache --action stats
gsd-browser action-cache --action get --intent ...
```

**MCP:** `browser_action_cache`. Low hit rates or stale mappings often explain "works sometimes" behavior in long-running agents.

**Step 9: Export everything for audit or handoff**

- `browser_recording_export`
- `browser_har_export`
- `browser_save_pdf`
- `browser_trace_stop`
- Full debug bundle + annotations

</process>

<success>
You have a complete, timestamped, annotated reproduction package plus console/network/timeline/screenshot artifacts that explain exactly what happened — suitable for filing a bug, handing off to a human, or feeding back into the agent's context for the next attempt.
</success>
