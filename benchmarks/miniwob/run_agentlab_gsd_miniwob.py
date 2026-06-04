#!/usr/bin/env python3
import argparse
import json
import logging
import os
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

import bgym
import browsergym.miniwob  # registers MiniWoB tasks/benchmarks
from agentlab.experiments.loop import EnvArgs, ExpArgs


class NullActionSet:
    def to_python_code(self, action):
        return action


def run_json(cmd, timeout=20):
    started = time.perf_counter()
    proc = subprocess.run(cmd, text=True, capture_output=True, timeout=timeout)
    elapsed_ms = (time.perf_counter() - started) * 1000
    if proc.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(cmd)}\nstdout={proc.stdout}\nstderr={proc.stderr}"
        )
    text = proc.stdout.strip()
    if not text:
        return None, elapsed_ms
    try:
        return json.loads(text), elapsed_ms
    except json.JSONDecodeError:
        return {"raw": text}, elapsed_ms


def extract_value(result):
    if isinstance(result, dict):
        value = result.get("result", result.get("value", result.get("data", result)))
        if isinstance(value, str):
            try:
                return json.loads(value)
            except json.JSONDecodeError:
                return value
        return value
    return result


def process_matches(*needles):
    proc = subprocess.run(
        ["ps", "-axo", "pid,command", "-ww"],
        text=True,
        capture_output=True,
        check=False,
    )
    matches = []
    for line in proc.stdout.splitlines()[1:]:
        stripped = line.strip()
        if not stripped:
            continue
        pid_text, _, command = stripped.partition(" ")
        if not pid_text.isdigit():
            continue
        if all(needle in command for needle in needles):
            matches.append((int(pid_text), command))
    return matches


def terminate_pids(pids, sig=signal.SIGTERM):
    own_pid = os.getpid()
    for pid in sorted(set(pids)):
        if pid == own_pid:
            continue
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            pass
        except PermissionError:
            pass


def cleanup_port(port):
    # BrowserGym/Playwright launches Chromium with this remote-debugging port.
    # gsd-browser daemons connect to the same port. Limit cleanup to those
    # command-line markers so other local browser work survives.
    tokens = [
        [f"--remote-debugging-port={port}"],
        [f"--remote-debugging-port {port}"],
        ["gsd-browser _serve", f"--cdp-url http://127.0.0.1:{port}"],
    ]
    pids = []
    for needles in tokens:
        pids.extend(pid for pid, _ in process_matches(*needles))
    terminate_pids(pids, signal.SIGTERM)
    time.sleep(0.4)
    remaining = []
    for needles in tokens:
        remaining.extend(pid for pid, _ in process_matches(*needles))
    terminate_pids(remaining, signal.SIGKILL)


def cleanup_session(binary, session, port):
    subprocess.run(
        [binary, "--session", session, "daemon", "stop"],
        text=True,
        capture_output=True,
        timeout=10,
        check=False,
    )
    pids = [
        pid
        for pid, _ in process_matches("gsd-browser _serve", f"--session {session}")
    ]
    terminate_pids(pids, signal.SIGTERM)
    time.sleep(0.2)
    remaining = [
        pid
        for pid, _ in process_matches("gsd-browser _serve", f"--session {session}")
    ]
    terminate_pids(remaining, signal.SIGKILL)
    cleanup_port(port)


