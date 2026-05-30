#!/usr/bin/env bash
set -euo pipefail

# gsd-browser installer
# Usage: curl -fsSL https://raw.githubusercontent.com/open-gsd/gsd-browser/main/install.sh | bash
#
# For AI agents (recommended 2026+ path): After install, run `gsd-browser mcp` as your MCP server.
#   - Tailored quickstart: $INSTALL_DIR/scripts/mcp-quickstart.sh cursor (or claude/vscode/generic)
#   - Full docs: docs/mcp.md + docs/AGENT-BEST-PRACTICES.md (50+ tools, live resources, executable prompts, refs, evidence bundles, human collaboration via viewer, self-healing action cache, batch, etc.)
#   - Example config: docs/examples/mcp-client-config.json
#
# The installer also offers to install the curated gsd-browser-skill/ pack for coding agents (Claude Code, Codex, Gemini CLI, etc.).
#
# Environment variables (advanced):
#   GSD_BROWSER_VERSION       - install a specific version (default: latest)
#   GSD_BROWSER_REPO          - repo for source files and release binaries (default: open-gsd/gsd-browser)
#   GSD_BROWSER_DIR           - override install directory (default: $HOME/.gsd-browser)
#   GSD_BROWSER_INSTALL_MODE  - install or update (default: install)
#   GSD_BROWSER_SKIP_CHROMIUM - skip Chrome/Chromium setup when set to 1
#   GSD_BROWSER_SKIP_SKILL    - skip AI agent skill installation when set to 1
#   GSD_BROWSER_INSTALL_CODEX_PLUGIN - install the Codex Plugin when set to 1

VERSION="${GSD_BROWSER_VERSION:-latest}"
REPO="${GSD_BROWSER_REPO:-open-gsd/gsd-browser}"
INSTALL_DIR="${GSD_BROWSER_DIR:-$HOME/.gsd-browser}"
BIN_DIR="$INSTALL_DIR/bin"
CHROMIUM_DIR="$INSTALL_DIR/chromium"
INSTALL_MODE="${GSD_BROWSER_INSTALL_MODE:-install}"
SKIP_CHROMIUM="${GSD_BROWSER_SKIP_CHROMIUM:-0}"
SKIP_SKILL="${GSD_BROWSER_SKIP_SKILL:-0}"
INSTALL_CODEX_PLUGIN="${GSD_BROWSER_INSTALL_CODEX_PLUGIN:-0}"

# Colors
cyan="\033[36m"
green="\033[32m"
yellow="\033[33m"
red="\033[31m"
dim="\033[2m"
bold="\033[1m"
reset="\033[0m"

banner() {
  echo ""
  printf "${cyan}${bold}"
  echo "   ██████╗ ███████╗██████╗       ██████╗ ██████╗  ██████╗ ██╗    ██╗███████╗███████╗██████╗ "
  echo "  ██╔════╝ ██╔════╝██╔══██╗      ██╔══██╗██╔══██╗██╔═══██╗██║    ██║██╔════╝██╔════╝██╔══██╗"
  echo "  ██║  ███╗███████╗██║  ██║█████╗██████╔╝██████╔╝██║   ██║██║ █╗ ██║███████╗█████╗  ██████╔╝"
  echo "  ██║   ██║╚════██║██║  ██║╚════╝██╔══██╗██╔══██╗██║   ██║██║███╗██║╚════██║██╔══╝  ██╔══██╗"
  echo "  ╚██████╔╝███████║██████╔╝      ██████╔╝██║  ██║╚██████╔╝╚███╔███╔╝███████║███████╗██║  ██║"
  echo "   ╚═════╝ ╚══════╝╚═════╝       ╚═════╝ ╚═╝  ╚═╝  ╚═════╝  ╚══╝╚══╝ ╚══════╝╚══════╝╚═╝  ╚═╝"
  printf "${reset}\n"
  printf "  ${dim}Browser automation for AI agents${reset}\n\n"
}

info()  { printf "  ${cyan}>${reset} %s\n" "$1"; }
ok()    { printf "  ${green}✓${reset} %s\n" "$1"; }
warn()  { printf "  ${yellow}!${reset} %s\n" "$1"; }
fail()  { printf "  ${red}✗${reset} %s\n" "$1"; exit 1; }

