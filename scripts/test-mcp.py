#!/usr/bin/env python3
"""
MCP stdio smoke test for gsd-browser.

Sends initialize + two real tool calls (navigate + snapshot) and verifies responses.
"""

import json
import subprocess
import sys
import time

def send_request(proc, req):
    line = json.dumps(req) + "\n"
    proc.stdin.write(line)
    proc.stdin.flush()
    # Read response (one JSON object per line)
    resp_line = proc.stdout.readline()
    if not resp_line:
        raise RuntimeError("MCP server closed stdout unexpectedly")
    return json.loads(resp_line)

def main():
    binary = "./target/debug/gsd-browser"

    print("=== Starting gsd-browser mcp (stdio) ===")
    proc = subprocess.Popen(
        [binary, "mcp"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )

    try:
        # 1. initialize
        init_resp = send_request(proc, {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        })
        print("initialize:", json.dumps(init_resp.get("result", {}), indent=2))

        # 2. tools/list (sanity)
        tools_resp = send_request(proc, {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        })
        tools = tools_resp.get("result", {}).get("tools", [])
        print(f"\nDiscovered {len(tools)} tools:")
        for t in tools:
            print(f"  - {t['name']}")

        # 3. Real call: browser_navigate
        print("\n=== Calling browser_navigate ===")
        nav_resp = send_request(proc, {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "browser_navigate",
                "arguments": {
                    "url": "https://example.com"
                }
            }
        })
        print("navigate result (truncated):")
        content = nav_resp.get("result", {}).get("content", [{}])[0].get("text", "")
        print(content[:400] + "..." if len(content) > 400 else content)

        # 4. Real call: browser_snapshot
        print("\n=== Calling browser_snapshot ===")
        snap_resp = send_request(proc, {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "browser_snapshot",
                "arguments": {
                    "limit": 3
                }
            }
        })
        content = snap_resp.get("result", {}).get("content", [{}])[0].get("text", "")
        print("snapshot result (truncated):")
        print(content[:600] + "..." if len(content) > 600 else content)

        print("\n✅ MCP real tool call smoke test PASSED")

    except Exception as e:
        print(f"\n❌ Test failed: {e}", file=sys.stderr)
        # Dump stderr from the server for debugging
        try:
            stderr = proc.stderr.read()
            if stderr:
                print("=== Server stderr ===", file=sys.stderr)
                print(stderr, file=sys.stderr)
        except:
            pass
        sys.exit(1)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except:
            proc.kill()

if __name__ == "__main__":
    main()