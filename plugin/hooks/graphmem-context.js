const eventName = process.argv[2];

const MAIN = `Graphmem (gmem) is connected as an MCP server for durable project memory.

Recall: before investigating or changing code for a substantive task (coding, debugging, review, refactoring, planning, maintenance), call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Natural language and paraphrases work.

Read the result critically: a weak or off-topic result means "nothing known" - continue from the code rather than treating it as fact.

Remember: after verifying a durable decision, convention, constraint, or resolved failure cause, call \`remember\` with a concise self-contained statement and attach its \`entities\` (and \`relations\`) in the same call. Recall the topic first and update your understanding instead of storing a duplicate.

Skip both for logistical or no-code requests, and when Graphmem is unavailable. Never store secrets, private data, speculation, temporary progress updates, raw debugging output, or source code that can be read from the repository.

Load the \`graphmem-mcp-for-dev\` skill for the full contract before storing anything, or before using \`relate\`, \`graph\`, \`inspect\`, or \`forget\`.`;

const SUBAGENT = `Graphmem (gmem) is connected as an MCP server for durable project memory.

If your task involves code, call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Treat a weak or off-topic result as "nothing known" and continue from the code.

Do not call \`remember\`, \`relate\`, or \`forget\`. Report what you found and let the main agent decide what is durable enough to store, so parallel agents do not write duplicate or unverified memories.`;

const output = {
  hookSpecificOutput: {
    hookEventName: eventName,
    additionalContext: eventName === 'SubagentStart' ? SUBAGENT : MAIN,
  },
};

if (eventName !== 'SubagentStart') {
  output.systemMessage = 'GRAPHMEM: memory guidance active';
}

process.stdout.write(JSON.stringify(output));