parse_args() {
  for arg in "$@"; do
    case "$arg" in
      --update)
        INSTALL_MODE="update"
        SKIP_CHROMIUM=1
        SKIP_SKILL=1
        ;;
      --skip-chromium)
        SKIP_CHROMIUM=1
        ;;
      --skip-skill)
        SKIP_SKILL=1
        ;;
      --codex-plugin)
        INSTALL_CODEX_PLUGIN=1
        ;;
      *)
        fail "Unknown option: $arg"
        ;;
    esac
  done

  case "$INSTALL_MODE" in
    install|update) ;;
    *) fail "Invalid GSD_BROWSER_INSTALL_MODE: $INSTALL_MODE" ;;
  esac
}

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin) os="darwin" ;;
    Linux)  os="linux"  ;;
    *) fail "Unsupported OS: $os (Windows users: use WSL)" ;;
  esac

  case "$arch" in
    x86_64|amd64)  arch="x64"   ;;
    arm64|aarch64) arch="arm64" ;;
    *) fail "Unsupported architecture: $arch" ;;
  esac

  PLATFORM="${os}-${arch}"

  # Chrome for Testing platform names (different from ours)
  case "$PLATFORM" in
    darwin-arm64) CHROME_PLATFORM="mac-arm64" ;;
    darwin-x64)   CHROME_PLATFORM="mac-x64"   ;;
    linux-x64)    CHROME_PLATFORM="linux64"    ;;
    linux-arm64)  CHROME_PLATFORM=""           ;; # Not available
    *) CHROME_PLATFORM="" ;;
  esac
}

resolve_version() {
  if [ "$VERSION" = "latest" ]; then
    info "Fetching latest release..."
    VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed -E 's/.*"v?([^"]+)".*/\1/')
    if [ -z "$VERSION" ]; then
      fail "Could not determine latest version from $REPO. Set GSD_BROWSER_VERSION manually."
    fi
  fi
  ok "Version: $VERSION"
}

download_binary() {
  local url filename
  filename="gsd-browser-${PLATFORM}"
  url="https://github.com/$REPO/releases/download/v${VERSION}/${filename}"

  info "Downloading gsd-browser for $PLATFORM..."
  mkdir -p "$BIN_DIR"

  local target="$BIN_DIR/gsd-browser"
  local tmp
  tmp="$(mktemp "$BIN_DIR/.gsd-browser.XXXXXX")"
  trap 'rm -f "$tmp"' EXIT

  if ! curl -fsSL -o "$tmp" "$url"; then
    rm -f "$tmp"
    fail "Download failed: $url"
  fi

  chmod +x "$tmp"
  mv -f "$tmp" "$target"
  chmod +x "$target"

  # On macOS, strip quarantine/provenance attributes added by curl, then verify
  # the Developer ID signature. Fall back to ad-hoc signing only if the binary
  # has no valid signature (manual builds, stripped signatures).
  if [ "$(uname -s)" = "Darwin" ]; then
    xattr -d com.apple.quarantine "$target" 2>/dev/null || true
    xattr -d com.apple.provenance "$target" 2>/dev/null || true

    if command -v codesign >/dev/null 2>&1; then
      if codesign --verify --strict "$target" 2>/dev/null; then
        if "$target" --version >/dev/null 2>&1; then
          ok "Binary signature verified (Developer ID)"
        else
          warn "Developer ID signature verifies but macOS refused to launch the binary"
          codesign --sign - --force "$target" 2>/dev/null && "$target" --version >/dev/null 2>&1 && {
            ok "Ad-hoc signed for macOS Gatekeeper"
          } || {
            warn "Ad-hoc signing failed — binary may be blocked by Gatekeeper"
            warn "Fix manually: codesign --sign - --force $target"
          }
        fi
      else
        codesign --sign - --force "$target" 2>/dev/null && {
          ok "Ad-hoc signed for macOS Gatekeeper"
        } || {
          warn "Ad-hoc signing failed — binary may be blocked by Gatekeeper"
          warn "Fix manually: codesign --sign - --force $target"
        }
      fi
    fi
  fi

  trap - EXIT

  ok "Binary installed: $target"
}

