#!/usr/bin/env node
"use strict";

const path = require("path");

const packageRoot = path.resolve(__dirname, "..");
const pkg = require(path.join(packageRoot, "package.json"));
const rawTag = process.argv[2] || process.env.GITHUB_REF_NAME;

function fail(message) {
  console.error(`gsd-browser npm release: ${message}`);
  process.exit(1);
}

if (!rawTag) {
  fail("release tag argument is required");
}

const tag = rawTag.replace(/^refs\/tags\//, "");
const version = tag.replace(/^v/, "");

if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
  fail(`release tag ${rawTag} does not contain a semver version`);
}

if (pkg.version !== version) {
  fail(`release tag ${tag} does not match npm package version ${pkg.version}`);
}

console.log(`gsd-browser npm release: ${tag} matches ${pkg.name}@${pkg.version}`);
