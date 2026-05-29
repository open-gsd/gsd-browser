#!/bin/bash
# gsd-browser MCP Quickstart Helper
# Usage: ./scripts/mcp-quickstart.sh [client-name]
# Helps set up gsd-browser as MCP server for common clients.

set -e

CLIENT=${1:-"generic"}

echo "gsd-browser MCP Quickstart for $CLIENT"
echo "========================================"

echo ""
echo "1. Ensure gsd-browser is built or installed:"
echo "   cargo install --path cli"
echo "   or use pre-built binary from releases."

echo ""
echo "2. Basic MCP server command:"
echo "   gsd-browser mcp"

echo ""
echo "3. Recommended client config snippet (add to your mcp.json / settings):"

case $CLIENT in
  cursor|vscode|copilot)
    cat <<EOF
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
EOF
    ;;
  claude|desktop)
    cat <<EOF
{
  "mcpServers": {
    "gsd-browser": {
      "command": "gsd-browser",
      "args": ["mcp"]
    }
  }
}
EOF
    ;;
  *)
    echo '  command: gsd-browser mcp'
    echo '  (pass --session myproject for isolated instances)'
    ;;
esac

echo ""
echo "4. Pro tips for agents (the high-value parts):"
echo "   - Use named sessions per project for isolation + persistent state/cache + self-healing."
echo "   - Set GSD_BROWSER_VAULT_KEY before first daemon start for auth features."
echo "   - Read resources like gsd-browser://latest-snapshot, current-state, current-refs for live context."
echo "   - Use the built-in prompts (autonomous_research_task, evidence_creation_workflow, debug_stuck_agent_flow) as high-level plans."
echo "   - Leverage batch for atomic complex flows, action_cache for long-term learning, and the full viewer + annotations + recordings for human collaboration + audit evidence."

echo ""
echo "5. Test it:"
echo "   echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}' | gsd-browser mcp"

echo ""
echo "Full docs: docs/mcp.md and docs/AGENT-BEST-PRACTICES.md"
echo "Run with GSD_BROWSER_DEBUG=1 for verbose daemon logs if needed."

echo ""
echo "Ready. Point your agent at gsd-browser mcp and unleash one of the most powerful browser surfaces available for serious agentic work."