detect_existing_chrome() {
  # Check standard Chrome/Chromium locations — mirrors find_chrome() in common/src/chrome.rs
  local candidates=()

  case "$(uname -s)" in
    Darwin)
      candidates=(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
      )
      ;;
    Linux)
      candidates=(
        "/usr/bin/google-chrome"
        "/usr/bin/google-chrome-stable"
        "/usr/bin/chromium-browser"
        "/usr/bin/chromium"
        "/snap/bin/chromium"
      )
      ;;
  esac

  for path in "${candidates[@]}"; do
    if [ -x "$path" ]; then
      echo "$path"
      return 0
    fi
  done

  # Check PATH
  for name in google-chrome google-chrome-stable chromium-browser chromium; do
    if command -v "$name" >/dev/null 2>&1; then
      command -v "$name"
      return 0
    fi
  done

  return 1
}

download_chromium() {
  if [ "$SKIP_CHROMIUM" = "1" ]; then
    info "Skipping Chrome/Chromium setup"
    return
  fi

  if [ -z "$CHROME_PLATFORM" ]; then
    warn "Chromium not available for $PLATFORM via Chrome for Testing"
    warn "Install Chrome/Chromium manually and set GSD_BROWSER_BROWSER_PATH"
    return
  fi

  # Check if a system Chrome/Chromium is already available
  local system_chrome
  if system_chrome=$(detect_existing_chrome); then
    ok "Found Chrome at: $system_chrome (skipping download)"
    SKIP_CHROMIUM_CONFIG=1
    return
  fi

  # Check if we previously downloaded Chrome for Testing
  if [ -f "$CHROMIUM_DIR/version" ]; then
    local existing
    existing=$(cat "$CHROMIUM_DIR/version")
    info "Chromium $existing already installed"
    read -rp "  Reinstall? [y/N] " yn < /dev/tty || yn="n"
    case "$yn" in
      [Yy]*) ;;
      *) ok "Keeping existing Chromium"; return ;;
    esac
  fi

  info "Fetching Chrome for Testing metadata..."
  local json chrome_url chrome_version
  json=$(curl -fsSL "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json")

  chrome_version=$(echo "$json" | grep -o '"Stable"[^}]*"version":"[^"]*"' | head -1 | grep -o '"version":"[^"]*"' | cut -d'"' -f4)
  chrome_url=$(echo "$json" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for entry in data['channels']['Stable']['downloads']['chrome']:
    if entry['platform'] == '$CHROME_PLATFORM':
        print(entry['url'])
        break
" 2>/dev/null || echo "")

  if [ -z "$chrome_url" ]; then
    warn "Could not find Chromium download for $CHROME_PLATFORM"
    return
  fi

  info "Downloading Chromium $chrome_version for $CHROME_PLATFORM..."
  mkdir -p "$CHROMIUM_DIR"

  local tmpzip="$CHROMIUM_DIR/chrome.zip"
  if ! curl -fsSL -o "$tmpzip" "$chrome_url"; then
    warn "Chromium download failed — you can install Chrome manually"
    rm -f "$tmpzip"
    return
  fi

  info "Extracting Chromium..."
  unzip -qo "$tmpzip" -d "$CHROMIUM_DIR"
  rm -f "$tmpzip"

  # Find the chrome binary inside the extracted directory
  local chrome_bin=""
  case "$CHROME_PLATFORM" in
    mac-arm64|mac-x64)
      chrome_bin=$(find "$CHROMIUM_DIR" -name "Google Chrome for Testing" -type f 2>/dev/null | head -1)
      if [ -z "$chrome_bin" ]; then
        chrome_bin=$(find "$CHROMIUM_DIR" -name "Chromium" -type f 2>/dev/null | head -1)
      fi
      ;;
    linux64)
      chrome_bin=$(find "$CHROMIUM_DIR" -name "chrome" -type f 2>/dev/null | head -1)
      ;;
  esac

  if [ -n "$chrome_bin" ]; then
    echo "$chrome_version" > "$CHROMIUM_DIR/version"
    echo "$chrome_bin" > "$CHROMIUM_DIR/binary_path"
    ok "Chromium $chrome_version installed"
  else
    warn "Chromium extracted but binary not found — check $CHROMIUM_DIR"
  fi
}

