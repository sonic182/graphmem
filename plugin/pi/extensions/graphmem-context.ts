import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// Session-start guidance, the pi equivalent of the Claude Code/Codex
// SessionStart hook. Injected once per session, before the first agent run.
const GUIDANCE = `Graphmem (gmem) is connected as an MCP server for durable project memory.

Recall: before investigating or changing code for a substantive task (coding, debugging, review, refactoring, planning, maintenance), call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Natural language and paraphrases work.

Read the result critically: a weak or off-topic result means "nothing known" - continue from the code rather than treating it as fact.

Remember: after verifying a durable decision, convention, constraint, or resolved failure cause, call \`remember\` with a concise self-contained statement and attach its \`entities\` (and \`relations\`) in the same call. Recall the topic first and update your understanding instead of storing a duplicate.

Skip both for logistical or no-code requests, and when Graphmem is unavailable. Never store secrets, private data, speculation, temporary progress updates, raw debugging output, or source code that can be read from the repository.

Load the \`graphmem-mcp-for-dev\` skill for the full contract before storing anything, or before using \`relate\`, \`graph\`, \`inspect\`, or \`forget\`.`;

export default function (pi: ExtensionAPI) {
  let injected = false;

  pi.on("session_start", () => {
    injected = false;
  });

  pi.on("before_agent_start", () => {
    if (injected) return;
    injected = true;
    return {
      message: {
        customType: "graphmem",
        content: GUIDANCE,
        display: false,
      },
    };
  });
}
