const event = process.argv[2];

process.stdout.write(JSON.stringify({
  systemMessage: 'GRAPHMEM: guidance loaded',
  hookSpecificOutput: {
    hookEventName: event,
    additionalContext: 'If a gmem MCP server is connected, before substantive coding, planning, debugging, review, refactoring, or maintenance, call recall once with a concise task-specific query and limit 3 to 5. Afterward, remember only verified durable decisions, constraints, conventions, or incidents; attach relevant entities and relations. Skip direct questions, logistics, temporary updates, source code, and secrets.',
  },
}));