setup_path() {
  local target="/usr/local/bin/gsd-browser"
  local source="$BIN_DIR/gsd-browser"

  # Try symlink to /usr/local/bin
  if [ -w "/usr/local/bin" ] || [ -w "$(dirname /usr/local/bin)" ]; then
    ln -sf "$source" "$target" 2>/dev/null && {
      ok "Linked to $target"
      return
    }
  fi

  # Try with sudo
  if command -v sudo >/dev/null 2>&1; then
    info "Linking to /usr/local/bin (may need password)..."
    sudo ln -sf "$source" "$target" 2>/dev/null && {
      ok "Linked to $target"
      return
    }
  fi

  # Fallback: tell user to add to PATH
  warn "Could not link to /usr/local/bin"
  echo ""
  printf "  Add this to your shell profile:\n"
  printf "  ${bold}export PATH=\"$BIN_DIR:\$PATH\"${reset}\n"
  echo ""
}

write_config() {
  # If system Chrome was detected, no config needed — find_chrome() will discover it
  if [ "${SKIP_CHROMIUM_CONFIG:-}" = "1" ]; then
    return
  fi

  # If we downloaded Chromium, write a config pointing to it
  if [ -f "$CHROMIUM_DIR/binary_path" ]; then
    local chrome_path
    chrome_path=$(cat "$CHROMIUM_DIR/binary_path")
    local config_dir="$HOME/.gsd-browser"
    mkdir -p "$config_dir"
    cat > "$config_dir/config.toml" << TOML
[browser]
path = "$chrome_path"
TOML
    ok "Config written: $config_dir/config.toml"
  fi
}

verify() {
  info "Verifying installation..."

  local bin=""
  if ! command -v gsd-browser >/dev/null 2>&1; then
    # Try the direct path
    if [ -x "$BIN_DIR/gsd-browser" ]; then
      bin="$BIN_DIR/gsd-browser"
    else
      warn "gsd-browser not in PATH yet — restart your shell or run: export PATH=\"$BIN_DIR:\$PATH\""
      return
    fi
  else
    bin="gsd-browser"
  fi

  "$bin" daemon stop >/dev/null 2>&1 || true

  if "$bin" daemon start >/dev/null 2>&1; then
    local health_output
    health_output=$("$bin" daemon health 2>&1 || true)
    if printf '%s\n' "$health_output" | grep -q '^Daemon: healthy$'; then
      if [ "$bin" = "$BIN_DIR/gsd-browser" ]; then
        ok "Daemon healthy (using direct path)"
      else
        ok "Daemon healthy"
      fi
      "$bin" daemon stop >/dev/null 2>&1 || true
      return
    fi
  fi

  "$bin" daemon stop >/dev/null 2>&1 || true
  warn "Daemon test failed — this is normal if Chrome is not in the default path"
  warn "Set browser.path in ~/.gsd-browser/config.toml"
}

SKILL_REPO_BASE="https://raw.githubusercontent.com/$REPO/main/gsd-browser-skill"
SKILL_FILES=(
  "SKILL.md"
  "references/command-reference.md"
  "references/snapshot-and-refs.md"
  "references/semantic-intents.md"
  "references/configuration.md"
  "references/error-recovery.md"
  "workflows/scrape-and-extract.md"
  "workflows/setup-and-configure.md"
  "workflows/live-viewer-and-narration.md"
  "workflows/navigate-and-interact.md"
  "workflows/test-and-assert.md"
  "workflows/debug-and-diagnose.md"
)

CODEX_PLUGIN_NAME="gsd-browser"
CODEX_PLUGIN_REPO_BASE="https://raw.githubusercontent.com/$REPO/main/codex-plugin"
CODEX_PLUGIN_ROOT="$HOME/plugins/$CODEX_PLUGIN_NAME"
CODEX_MARKETPLACE_PATH="$HOME/.agents/plugins/marketplace.json"

install_skill_to() {
  local dest="$1"
  mkdir -p "$dest" "$dest/references" "$dest/workflows"

  local failed=0
  for file in "${SKILL_FILES[@]}"; do
    if ! curl -fsSL -o "$dest/$file" "$SKILL_REPO_BASE/$file" 2>/dev/null; then
      failed=1
      break
    fi
  done

  if [ "$failed" -eq 1 ]; then
    warn "Failed to download skill files — check network or repo access"
    rm -rf "$dest"
    return 1
  fi

  ok "Skill installed: $dest"
  return 0
}