class GsdBrowserMiniwobAgent(bgym.Agent):
    def __init__(self, binary, cdp_url, session):
        self.binary = binary
        self.cdp_url = cdp_url
        self.session = session
        self.action_set = NullActionSet()
        self.step = 0

    def obs_preprocessor(self, obs):
        return obs

    def _gsd(self, *args, timeout=20):
        return run_json(
            [
                self.binary,
                "--session",
                self.session,
                "--cdp-url",
                self.cdp_url,
                "--json",
                *args,
            ],
            timeout=timeout,
        )

    def _eval(self, expression, timeout=20):
        value, elapsed_ms = self._gsd("eval", expression, timeout=timeout)
        return extract_value(value), elapsed_ms

    def _act_js(self, goal):
        goal_json = json.dumps(goal)
        return f"""(() => {{
          const utterance = {goal_json};
          if (/^Select\\s+/i.test(utterance)) {{
            const wantedText = utterance
              .replace(/^Select\\s+/i, '')
              .replace(/\\s+and\\s+click\\s+Submit\\.?$/i, '');
            const wanted = wantedText.toLowerCase() === 'nothing'
              ? []
              : wantedText.split(/\\s*,\\s*/).map(s => s.trim()).filter(Boolean);
            const clicked = [];
            for (const label of Array.from(document.querySelectorAll('label'))) {{
              const text = (label.textContent || '').trim();
              const input = label.querySelector('input[type=checkbox]');
              if (input && wanted.includes(text) && !input.checked) {{
                input.click();
                clicked.push(text);
              }}
            }}
            const submit = Array.from(document.querySelectorAll('button, input[type=submit], input[type=button]'))
              .find(el => /submit/i.test(el.textContent || el.value || '')) ||
              document.querySelector('button, input[type=submit], input[type=button]');
            if (submit) submit.click();
            return {{mode: 'checkboxes', wanted, clicked, submitted: !!submit}};
          }}

          const quoted = Array.from(utterance.matchAll(/"([^"]+)"/g)).map(m => m[1].toLowerCase());
          const wantButton = /button/i.test(utterance);
          const wantLink = /link/i.test(utterance);
          let candidates = Array.from(document.querySelectorAll('button, input[type=button], input[type=submit], a, .alink'));
          if (wantButton) candidates = candidates.filter(el => el.tagName === 'BUTTON' || el.getAttribute('role') === 'button' || el.type === 'button' || el.type === 'submit');
          if (wantLink) candidates = candidates.filter(el => el.tagName === 'A' || el.classList.contains('alink'));
          let el = null;
          if (quoted.length) {{
            el = candidates.find(candidate => {{
              const text = (candidate.textContent || candidate.value || candidate.getAttribute('aria-label') || '').trim().toLowerCase();
              return quoted.some(q => text.includes(q));
            }});
          }}
          if (!el) el = candidates[0];
          if (!el) return {{mode: 'none'}};
          const text = el.textContent || el.value || el.getAttribute('aria-label') || el.tagName;
          el.click();
          return {{mode: 'click', text}};
        }})()"""

    def get_action(self, obs):
        self.step += 1
        goal = obs.get("goal", "")
        started = time.perf_counter()
        result, eval_ms = self._eval(self._act_js(goal), timeout=20)
        elapsed_ms = (time.perf_counter() - started) * 1000
        return (
            "noop(50)",
            bgym.AgentInfo(
                think=f"gsd-browser acted externally through {self.cdp_url}",
                stats={"gsd_agent_elapsed_ms": elapsed_ms, "gsd_eval_ms": eval_ms},
                extra_info={"goal": goal, "gsd_result": result, "session": self.session},
            ),
        )


@dataclass
class GsdBrowserMiniwobAgentArgs(bgym.AbstractAgentArgs):
    binary: str = ""
    cdp_url: str = "http://127.0.0.1:9733"
    session: str = "agentlab-gsd-miniwob"
    agent_name: str = "GsdBrowserMiniwobAgent"

    def make_agent(self):
        return GsdBrowserMiniwobAgent(self.binary, self.cdp_url, self.session)


def benchmark_env_args(args):
    if args.benchmark == "miniwob":
        env_args_list = bgym.DEFAULT_BENCHMARKS["miniwob"]().env_args_list
        if args.offset:
            env_args_list = env_args_list[args.offset :]
        if args.limit:
            env_args_list = env_args_list[: args.limit]
        return env_args_list

    return [
        EnvArgs(
            task_name=task,
            task_seed=seed,
            max_steps=args.max_steps,
            headless=not args.headed,
            pre_observation_delay=args.pre_observation_delay,
        )
        for task in args.task
        for seed in range(args.runs)
    ]


