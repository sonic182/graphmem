import { readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { binaryInfo, installBinary, missingBinaryMessage, resolveBinary, validateBinary } from "../../../scripts/binary.js";

// Session-start guidance, the pi equivalent of the Claude Code/Codex
// SessionStart hook. Injected once per session, before the first agent run.
const GUIDANCE = `Graphmem (gmem) is connected as an MCP server for durable project memory.

Recall: before investigating or changing code for a substantive task (coding, debugging, review, refactoring, planning, maintenance), call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Natural language and paraphrases work.

Read the result critically: a weak or off-topic result means "nothing known" - continue from the code rather than treating it as fact.

Remember: after verifying a durable decision, convention, constraint, or resolved failure cause, call \`remember\` with a self-contained statement and attach its \`entities\` (and \`relations\`) in the same call. Shape a durable save as \`**What**\` (the fact or decision), \`**Why**\` (what motivated it), \`**Where**\` (files or components affected), and \`**Learned**\` (the gotcha, omit if none); a single fact may stay one line.

Update, do not duplicate: recall the topic first, and when a stored memory is now wrong or superseded, call \`update\` with its \`id\` instead of storing a competing second one.

Skip both for logistical or no-code requests, and when Graphmem is unavailable. Never store secrets, private data, speculation, temporary progress updates, raw debugging output, or source code that can be read from the repository.

Code navigation: when the gmem \`find_symbol\`, \`code_outline\`, \`code_imports\`, and \`code_diff\` tools are listed, use them to find a definition by name, to list a file's symbols or imports, or to list the symbols changed between Git revisions before grepping or reading whole files; they keep their own index fresh. Use \`rg\`/\`ast-grep\` for plain text, call sites, and references, which they do not index. Before the first code lookup of the session, load the \`graphmem-code-analysis\` skill for which tool fits a task and how to read their results.

Load the \`graphmem-mcp-for-dev\` skill for the full contract before storing anything, or before using \`update\`, \`relate\`, \`graph\`, \`inspect\`, or \`forget\`.`;

/** @param {import("@earendil-works/pi-coding-agent").ExtensionAPI} pi */
export default function (pi) {
  let injected = false;
  let owned = false;
  let setup;
  let controller = new AbortController();

  function report(ctx, message, error = false) {
    if (ctx.mode === "tui" && ctx.hasUI) ctx.ui.notify(message, error ? "error" : "info");
    else console.error(message);
  }

  async function configured(ctx) {
    if (pi.getMcpServers().some(entry => entry.name === "gmem") && !owned) return true;
    const agentDir = process.env.PI_CODING_AGENT_DIR ?? path.join(os.homedir(), ".pi", "agent");
    const files = [path.join(agentDir, "mcp.json")];
    if (ctx.isProjectTrusted()) files.push(path.join(ctx.cwd, ".pi", "mcp.json"));
    for (const file of files) {
      try {
        const config = JSON.parse(await readFile(file, "utf8"));
        if (Object.hasOwn(config.mcpServers ?? {}, "gmem")) return true;
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
      }
    }
    return false;
  }

  async function ensureBinary(ctx, customPath) {
    const signal = controller.signal;
    try {
      if (await configured(ctx)) {
        if (customPath) report(ctx, "Graphmem uses your existing MCP configuration. Change it through /mcp instead.");
        return;
      }
      let executable = customPath ? validateBinary(expandPath(customPath, ctx.cwd)) : resolveBinary();
      if (!executable && ctx.mode === "tui" && ctx.hasUI) {
        const choice = await ctx.ui.select(
          binaryInfo.target
            ? `Graphmem needs gmem ${binaryInfo.version} for ${binaryInfo.platform}. Download from GitHub and verify its checksum?`
            : `No prebuilt gmem for ${binaryInfo.platform}. Use a trusted source-built executable or defer setup.`,
          binaryInfo.target ? ["Download", "Use existing executable", "Not now"] : ["Use existing executable", "Not now"],
        );
        signal.throwIfAborted();
        if (choice === "Download" && binaryInfo.target) {
          report(ctx, `Downloading and verifying gmem ${binaryInfo.version}…`);
          executable = await installBinary({ signal });
        } else if (choice === "Use existing executable") {
          const input = await ctx.ui.input("Path to a trusted gmem executable", "/absolute/path/to/gmem");
          signal.throwIfAborted();
          if (input?.trim()) executable = validateBinary(expandPath(input.trim(), ctx.cwd));
          else return;
        } else {
          report(ctx, "Graphmem setup deferred. Run /graphmem-setup when ready.");
          return;
        }
      }
      signal.throwIfAborted();
      if (!executable) {
        report(ctx, `${missingBinaryMessage()}\nAfter installing, restart Pi or run /graphmem-setup.`);
        return;
      }
      pi.registerMcpServer("gmem", {
        command: executable, args: ["mcp"], exposure: "direct",
        description: "Shared local project memory and code navigation",
      });
      owned = true;
      report(ctx, "Graphmem MCP registered for this session. Check /mcp for connection status.");
    } catch (error) {
      if (!signal.aborted) report(ctx, `Graphmem setup failed: ${error.message}\nRun /graphmem-setup to retry.`, true);
    }
  }

  function startSetup(ctx, customPath) {
    if (setup) return setup;
    const pending = ensureBinary(ctx, customPath).finally(() => {
      if (setup === pending) setup = undefined;
    });
    setup = pending;
    return pending;
  }

  pi.registerCommand("graphmem-setup", {
    description: "Set up Graphmem with consent, or use a trusted executable path",
    handler: (args, ctx) => startSetup(ctx, args.trim() || undefined),
  });

  pi.on("session_start", async (_event, ctx) => {
    controller.abort();
    controller = new AbortController();
    setup = undefined;
    injected = false;
    await startSetup(ctx);
  });
  pi.on("session_shutdown", () => { controller.abort(); });

  pi.on("before_agent_start", () => {
    // Registration starts a background connection; only actual tools establish readiness.
    const connected = pi.getAllTools().some(tool => tool.name.startsWith("mcp__gmem__"));
    if (injected || !connected) return;
    injected = true;
    return { message: { customType: "graphmem", content: GUIDANCE, display: false } };
  });
}

function expandPath(input, cwd) {
  return input.startsWith("~/") ? path.join(os.homedir(), input.slice(2)) : path.resolve(cwd, input);
}