install_codex_plugin_files() {
  local plugin_root="$1"
  mkdir -p "$plugin_root/.codex-plugin"

  local manifest_tmp
  manifest_tmp="$(mktemp "$plugin_root/.codex-plugin/plugin.json.XXXXXX")"
  if ! curl -fsSL -o "$manifest_tmp" "$CODEX_PLUGIN_REPO_BASE/.codex-plugin/plugin.json" 2>/dev/null; then
    rm -f "$manifest_tmp"
    warn "Failed to download Codex plugin manifest — check network or repo access"
    return 1
  fi

  local skill_tmp
  skill_tmp="$(mktemp -d "$plugin_root/.skill.XXXXXX")"
  if ! install_skill_to "$skill_tmp"; then
    rm -f "$manifest_tmp"
    rm -rf "$skill_tmp"
    return 1
  fi

  mkdir -p "$plugin_root/skills"
  local skill_dest="$plugin_root/skills/$CODEX_PLUGIN_NAME"
  local manifest_dest="$plugin_root/.codex-plugin/plugin.json"
  local skill_backup=""
  local manifest_backup=""

  if [ -e "$skill_dest" ] || [ -L "$skill_dest" ]; then
    skill_backup="$(mktemp -d "$plugin_root/skills/.${CODEX_PLUGIN_NAME}.backup.XXXXXX")"
    rmdir "$skill_backup"
    if ! mv "$skill_dest" "$skill_backup"; then
      rm -f "$manifest_tmp"
      rm -rf "$skill_tmp"
      return 1
    fi
  fi

  if [ -e "$manifest_dest" ] || [ -L "$manifest_dest" ]; then
    manifest_backup="$(mktemp "$plugin_root/.codex-plugin/plugin.json.backup.XXXXXX")"
    if ! mv -f "$manifest_dest" "$manifest_backup"; then
      [ -n "$skill_backup" ] && mv "$skill_backup" "$skill_dest"
      rm -f "$manifest_tmp"
      rm -rf "$skill_tmp"
      return 1
    fi
  fi

  if ! mv "$skill_tmp" "$skill_dest"; then
    [ -n "$skill_backup" ] && mv "$skill_backup" "$skill_dest"
    [ -n "$manifest_backup" ] && mv "$manifest_backup" "$manifest_dest"
    rm -f "$manifest_tmp"
    rm -rf "$skill_tmp"
    return 1
  fi

  if ! mv -f "$manifest_tmp" "$manifest_dest"; then
    rm -rf "$skill_dest"
    [ -n "$skill_backup" ] && mv "$skill_backup" "$skill_dest"
    [ -n "$manifest_backup" ] && mv "$manifest_backup" "$manifest_dest"
    rm -f "$manifest_tmp"
    rm -rf "$skill_tmp"
    return 1
  fi

  if [ -n "$skill_backup" ]; then
    rm -rf "$skill_backup"
  fi
  if [ -n "$manifest_backup" ]; then
    rm -f "$manifest_backup"
  fi
  return 0
}

codex_marketplace_updater_available() {
  command -v python3 >/dev/null 2>&1 || command -v node >/dev/null 2>&1
}

update_codex_marketplace() {
  local marketplace_path="$1"
  local default_marketplace_name="${2:-personal}"
  local default_display_name="${3:-Personal}"
  local python_status=127

  if command -v python3 >/dev/null 2>&1; then
    if python3 - "$marketplace_path" "$CODEX_PLUGIN_NAME" "$default_marketplace_name" "$default_display_name" <<'PY'
import json
import os
import sys
import tempfile
from pathlib import Path

path = Path(sys.argv[1]).expanduser()
plugin_name = sys.argv[2]
default_name = sys.argv[3]
default_display_name = sys.argv[4]
entry = {
    "name": plugin_name,
    "source": {"source": "local", "path": f"./plugins/{plugin_name}"},
    "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"},
    "category": "Coding",
}

if path.exists():
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"Unable to parse {path}: {exc}", file=sys.stderr)
        raise SystemExit(2)
    if not isinstance(payload, dict):
        print(f"{path} must contain a JSON object", file=sys.stderr)
        raise SystemExit(2)
else:
    payload = {"name": default_name, "interface": {"displayName": default_display_name}, "plugins": []}

name = payload.get("name")
name_changed = False
if not isinstance(name, str) or not name.strip() or (default_name != "personal" and name == "personal"):
    name = default_name
    payload["name"] = name
    name_changed = True

interface = payload.get("interface")
if not isinstance(interface, dict):
    interface = {}
    payload["interface"] = interface
