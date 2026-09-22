const eventName = process.argv[2];

const MAIN = `Graphmem (gmem) is connected as an MCP server for durable project memory.

Recall: before investigating or changing code for a substantive task (coding, debugging, review, refactoring, planning, maintenance), call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Natural language and paraphrases work.

Read the result critically: a weak or off-topic result means "nothing known" - continue from the code rather than treating it as fact.

Remember: after verifying a durable decision, convention, constraint, or resolved failure cause, call \`remember\` with a self-contained statement and attach its \`entities\` (and \`relations\`) in the same call. Shape a durable save as \`**What**\` (the fact or decision), \`**Why**\` (what motivated it), \`**Where**\` (files or components affected), and \`**Learned**\` (the gotcha, omit if none); a single fact may stay one line.

Update, do not duplicate: recall the topic first, and when a stored memory is now wrong or superseded, call \`update\` with its \`id\` instead of storing a competing second one.

Skip both for logistical or no-code requests, and when Graphmem is unavailable. Never store secrets, private data, speculation, temporary progress updates, raw debugging output, or source code that can be read from the repository.

Load the \`graphmem-mcp-for-dev\` skill for the full contract before storing anything, or before using \`update\`, \`relate\`, \`graph\`, \`inspect\`, or \`forget\`.`;

const SUBAGENT = `Graphmem (gmem) is connected as an MCP server for durable project memory.

If your task involves code, call \`recall\` exactly once with \`limit\` 3 to 5 and a query naming the concrete component, symbol, error, or decision. Treat a weak or off-topic result as "nothing known" and continue from the code.

Do not call \`remember\`, \`update\`, \`relate\`, or \`forget\`. Report what you found and let the main agent decide what is durable enough to store, so parallel agents do not write duplicate or unverified memories.`;

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
