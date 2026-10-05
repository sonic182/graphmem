import assert from "node:assert/strict";
import { chmod, copyFile, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function fixture(run) {
  const dir = await mkdtemp(path.join(os.tmpdir(), "graphmem-plugin-test-"));
  const originalPath = process.env.PATH;
  const originalAgent = process.env.PI_CODING_AGENT_DIR;
  const originalError = console.error;
  try {
    for (const name of ["scripts/binary.js", "scripts/npm-install.js", "plugin/pi/extensions/graphmem-context.js", "plugin/opencode/index.js"]) {
      await mkdir(path.dirname(path.join(dir, name)), { recursive: true });
      await copyFile(new URL(name, root), path.join(dir, name));
    }
    await writeFile(path.join(dir, "package.json"), JSON.stringify({ type: "module", version: "0.9.0" }));
    process.env.PATH = "";
    process.env.PI_CODING_AGENT_DIR = path.join(dir, "agent");
    const messages = [];
    console.error = message => messages.push(message);
    const { default: extension } = await import(pathToFileURL(path.join(dir, "plugin/pi/extensions/graphmem-context.js")));
    const events = {};
    const commands = {};
    const servers = [];
    const tools = [];
    const pi = {
      on: (name, handler) => { events[name] = handler; },
      registerCommand: (name, command) => { commands[name] = command; },
      registerMcpServer: (name, config) => { servers.push({ name, config }); },
      getMcpServers: () => servers,
      getAllTools: () => tools,
    };
    extension(pi);
    const ctx = {
      cwd: dir, mode: "print", hasUI: false, isProjectTrusted: () => false,
      ui: { notify: message => messages.push(message) },
    };
    const { default: openCodePlugin } = await import(pathToFileURL(path.join(dir, "plugin/opencode/index.js")));
    await run({ dir, events, commands, servers, tools, ctx, messages, openCodePlugin });
  } finally {
    if (originalPath === undefined) delete process.env.PATH; else process.env.PATH = originalPath;
    if (originalAgent === undefined) delete process.env.PI_CODING_AGENT_DIR; else process.env.PI_CODING_AGENT_DIR = originalAgent;
    console.error = originalError;
    await rm(dir, { recursive: true, force: true });
  }
}

// These tests cover the Pi lifecycle boundary, not the downloader's checksum contract.
test("Pi refusal never downloads, registers MCP, or injects connected guidance", async () => {
  await fixture(async ({ events, commands, servers, ctx, messages }) => {
    let prompts = 0;
    ctx.mode = "tui";
    ctx.hasUI = true;
    ctx.ui.select = async () => { prompts++; return "Not now"; };
    const fetch = global.fetch;
    let downloads = 0;
    global.fetch = () => { downloads++; throw new Error("No download allowed"); };
    try {
      await events.session_start({}, ctx);
      assert.equal(await events.before_agent_start({}, ctx), undefined);
      assert.equal(await events.before_agent_start({}, ctx), undefined);
      assert.equal(prompts, 1);
      assert.deepEqual(servers, []);
      assert.ok(messages.some(message => message.includes("/graphmem-setup")));
      await commands["graphmem-setup"].handler("", ctx);
      assert.equal(prompts, 2, "only an explicit retry asks again");
      assert.equal(downloads, 0);
    } finally { global.fetch = fetch; }
  });
});

test("Pi headless setup gives an absolute installer command without prompting", async () => {
  await fixture(async ({ dir, events, servers, ctx, messages }) => {
    ctx.mode = "rpc";
    ctx.hasUI = true;
    ctx.ui.select = () => { assert.fail("RPC must not wait for approval input"); };
    await events.session_start({}, ctx);
    assert.deepEqual(servers, []);
    assert.ok(messages.some(message => message.includes(path.join(dir, "scripts/npm-install.js"))));
  });
});

test("Pi preserves a disabled user MCP entry without prompting or changing it", async () => {
  await fixture(async ({ dir, events, servers, ctx }) => {
    const file = path.join(dir, "agent/mcp.json");
    await mkdir(path.dirname(file));
    const config = JSON.stringify({ mcpServers: { gmem: { enabled: false } } });
    await writeFile(file, config);
    ctx.mode = "tui";
    ctx.hasUI = true;
    let prompts = 0;
    ctx.ui.select = () => { prompts++; throw new Error("Existing config owns setup"); };
    await events.session_start({}, ctx);
    assert.deepEqual(servers, []);
    assert.equal(await readFile(file, "utf8"), config);
    assert.equal(prompts, 0);
    assert.equal(await events.before_agent_start({}, ctx), undefined);
  });
});

test("Pi custom executable registers an absolute path and waits for connected tools", { skip: process.platform === "win32" }, async () => {
  await fixture(async ({ dir, events, commands, servers, tools, ctx }) => {
    const executable = path.join(dir, "custom gmem");
    await writeFile(executable, "#!/bin/sh\necho 'gmem 0.9.0'\n");
    await chmod(executable, 0o755);
    await commands["graphmem-setup"].handler(executable, ctx);
    assert.equal(servers[0].config.command, executable);
    assert.deepEqual(servers[0].config.args, ["mcp"]);
    assert.equal(await events.before_agent_start({}, ctx), undefined);
    tools.push({ name: "mcp__gmem__recall" });
    assert.match((await events.before_agent_start({}, ctx)).message.content, /Graphmem/);
    assert.equal(await events.before_agent_start({}, ctx), undefined);
  });
});

test("Pi approval downloads only after consent and registers the verified package binary", { skip: process.platform === "win32" }, async () => {
  await fixture(async ({ dir, events, servers, ctx }) => {
    const payload = path.join(dir, "payload");
    await mkdir(payload);
    await writeFile(path.join(payload, "gmem"), "#!/bin/sh\necho 'gmem 0.9.0'\n");
    const archive = path.join(dir, "release.tar.gz");
    const result = spawnSync("/usr/bin/tar", ["-czf", archive, "-C", payload, "gmem"], { env: { ...process.env, PATH: "/usr/bin:/bin" } });
    assert.equal(result.status, 0);
    const bytes = await readFile(archive);
    const digest = createHash("sha256").update(bytes).digest("hex");
    const target = process.platform === "darwin" ? `${process.arch === "arm64" ? "aarch64" : "x86_64"}-apple-darwin` : "x86_64-unknown-linux-gnu";
    const archiveName = `gmem-0.9.0-${target}.tar.gz`;
    let approve;
    let shown;
    const prompted = new Promise(resolve => { shown = resolve; });
    ctx.mode = "tui";
    ctx.hasUI = true;
    ctx.ui.select = () => { shown(); return new Promise(resolve => { approve = resolve; }); };
    const fetch = global.fetch;
    let requests = 0;
    global.fetch = async url => {
      requests++;
      if (url.endsWith(`/${archiveName}`)) return new Response(bytes);
      if (url.endsWith("/SHA256SUMS")) return new Response(`${digest}  ${archiveName}\n`);
      throw new Error(`Unexpected URL: ${url}`);
    };
    try {
      const setup = events.session_start({}, ctx);
      await prompted;
      assert.equal(requests, 0);
      // Only extraction needs tar on PATH; no host gmem may leak into this fixture.
      const tools = path.join(dir, "tools");
      await mkdir(tools);
      await symlink("/usr/bin/tar", path.join(tools, "tar"));
      await symlink("/usr/bin/gzip", path.join(tools, "gzip"));
      process.env.PATH = tools;
      approve("Download");
      await setup;
      assert.equal(requests, 2);
      assert.equal(servers[0].config.command, path.join(dir, "vendor/0.9.0", target, "gmem"));
      assert.equal(await events.before_agent_start({}, ctx), undefined, "validated binary is not yet a connected MCP server");
    } finally { global.fetch = fetch; }
  });
});

test("Pi shutdown invalidates a pending approval without downloading", async () => {
  await fixture(async ({ events, servers, ctx }) => {
    let approve;
    let shown;
    const prompted = new Promise(resolve => { shown = resolve; });
    ctx.mode = "tui";
    ctx.hasUI = true;
    ctx.ui.select = () => { shown(); return new Promise(resolve => { approve = resolve; }); };
    const fetch = global.fetch;
    let requests = 0;
    global.fetch = async () => { requests++; throw new Error("Unexpected network"); };
    try {
      const setup = events.session_start({}, ctx);
      await prompted;
      events.session_shutdown();
      approve("Download");
      await setup;
      assert.equal(requests, 0);
      assert.deepEqual(servers, []);
    } finally { global.fetch = fetch; }
  });
});

test("OpenCode missing binary reports one runnable command, with no phantom MCP or guidance", async () => {
  await fixture(async ({ dir, openCodePlugin, messages }) => {
    const hooks = await openCodePlugin();
    const config = {};
    await hooks.config(config);
    await hooks.config(config);
    assert.equal(config.mcp.gmem, undefined);
    assert.deepEqual(config.skills.paths, [path.join(dir, "plugin/skills")]);
    assert.equal(messages.length, 1);
    assert.ok(messages[0].includes(path.join(dir, "scripts/npm-install.js")));
    const output = { system: [], context: [] };
    await hooks["experimental.chat.system.transform"]({}, output);
    await hooks["experimental.session.compacting"]({}, output);
    assert.deepEqual(output, { system: [], context: [] });
  });
});

test("OpenCode preserves user configuration and injects guidance only after connection", async () => {
  await fixture(async ({ openCodePlugin, messages }) => {
    let status = "failed";
    const hooks = await openCodePlugin({ client: { mcp: { status: async () => ({ data: { gmem: { status } } }) } } });
    const existing = { type: "local", command: ["custom-gmem", "mcp"], enabled: false };
    const config = { mcp: { gmem: existing } };
    await hooks.config(config);
    assert.deepEqual(config.mcp.gmem, existing);
    assert.deepEqual(messages, []);
    const output = { system: [], context: [] };
    await hooks["experimental.chat.system.transform"]({}, output);
    assert.deepEqual(output.system, []);
    status = "connected";
    await hooks["experimental.chat.system.transform"]({}, output);
    await hooks["experimental.chat.system.transform"]({}, output);
    await hooks["experimental.session.compacting"]({}, output);
    assert.equal(output.system.length, 1);
    assert.match(output.system[0], /Graphmem/);
    assert.equal(output.context.length, 1);
  });
});
