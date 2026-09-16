const event = process.argv[2];

process.stdout.write(JSON.stringify({
  systemMessage: 'GRAPHMEM: MCP ready',
  hookSpecificOutput: {
    hookEventName: event,
    additionalContext: 'Graphmem MCP is available as gmem. Before substantive coding, planning, debugging, review, refactoring, or maintenance, call recall once with a concise task-specific query and limit 3 to 5. Afterward, remember only verified durable decisions, constraints, conventions, or incidents; attach relevant entities and relations. Skip direct questions, logistics, temporary updates, source code, and secrets.',
  },
}));