display_name = interface.get("displayName")
if not isinstance(display_name, str) or not display_name.strip() or (name_changed and display_name == "Personal"):
    interface["displayName"] = default_display_name if name == default_name else name

plugins = payload.get("plugins")
if plugins is None:
    plugins = []
    payload["plugins"] = plugins
if not isinstance(plugins, list):
    print(f"{path} field 'plugins' must be an array", file=sys.stderr)
    raise SystemExit(2)

for index, plugin in enumerate(plugins):
    if isinstance(plugin, dict) and plugin.get("name") == plugin_name:
        plugins[index] = entry
        break
else:
    plugins.append(entry)

path.parent.mkdir(parents=True, exist_ok=True)
data = json.dumps(payload, indent=2) + "\n"
fd, tmp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=str(path.parent))
try:
    with os.fdopen(fd, "w", encoding="utf-8") as tmp_file:
        tmp_file.write(data)
        tmp_file.flush()
        os.fsync(tmp_file.fileno())
    os.replace(tmp_name, path)
finally:
    if os.path.exists(tmp_name):
        os.unlink(tmp_name)
print(name)
PY
    then
      return 0
    fi
    python_status=$?
  fi

  if command -v node >/dev/null 2>&1; then
    node - "$marketplace_path" "$CODEX_PLUGIN_NAME" "$default_marketplace_name" "$default_display_name" <<'JS'
const fs = require("fs");
const path = require("path");

const marketplacePath = process.argv[2];
const pluginName = process.argv[3];
const defaultName = process.argv[4];
const defaultDisplayName = process.argv[5];
const entry = {
  name: pluginName,
  source: { source: "local", path: `./plugins/${pluginName}` },
  policy: { installation: "AVAILABLE", authentication: "ON_INSTALL" },
  category: "Coding",
};

let payload;
if (fs.existsSync(marketplacePath)) {
  try {
    payload = JSON.parse(fs.readFileSync(marketplacePath, "utf8"));
  } catch (error) {
    console.error(`Unable to parse ${marketplacePath}: ${error.message}`);
    process.exit(2);
  }
  if (!payload || Array.isArray(payload) || typeof payload !== "object") {
    console.error(`${marketplacePath} must contain a JSON object`);
    process.exit(2);
  }
} else {
  payload = { name: defaultName, interface: { displayName: defaultDisplayName }, plugins: [] };
}

let nameChanged = false;
if (typeof payload.name !== "string" || payload.name.trim() === "" || (defaultName !== "personal" && payload.name === "personal")) {
  payload.name = defaultName;
  nameChanged = true;
}
if (!payload.interface || Array.isArray(payload.interface) || typeof payload.interface !== "object") {
  payload.interface = {};
}
if (
  typeof payload.interface.displayName !== "string" ||
  payload.interface.displayName.trim() === "" ||
  (nameChanged && payload.interface.displayName === "Personal")
) {
  payload.interface.displayName = payload.name === defaultName ? defaultDisplayName : payload.name;
}
if (payload.plugins == null) {
  payload.plugins = [];
}
if (!Array.isArray(payload.plugins)) {
  console.error(`${marketplacePath} field 'plugins' must be an array`);
  process.exit(2);
}

const existing = payload.plugins.findIndex((plugin) => plugin && plugin.name === pluginName);
if (existing >= 0) {
  payload.plugins[existing] = entry;
} else {
  payload.plugins.push(entry);
}

const marketplaceDir = path.dirname(marketplacePath);
const marketplaceTmp = path.join(
  marketplaceDir,
  `.${path.basename(marketplacePath)}.${process.pid}.${Date.now()}.tmp`,
);
fs.mkdirSync(marketplaceDir, { recursive: true });
try {
  const fd = fs.openSync(marketplaceTmp, "w");
  try {
    fs.writeFileSync(fd, `${JSON.stringify(payload, null, 2)}\n`);
    fs.fsyncSync(fd);
  } finally {
    fs.closeSync(fd);
  }
  fs.renameSync(marketplaceTmp, marketplacePath);
} catch (error) {
  try {
    fs.unlinkSync(marketplaceTmp);
  } catch (_) {
    // Best effort cleanup; preserve the original marketplace on failure.
  }
  throw error;
}
console.log(payload.name);
JS
    return $?
  fi

  if [ "$python_status" -ne 127 ]; then
    return "$python_status"
  fi
  printf "Python 3 or Node.js is required to update Codex marketplace.json automatically\n" >&2
  return 1
}

