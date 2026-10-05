import assert from "node:assert/strict";
import { mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../", import.meta.url));

test("packed npm package installs offline with scripts disabled and gives usable recovery without PATH", { skip: process.platform === "win32" || !process.env.npm_execpath }, async () => {
  const dir = await realpath(await mkdtemp(path.join(os.tmpdir(), "graphmem-fresh-install-")));
  try {
    const npm = args => spawnSync(process.execPath, [process.env.npm_execpath, ...args], {
      cwd: root, encoding: "utf8", timeout: 30000,
    });
    const packed = npm(["pack", "--ignore-scripts", "--offline", "--json", "--pack-destination", dir]);
    assert.equal(packed.status, 0, packed.stderr);
    const [{ filename }] = JSON.parse(packed.stdout);
    const prefix = path.join(dir, "installation with spaces");
    const installed = npm(["install", "--prefix", prefix, "--offline", "--ignore-scripts", "--legacy-peer-deps", "--no-audit", "--no-fund", path.join(dir, filename)]);
    assert.equal(installed.status, 0, installed.stderr);
    const pkg = path.join(prefix, "node_modules/@sonic182/graphmem");
    const manifest = JSON.parse(await readFile(path.join(pkg, "package.json"), "utf8"));
    assert.equal(manifest.scripts.postinstall, undefined, "published package must not execute an installation script");
    const marker = path.join(dir, "network.txt");
    const preload = path.join(dir, "no-network.cjs");
    await writeFile(preload, `global.fetch = () => { require('node:fs').writeFileSync(${JSON.stringify(marker)}, 'network'); throw new Error('Unexpected network'); };`);
    const result = spawnSync(process.execPath, ["--require", preload, path.join(pkg, "scripts/gmem.js"), "mcp"], {
      encoding: "utf8", timeout: 5000, env: { ...process.env, PATH: "" },
    });
    assert.equal(result.status, 1);
    assert.equal(result.stdout, "", "MCP stdout must not contain setup messages");
    assert.ok(result.stderr.includes(path.join(pkg, "scripts/npm-install.js")));
    await assert.rejects(readFile(marker), { code: "ENOENT" });
    // Exercise public npm resolution, not a direct file import that bypasses exports.
    const loaded = spawnSync(process.execPath, ["--input-type=module", "-e", `await import("@sonic182/graphmem"); await import("@sonic182/graphmem/server"); await import(${JSON.stringify(path.join(pkg, "plugin/pi/extensions/graphmem-context.js"))});`], {
      cwd: prefix, encoding: "utf8", timeout: 5000,
    });
    assert.equal(loaded.status, 0, loaded.stderr);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test("unsupported platforms receive source-build advice without a download", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "graphmem-unsupported-"));
  try {
    const preload = path.join(dir, "platform.cjs");
    await writeFile(preload, "Object.defineProperty(process, 'arch', {value: 'riscv64'}); global.fetch = () => { throw new Error('Unexpected network'); };\n");
    const result = spawnSync(process.execPath, ["--require", preload, path.join(root, "scripts/gmem.js"), "version"], {
      encoding: "utf8", timeout: 5000, env: { ...process.env, PATH: "" },
    });
    assert.equal(result.status, 1);
    assert.equal(result.stdout, "");
    assert.ok(result.stderr.includes(`No prebuilt gmem binary for ${process.platform}/riscv64`));
    assert.match(result.stderr, /Install from source/);
  } finally { await rm(dir, { recursive: true, force: true }); }
});
