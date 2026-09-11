#!/usr/bin/env node
/**
 * PrismGate bypass guard — host PreToolUse hook.
 *
 * Redirects Bash, Read, and WebFetch calls through gatemini MCP tools
 * so raw output never enters the conversation.
 *
 * Designed to run as a Claude Code hook (command type).
 * Reads CLAUDE_PLUGIN_ROOT, checks for gatemini availability,
 * and emits Claude Code hook-compatible JSON.
 *
 * Strategy:
 * - For Bash: if command likely produces >20 lines or is NOT git/mkdir/rm/mv/cd/ls/npm/pip,
 *   redirect to call_tool_chain with a shell snippet.
 * - For Read: always redirect to execute_file.
 * - For WebFetch: always block, suggest fetch_and_index.
 *
 * In headless mode (CLAUDE_CODE_HEADLESS=1), we inject context rather than deny,
 * because headless agents have no UI to reconsider.
 */

const isHeadless = () => process.env.CLAUDE_CODE_HEADLESS === "1";
const PLUGIN_ROOT = process.env.CLAUDE_PLUGIN_ROOT || ".";
const GATEMINI_AVAILABLE = `${PLUGIN_ROOT}/gatemini` || null; // check at runtime

// Tools that are safe to run raw (short output, low context risk)
const SAFE_BASH_PATTERNS = [
  /^(git\s)/,
  /^(mkdir\s)/,
  /^(rm\s)/,
  /^(mv\s)/,
  /^(cd\s)/,
  /^(ls\s)/,
  /^(npm\s)/,
  /^(pip\s)/,
  /^(npx\s)/,
  /^(cargo\s)/,
  /^(make\s)/,
  /^(echo\s)/,
  /^(cat\s)/,  // cat is small-file reads, OK
];

function isSafeBash(command) {
  return SAFE_BASH_PATTERNS.some(p => p.test(command));
}

function estimateLines(command) {
  // Rough estimate: count pipes, redirects, and heuristics
  const pipes = (command.match(/\|/g) || []).length;
  const hasRedirect = />[>]?|\|/.test(command);
  const isDataFetch = /(curl|wget|gh|fetch|http|api|json)/i.test(command);
  return { pipes, hasRedirect, isDataFetch };
}

/**
 * Emit a Claude Code PreToolUse hook response that injects context
 * suggesting the agent use gatemini tools.
 */
function injectContext(suggestion) {
  if (isHeadless()) return null; // In headless, let the tool through (same as context-mode)
  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      additionalContext: suggestion,
    },
  };
}

/**
 * For Bash: if dangerous, inject context to use call_tool_chain.
 * For Read: inject context to use execute_file.
 * For WebFetch: block or inject context to use fetch_and_index.
 */
function routeTool(eventName, toolName, input) {
  if (eventName === "PreToolUse") {
    if (toolName === "WebFetch") {
      const url = input?.url || input?.urlString || "";
      if (isHeadless()) return null; // Let it through in headless
      return {
        hookSpecificOutput: {
          hookEventName: "PreToolUse",
          permissionDecision: "deny",
          reason: `WebFetch blocked — raw HTML floods context. Use fetch_and_index("${url}") then session_search(query) instead. See gatemini context skills.`,
        },
      };
    }

    if (toolName === "Read") {
      const path = input?.path || "";
      if (!path) return null; // Can't redirect without a path
      if (isHeadless()) return null;
      return injectContext(
        `Use execute_file("${path}") via gatemini instead of Read. ` +
        `It reads the file server-side and returns only structured output. ` +
        `Raw file content stays behind result handles and is searchable via session_search. ` +
        `If gatemini is unavailable, Read with analysis intent only.`
      );
    }

    if (toolName === "Bash") {
      const command = input?.command || "";
      if (!command) return null;
      const { pipes, hasRedirect, isDataFetch } = estimateLines(command);
      const isSafe = isSafeBash(command);
      const likelyLarge = pipes > 1 || hasRedirect || isDataFetch;

      if (isSafe && !likelyLarge) return null; // Safe bash passes through

      if (isHeadless()) return null; // Let it through in headless mode

      return injectContext(
        `For this command, prefer gatemini tools to avoid context flooding: ` +
        `call_tool_chain("const r = await backend.exec({command: \`${command}\"}); return r;") ` +
        `or execute_file for file operations. ` +
        `Single git/mkdir/rm/mv/cd/ls/npm/pip calls are OK to run directly.`
      );
    }
  }

  return null; // Passthrough for all other tools
}

// Claude Code calls this as: node hook.mjs < hook-input.json
// stdin contains: { "session_id": "...", "tool_name": "...", "input": {...}, "permission_mode": "..." }
async function main() {
  let input = {};
  try {
    const stdin = await readStdin();
    if (stdin) input = JSON.parse(stdin);
  } catch {
    // No stdin or invalid JSON — emit passthrough
    process.exit(0);
  }

  const toolName = input.tool_name || "";
  const eventName = input.hookEvent || "PreToolUse";
  const toolInput = input.tool_input || input.input || {};

  const result = routeTool(eventName, toolName, toolInput);
  if (result) {
    console.log(JSON.stringify(result));
  }
  // No output = passthrough
}

function readStdin() {
  return new Promise((resolve) => {
    let data = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => { data += chunk; });
    process.stdin.on("end", () => resolve(data));
    process.stdin.resume();
    // Timeout: if no data in 100ms, return empty
    setTimeout(() => resolve(data), 100);
  });
}

main().catch(() => process.exit(0));
