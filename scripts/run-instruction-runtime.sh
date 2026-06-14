#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/run-instruction-runtime.sh [options]

Runs cli/tests/instruction_runtime.rs one exact test at a time, cleaning managed
browser processes at startup, on exit, and optionally between tests.

Options:
  --filter TEXT      Run tests whose names contain TEXT.
  --start-at TEST   Skip tests until TEST, then run from there.
  --offset N        Skip the first N selected tests.
  --limit N         Run at most N selected tests.
  --keep-going      Continue after failures and report them at the end.
  --retries N       Retry transient daemon/startup/termination flakes N times (default: 3).
  --wait-lock       Wait if another instruction runtime runner owns this worktree.
  --log-file PATH   Append full per-test output to PATH while printing only progress markers.
  --results-file PATH
                    Append per-test PASS/FAIL records to PATH.
  --skip-passed     With --results-file, skip tests already recorded as PASS.
  --summary         With --results-file, summarize latest selected-test statuses.
  --cleanup-between
                    Clean managed browser processes between successful exact tests.
  --no-cleanup-between
                    Compatibility no-op; cleanup between successful tests is off by default.
  --list            Print selected tests without running them.
  -h, --help        Show this help.

Runner deny files:
  Before acquiring the worktree lock, the runner checks these optional files:
    .instruction-runtime-deny
    ${TMPDIR:-/tmp}/gsd-browser-instruction-runtime-deny-<worktree-key>.txt
    /tmp/gsd-browser-instruction-runtime-deny-<worktree-key>.txt

  Each non-empty, non-comment line is matched against --results-file and the
  full runner command. Matching invocations exit before launching browser work.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

original_args=("$@")
filter=""
start_at=""
offset=0
limit=""
keep_going=0
list_only=0
retries=3
wait_lock=0
runner_log_file=""
cleanup_between=0
results_file=""
skip_passed=0
summary_only=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --filter)
      filter="${2:?missing value for --filter}"
      shift 2
      ;;
    --start-at)
      start_at="${2:?missing value for --start-at}"
      shift 2
      ;;
    --offset)
      offset="${2:?missing value for --offset}"
      shift 2
      ;;
    --limit)
      limit="${2:?missing value for --limit}"
      shift 2
      ;;
    --keep-going)
      keep_going=1
      shift
      ;;
    --retries)
      retries="${2:?missing value for --retries}"
      shift 2
      ;;
    --wait-lock)
      wait_lock=1
      shift
      ;;
    --log-file)
      runner_log_file="${2:?missing value for --log-file}"
      shift 2
      ;;
    --results-file)
      results_file="${2:?missing value for --results-file}"
      shift 2
      ;;
    --skip-passed)
      skip_passed=1
      shift
      ;;
    --summary)
      summary_only=1
      shift
      ;;
    --cleanup-between)
      cleanup_between=1
      shift
      ;;
    --no-cleanup-between)
      cleanup_between=0
      shift
      ;;
    --list)
      list_only=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ! [[ "$offset" =~ ^[0-9]+$ ]]; then
  echo "--offset must be a non-negative integer" >&2
  exit 2
fi
if [[ -n "$limit" && ! "$limit" =~ ^[0-9]+$ ]]; then
  echo "--limit must be a non-negative integer" >&2
  exit 2
fi
if ! [[ "$retries" =~ ^[0-9]+$ ]]; then
  echo "--retries must be a non-negative integer" >&2
  exit 2
fi
if [[ "$skip_passed" -eq 1 && -z "$results_file" ]]; then
  echo "--skip-passed requires --results-file" >&2
  exit 2
fi
if [[ "$summary_only" -eq 1 && -z "$results_file" ]]; then
  echo "--summary requires --results-file" >&2
  exit 2
fi

lock_key="$(printf '%s' "$repo_root" | cksum | awk '{ print $1 }')"
lock_dir="${TMPDIR:-/tmp}/gsd-browser-instruction-runtime-${lock_key}.lock"
lock_acquired=0
deny_files=(
  "$repo_root/.instruction-runtime-deny"
  "${TMPDIR:-/tmp}/gsd-browser-instruction-runtime-deny-${lock_key}.txt"
  "/tmp/gsd-browser-instruction-runtime-deny-${lock_key}.txt"
)

lock_owner_alive() {
  local pid_file="$lock_dir/pid"
  local owner_pid=""
  [[ -f "$pid_file" ]] && owner_pid="$(cat "$pid_file" 2>/dev/null || true)"
  [[ "$owner_pid" =~ ^[0-9]+$ ]] && kill -0 "$owner_pid" 2>/dev/null
}

command_line() {
  local args=("$0" "${original_args[@]}")
  printf '%q ' "${args[@]}"
  printf '\n'
}

