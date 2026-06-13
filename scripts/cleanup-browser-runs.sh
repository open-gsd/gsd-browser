#!/usr/bin/env bash
set -euo pipefail

dry_run=0
include_test_runners=0
include_runner_shells=0
runner_command_filters=()
original_arg_count=$#
original_args=("$@")

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      dry_run=1
      shift
      ;;
    --include-test-runners)
      include_test_runners=1
      shift
      ;;
    --include-runner-shells)
      include_runner_shells=1
      shift
      ;;
    --results-file)
      runner_command_filters+=("${2:?missing value for --results-file}")
      shift 2
      ;;
    --runner-command-contains)
      runner_command_filters+=("${2:?missing value for --runner-command-contains}")
      shift 2
      ;;
    -h|--help)
      cat <<'USAGE'
Usage: scripts/cleanup-browser-runs.sh [--dry-run] [--include-test-runners] [--include-runner-shells]
                                      [--results-file PATH]
                                      [--runner-command-contains TEXT]

Stops managed gsd-browser daemon/Chrome processes from local test runs.
With --include-test-runners, also stops cargo/test processes for
cli/tests/instruction_runtime.rs. With --include-runner-shells, also stops
scripts/run-instruction-runtime.sh and direct shell wrapper processes that launch
instruction_runtime Cargo tests. Cleanup is PID-only so it does not kill the
invoking shell through shared terminal ownership.

With --results-file or --runner-command-contains, only matching runner shells are
targeted, together with their descendants. This is useful for stopping one stale
queued run without interrupting unrelated local validation work.
USAGE
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "${#runner_command_filters[@]}" -gt 0 ]]; then
  include_runner_shells=1
fi

pids=()
matched_runner_pids=()
stale_lock_dirs=()
state_session_dirs=()
targeted_mode=0
[[ "${#runner_command_filters[@]}" -gt 0 ]] && targeted_mode=1
skip_pids=" $$ ${BASHPID:-} "
ancestor="$$"
while [[ -n "$ancestor" && "$ancestor" != "0" ]]; do
  parent="$(ps -o ppid= -p "$ancestor" 2>/dev/null | tr -d ' ')"
  [[ -z "$parent" || "$parent" == "0" ]] && break
  skip_pids="$skip_pids $parent "
  ancestor="$parent"
done

first_word() {
  local command="$1"
  command="${command#"${command%%[![:space:]]*}"}"
  printf '%s\n' "${command%%[[:space:]]*}"
}

base_name() {
  local path="$1"
  printf '%s\n' "${path##*/}"
}

is_cargo_instruction_runtime_test() {
  local command="$1"
  local exe
  exe="$(base_name "$(first_word "$command")")"
  [[ "$exe" == "cargo" && "$command" == *" test "* && "$command" == *"--test instruction_runtime"* ]]
}

is_instruction_runtime_binary() {
  local command="$1"
  local exe
  exe="$(base_name "$(first_word "$command")")"
  [[ "$exe" == instruction_runtime-* ]]
}

is_instruction_runtime_runner() {
  local command="$1"
  local exe
  exe="$(base_name "$(first_word "$command")")"
  [[ "$command" == *"SkyComputerUseClient"* || "$command" == *" turn-ended "* ]] && return 1
  [[ "$exe" == "cleanup-browser-runs.sh" ]] && return 1
  [[ "$exe" =~ ^(bash|zsh|sh)$ &&
     "$command" == *"cargo test -p gsd-browser"* &&
     "$command" == *"--test instruction_runtime"* ]] && return 0
  [[ "$command" == *" --summary "* || "$command" == *" --list "* ]] && return 1
  [[ "$command" == *"scripts/run-instruction-runtime.sh"* ]] && return 0
  return 1
}

is_gsd_browser_daemon() {
  local command="$1"
  local exe
  exe="$(base_name "$(first_word "$command")")"
  [[ "$exe" == "gsd-browser" && "$command" == *" _serve --session "* ]]
}

is_managed_chrome() {
  local command="$1"
  [[ "$command" == *"Google Chrome.app/Contents/"* &&
     "$command" =~ --user-data-dir=[^[:space:]]*/gb-[^[:space:]]* ]]
}

add_managed_process() {
  local pid="$1"
  [[ -z "$pid" || " $skip_pids " == *" $pid "* ]] && return
  pids+=("$pid")
}

