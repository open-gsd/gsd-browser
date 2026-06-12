const fs = require("fs");

const releaseVersion = (process.env.RELEASE_VERSION || "").trim();
const bump = (process.env.VERSION_BUMP || "patch").trim() || "patch";
const current = require("../npm/package.json").version;

function fail(message) {
  console.error(message);
  process.exit(1);
}

function parseVersion(version) {
  const match = version.match(/^(\d+)\.(\d+)\.(\d+)$/);
  if (!match) fail(`Version ${version} must be stable semver like 0.1.30.`);
  return match.slice(1).map(Number);
}

function compareVersions(left, right) {
  const leftParts = parseVersion(left);
  const rightParts = parseVersion(right);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] !== rightParts[index]) {
      return leftParts[index] - rightParts[index];
    }
  }
  return 0;
}

function nextVersion() {
  if (releaseVersion) {
    parseVersion(releaseVersion);
    return releaseVersion;
  }

  const [major, minor, patch] = parseVersion(current);
  if (bump === "major") return `${major + 1}.0.0`;
  if (bump === "minor") return `${major}.${minor + 1}.0`;
  if (bump === "patch") return `${major}.${minor}.${patch + 1}`;
  fail(`Unsupported version_bump ${bump}.`);
}

function updatePackageJson(file, version) {
  const pkg = JSON.parse(fs.readFileSync(file, "utf8"));
  pkg.version = version;
  fs.writeFileSync(file, `${JSON.stringify(pkg, null, 2)}\n`);
}

function updateManifest(file, version) {
  const text = fs.readFileSync(file, "utf8");
  const updated = text.replace(/^(version = ")[^"]+(")$/m, `$1${version}$2`);
  if (updated === text) fail(`Could not update ${file}.`);
  fs.writeFileSync(file, updated);
}

function updateLockPackage(lockText, packageName, version) {
  const pattern = new RegExp(
    `(\\[\\[package\\]\\]\\nname = "${packageName}"\\nversion = ")[^"]+(")`,
    "m",
  );
  const updated = lockText.replace(pattern, `$1${version}$2`);
  if (updated === lockText) fail(`Could not update ${packageName} in Cargo.lock.`);
  return updated;
}

const version = nextVersion();
if (compareVersions(version, current) <= 0) {
  fail(`Resolved version ${version} must be greater than current version ${current}.`);
}

updatePackageJson("npm/package.json", version);
updateManifest("cli/Cargo.toml", version);
updateManifest("common/Cargo.toml", version);

let lockText = fs.readFileSync("Cargo.lock", "utf8");
lockText = updateLockPackage(lockText, "gsd-browser", version);
lockText = updateLockPackage(lockText, "gsd-browser-common", version);
fs.writeFileSync("Cargo.lock", lockText);

process.stdout.write(version);
