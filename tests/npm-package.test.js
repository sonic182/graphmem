import assert from "node:assert/strict";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../", import.meta.url));
const launcher = path.join(root, "scripts/gmem.js");
const installer = path.join(root, "scripts/npm-install.js");

async function withFakeGmem(run) {
  const dir = await mkdtemp(path.join(os.tmpdir(), "graphmem-npm-test-"));
  try {
    const binary = path.join(dir, process.platform === "win32" ? "gmem.cmd" : "gmem");
    const log = path.join(dir, "args.txt");
    const body = process.platform === "win32"
      ? `@echo off\nif "%1"=="version" (echo gmem 0.9.0 & exit /b 0)\necho %*>>"${log}"\n`
      : `#!/bin/sh\nif [ "$1" = version ]; then echo 'gmem 0.9.0'; exit 0; fi\nprintf '%s\\n' "$*" >> '${log}'\n`;
    await writeFile(binary, body);
    if (process.platform !== "win32") await chmod(binary, 0o755);
    await run({ dir, binary, log });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

test("global launcher reuses a working gmem from PATH and forwards arguments", { skip: process.platform === "win32" }, async () => {
  await withFakeGmem(async ({ dir, log }) => {
    const result = spawnSync(process.execPath, [launcher, "mcp", "--flag"], {
      encoding: "utf8",
      env: { ...process.env, PATH: `${dir}${path.delimiter}${process.env.PATH}` },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await readFile(log, "utf8"), "mcp --flag\n");
    assert.equal(result.stdout, "");
  });
});

test("launcher ignores its own npm symlink instead of recursively invoking itself", { skip: process.platform === "win32" }, async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "graphmem-npm-link-test-"));
  try {
    await (await import("node:fs/promises")).symlink(launcher, path.join(dir, "gmem"));
    const result = spawnSync(process.execPath, [launcher, "version"], {
      encoding: "utf8",
      env: { ...process.env, PATH: dir },
      timeout: 5000,
    });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /gmem binary is not installed/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("npm installer skips release downloads when a working gmem is on PATH", { skip: process.platform === "win32" }, async () => {
  await withFakeGmem(async ({ dir }) => {
    const result = spawnSync(process.execPath, [installer], {
      encoding: "utf8",
      env: { ...process.env, PATH: `${dir}${path.delimiter}${process.env.PATH}` },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /Using existing gmem on PATH/);
  });
});