add_matched_runner_process() {
  local pid="$1"
  add_managed_process "$pid"
  [[ -z "$pid" || " $skip_pids " == *" $pid "* ]] && return
  matched_runner_pids+=("$pid")
}

runner_matches_filters() {
  local command="$1"
  local filter
  [[ "$targeted_mode" -eq 0 ]] && return 0
  for filter in "${runner_command_filters[@]}"; do
    [[ "$command" == *"$filter"* ]] && return 0
  done
  return 1
}

add_descendants() {
  local parent="$1"
  local child
  while IFS= read -r child; do
    [[ -z "$child" || " $skip_pids " == *" $child "* ]] && continue
    add_managed_process "$child"
    add_descendants "$child"
  done < <(pgrep -P "$parent" 2>/dev/null || true)
}

unique_numbers() {
  awk '!seen[$1]++'
}

live_pids() {
  local pid
  for pid in "$@"; do
    if kill -0 "$pid" 2>/dev/null; then
      printf '%s\n' "$pid"
    fi
  done
}

add_unique_dir() {
  local dir="$1"
  local existing
  [[ -z "$dir" ]] && return
  for existing in "${tmp_dirs[@]:-}"; do
    [[ "$existing" == "$dir" ]] && return
  done
  tmp_dirs+=("$dir")
}

collect_temp_dirs() {
  local root="$1"
  [[ -d "$root" ]] || return
  while IFS= read -r dir; do
    add_unique_dir "$dir"
  done < <(
    find "$root" -maxdepth 1 -type d \( \
      -name 'gb-instruction-*' -o \
      -name 'gb-debug-*' -o \
      -name 'gb-phone-debug-*' -o \
      -name 'gb-upload-*' -o \
      -name 'gb-debug-form-result-*' -o \
      -name 'gb-debug-command-surface-*' \
    \) -print 2>/dev/null
  )
}

collect_stale_locks() {
  local root="$1"
  local dir
  local owner_pid
  local command
  [[ -d "$root" ]] || return
  while IFS= read -r dir; do
    if [[ "$targeted_mode" -eq 1 ]]; then
      command=""
      [[ -f "$dir/command" ]] && command="$(cat "$dir/command" 2>/dev/null || true)"
      runner_matches_filters "$command" || continue
    fi
    owner_pid=""
    [[ -f "$dir/pid" ]] && owner_pid="$(cat "$dir/pid" 2>/dev/null || true)"
    if [[ ! "$owner_pid" =~ ^[0-9]+$ ]] || ! kill -0 "$owner_pid" 2>/dev/null; then
      stale_lock_dirs+=("$dir")
    fi
  done < <(find "$root" -maxdepth 1 -type d -name 'gsd-browser-instruction-runtime-*.lock' -print 2>/dev/null)
}

collect_instruction_state_sessions() {
  local sessions_root="${HOME:-}/.gsd-browser/sessions"
  [[ -d "$sessions_root" ]] || return
  while IFS= read -r dir; do
    state_session_dirs+=("$dir")
  done < <(find "$sessions_root" -maxdepth 1 -type d -name 'instruction-*' -print 2>/dev/null)
}

wait_for_exit() {
  local timeout="$1"
  shift
  local start
  local remaining_count
  start="$(date +%s)"
  while [[ "$(date +%s)" -lt $((start + timeout)) ]]; do
    remaining_count="$(live_pids "$@" | wc -l | tr -d ' ')"
    [[ "$remaining_count" -eq 0 ]] && return 0
    sleep 0.2
  done
  return 1
}

while IFS= read -r line; do
  read -r pid command <<<"$line"
  [[ -z "$pid" ]] && continue
  if [[ "$targeted_mode" -eq 1 ]]; then
    if [[ "$include_runner_shells" -eq 1 ]] && is_instruction_runtime_runner "$command" && runner_matches_filters "$command"; then
      add_matched_runner_process "$pid"
    fi
    continue
  fi
  if is_gsd_browser_daemon "$command"; then
    add_managed_process "$pid"
  elif is_managed_chrome "$command"; then
    add_managed_process "$pid"
  elif [[ "$include_test_runners" -eq 1 ]] && is_cargo_instruction_runtime_test "$command"; then
    add_managed_process "$pid"
  elif [[ "$include_test_runners" -eq 1 ]] && is_instruction_runtime_binary "$command"; then
    add_managed_process "$pid"
  elif [[ "$include_runner_shells" -eq 1 ]] && is_instruction_runtime_runner "$command" && runner_matches_filters "$command"; then
    add_matched_runner_process "$pid"
  fi