def write_summary(exp_root, result):
    out = exp_root / "agentlab_gsd_summary.json"
    out.write_text(json.dumps(result, indent=2) + "\n")
    return out


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--exp-root", required=True)
    parser.add_argument("--port", type=int, default=9733)
    parser.add_argument("--benchmark", choices=["custom", "miniwob"], default="custom")
    parser.add_argument("--task", action="append", default=[
        "miniwob.click-button",
        "miniwob.click-link",
        "miniwob.click-checkboxes",
        "miniwob.click-test",
    ])
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--offset", type=int, default=0)
    parser.add_argument("--max-steps", type=int, default=2)
    parser.add_argument("--pre-observation-delay", type=float, default=0.1)
    parser.add_argument("--headed", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-cleanup", action="store_true")
    args = parser.parse_args()

    os.environ["BROWSERGYM_REMOTE_DEBUGGING_PORT"] = str(args.port)
    exp_root = Path(args.exp_root)
    exp_root.mkdir(parents=True, exist_ok=True)
    env_args_list = benchmark_env_args(args)

    if args.dry_run:
        print(json.dumps({
            "benchmark": args.benchmark,
            "episodes": len(env_args_list),
            "port": args.port,
            "exp_root": str(exp_root),
            "first": [
                {"task": item.task_name, "seed": item.task_seed}
                for item in env_args_list[:5]
            ],
            "last": [
                {"task": item.task_name, "seed": item.task_seed}
                for item in env_args_list[-5:]
            ],
        }, indent=2))
        return

    summaries = []
    started = time.time()
    result = {
        "benchmark": "agentlab-gsd-browser-miniwob-bridge",
        "benchmark_mode": args.benchmark,
        "port": args.port,
        "total_runs": 0,
        "successes": 0,
        "success_rate": 0,
        "started_at": started,
        "completed_at": None,
        "summaries": summaries,
    }

    try:
        for index, env_args in enumerate(env_args_list, 1):
            session = f"agentlab-{env_args.task_name.replace('.', '-')}-{env_args.task_seed}-{index}"
            item = {
                "index": index,
                "task": env_args.task_name,
                "seed": env_args.task_seed,
                "session": session,
                "exp_dir": None,
                "cum_reward": None,
                "cum_raw_reward": None,
                "err_msg": None,
            }
            try:
                exp = ExpArgs(
                    agent_args=GsdBrowserMiniwobAgentArgs(
                        binary=args.binary,
                        cdp_url=f"http://127.0.0.1:{args.port}",
                        session=session,
                    ),
                    env_args=EnvArgs(
                        task_name=env_args.task_name,
                        task_seed=env_args.task_seed,
                        max_steps=env_args.max_steps,
                        headless=env_args.headless,
                        pre_observation_delay=args.pre_observation_delay,
                    ),
                    logging_level=logging.INFO,
                    logging_level_stdout=logging.WARNING,
                    save_screenshot=False,
                )
                exp.prepare(exp_root)
                item["exp_dir"] = str(exp.exp_dir)
                exp.run()
                summary_path = Path(exp.exp_dir) / "summary_info.json"
                summary = json.loads(summary_path.read_text())
                item["cum_reward"] = summary.get("cum_reward")
                item["cum_raw_reward"] = summary.get("cum_raw_reward")
                item["err_msg"] = summary.get("err_msg")
            except Exception as exc:
                item["err_msg"] = repr(exc)
            finally:
                if not args.no_cleanup:
                    cleanup_session(args.binary, session, args.port)

            summaries.append(item)
            result["total_runs"] = len(summaries)
            result["successes"] = sum(
                1 for summary in summaries
                if summary["cum_reward"] and summary["cum_reward"] > 0
            )
            result["success_rate"] = (
                result["successes"] / result["total_runs"]
                if result["total_runs"]
                else 0
            )
            write_summary(exp_root, result)
            print(json.dumps(item), flush=True)
            print(
                "PROGRESS "
                + json.dumps({
                    "completed": result["total_runs"],
                    "total": len(env_args_list),
                    "successes": result["successes"],
                    "success_rate": result["success_rate"],
                }),
                flush=True,
            )
    finally:
        result["completed_at"] = time.time()
        summary_path = write_summary(exp_root, result)
        if not args.no_cleanup:
            cleanup_port(args.port)
        print(
            "SUMMARY "
            + json.dumps({
                "path": str(summary_path),
                "benchmark": result["benchmark"],
                "total_runs": result["total_runs"],
                "successes": result["successes"],
                "success_rate": result["success_rate"],
            }),
            flush=True,
        )


if __name__ == "__main__":
    main()