install_codex_plugin() {
  local plugin_root="${1:-$CODEX_PLUGIN_ROOT}"
  local marketplace_path="${2:-$CODEX_MARKETPLACE_PATH}"
  local default_marketplace_name="personal"
  local default_display_name="Personal"

  info "Installing Codex Plugin → $plugin_root"
  if ! codex_marketplace_updater_available; then
    printf "Python 3 or Node.js is required to update Codex marketplace.json automatically\n" >&2
    return 1
  fi

  if ! install_codex_plugin_files "$plugin_root"; then
    return 1
  fi
  ok "Codex Plugin files installed: $plugin_root"

  if [ "$marketplace_path" != "$CODEX_MARKETPLACE_PATH" ]; then
    default_marketplace_name="$CODEX_PLUGIN_NAME-local"
    default_display_name="gsd-browser Local"
  fi

  local marketplace_name
  if ! marketplace_name="$(update_codex_marketplace "$marketplace_path" "$default_marketplace_name" "$default_display_name")"; then
    warn "Codex Plugin files were installed, but marketplace update failed"
    return 1
  fi
  ok "Codex marketplace updated: $marketplace_path"

  if command -v codex >/dev/null 2>&1; then
    local can_add_plugin=1
    if [ "$marketplace_path" != "$CODEX_MARKETPLACE_PATH" ]; then
      local marketplace_root="$marketplace_path"
      case "$marketplace_path" in
        */.agents/plugins/marketplace.json)
          marketplace_root="${marketplace_path%/.agents/plugins/marketplace.json}"
          ;;
      esac
      if codex plugin marketplace add "$marketplace_root" >/dev/null 2>&1; then
        ok "Codex marketplace registered: $marketplace_root"
      else
        can_add_plugin=0
        warn "Marketplace updated, but 'codex plugin marketplace add $marketplace_root' failed"
        warn "Open the Codex plugin UI or run that command after Codex is available"
      fi
    fi

    if [ "$can_add_plugin" -eq 1 ]; then
      if codex plugin add "$CODEX_PLUGIN_NAME@$marketplace_name" >/dev/null 2>&1; then
        ok "Codex Plugin installed: $CODEX_PLUGIN_NAME@$marketplace_name"
      else
        warn "Marketplace updated, but 'codex plugin add $CODEX_PLUGIN_NAME@$marketplace_name' failed"
        warn "Open the Codex plugin UI or run that command after Codex is available"
      fi
    fi
  else
    info "Codex CLI not found; install from the Codex plugin UI when Codex is available"
  fi
}

