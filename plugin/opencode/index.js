import { fileURLToPath } from "node:url";
import { missingBinaryMessage, resolveBinary } from "../../scripts/binary.js";

const GUIDANCE = `Graphmem (gmem) is connected as an MCP server for durable project memory.

Recall: before investigating or changing code for a substantive task (coding, debugging, review, refactoring, planning, maintenance), call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Natural language and paraphrases work.

Read the result critically: a weak or off-topic result means "nothing known" - continue from the code rather than treating it as fact.

Remember: after verifying a durable decision, convention, constraint, or resolved failure cause, call \`remember\` with a self-contained statement and attach its \`entities\` (and \`relations\`) in the same call. Shape a durable save as \`**What**\` (the fact or decision), \`**Why**\` (what motivated it), \`**Where**\` (files or components affected), and \`**Learned**\` (the gotcha, omit if none); a single fact may stay one line.

Update, do not duplicate: recall the topic first, and when a stored memory is now wrong or superseded, call \`update\` with its \`id\` instead of storing a competing second one.

Skip both for logistical or no-code requests, and when Graphmem is unavailable. Never store secrets, private data, speculation, temporary progress updates, raw debugging output, or source code that can be read from the repository.

Code navigation: when the gmem \`find_symbol\`, \`code_outline\`, \`code_imports\`, and \`code_diff\` tools are listed, use them to find a definition by name, to list a file's symbols or imports, or to list the symbols changed between Git revisions before grepping or reading whole files; they keep their own index fresh. Use \`rg\`/\`ast-grep\` for plain text, call sites, and references, which they do not index. Before the first code lookup of the session, load the \`graphmem-code-analysis\` skill for which tool fits a task and how to read their results.

Load the \`graphmem-mcp-for-dev\` skill for the full contract before storing anything, or before using \`update\`, \`relate\`, \`graph\`, \`inspect\`, or \`forget\`.`;

const COMPACTION = `Graphmem (gmem) MCP memory guidance: keep durable decisions, conventions, constraints, and resolved failure causes across compaction. Recall the topic before storing a duplicate, and revise a superseded memory with \`update\` rather than storing a competing one; attach entities and relations on remember.`;

function skillsDir() {
  return fileURLToPath(new URL("../skills", import.meta.url));
}

function ensureSkillsPath(cfg) {
  const dir = skillsDir();
  cfg.skills ??= {};
  const paths = cfg.skills.paths;
  if (Array.isArray(paths)) {
    if (!paths.includes(dir)) paths.push(dir);
  } else {
    cfg.skills.paths = [dir];
  }
}

export const GraphmemPlugin = async ({ client } = {}) => {
  let reported = false;
  function setupMessage(message) {
    if (reported) return;
    reported = true;
    console.error(`${message}\nAfter explicit installation, restart OpenCode and check opencode mcp list.`);
  }
  async function connected() {
    try {
      const result = await client?.mcp?.status();
      return result?.data?.gmem?.status === "connected";
    } catch { return false; }
  }
  return {
    config: async cfg => {
      ensureSkillsPath(cfg);
      cfg.mcp ??= {};
      if (Object.hasOwn(cfg.mcp, "gmem")) return;
      const executable = resolveBinary();
      if (!executable) {
        setupMessage(missingBinaryMessage());
        return;
      }
      cfg.mcp.gmem = { type: "local", command: [executable, "mcp"], enabled: true };
    },
    "experimental.chat.system.transform": async (_input, output) => {
      if (!await connected()) return;
      const system = output.system ??= [];
      if (!system.some((entry) => typeof entry === "string" && entry.includes("Graphmem (gmem)"))) {
        system.push(GUIDANCE);
      }
    },
    "experimental.session.compacting": async (_input, output) => {
      if (!await connected()) return;
      const context = output.context ??= [];
      if (!context.some((entry) => typeof entry === "string" && entry.includes("Graphmem (gmem)"))) {
        context.push(COMPACTION);
      }
    },
  };
};

export default GraphmemPlugin;
