#!/usr/bin/env node
import { createHash } from "node:crypto";
import { realpathSync } from "node:fs";
import { chmod, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";

const packageRoot = fileURLToPath(new URL("../", import.meta.url));
const launcherPath = fileURLToPath(new URL("./gmem.js", import.meta.url));
const packageJson = JSON.parse(await readFile(path.join(packageRoot, "package.json"), "utf8"));
const version = packageJson.version;
const isWindows = process.platform === "win32";
const executable = isWindows ? "gmem.exe" : "gmem";
const target = {
  "linux-x64": "x86_64-unknown-linux-gnu",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
}[`${process.platform}-${process.arch}`];

function isUsableBinary(candidate) {
  const result = spawnSync(candidate, ["version"], { encoding: "utf8", timeout: 5000, windowsHide: true });
  return result.status === 0 && !result.error;
}

function findExistingBinary() {
  const npmBinDirs = [process.env.npm_config_prefix, process.env.N_PREFIX, process.env.NVM_BIN]
    .filter(Boolean)
    .map((prefix) => path.resolve(prefix, "bin"));
  for (const dir of (process.env.PATH ?? "").split(path.delimiter).filter(Boolean)) {
    const resolvedDir = path.resolve(dir);
    if (resolvedDir === path.resolve(packageRoot) || npmBinDirs.includes(resolvedDir)) continue;
    const candidate = path.join(dir, executable);
    try {
      const resolved = realpathSync(candidate);
      if (resolved === realpathSync(fileURLToPath(import.meta.url)) || resolved === realpathSync(launcherPath)) continue;
    } catch {
      continue;
    }
    if (isUsableBinary(candidate)) return candidate;
  }
  return null;
}

async function install() {
  const existing = findExistingBinary();
  if (existing) {
    const result = spawnSync(existing, ["version"], { encoding: "utf8", timeout: 5000, windowsHide: true });
    console.log(`Using existing gmem on PATH (${existing}): ${(result.stdout || result.stderr).trim()}`);
    return;
  }

  if (!target) throw new Error(`No prebuilt gmem binary for ${process.platform}/${process.arch}. Install from source: https://github.com/sonic182/graphmem#install`);

  const archiveName = `gmem-${version}-${target}.${isWindows ? "zip" : "tar.gz"}`;
  const releaseUrl = `https://github.com/sonic182/graphmem/releases/download/${version}`;
  const directory = path.join(packageRoot, "vendor", target);
  await mkdir(directory, { recursive: true });
  const archivePath = path.join(os.tmpdir(), `${archiveName}-${process.pid}`);
  const binaryPath = path.join(directory, executable);
  const temporaryBinary = `${binaryPath}.${process.pid}.tmp`;

  try {
    const [archiveResponse, sumsResponse] = await Promise.all([
      fetch(`${releaseUrl}/${archiveName}`),
      fetch(`${releaseUrl}/SHA256SUMS`),
    ]);
    if (!archiveResponse.ok || !sumsResponse.ok) throw new Error(`GitHub Release download failed (${archiveResponse.status}/${sumsResponse.status})`);
    const sums = await sumsResponse.text();
    const line = sums.split(/\r?\n/).find((entry) => entry.trim().split(/\s+/).at(-1) === archiveName);
    if (!line) throw new Error(`SHA256SUMS does not contain ${archiveName}`);
    const expected = line.trim().split(/\s+/)[0].toLowerCase();
    const archiveBuffer = Buffer.from(await archiveResponse.arrayBuffer());
    const actual = createHash("sha256").update(archiveBuffer).digest("hex");
    if (actual !== expected) throw new Error(`SHA256 mismatch for ${archiveName}`);

    if (isWindows) {
      const { mkdtemp } = await import("node:fs/promises");
      const { execFileSync } = await import("node:child_process");
      const tempDir = await mkdtemp(path.join(os.tmpdir(), "gmem-npm-"));
      try {
        const zipPath = path.join(tempDir, archiveName);
        await writeFile(zipPath, archiveBuffer);
        execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", "Expand-Archive -LiteralPath $env:GMEM_ZIP -DestinationPath $env:GMEM_DEST -Force"], {
          env: { ...process.env, GMEM_ZIP: zipPath, GMEM_DEST: tempDir },
          stdio: "ignore",
          windowsHide: true,
        });
        await rename(path.join(tempDir, executable), temporaryBinary);
      } finally {
        await rm(tempDir, { recursive: true, force: true });
      }
    } else {
      await writeFile(archivePath, archiveBuffer);
      // Extract only the named executable from tar without shell interpolation.
      const { execFileSync } = await import("node:child_process");
      const tempDir = await import("node:fs/promises").then(({ mkdtemp }) => mkdtemp(path.join(os.tmpdir(), "gmem-npm-")));
      try {
        execFileSync("tar", ["-xzf", archivePath, "-C", tempDir, executable], { stdio: "ignore" });
        await rename(path.join(tempDir, executable), temporaryBinary);
      } finally {
        await rm(tempDir, { recursive: true, force: true });
      }
    }

    if (!isWindows) await chmod(temporaryBinary, 0o755);
    if (!isUsableBinary(temporaryBinary)) throw new Error("Downloaded gmem binary failed `gmem version` smoke test");
    await rename(temporaryBinary, binaryPath);
    console.log(`Installed gmem ${version} for ${target}`);
  } finally {
    await Promise.all([rm(archivePath, { force: true }), rm(temporaryBinary, { force: true })]);
  }
}

try {
  await install();
} catch (error) {
  console.error(`Could not install the gmem binary: ${error.message}`);
  console.error("Retry with `gmem-install` or install manually: https://github.com/sonic182/graphmem#install");
  process.exitCode = 1;
}