install_skill() {
  if [ "$INSTALL_CODEX_PLUGIN" = "1" ]; then
    install_codex_plugin
    return
  fi

  if [ "$SKIP_SKILL" = "1" ]; then
    info "Skipping skill installation"
    return
  fi

  echo ""
  printf "  ${cyan}${bold}AI Agent Integration Installation${reset}\n"
  printf "  ${dim}Install gsd-browser support so your AI coding agent knows how to use it${reset}\n"
  echo ""

  # Detect available AI CLIs
  local available=()
  local labels=()

  if command -v claude >/dev/null 2>&1; then
    available+=("claude")
    labels+=("Claude Code")
  fi
  if command -v codex >/dev/null 2>&1; then
    available+=("codex")
    labels+=("OpenAI Codex CLI Skill")
  fi
  if command -v codex >/dev/null 2>&1 || [ -d "$HOME/.codex" ] || [ -d "$HOME/.agents" ]; then
    available+=("codex-plugin")
    labels+=("OpenAI Codex Plugin")
  fi
  if command -v gemini >/dev/null 2>&1; then
    available+=("gemini")
    labels+=("Google Gemini CLI")
  fi

  if [ ${#available[@]} -eq 0 ]; then
    info "No AI coding agents detected (claude, codex, gemini)"
    info "Install one, then re-run this installer"
    return
  fi

  printf "  Detected AI agents:\n"
  local idx=1
  for label in "${labels[@]}"; do
    printf "    ${bold}%d)${reset} %s\n" "$idx" "$label"
    idx=$((idx + 1))
  done
  printf "    ${bold}a)${reset} All detected agents\n"
  printf "    ${bold}s)${reset} Skip\n"
  echo ""

  local choice
  local prompted=1
  read -rp "  Install support for which agent(s)? [a]: " choice < /dev/tty || {
    choice="a"
    prompted=0
  }
  choice="${choice:-a}"

  if [ "$choice" = "s" ] || [ "$choice" = "S" ]; then
    info "Skipping AI agent integration installation"
    return
  fi

  # Build list of selected tools
  local selected=()
  if [ "$choice" = "a" ] || [ "$choice" = "A" ]; then
    for tool in "${available[@]}"; do
      if [ "$prompted" -eq 0 ] && [ "$tool" = "codex-plugin" ]; then
        continue
      fi
      selected+=("$tool")
    done
  else
    # Parse comma-separated or single number
    IFS=',' read -ra nums <<< "$choice"
    for num in "${nums[@]}"; do
      num=$(echo "$num" | tr -d ' ')
      if [[ "$num" =~ ^[0-9]+$ ]] && [ "$num" -ge 1 ] && [ "$num" -le ${#available[@]} ]; then
        selected+=("${available[$((num - 1))]}")
      fi
    done
  fi

  if [ ${#selected[@]} -eq 0 ]; then
    warn "No valid selection — skipping AI agent integration installation"
    return
  fi

  # Ask scope
  local needs_scope=1

  local scope="g"
  if [ "$needs_scope" -eq 1 ]; then
    echo ""
    printf "  Install scope:\n"
    printf "    ${bold}g)${reset} Global (available in all projects)\n"
    printf "    ${bold}l)${reset} Local  (current directory only)\n"
    echo ""
    read -rp "  Scope? [g]: " scope < /dev/tty || scope="g"
    scope="${scope:-g}"
  fi

  for tool in "${selected[@]}"; do
    local dest=""
    case "$tool" in
      claude)
        if [ "$scope" = "l" ] || [ "$scope" = "L" ]; then
          dest=".claude/skills/gsd-browser"
        else
          dest="$HOME/.claude/skills/gsd-browser"
        fi
        ;;
      codex)
        if [ "$scope" = "l" ] || [ "$scope" = "L" ]; then
          dest=".codex/skills/gsd-browser"
        else
          dest="$HOME/.codex/skills/gsd-browser"
        fi
        ;;
      codex-plugin)
        if [ "$scope" = "l" ] || [ "$scope" = "L" ]; then
          install_codex_plugin "$PWD/plugins/$CODEX_PLUGIN_NAME" "$PWD/.agents/plugins/marketplace.json"
        else
          install_codex_plugin
        fi
        continue
        ;;
      gemini)
        if [ "$scope" = "l" ] || [ "$scope" = "L" ]; then
          dest=".gemini/skills/gsd-browser"
        else
          dest="$HOME/.gemini/skills/gsd-browser"
        fi
        ;;
    esac

    if [ -n "$dest" ]; then
      info "Installing skill for $tool → $dest"
      install_skill_to "$dest"
    fi
  done
}

main() {
  parse_args "$@"
  if [ "$INSTALL_MODE" = "update" ]; then
    echo ""
    printf "  ${cyan}${bold}Updating gsd-browser${reset}\n\n"
  else
    banner
  fi
  detect_platform
  ok "Platform: $PLATFORM"
  resolve_version
  download_binary

  if [ "$INSTALL_MODE" = "update" ]; then
    verify
    if [ "$INSTALL_CODEX_PLUGIN" = "1" ]; then
      install_codex_plugin
    fi
    echo ""
    printf "  ${green}${bold}Update complete!${reset}\n"
    echo ""
    return
  fi

  download_chromium
  setup_path
  write_config
  verify
  install_skill

  echo ""
  printf "  ${green}${bold}Installation complete!${reset}\n"
  echo ""
  printf "  ${dim}Quick start:${reset}\n"
  printf "    gsd-browser navigate https://example.com\n"
  printf "    gsd-browser screenshot --output page.png\n"
  printf "    gsd-browser snapshot\n"
  echo ""
  printf "  ${dim}For AI agents (MCP primary path):${reset}\n"
  printf "    gsd-browser mcp\n"
  printf "    $INSTALL_DIR/scripts/mcp-quickstart.sh cursor   # or claude/vscode\n"
  printf "    See docs/mcp.md and docs/AGENT-BEST-PRACTICES.md\n"
  echo ""
}

main "$@"
