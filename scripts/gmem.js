#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { access } from "node:fs/promises";
import { constants, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

// A binary discovery probe must not enter another npm launcher's discovery.
if (process.env.GMEM_NPM_BINARY_PROBE === "1") process.exit(1);

const packageRoot = fileURLToPath(new URL("../", import.meta.url));
const bundled = path.join(packageRoot, "vendor", process.platform === "win32" ? "x86_64-pc-windows-msvc" : `${process.arch === "arm64" ? "aarch64" : "x86_64"}-${process.platform === "darwin" ? "apple-darwin" : "unknown-linux-gnu"}`, process.platform === "win32" ? "gmem.exe" : "gmem");
const executable = process.platform === "win32" ? "gmem.exe" : "gmem";

function usable(candidate) {
  const result = spawnSync(candidate, ["version"], {
    encoding: "utf8", timeout: 5000, windowsHide: true,
    env: { ...process.env, GMEM_NPM_BINARY_PROBE: "1" },
  });
  return result.status === 0 && !result.error;
}

function findExisting() {
  const npmBinDirs = [process.env.npm_config_prefix, process.env.N_PREFIX, process.env.NVM_BIN]
    .filter(Boolean).map((prefix) => path.resolve(prefix, "bin"));
  for (const dir of (process.env.PATH ?? "").split(path.delimiter).filter(Boolean)) {
    if (path.resolve(dir) === path.resolve(packageRoot) || npmBinDirs.includes(path.resolve(dir))) continue;
    const candidate = path.join(dir, executable);
    try {
      if (realpathSync(candidate) === realpathSync(fileURLToPath(import.meta.url))) continue;
    } catch {
      continue;
    }
    if (usable(candidate)) return candidate;
  }
  return null;
}

let command = findExisting();
if (!command) {
  try {
    await access(bundled, constants.X_OK);
    command = bundled;
  } catch {
    console.error("gmem binary is not installed. Run `gmem-install` or install manually: https://github.com/sonic182/graphmem#install");
    process.exit(1);
  }
}

const child = spawnSync(command, process.argv.slice(2), { stdio: "inherit", windowsHide: false });
if (child.error) {
  console.error(`Could not start gmem: ${child.error.message}`);
  process.exit(1);
}
if (child.signal) {
  process.kill(process.pid, child.signal);
} else {
  process.exit(child.status ?? 1);
}
