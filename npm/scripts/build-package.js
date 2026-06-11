#!/usr/bin/env node
"use strict";

const path = require("path");
const { spawnSync } = require("child_process");

const packageRoot = path.resolve(__dirname, "..");
const pkg = require(path.join(packageRoot, "package.json"));
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";

function fail(message) {
  console.error(`gsd-browser npm build: ${message}`);
  process.exit(1);
}

function parsePackOutput(stdout) {
  const jsonStart = stdout.indexOf("[");

  if (jsonStart === -1) {
    fail("npm pack did not return JSON output");
  }

  try {
    return JSON.parse(stdout.slice(jsonStart));
  } catch (error) {
    fail(`could not parse npm pack JSON: ${error.message}`);
  }
}

const result = spawnSync(npmCommand, ["pack", "--dry-run", "--json"], {
  cwd: packageRoot,
  encoding: "utf8",
});

if (result.status !== 0) {
  process.stderr.write(result.stderr);
  process.stdout.write(result.stdout);
  fail(`npm pack --dry-run exited with ${result.status}`);
}

const packEntries = parsePackOutput(result.stdout);
const pack = packEntries[0];

if (!pack) {
  fail("npm pack returned no package entries");
}

const files = new Set(pack.files.map((file) => file.path));
const requiredFiles = [
  "AGENTS.md",
  "LICENSE-APACHE",
  "LICENSE-MIT",
  "README.md",
  "SKILL.md",
  "bin/gsd-browser",
  "package.json",
  "scripts/postinstall.js",
];

for (const file of requiredFiles) {
  if (!files.has(file)) {
    fail(`package tarball is missing ${file}`);
  }
}

const nativeBinaries = [...files].filter((file) =>
  file === "bin/gsd-browser-bin" || file === "bin/gsd-browser.exe"
);

if (nativeBinaries.length > 0) {
  fail(`package tarball must not include local native binaries: ${nativeBinaries.join(", ")}`);
}

console.log(
  `gsd-browser npm build: ${pkg.name}@${pkg.version} packs ${files.size} files ` +
    `(${pack.unpackedSize} bytes unpacked)`
);
