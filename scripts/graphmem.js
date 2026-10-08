#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { installBinary, installerCommand, resolveBinary } from "./binary.js";

if (process.env.GMEM_NPM_BINARY_PROBE === "1") process.exit(1);

let command = resolveBinary();
if (!command) {
  const controller = new AbortController();
  const cancel = () => controller.abort(new Error("Installation cancelled"));
  process.once("SIGINT", cancel);
  process.once("SIGTERM", cancel);
  console.error("graphmem: downloading the gmem binary (first run)...");
  try {
    command = await installBinary({ signal: controller.signal });
    console.error(`graphmem: using ${command}`);
  } catch (error) {
    console.error(`Could not install the gmem binary: ${error.message}`);
    console.error(`Retry explicitly: ${installerCommand()}`);
    process.exit(1);
  } finally {
    process.removeListener("SIGINT", cancel);
    process.removeListener("SIGTERM", cancel);
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
