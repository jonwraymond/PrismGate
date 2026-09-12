#!/usr/bin/env node
/**
 * PrismGate SessionStart hook — inject resume context at session start.
 *
 * Runs as a Claude Code SessionStart hook.
 * Injects a resume card XML/block telling the agent to:
 * 1. Search prior context before asking what we're working on
 * 2. Call session_search(card=true) for open handles, decisions, constraints
 * 3. Use read_result(handle) for retained payloads
 *
 * Reads CLAUDE_PLUGIN_ROOT to find the PrismGate repo.
 * If prismgate is not reachable, emits a gentle instruction block.
 */

const PLUGIN_ROOT = process.env.CLAUDE_PLUGIN_ROOT || ".";

function buildResumeBlock() {
  return [
    "## PrismGate — Resume Instructions",
    "",
    "After compact or resume, recover context BEFORE asking the user what we're working on:",
    "",
    "1. **Search first**: `session_search(queries: ["summary"], sort: "timeline")` — check what we decided, what we're doing, constraints",
    "2. **Resume card**: `session_search(card=true)` — open handles, recent tools, decisions, constraints",
    "3. **Read handles**: For each listed handle, call `read_result(handle)` to get the retained payload. Do NOT paste raw output into context.",
    "4. **Search specific**: `session_search(query, source="decision")` — find decisions; `session_search(query, source="constraint")` — find constraints",
    "5. **Remember**: Durable facts are stored via `session_note(kind=decision|constraint|note)` — they survive compaction without stuffing the prompt.",
    "",
    "**DO NOT ask "what were we working on?" — SEARCH FIRST.**",
    "If search returns 0 results, proceed as a fresh session and note that in your response.",
    "",
    "### Token savings rules",
    "- Large outputs are behind handles: use `read_result(handle)`, not raw dump",
    "- Overflow content is FTS5-indexed: use `session_search(query)` to find relevant sections",
    "- Never paste raw HTML: use `fetch_and_index(url)` for web content, then search",
    "- Bash with >20 lines of output: use `call_tool_chain` with shell code instead",
  ].join("\n");
}

function buildResumeCard() {
  return [
    "```xml",
    "<resume>",
    "  <step order=\"1\">session_search(card=true) — open handles, decisions, constraints</step>",
    "  <step order=\"2\">For each handle in the card, call read_result(handle) to get retained payloads</step>",
    "  <step order=\"3\">session_search(queries=[\"summary\",\"decision\",\"constraint\"], sort=\"timeline\") — search prior context</step>",
    "  <step order=\"4\">Analyze retained payloads via call_tool_chain; do NOT paste into context</step>",
    "  <step order=\"5\">Store durable facts via session_note(kind=decision|constraint|note)</step>",
    "</resume>",
    "```",
    "",
    "**PrismGate is running. All large outputs are stored behind handles and indexed for search.**",
    "Use `read_result(handle)` and `session_search(query)` instead of re-reading or re-running.",
  ].join("\n");
}

// Claude Code SessionStart hook input: { "session_id": "...", "data": {...} }
async function main() {
  let input = {};
  try {
    const stdin = await readStdin();
    if (stdin) input = JSON.parse(stdin);
  } catch {
    // Emit instruction block even without input
    console.log(buildResumeBlock());
    process.exit(0);
  }

  // In headless mode, emit the card XML (Claude Code reads it)
  // In interactive mode, emit the resume block as additionalContext
  const isHeadless = process.env.CLAUDE_CODE_HEADLESS === "1";

  if (isHeadless) {
    // Emit as hook-specific output with additional context
    const result = {
      hookSpecificOutput: {
        hookEventName: "SessionStart",
        additionalContext: buildResumeCard(),
      },
    };
    console.log(JSON.stringify(result));
  } else {
    console.log(buildResumeBlock());
  }
}

function readStdin() {
  return new Promise((resolve) => {
    let data = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => { data += chunk; });
    process.stdin.on("end", () => resolve(data));
    process.stdin.resume();
    setTimeout(() => resolve(data), 100);
  });
}

main().catch(() => {
  console.log(buildResumeBlock());
  process.exit(0);
});
