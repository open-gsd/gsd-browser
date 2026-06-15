'use strict';

const assert = require('node:assert/strict');
const { describe, test } = require('node:test');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');

const { downloadFile, ensureDir } = require('../npm/scripts/postinstall.js');

describe('postinstall', () => {
  test('ensureDir creates nested directories recursively', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gsd-browser-ensure-dir-'));
    const nested = path.join(tempDir, 'a', 'b', 'c');
    try {
      assert.ok(!fs.existsSync(nested));
      ensureDir(nested);
      assert.ok(fs.existsSync(nested));
      assert.ok(fs.statSync(nested).isDirectory());
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  test('downloadFile creates missing parent directory before writing', async () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gsd-browser-download-'));
    const payload = Buffer.from('hello gsd-browser');

    const server = http.createServer((req, res) => {
      res.writeHead(200, { 'Content-Type': 'application/octet-stream' });
      res.end(payload);
    });

    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    const { port } = server.address();

    const destDir = path.join(tempDir, 'bin');
    const dest = path.join(destDir, 'gsd-browser-bin');

    try {
      assert.ok(!fs.existsSync(destDir));
      await downloadFile(`http://127.0.0.1:${port}/binary`, dest, http.get);
      assert.ok(fs.existsSync(dest));
      assert.equal(fs.readFileSync(dest).toString(), payload.toString());
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
      server.close();
    }
  });

  test('downloadFile removes the partial file when the download is interrupted', async () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gsd-browser-download-partial-'));

    const server = http.createServer((req, res) => {
      res.writeHead(200, {
        'Content-Type': 'application/octet-stream',
        'Content-Length': '1024',
      });
      res.write(Buffer.from('partial'));
      res.socket.destroy();
    });

    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    const { port } = server.address();

    const destDir = path.join(tempDir, 'bin');
    const dest = path.join(destDir, 'gsd-browser-bin');

    try {
      await assert.rejects(downloadFile(`http://127.0.0.1:${port}/binary`, dest, http.get));
      assert.ok(!fs.existsSync(dest), 'partial download should be removed');
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
      server.close();
    }
  });

  test('downloadFile rejects when the destination directory cannot be created', async () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gsd-browser-download-err-'));
    const payload = Buffer.from('hello gsd-browser');

    const server = http.createServer((req, res) => {
      res.writeHead(200, { 'Content-Type': 'application/octet-stream' });
      res.end(payload);
    });

    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    const { port } = server.address();

    const parentFile = path.join(tempDir, 'not-a-dir');
    const dest = path.join(parentFile, 'gsd-browser-bin');

    try {
      fs.writeFileSync(parentFile, 'i am a file, not a directory');
      await assert.rejects(
        downloadFile(`http://127.0.0.1:${port}/binary`, dest, http.get),
        /failed|EEXIST|ENOTDIR/
      );
    } finally {
      fs.rmSync(tempDir, { recursive: true, force: true });
      server.close();
    }
  });
});
