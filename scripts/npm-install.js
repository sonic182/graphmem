#!/usr/bin/env node
import { installBinary, installerCommand } from "./binary.js";

const controller = new AbortController();
const cancel = () => controller.abort(new Error("Installation cancelled"));
process.once("SIGINT", cancel);
process.once("SIGTERM", cancel);
try {
  const executable = await installBinary({ signal: controller.signal });
  console.error(`Using gmem: ${executable}`);
} catch (error) {
  console.error(`Could not install the gmem binary: ${error.message}`);
  console.error(`Retry explicitly: ${installerCommand()}`);
  process.exitCode = 1;
} finally {
  process.removeListener("SIGINT", cancel);
  process.removeListener("SIGTERM", cancel);
}
