'use strict';

const assert = require('node:assert/strict');
const { describe, test } = require('node:test');
const {
  buildDiscordReleasePayload,
  cleanBullet,
  collectSections,
  extractInstallCommand,
  loadPackageName,
} = require('../scripts/release-notes/discord-release-summary.cjs');

const sampleRelease = {
  tagName: 'v0.2.0',
  name: 'v0.2.0',
  isPrerelease: false,
  url: 'https://github.com/open-gsd/gsd-browser/releases/tag/v0.2.0',
  body: [
    '## What\'s Changed',
    '* feat: add action cache replay by @trek-e in https://github.com/open-gsd/gsd-browser/pull/10',
    '* fix: keep snapshots stable by @trek-e in https://github.com/open-gsd/gsd-browser/pull/11',
    '* docs: explain MCP setup by @trek-e in https://github.com/open-gsd/gsd-browser/pull/12',
  ].join('\n'),
};

describe('discord release summary', () => {
  test('builds a Discord payload for generated GitHub release notes', () => {
    const payload = buildDiscordReleasePayload({
      release: sampleRelease,
      packageName: '@opengsd/gsd-browser',
      maxContent: 1850,
    });

    assert.equal(payload.username, 'GSD Releases');
    assert.match(payload.content, /\*\*@opengsd\/gsd-browser v0\.2\.0 is out\*\*/);
    assert.match(payload.content, /`npm i @opengsd\/gsd-browser@latest`/);
    assert.match(payload.content, /add action cache replay \(#10\)/);
    assert.match(payload.content, /keep snapshots stable \(#11\)/);
    assert.match(payload.content, /explain MCP setup \(#12\)/);
    assert.match(payload.content, /Full changelog: https:\/\/github\.com\/open-gsd\/gsd-browser\/releases\/tag\/v0\.2\.0/);
    assert.equal(payload.embeds[0].fields[1].value, '`latest`');
  });

  test('preserves an explicit install command from curated release notes', () => {
    assert.equal(
      extractInstallCommand(
        ['## Install', '', '```bash', 'npm install -g @opengsd/gsd-browser@0.2.0', '```'].join('\n'),
        '@opengsd/gsd-browser',
        sampleRelease
      ),
      'npm install -g @opengsd/gsd-browser@0.2.0'
    );
  });

  test('classifies auto-generated What Changed bullets', () => {
    const sections = collectSections(sampleRelease.body);
    assert.deepEqual(sections.get('Feature'), ['add action cache replay (#10)']);
    assert.deepEqual(sections.get('Fix'), ['keep snapshots stable (#11)']);
    assert.deepEqual(sections.get('Enhancement'), ['explain MCP setup (#12)']);
  });

  test('cleans GitHub release note link noise', () => {
    assert.equal(
      cleanBullet('* fix: repair [#670](https://github.com/open-gsd/gsd-browser/issues/670) by @trek-e in https://github.com/open-gsd/gsd-browser/pull/675'),
      'repair #670 (#675)'
    );
  });

  test('defaults to the current npm package name', () => {
    assert.equal(loadPackageName(), '@opengsd/gsd-browser');
  });
});
