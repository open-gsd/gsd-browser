#!/usr/bin/env node
"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const packageRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(packageRoot, "..");
const pkg = require(path.join(packageRoot, "package.json"));

function fail(message) {
  console.error(`gsd-browser npm test: ${message}`);
  process.exit(1);
}

function assert(condition, message) {
  if (!condition) {
    fail(message);
  }
}

function readText(filePath) {
  return fs.readFileSync(filePath, "utf8");
}

function packageFile(relativePath) {
  return path.join(packageRoot, relativePath);
}

function manifestVersion(relativePath) {
  const manifest = readText(path.join(repoRoot, relativePath));
  const match = manifest.match(/^version\s*=\s*"([^"]+)"/m);

  assert(match, `${relativePath} is missing a package version`);
  return match[1];
}

function copyLauncher(tempDir) {
  const tempBin = path.join(tempDir, "bin");
  fs.mkdirSync(tempBin, { recursive: true });

  const launcherPath = path.join(tempBin, "gsd-browser");
  fs.copyFileSync(packageFile("bin/gsd-browser"), launcherPath);
  fs.chmodSync(launcherPath, 0o755);

  return launcherPath;
}

function runNodeScript(scriptPath, args = []) {
  return spawnSync(process.execPath, [scriptPath, ...args], {
    encoding: "utf8",
  });
}

function testMissingNativeBinary() {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "gsd-browser-npm-missing-"));

  try {
    const launcherPath = copyLauncher(tempDir);
    const result = runNodeScript(launcherPath, ["--version"]);

    assert(result.status !== 0, "launcher should fail when native binary is missing");
    assert(
      result.stderr.includes("native binary is missing"),
      "launcher should explain that the native binary is missing"
    );
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

function testLauncherExecutesNativeBinary() {
  if (process.platform === "win32") {
    console.log("gsd-browser npm test: skipping fake binary launcher test on Windows");
    return;
  }

  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "gsd-browser-npm-launcher-"));

  try {
    const launcherPath = copyLauncher(tempDir);
    const fakeBinaryPath = path.join(path.dirname(launcherPath), "gsd-browser-bin");

    fs.writeFileSync(fakeBinaryPath, '#!/bin/sh\necho "fake-gsd-browser:$*"\n');
    fs.chmodSync(fakeBinaryPath, 0o755);

    const result = runNodeScript(launcherPath, ["--version"]);

    assert(result.status === 0, `launcher exited with ${result.status}: ${result.stderr}`);
    assert(
      result.stdout.trim() === "fake-gsd-browser:--version",
      "launcher should forward arguments to the native binary"
    );
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

assert(pkg.name === "@opengsd/gsd-browser", "package name changed unexpectedly");
assert(pkg.version === manifestVersion("cli/Cargo.toml"), "npm version must match cli crate");
assert(pkg.version === manifestVersion("common/Cargo.toml"), "npm version must match common crate");
assert(pkg.license === "MIT OR Apache-2.0", "package license changed unexpectedly");
assert(pkg.publishConfig?.access === "public", "scoped npm package must publish publicly");
assert(pkg.bin?.["gsd-browser"] === "bin/gsd-browser", "gsd-browser bin entry is missing");
assert(pkg.scripts?.postinstall === "node scripts/postinstall.js", "postinstall script changed unexpectedly");

for (const file of [
  "AGENTS.md",
  "LICENSE-APACHE",
  "LICENSE-MIT",
  "README.md",
  "SKILL.md",
  "bin/gsd-browser",
  "scripts/postinstall.js",
]) {
  assert(fs.existsSync(packageFile(file)), `${file} is missing from npm package`);
}

const launcher = readText(packageFile("bin/gsd-browser"));
assert(launcher.startsWith("#!/usr/bin/env node"), "launcher must keep its node shebang");

const postinstall = readText(packageFile("scripts/postinstall.js"));
for (const asset of [
  "gsd-browser-darwin-arm64",
  "gsd-browser-darwin-x64",
  "gsd-browser-linux-arm64",
  "gsd-browser-linux-x64",
  "gsd-browser-windows-arm64.exe",
  "gsd-browser-windows-x64.exe",
]) {
  assert(postinstall.includes(asset), `postinstall must know release asset ${asset}`);
}

testMissingNativeBinary();
testLauncherExecutesNativeBinary();

console.log(`gsd-browser npm test: ${pkg.name}@${pkg.version} package checks passed`);
