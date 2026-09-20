import { fileURLToPath } from "node:url";

const GUIDANCE = `Graphmem (gmem) is connected as an MCP server for durable project memory.

Recall: before investigating or changing code for a substantive task (coding, debugging, review, refactoring, planning, maintenance), call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Natural language and paraphrases work.

Read the result critically: a weak or off-topic result means "nothing known" - continue from the code rather than treating it as fact.

Remember: after verifying a durable decision, convention, constraint, or resolved failure cause, call \`remember\` with a concise self-contained statement and attach its \`entities\` (and \`relations\`) in the same call. Recall the topic first and update your understanding instead of storing a duplicate.

Skip both for logistical or no-code requests, and when Graphmem is unavailable. Never store secrets, private data, speculation, temporary progress updates, raw debugging output, or source code that can be read from the repository.

Load the \`graphmem-mcp-for-dev\` skill for the full contract before storing anything, or before using \`relate\`, \`graph\`, \`inspect\`, or \`forget\`.`;

const COMPACTION = `Graphmem (gmem) MCP memory guidance: keep durable decisions, conventions, constraints, and resolved failure causes across compaction. Recall the topic before storing a duplicate; attach entities and relations on remember.`;

function skillsDir(): string {
  return fileURLToPath(new URL("../skills", import.meta.url));
}

function ensureSkillsPath(cfg: any): void {
  const dir = skillsDir();
  cfg.skills ??= {};
  const paths = cfg.skills.paths;
  if (Array.isArray(paths)) {
    if (!paths.includes(dir)) paths.push(dir);
  } else {
    cfg.skills.paths = [dir];
  }
}

function ensureMcp(cfg: any): void {
  cfg.mcp ??= {};
  const existing = cfg.mcp.gmem;
  if (existing && typeof existing === "object" && Object.keys(existing).length > 0) return;
  cfg.mcp.gmem = {
    type: "local",
    command: ["gmem", "mcp"],
    enabled: true,
  };
}

export const GraphmemPlugin = async () => {
  return {
    config: async (cfg: any) => {
      ensureMcp(cfg);
      ensureSkillsPath(cfg);
    },
    "experimental.chat.system.transform": async (_input: any, output: any) => {
      const system: string[] = output.system ??= [];
      if (!system.some((entry) => typeof entry === "string" && entry.includes("Graphmem (gmem)"))) {
        system.push(GUIDANCE);
      }
    },
    "experimental.session.compacting": async (_input: any, output: any) => {
      const context: string[] = output.context ??= [];
      if (!context.some((entry) => typeof entry === "string" && entry.includes("Graphmem (gmem)"))) {
        context.push(COMPACTION);
      }
    },
  };
};

export default GraphmemPlugin;
