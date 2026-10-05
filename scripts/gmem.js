#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { missingBinaryMessage, resolveBinary } from "./binary.js";

if (process.env.GMEM_NPM_BINARY_PROBE === "1") process.exit(1);

const command = resolveBinary();
if (!command) {
  console.error(missingBinaryMessage());
  process.exit(1);
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
