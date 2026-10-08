import assert from "node:assert/strict";
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import test from "node:test";

const root = fileURLToPath(new URL("../", import.meta.url));
const launcher = path.join(root, "scripts/gmem.js");
const installer = path.join(root, "scripts/npm-install.js");
const graphmem = path.join(root, "scripts/graphmem.js");

async function copyPackage(directory) {
  await mkdir(path.join(directory, "scripts"), { recursive: true });
  await copyFile(launcher, path.join(directory, "scripts/gmem.js"));
  await copyFile(installer, path.join(directory, "scripts/npm-install.js"));
  await copyFile(graphmem, path.join(directory, "scripts/graphmem.js"));
  await copyFile(path.join(root, "scripts/binary.js"), path.join(directory, "scripts/binary.js"));
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

test("graphmem command reuses a working gmem from PATH without downloading", { skip: process.platform === "win32" }, async () => {
  await withFakeGmem(async ({ dir, log }) => {
    const result = spawnSync(process.execPath, [graphmem, "mcp", "--flag"], {
      encoding: "utf8",
      env: { ...process.env, PATH: `${dir}${path.delimiter}${process.env.PATH}` },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await readFile(log, "utf8"), "mcp --flag\n");
    assert.equal(result.stdout, "");
    assert.equal(result.stderr, "");
  });
});

test("graphmem command downloads the binary on first run and keeps stdout for the server", { skip: process.platform === "win32" }, async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "graphmem-npm-first-run-test-"));
  try {
    const directory = path.join(dir, "package");
    await copyPackage(directory);
    const payload = path.join(dir, "payload");
    await mkdir(payload);
    await writeFile(path.join(payload, "gmem"), "#!/bin/sh\necho 'gmem 0.9.0'\n");
    const archive = path.join(dir, "release.tar.gz");
    const packed = spawnSync("tar", ["-czf", archive, "-C", payload, "gmem"], { encoding: "utf8" });
    assert.equal(packed.status, 0, packed.stderr);
    const digest = createHash("sha256").update(await readFile(archive)).digest("hex");
    const archiveName = `gmem-0.9.0-${target}.tar.gz`;
    const preload = path.join(dir, "download.cjs");
    await writeFile(preload, `
      const fs = require('node:fs');
      global.fetch = async (url) => {
        if (url.endsWith(${JSON.stringify(archiveName)})) return new Response(fs.readFileSync(${JSON.stringify(archive)}));
        if (url.endsWith('/SHA256SUMS')) return new Response(${JSON.stringify(`${digest}  ${archiveName}\n`)});
        throw new Error('Unexpected download: ' + url);
      };
    `);
    const result = spawnSync(process.execPath, ["--require", preload, path.join(directory, "scripts/graphmem.js"), "version"], {
      encoding: "utf8", timeout: 10000,
      env: { ...process.env, PATH: "/usr/bin:/bin" },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "gmem 0.9.0\n");
    assert.match(result.stderr, /downloading the gmem binary/);
    assert.deepEqual(await readdir(path.join(directory, "vendor", "0.9.0", target)), ["gmem"]);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("server.json versions and name match package.json", async () => {
  const pkg = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
  const server = JSON.parse(await readFile(path.join(root, "server.json"), "utf8"));
  assert.equal(server.name, pkg.mcpName);
  assert.equal(server.version, pkg.version);
  assert.equal(server.packages[0].identifier, pkg.name);
  assert.equal(server.packages[0].version, pkg.version);
});

test("npm installer skips release downloads when a working gmem is on PATH", { skip: process.platform === "win32" }, async () => {
  await withFakeGmem(async ({ dir }) => {
    const result = spawnSync(process.execPath, [installer], {
      encoding: "utf8",
      env: { ...process.env, PATH: `${dir}${path.delimiter}${process.env.PATH}` },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /Using gmem:/);
    assert.equal(result.stdout, "");
  });
});

test("Windows recovery command preserves paths with spaces and apostrophes", async () => {
  const dir = await realpath(await mkdtemp(path.join(os.tmpdir(), "graphmem-npm-quoting-test-")));
  try {
    const directory = path.join(dir, "package with spaces and user's files & (cache)");
    await copyPackage(directory);
    const script = path.join(directory, "scripts/npm-install.js");
    await writeFile(script, "console.log(process.argv[1]);\n");
    const moduleUrl = pathToFileURL(path.join(directory, "scripts/binary.js")).href;
    const result = spawnSync(process.execPath, ["--input-type=module", "-e", `
      Object.defineProperty(process, "platform", {value: "win32"});
      const {installerCommand} = await import(${JSON.stringify(moduleUrl)});
      console.log(JSON.stringify(installerCommand()));
    `], { encoding: "utf8", timeout: 5000 });
    assert.equal(result.status, 0, result.stderr);
    const command = JSON.parse(result.stdout);
    assert.equal(command, `node "${script}"`, "cmd.exe requires double quotes around the script path");
    if (process.platform === "win32") {
      // Run the printed command in both real Windows shells, without downloads.
      for (const shell of [process.env.ComSpec || "cmd.exe", "powershell.exe"]) {
        const executed = spawnSync(command, {
          shell, encoding: "utf8", timeout: 10000,
          env: { ...process.env, PATH: `${path.dirname(process.execPath)}${path.delimiter}${process.env.PATH}` },
        });
        assert.equal(executed.status, 0, `${shell}: ${executed.stderr}`);
        assert.equal(executed.stdout.trim(), script);
      }
    }
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
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

for (const scenario of ["valid", "checksum mismatch", "unusable binary", "wrong version", "download failure", "interrupted", "concurrent"]) {
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
        : `#!/bin/sh\necho 'gmem ${scenario === "wrong version" ? "0.8.0" : "0.9.0"}'\n`);
      const archive = path.join(dir, "release.tar.gz");
      const packed = spawnSync("tar", ["-czf", archive, "-C", payload, "gmem"], { encoding: "utf8" });
      assert.equal(packed.status, 0, packed.stderr);
      const digest = createHash("sha256").update(await readFile(archive)).digest("hex");
      const archiveName = `gmem-0.9.0-${target}.tar.gz`;
      const preload = path.join(dir, "download.cjs");
      // Fake only the network boundary; verification, tar, smoke testing and
      // filesystem operations remain real.
      const requests = path.join(dir, "requests.txt");
      await writeFile(preload, `
        const fs = require('node:fs');
        let cancelling = false;
        global.fetch = async (url, options) => {
          fs.appendFileSync(${JSON.stringify(requests)}, 'request\\n');
          if (${JSON.stringify(scenario)} === 'download failure') return new Response('', {status: 503});
          if (${JSON.stringify(scenario)} === 'interrupted') {
            if (!cancelling) { cancelling = true; setTimeout(() => process.kill(process.pid, 'SIGINT'), 20); }
            return new Promise((resolve, reject) => options.signal.addEventListener('abort', () => reject(options.signal.reason), {once: true}));
          }
          if (${JSON.stringify(scenario)} === 'concurrent') await new Promise(resolve => setTimeout(resolve, 150));
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
      const args = ["--require", preload, path.join(directory, "scripts/npm-install.js")];
      const options = {
        encoding: "utf8", timeout: 10000,
        env: { ...process.env, PATH: "/usr/bin:/bin", TMPDIR: systemTemp, TMP: systemTemp, TEMP: systemTemp },
      };
      let result;
      if (scenario === "concurrent") {
        const run = () => new Promise((resolve, reject) => {
          const child = spawn(process.execPath, args, options);
          let stderr = "";
          child.stderr.on("data", chunk => { stderr += chunk; });
          child.on("error", reject);
          child.on("close", status => resolve({ status, stderr }));
        });
        const results = await Promise.all([run(), run()]);
        for (const entry of results) assert.equal(entry.status, 0, entry.stderr);
        assert.equal((await readFile(requests, "utf8")).trim().split("\n").length, 2, "concurrent installers must download only one archive and checksum file");
        result = results[0];
      } else result = spawnSync(process.execPath, args, options);
      const vendor = path.join(directory, "vendor", "0.9.0", target);
      if (scenario === "valid" || scenario === "concurrent") {
        assert.equal(result.status, 0, result.stderr);
        assert.deepEqual(await readdir(vendor), ["gmem"]);
        const launched = spawnSync(process.execPath, [path.join(directory, "scripts/gmem.js"), "version"], {
          encoding: "utf8", env: { ...process.env, PATH: "" },
        });
        assert.equal(launched.status, 0, launched.stderr);
        assert.equal(launched.stdout, "gmem 0.9.0\n");
      } else {
        assert.equal(result.status, 1);
        const error = {
          "checksum mismatch": /SHA256 mismatch/,
          "unusable binary": /failed `gmem version` smoke test/,
          "wrong version": /does not report gmem 0\.9\.0/,
          "download failure": /download failed \(503\)/,
          "interrupted": /cancelled/i,
        }[scenario];
        assert.match(result.stderr, error);
        assert.deepEqual(await readdir(vendor), []);
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
      if (systemTemp) await rm(systemTemp, { recursive: true, force: true });
    }
  });
}