done < <(ps -axo pid=,command=)

if [[ "$targeted_mode" -eq 1 && "${#matched_runner_pids[@]}" -gt 0 ]]; then
  for pid in "${matched_runner_pids[@]}"; do
    add_descendants "$pid"
  done
fi

if [[ "${#pids[@]}" -gt 0 ]]; then
  unique_pids=()
  while IFS= read -r pid; do
    unique_pids+=("$pid")
  done < <(printf '%s\n' "${pids[@]}" | unique_numbers)
  pids=("${unique_pids[@]}")
fi
tmp_dirs=()
if [[ "$targeted_mode" -eq 0 ]]; then
  collect_temp_dirs "/tmp"
  collect_instruction_state_sessions
fi
collect_stale_locks "/tmp"
if [[ -n "${TMPDIR:-}" ]]; then
  tmp_root="${TMPDIR%/}"
  if [[ -n "$tmp_root" && "$tmp_root" != "/tmp" ]]; then
    if [[ "$targeted_mode" -eq 0 ]]; then
      collect_temp_dirs "$tmp_root"
    fi
    collect_stale_locks "$tmp_root"
  fi
fi

if [[ "${#pids[@]}" -eq 0 && "${#tmp_dirs[@]}" -eq 0 && "${#stale_lock_dirs[@]}" -eq 0 ]]; then
  if [[ "${#state_session_dirs[@]}" -eq 0 ]]; then
    echo "No managed browser run processes found."
    exit 0
  fi
fi

if [[ "${#pids[@]}" -eq 0 && "${#tmp_dirs[@]}" -eq 0 && "${#stale_lock_dirs[@]}" -eq 0 && "${#state_session_dirs[@]}" -eq 0 ]]; then
  echo "No managed browser run processes found."
  exit 0
fi

if [[ "${#pids[@]}" -gt 0 ]]; then
  printf 'Managed browser run processes: %s\n' "${pids[*]}"
fi
if [[ "${#tmp_dirs[@]}" -gt 0 ]]; then
  printf 'Managed browser temp dirs: %s\n' "${#tmp_dirs[@]}"
fi
if [[ "${#stale_lock_dirs[@]}" -gt 0 ]]; then
  printf 'Stale instruction runtime locks: %s\n' "${#stale_lock_dirs[@]}"
fi
if [[ "${#state_session_dirs[@]}" -gt 0 ]]; then
  printf 'Instruction runtime state sessions: %s\n' "${#state_session_dirs[@]}"
fi
if [[ "$dry_run" -eq 1 ]]; then
  exit 0
fi

if [[ "${#pids[@]}" -gt 0 ]]; then
  kill -TERM "${pids[@]}" 2>/dev/null || true
fi
if [[ "${#pids[@]}" -gt 0 ]] && ! wait_for_exit 5 "${pids[@]}"; then
  remaining=()
  while IFS= read -r pid; do
    remaining+=("$pid")
  done < <(live_pids "${pids[@]}")
  if [[ "${#remaining[@]}" -gt 0 ]]; then
    kill -KILL "${remaining[@]}" 2>/dev/null || true
    wait_for_exit 2 "${remaining[@]}" || true
  fi
fi
if [[ "${#tmp_dirs[@]}" -gt 0 ]]; then
  rm -rf "${tmp_dirs[@]}" 2>/dev/null || true
fi
if [[ "${#stale_lock_dirs[@]}" -gt 0 ]]; then
  rm -rf "${stale_lock_dirs[@]}" 2>/dev/null || true
fi
if [[ "${#state_session_dirs[@]}" -gt 0 ]]; then
  rm -rf "${state_session_dirs[@]}" 2>/dev/null || true
fi
if [[ "$dry_run" -eq 0 && "${GSD_BROWSER_CLEANUP_RESWEEP:-0}" != "1" ]]; then
  sleep 1
  if [[ "$original_arg_count" -gt 0 ]]; then
    GSD_BROWSER_CLEANUP_RESWEEP=1 "$0" "${original_args[@]}" || true
  else
    GSD_BROWSER_CLEANUP_RESWEEP=1 "$0" || true
  fi
fi
echo "Cleanup complete."
