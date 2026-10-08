import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { chmod, mkdir, mkdtemp, open, readFile, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const packageRoot = fileURLToPath(new URL("../", import.meta.url));
const { version } = JSON.parse(await readFile(path.join(packageRoot, "package.json"), "utf8"));
const targets = {
  "linux/x64": "x86_64-unknown-linux-gnu",
  "darwin/x64": "x86_64-apple-darwin",
  "darwin/arm64": "aarch64-apple-darwin",
  "win32/x64": "x86_64-pc-windows-msvc",
};
const target = targets[`${process.platform}/${process.arch}`];
const executable = process.platform === "win32" ? "gmem.exe" : "gmem";
const directory = path.join(packageRoot, "vendor", version, target ?? "unsupported");
const cached = path.join(directory, executable);
const launcher = fileURLToPath(new URL("./gmem.js", import.meta.url));

export const binaryInfo = { version, target, platform: `${process.platform}/${process.arch}` };

export function installerCommand() {
  const script = fileURLToPath(new URL("./npm-install.js", import.meta.url));
  const quoted = process.platform === "win32"
    ? `"${script}"`
    : `'${script.replaceAll("'", "'\\''")}'`;
  return `node ${quoted}`;
}

export function missingBinaryMessage() {
  if (!target) return `No prebuilt gmem binary for ${binaryInfo.platform}. Install from source: https://github.com/sonic182/graphmem#install`;
  let detail = "";
  for (const candidate of [cached, path.join(packageRoot, "vendor", target, executable)]) {
    if (!existsSync(candidate)) continue;
    try { validateBinary(candidate, version); } catch (error) { detail = `\n${error.message}`; break; }
  }
  return `gmem binary is not installed or cannot run (${binaryInfo.platform}, version ${version}).${detail}\nDownload and verify the matching release explicitly: ${installerCommand()}\nLinux binaries require glibc 2.39 and OpenSSL 3; see https://github.com/sonic182/graphmem#install`;
}

export function validateBinary(candidate, expectedVersion) {
  const absolute = path.resolve(candidate);
  const result = spawnSync(absolute, ["version"], {
    encoding: "utf8", timeout: 5000, windowsHide: true,
    env: { ...process.env, GMEM_NPM_BINARY_PROBE: "1" },
  });
  if (result.error || result.status !== 0) {
    throw new Error(`Could not run ${absolute}: ${result.error?.message || result.stderr?.trim() || `exit ${result.status}`}. Linux release binaries require glibc 2.39 and OpenSSL 3.`);
  }
  if (expectedVersion && result.stdout.trim() !== `gmem ${expectedVersion}`) {
    throw new Error(`${absolute} does not report gmem ${expectedVersion}`);
  }
  return absolute;
}

function usable(candidate, expectedVersion) {
  try { return validateBinary(candidate, expectedVersion); } catch { return null; }
}

// Probe flags stop discovery from recursively entering another npm launcher.
export function resolveBinary() {
  for (const dir of (process.env.PATH ?? "").split(path.delimiter).filter(Boolean)) {
    const candidate = path.resolve(dir, executable);
    try {
      if (realpathSync(candidate) === realpathSync(launcher)) continue;
    } catch { continue; }
    const native = usable(candidate);
    if (native) return native;
  }
  if (!target) return null;
  return usable(cached, version)
    ?? usable(path.join(packageRoot, "vendor", target, executable), version);
}

async function download(url, signal) {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error(`GitHub Release download failed (${response.status}): ${url}`);
  return response;
}

export async function installBinary({ signal } = {}) {
  const existing = resolveBinary();
  if (existing) return existing;
  if (!target) throw new Error(missingBinaryMessage());
  signal?.throwIfAborted();
  await mkdir(directory, { recursive: true });
  const lockPath = path.join(directory, ".install.lock");
  const deadline = Date.now() + 60000;
  let lock;
  while (!lock) {
    signal?.throwIfAborted();
    try {
      lock = await open(lockPath, "wx");
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      if (Date.now() >= deadline) throw new Error(`Another install is still running. Retry later; if it was interrupted, remove ${lockPath} before retrying.`);
      await delay(100, undefined, { signal });
    }
  }
  let staging;
  let downloadTimer;
  const controller = new AbortController();
  const cancel = () => controller.abort(signal.reason);
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    const ready = usable(cached, version);
    if (ready) return ready;
    staging = await mkdtemp(path.join(directory, ".install-"));
    const archiveName = `gmem-${version}-${target}.${process.platform === "win32" ? "zip" : "tar.gz"}`;
    const release = `https://github.com/sonic182/graphmem/releases/download/${version}`;
    signal?.throwIfAborted();
    downloadTimer = setTimeout(() => controller.abort(new Error("Download timed out")), 120000);
    const downloadSignal = controller.signal;
    const [archiveResponse, sumsResponse] = await Promise.all([
      download(`${release}/${archiveName}`, downloadSignal),
      download(`${release}/SHA256SUMS`, downloadSignal),
    ]);
    const sums = await sumsResponse.text();
    const entry = sums.split(/\r?\n/).find(line => line.trim().split(/\s+/).at(-1) === archiveName);
    if (!entry) throw new Error(`SHA256SUMS does not contain ${archiveName}`);
    const expected = entry.trim().split(/\s+/)[0].toLowerCase();
    const bytes = Buffer.from(await archiveResponse.arrayBuffer());
    if (!/^[a-f0-9]{64}$/.test(expected) || createHash("sha256").update(bytes).digest("hex") !== expected) {
      throw new Error(`SHA256 mismatch for ${archiveName}`);
    }
    signal?.throwIfAborted();
    const archivePath = path.join(staging, archiveName);
    await writeFile(archivePath, bytes);
    const extracted = path.join(staging, executable);
    const result = process.platform === "win32"
      ? spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", "Expand-Archive -LiteralPath $env:GMEM_ZIP -DestinationPath $env:GMEM_DEST -Force"], {
        env: { ...process.env, GMEM_ZIP: archivePath, GMEM_DEST: staging }, encoding: "utf8", windowsHide: true, timeout: 30000,
      })
      : spawnSync("tar", ["-xzf", archivePath, "-C", staging, executable], { encoding: "utf8", timeout: 30000 });
    if (result.error || result.status !== 0) throw new Error(`Could not extract gmem: ${result.error?.message || result.stderr}`);
    if (process.platform !== "win32") await chmod(extracted, 0o755);
    try { validateBinary(extracted, version); } catch (error) {
      throw new Error(`Downloaded gmem binary failed \`gmem version\` smoke test: ${error.message}`);
    }
    signal?.throwIfAborted();
    await rename(extracted, cached);
    return cached;
  } finally {
    controller.abort();
    clearTimeout(downloadTimer);
    signal?.removeEventListener("abort", cancel);
    try {
      if (staging) await rm(staging, { recursive: true, force: true });
    } finally {
      await lock.close();
      await rm(lockPath, { force: true });
    }
  }
}