denied_by_runner_file() {
  local command_text="$1"
  local file
  local pattern
  for file in "${deny_files[@]}"; do
    [[ -f "$file" ]] || continue
    while IFS= read -r pattern || [[ -n "$pattern" ]]; do
      pattern="${pattern%%#*}"
      pattern="${pattern#"${pattern%%[![:space:]]*}"}"
      pattern="${pattern%"${pattern##*[![:space:]]}"}"
      [[ -z "$pattern" ]] && continue
      if [[ "$pattern" == "*" || "$results_file" == "$pattern" || "$command_text" == *"$pattern"* ]]; then
        printf 'Instruction runtime runner denied by %s: %s\n' "$file" "$pattern" >&2
        return 0
      fi
    done <"$file"
  done
  return 1
}

runner_command_text="$(command_line)"
if denied_by_runner_file "$runner_command_text"; then
  exit 75
fi

acquire_lock() {
  [[ "$list_only" -eq 1 ]] && return 0
  while true; do
    if mkdir "$lock_dir" 2>/dev/null; then
      lock_acquired=1
      printf '%s\n' "$$" >"$lock_dir/pid"
      printf '%s\n' "$repo_root" >"$lock_dir/repo"
      command_line >"$lock_dir/command"
      return 0
    fi

    if ! lock_owner_alive; then
      rm -rf "$lock_dir" 2>/dev/null || true
      continue
    fi

    if [[ "$wait_lock" -eq 1 ]]; then
      printf 'Another instruction runtime runner owns %s; waiting.\n' "$repo_root" >&2
      sleep 2
      continue
    fi

    printf 'Another instruction runtime runner owns %s. Use --wait-lock or stop the existing runner before starting another.\n' "$repo_root" >&2
    if [[ -f "$lock_dir/pid" ]]; then
      printf 'Owner PID: %s\n' "$(cat "$lock_dir/pid" 2>/dev/null || true)" >&2
    fi
    exit 75
  done
}

release_lock() {
  if [[ "$lock_acquired" -eq 1 ]]; then
    rm -rf "$lock_dir" 2>/dev/null || true
  fi
}

cleanup() {
  GSD_BROWSER_CLEANUP_RESWEEP=1 scripts/cleanup-browser-runs.sh >/dev/null 2>&1 || true
}

cleanup_all() {
  scripts/cleanup-browser-runs.sh >/dev/null 2>&1 || true
}
on_exit() {
  cleanup_all
  release_lock
}
on_signal() {
  local status="$1"
  trap - INT TERM EXIT
  on_exit
  exit "$status"
}
trap on_exit EXIT
trap 'on_signal 130' INT
trap 'on_signal 143' TERM

is_transient_infra_failure() {
  local log_file="$1"
  grep -Eq \
    'daemon exited during startup with status signal: 15|daemon closed connection without response|request failed while the daemon PID was still alive|Refusing to replace a live browser session automatically|navigation failed: send failed because receiver is gone|Terminated: 15' \
    "$log_file" || grep -Eq \
    'send failed because receiver is gone|oneshot canceled|session replacement requires explicit recovery' \
    "$log_file"
}

is_transient_infra_status() {
  local status="$1"
  [[ "$status" -eq 143 ]]
}

record_result() {
  local result="$1"
  local test_name="$2"
  local status="${3:-0}"
  [[ -z "$results_file" ]] && return 0
  mkdir -p "$(dirname "$results_file")"
  printf '%s\t%s\t%s\t%s\n' "$result" "$test_name" "$status" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$results_file"
}

latest_test_status() {
  local test_name="$1"
  [[ -n "$results_file" && -f "$results_file" ]] || return 1
  awk -F'\t' -v test_name="$test_name" '
    $2 == test_name { latest_status = $1; latest_exit = $3; latest_time = $4 }
    END {
      if (latest_status == "") exit 1
      printf "%s\t%s\t%s\n", latest_status, latest_exit, latest_time
    }
  ' "$results_file"
}

test_was_passed() {
  local test_name="$1"
  local latest
  latest="$(latest_test_status "$test_name" 2>/dev/null || true)"
  [[ "${latest%%$'\t'*}" == "PASS" ]]
}

run_exact_test() {
  local test_name="$1"
  local attempt=0
  local status=0
  local test_log_file

  while true; do
    if [[ "$cleanup_between" -eq 1 ]]; then
      cleanup
    fi
    test_log_file="$(mktemp -t gsd-browser-instruction-runtime.XXXXXX)"
    set +e
    cargo test -p gsd-browser --test instruction_runtime "$test_name" -- --exact --test-threads=1 >"$test_log_file" 2>&1
    status="$?"
    set -e
    if [[ -n "$runner_log_file" ]]; then
      cat "$test_log_file" >>"$runner_log_file"
    else
      cat "$test_log_file"
    fi

    if [[ "$status" -eq 0 ]]; then
      rm -f "$test_log_file"
      record_result PASS "$test_name" 0
      if [[ "$cleanup_between" -eq 1 ]]; then
        cleanup
      fi
      return 0
    fi

    if [[ "$attempt" -lt "$retries" ]] && { is_transient_infra_status "$status" || is_transient_infra_failure "$test_log_file"; }; then
      attempt=$((attempt + 1))
      printf 'Transient instruction-runtime infrastructure failure in %s; retrying (%s/%s).\n' "$test_name" "$attempt" "$retries" >&2
      if [[ -n "$runner_log_file" ]]; then
        printf 'Transient instruction-runtime infrastructure failure in %s; retrying (%s/%s).\n' "$test_name" "$attempt" "$retries" >>"$runner_log_file"
      fi
      rm -f "$test_log_file"
      cleanup_all
      sleep 1
      continue
    fi

    rm -f "$test_log_file"
    cleanup_all
    record_result FAIL "$test_name" "$status"
    return "$status"
  done
}

all_tests=()
list_log_file="$(mktemp -t gsd-browser-instruction-runtime-list.XXXXXX)"
set +e
list_output="$(cargo test -p gsd-browser --test instruction_runtime -- --list 2>"$list_log_file")"
list_status="$?"
set -e
if [[ "$list_status" -ne 0 ]]; then
  cat "$list_log_file" >&2
  rm -f "$list_log_file"
  exit "$list_status"
fi
rm -f "$list_log_file"
while IFS= read -r test_name; do
  all_tests+=("$test_name")
done < <(printf '%s\n' "$list_output" | awk -F': ' '$2 == "test" { print $1 }')

selected=()
started=0
if [[ -z "$start_at" ]]; then
  started=1
fi

for test_name in "${all_tests[@]}"; do
  if [[ "$started" -eq 0 ]]; then
    if [[ "$test_name" == "$start_at" ]]; then
      started=1
    else
      continue
    fi
  fi
  if [[ -n "$filter" && "$test_name" != *"$filter"* ]]; then
    continue
  fi
  selected+=("$test_name")
done

if [[ "${#selected[@]}" -eq 0 ]]; then
  echo "No instruction runtime tests selected." >&2
  exit 1
fi

if [[ "$summary_only" -eq 1 ]]; then
  pass_count=0
  fail_count=0
  missing_count=0
  for test_name in "${selected[@]}"; do
    latest="$(latest_test_status "$test_name" 2>/dev/null || true)"
    if [[ -z "$latest" ]]; then
      status="MISSING"
      exit_code=""
      recorded_at=""
      missing_count=$((missing_count + 1))
    else
      IFS=$'\t' read -r status exit_code recorded_at <<<"$latest"
      if [[ "$status" == "PASS" ]]; then
        pass_count=$((pass_count + 1))
      else
        fail_count=$((fail_count + 1))
      fi
    fi
    printf '%s\t%s\t%s\t%s\n' "$status" "$test_name" "$exit_code" "$recorded_at"
  done
  printf 'Summary: PASS=%s FAIL=%s MISSING=%s TOTAL=%s\n' "$pass_count" "$fail_count" "$missing_count" "${#selected[@]}"
  if [[ "$fail_count" -eq 0 && "$missing_count" -eq 0 ]]; then
    exit 0
  fi
  exit 1
fi

pre_skip_selected_count="${#selected[@]}"
if [[ "$skip_passed" -eq 1 ]]; then
  filtered_selected=()
  for test_name in "${selected[@]}"; do
    if ! test_was_passed "$test_name"; then
      filtered_selected+=("$test_name")
    fi
  done
  selected=()
  if [[ "${#filtered_selected[@]}" -gt 0 ]]; then
    selected=("${filtered_selected[@]}")
  fi
fi

if [[ "$offset" -gt 0 ]]; then
  if [[ "$offset" -ge "${#selected[@]}" ]]; then
    selected=()
  else
    selected=("${selected[@]:$offset}")
  fi
fi

if [[ -n "$limit" ]]; then
  selected=("${selected[@]:0:$limit}")
fi

if [[ "${#selected[@]}" -eq 0 ]]; then
  if [[ "$skip_passed" -eq 1 && "$pre_skip_selected_count" -gt 0 ]]; then
    echo "All selected instruction runtime tests were already recorded as passed."
    exit 0
  fi
  echo "No instruction runtime tests selected." >&2
  exit 1
fi

if [[ "$list_only" -eq 1 ]]; then
  printf '%s\n' "${selected[@]}"
  exit 0
fi

acquire_lock "$@"
if [[ -n "$runner_log_file" ]]; then
  : >"$runner_log_file"
fi
cleanup_all

failures=()
total="${#selected[@]}"
for index in "${!selected[@]}"; do
  test_name="${selected[$index]}"
  ordinal=$((index + 1))
  printf '[%s/%s] %s\n' "$ordinal" "$total" "$test_name"
  if run_exact_test "$test_name"; then
    :
  else
    status=$?
    failures+=("$test_name")
    if [[ "$keep_going" -eq 0 ]]; then
      exit "$status"
    fi
  fi
done

if [[ "${#failures[@]}" -gt 0 ]]; then
  printf 'Failed tests:\n' >&2
  printf '  %s\n' "${failures[@]}" >&2
  exit 1
fi

echo "All selected instruction runtime tests passed."
