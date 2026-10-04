import assert from "node:assert/strict";
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../", import.meta.url));
const launcher = path.join(root, "scripts/gmem.js");
const installer = path.join(root, "scripts/npm-install.js");

async function copyPackage(directory) {
  await mkdir(path.join(directory, "scripts"), { recursive: true });
  await copyFile(launcher, path.join(directory, "scripts/gmem.js"));
  await copyFile(installer, path.join(directory, "scripts/npm-install.js"));
  await writeFile(path.join(directory, "package.json"), JSON.stringify({ type: "module", version: "0.9.0" }));
}

const target = process.platform === "darwin"
  ? `${process.arch === "arm64" ? "aarch64" : "x86_64"}-apple-darwin`
  : "x86_64-unknown-linux-gnu";

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
    await symlink(launcher, path.join(dir, "gmem"));
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

test("binary discovery probes reject npm launchers", () => {
  const result = spawnSync(process.execPath, [launcher, "version"], {
    encoding: "utf8",
    env: { ...process.env, GMEM_NPM_BINARY_PROBE: "1" },
  });
  assert.equal(result.status, 1, result.stderr);
  assert.equal(result.stdout, "");
  assert.equal(result.stderr, "");
});

test("multiple npm installations do not recursively probe each other", { skip: process.platform === "win32" }, async () => {
  await withFakeGmem(async ({ dir, binary, log }) => {
    const first = path.join(dir, "first");
    const second = path.join(dir, "second");
    const bins = [path.join(first, "bin"), path.join(second, "bin")];
    for (const [index, directory] of [first, second].entries()) {
      await copyPackage(directory);
      await mkdir(bins[index]);
      const copiedLauncher = path.join(directory, "scripts/gmem.js");
      // An absolute shebang keeps this fixture independent of the host PATH.
      const source = await readFile(copiedLauncher, "utf8");
      await writeFile(copiedLauncher, source.replace("#!/usr/bin/env node", `#!${process.execPath}`));
      await chmod(copiedLauncher, 0o755);
      await symlink(copiedLauncher, path.join(bins[index], "gmem"));
      const vendor = path.join(directory, "vendor", target);
      await mkdir(vendor, { recursive: true });
      await copyFile(binary, path.join(vendor, "gmem"));
    }
    // Bound process creation even on the broken implementation, so the
    // regression fails safely rather than leaving a recursive process tree.
    const counter = path.join(dir, "probes.txt");
    const preload = path.join(dir, "count.cjs");
    await writeFile(preload, `
      const fs = require('node:fs');
      fs.appendFileSync(${JSON.stringify(counter)}, 'probe\\n');
      if (fs.readFileSync(${JSON.stringify(counter)}, 'utf8').trim().split('\\n').length > 4) process.exit(1);
    `);
    const env = {
      ...process.env,
      PATH: [...bins, dir].join(path.delimiter),
      NODE_OPTIONS: `--require=${preload}`,
    };
    for (const entry of ["scripts/gmem.js", "scripts/npm-install.js"]) {
      await writeFile(counter, "");
      const result = spawnSync(process.execPath, [path.join(first, entry), ...(entry.endsWith("gmem.js") ? ["mcp", "--flag"] : [])], {
        encoding: "utf8", env, timeout: 10000,
      });
      assert.equal(result.status, 0, result.stderr);
      assert.equal((await readFile(counter, "utf8")).trim().split("\n").length, 2);
    }
    assert.equal(await readFile(log, "utf8"), "mcp --flag\n");
  });
});

for (const scenario of ["valid", "checksum mismatch", "unusable binary"]) {
  test(`release installation: ${scenario}`, { skip: process.platform === "win32" }, async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "graphmem-npm-download-test-"));
    let systemTemp;
    try {
      const directory = path.join(dir, "package");
      await copyPackage(directory);
      const payload = path.join(dir, "payload");
      await mkdir(payload);
      await writeFile(path.join(payload, "gmem"), scenario === "unusable binary"
        ? "#!/bin/sh\nexit 1\n"
        : "#!/bin/sh\necho 'gmem 0.9.0'\n");
      const archive = path.join(dir, "release.tar.gz");
      const packed = spawnSync("tar", ["-czf", archive, "-C", payload, "gmem"], { encoding: "utf8" });
      assert.equal(packed.status, 0, packed.stderr);
      const digest = createHash("sha256").update(await readFile(archive)).digest("hex");
      const archiveName = `gmem-0.9.0-${target}.tar.gz`;
      const preload = path.join(dir, "download.cjs");
      // Fake only the network boundary; verification, tar, smoke testing and
      // filesystem operations remain real.
      await writeFile(preload, `
        const fs = require('node:fs');
        global.fetch = async (url) => {
          if (url === ${JSON.stringify(`https://github.com/sonic182/graphmem/releases/download/0.9.0/${archiveName}`)})
            return new Response(fs.readFileSync(${JSON.stringify(archive)}));
          if (url === 'https://github.com/sonic182/graphmem/releases/download/0.9.0/SHA256SUMS')
            return new Response(${JSON.stringify(`${scenario === "checksum mismatch" ? "0".repeat(64) : digest}  ${archiveName}\n`)});
          throw new Error('Unexpected download: ' + url);
        };
      `);
      // Linux commonly mounts /dev/shm separately: exercise an actual
      // cross-filesystem install there. Else use an unavailable system temp
      // directory, which must not be needed for destination-local staging.
      try {
        systemTemp = await mkdtemp("/dev/shm/graphmem-npm-test-");
      } catch {
        systemTemp = path.join(dir, "unavailable-system-temp");
      }
      const result = spawnSync(process.execPath, ["--require", preload, path.join(directory, "scripts/npm-install.js")], {
        encoding: "utf8", timeout: 10000,
        env: { ...process.env, PATH: "/usr/bin:/bin", TMPDIR: systemTemp, TMP: systemTemp, TEMP: systemTemp },
      });
      const vendor = path.join(directory, "vendor", target);
      if (scenario === "valid") {
        assert.equal(result.status, 0, result.stderr);
        assert.deepEqual(await readdir(vendor), ["gmem"]);
        const launched = spawnSync(process.execPath, [path.join(directory, "scripts/gmem.js"), "version"], {
          encoding: "utf8", env: { ...process.env, PATH: "" },
        });
        assert.equal(launched.status, 0, launched.stderr);
        assert.equal(launched.stdout, "gmem 0.9.0\n");
      } else {
        assert.equal(result.status, 1);
        assert.match(result.stderr, scenario === "checksum mismatch" ? /SHA256 mismatch/ : /failed `gmem version` smoke test/);
        assert.deepEqual(await readdir(vendor), []);
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
      if (systemTemp) await rm(systemTemp, { recursive: true, force: true });
    }
  });
}
