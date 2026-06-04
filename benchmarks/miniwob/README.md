# MiniWoB BrowserGym Benchmark Harness

This harness runs BrowserGym MiniWoB tasks through AgentLab while the agent actions
are dispatched through a local `gsd-browser --cdp-url` bridge.

Setup:

```bash
cd /Users/solvely/.codex/worktrees/gsdbrowse-benchmark-full
cargo build -p gsd-browser

export MINIWOB_URL=file:///tmp/gsdbrowse-agentlab/miniwob-plusplus/miniwob/html/miniwob/
```

Sanity check the full benchmark list:

```bash
/tmp/gsdbrowse-agentlab/.venv/bin/python benchmarks/miniwob/run_agentlab_gsd_miniwob.py \
  --binary "$PWD/target/debug/gsd-browser" \
  --benchmark miniwob \
  --exp-root /tmp/gsdbrowse-agentlab/agentlab-gsd-miniwob-full \
  --dry-run
```

Run a 25-episode sample:

```bash
/tmp/gsdbrowse-agentlab/.venv/bin/python benchmarks/miniwob/run_agentlab_gsd_miniwob.py \
  --binary "$PWD/target/debug/gsd-browser" \
  --benchmark miniwob \
  --limit 25 \
  --exp-root /tmp/gsdbrowse-agentlab/agentlab-gsd-miniwob-sample25-$(date +%Y%m%d-%H%M%S)
```

Run the full MiniWoB benchmark:

```bash
/tmp/gsdbrowse-agentlab/.venv/bin/python benchmarks/miniwob/run_agentlab_gsd_miniwob.py \
  --binary "$PWD/target/debug/gsd-browser" \
  --benchmark miniwob \
  --exp-root /tmp/gsdbrowse-agentlab/agentlab-gsd-miniwob-full-$(date +%Y%m%d-%H%M%S)
```

Cleanup is enabled by default. After every episode, the runner stops the matching
`gsd-browser` session and kills only leftover browser/daemon processes scoped to
the benchmark CDP port. The default port is `9733`; override it with `--port`.
