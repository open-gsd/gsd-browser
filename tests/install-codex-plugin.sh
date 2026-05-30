#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
INSTALLER="$PROJECT_DIR/install.sh"
TMP_ROOT="$(mktemp -d)"
REAL_NODE="$(command -v node || true)"

cleanup() {
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

load_installer_functions() {
  sed '/^main "\$@"/,$d' "$INSTALLER" > "$1"
}

write_fake_curl() {
  local bin_dir="$1"
  cat > "$bin_dir/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      out="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

[ -n "$out" ] || exit 1
mkdir -p "$(dirname "$out")"
case "$out" in
  */plugin.json*)
    printf '{"name":"gsd-browser","skills":"./skills/"}\n' > "$out"
    ;;
  *)
    printf 'fake skill content\n' > "$out"
    ;;
esac
SH
  chmod +x "$bin_dir/curl"
}

test_interactive_codex_plugin_can_install_locally() {
  local load_file="$TMP_ROOT/install-functions.sh"
  local bin_dir="$TMP_ROOT/bin-interactive"
  local home_dir="$TMP_ROOT/home-interactive"
  local project_dir="$TMP_ROOT/project-interactive"
  mkdir -p "$bin_dir" "$home_dir/.agents" "$project_dir"
  load_installer_functions "$load_file"
  write_fake_curl "$bin_dir"

  printf '1\nl\n' | HOME="$home_dir" PATH="$bin_dir:/usr/bin:/bin" script -qfec "cd '$project_dir' && source '$load_file' && install_skill" /dev/null >/dev/null

  [ -f "$project_dir/plugins/gsd-browser/.codex-plugin/plugin.json" ] || fail "interactive Codex-only install did not create a local plugin"
  [ -f "$project_dir/.agents/plugins/marketplace.json" ] || fail "interactive Codex-only install did not create a local marketplace"
}

test_noninteractive_default_all_skips_codex_plugin() {
  local load_file="$TMP_ROOT/install-functions.sh"
  local bin_dir="$TMP_ROOT/bin-noninteractive"
  local home_dir="$TMP_ROOT/home-noninteractive"
  mkdir -p "$bin_dir" "$home_dir/.agents"
  load_installer_functions "$load_file"

  (
    HOME="$home_dir"
    PATH="$bin_dir"
    source "$load_file"
    read() { return 1; }
    install_codex_plugin() {
      fail "non-interactive default all installed the Codex plugin"
    }
    install_skill
  ) >/dev/null
}

test_marketplace_update_falls_back_to_node() {
  [ -n "$REAL_NODE" ] || fail "node is required for fallback test"

  local load_file="$TMP_ROOT/install-functions.sh"
  local bin_dir="$TMP_ROOT/bin-fallback"
  local home_dir="$TMP_ROOT/home-fallback"
  local marketplace="$TMP_ROOT/fallback/.agents/plugins/marketplace.json"
  mkdir -p "$bin_dir" "$home_dir"
  load_installer_functions "$load_file"

  cat > "$bin_dir/python3" <<'SH'
#!/usr/bin/env bash
exit 42
SH
  cat > "$bin_dir/node" <<SH
#!/usr/bin/env bash
exec "$REAL_NODE" "\$@"
SH
  chmod +x "$bin_dir/python3" "$bin_dir/node"

  (
    HOME="$home_dir"
    PATH="$bin_dir:/usr/bin:/bin"
    source "$load_file"
    name="$(update_codex_marketplace "$marketplace" "gsd-browser-local" "gsd-browser Local")"
    [ "$name" = "gsd-browser-local" ] || fail "Node fallback returned unexpected marketplace name: $name"
  )
}

test_local_plugin_registers_local_marketplace() {
  local load_file="$TMP_ROOT/install-functions.sh"
  local bin_dir="$TMP_ROOT/bin-local"
  local home_dir="$TMP_ROOT/home-local"
  local project_dir="$TMP_ROOT/project-local"
  local codex_log="$TMP_ROOT/codex.log"
  mkdir -p "$bin_dir" "$home_dir" "$project_dir"
  load_installer_functions "$load_file"
  write_fake_curl "$bin_dir"

  cat > "$bin_dir/codex" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$CODEX_LOG"
SH
  chmod +x "$bin_dir/codex"

  (
    HOME="$home_dir"
    PATH="$bin_dir:/usr/bin:/bin"
    CODEX_LOG="$codex_log"
    export CODEX_LOG
    cd "$project_dir"
    source "$load_file"
    install_codex_plugin "$PWD/plugins/$CODEX_PLUGIN_NAME" "$PWD/.agents/plugins/marketplace.json"
  ) >/dev/null

  grep -q '"name": "gsd-browser-local"' "$project_dir/.agents/plugins/marketplace.json" || fail "local marketplace was not given a local name"
  grep -q "^plugin marketplace add $project_dir$" "$codex_log" || fail "local marketplace root was not registered with Codex"
  grep -q '^plugin add gsd-browser@gsd-browser-local$' "$codex_log" || fail "Codex plugin add did not use the local marketplace name"
}

test_plugin_file_replacement_rolls_back_on_failure() {
  local load_file="$TMP_ROOT/install-functions.sh"
  local bin_dir="$TMP_ROOT/bin-atomic"
  local home_dir="$TMP_ROOT/home-atomic"
  local plugin_root="$TMP_ROOT/plugin-root"
  mkdir -p "$bin_dir" "$home_dir" "$plugin_root/skills/gsd-browser" "$plugin_root/.codex-plugin"
  load_installer_functions "$load_file"
  write_fake_curl "$bin_dir"
  printf 'old skill\n' > "$plugin_root/skills/gsd-browser/SKILL.md"
  printf 'old manifest\n' > "$plugin_root/.codex-plugin/plugin.json"

  (
    HOME="$home_dir"
    PATH="$bin_dir:/usr/bin:/bin"
    PLUGIN_ROOT="$plugin_root"
    export PLUGIN_ROOT
    source "$load_file"
    mv() {
      if [ "$#" -eq 2 ] && [ "$2" = "$PLUGIN_ROOT/skills/$CODEX_PLUGIN_NAME" ] && [[ "$1" == "$PLUGIN_ROOT/.skill."* ]]; then
        return 1
      fi
      command mv "$@"
    }
    if install_codex_plugin_files "$PLUGIN_ROOT" >/dev/null; then
      fail "plugin file install unexpectedly succeeded"
    fi
  )

  grep -q '^old skill$' "$plugin_root/skills/gsd-browser/SKILL.md" || fail "old skill was not restored after failed move"
  grep -q '^old manifest$' "$plugin_root/.codex-plugin/plugin.json" || fail "old manifest was not preserved after failed move"
}

test_interactive_codex_plugin_can_install_locally
test_noninteractive_default_all_skips_codex_plugin
test_marketplace_update_falls_back_to_node
test_local_plugin_registers_local_marketplace
test_plugin_file_replacement_rolls_back_on_failure

echo "install-codex-plugin.sh: all checks passed"
