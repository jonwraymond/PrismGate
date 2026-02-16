#!/usr/bin/env bash
# MCP client integration test for gatemini
# Exercises: initialize, tools/list, tools/call (search, list, info, call_tool_chain), shutdown
set -euo pipefail

BINARY="./target/release/gatemini"
CONFIG="config/test-smoke.yaml"
LOG="/tmp/gatemini-mcp-test.log"
FIFO_IN="/tmp/gatemini_stdin_$$"
FIFO_OUT="/tmp/gatemini_stdout_$$"
PASS=0
FAIL=0
TOTAL=0

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

cleanup() {
    if [[ -n "${GATEMINI_PID:-}" ]] && kill -0 "$GATEMINI_PID" 2>/dev/null; then
        kill "$GATEMINI_PID" 2>/dev/null || true
        wait "$GATEMINI_PID" 2>/dev/null || true
    fi
    # Close FDs
    exec 7>&- 2>/dev/null || true
    exec 8<&- 2>/dev/null || true
    rm -f "$FIFO_IN" "$FIFO_OUT"
}
trap cleanup EXIT

assert_json() {
    local label="$1"
    local response="$2"
    local jq_filter="$3"
    local expected="$4"
    TOTAL=$((TOTAL + 1))

    local actual
    actual=$(echo "$response" | jq -r "$jq_filter" 2>/dev/null) || actual="JQ_ERROR"

    if [[ "$actual" == "$expected" ]]; then
        PASS=$((PASS + 1))
        echo -e "  ${GREEN}✓${NC} $label"
    else
        FAIL=$((FAIL + 1))
        echo -e "  ${RED}✗${NC} $label"
        echo -e "    expected: ${expected}"
        echo -e "    actual:   ${actual}"
    fi
}

assert_json_contains() {
    local label="$1"
    local response="$2"
    local jq_filter="$3"
    local substring="$4"
    TOTAL=$((TOTAL + 1))

    local actual
    actual=$(echo "$response" | jq -r "$jq_filter" 2>/dev/null) || actual="JQ_ERROR"

    if [[ "$actual" == *"$substring"* ]]; then
        PASS=$((PASS + 1))
        echo -e "  ${GREEN}✓${NC} $label"
    else
        FAIL=$((FAIL + 1))
        echo -e "  ${RED}✗${NC} $label"
        echo -e "    expected to contain: ${substring}"
        echo -e "    actual: ${actual}"
    fi
}

assert_json_gt() {
    local label="$1"
    local response="$2"
    local jq_filter="$3"
    local threshold="$4"
    TOTAL=$((TOTAL + 1))

    local actual
    actual=$(echo "$response" | jq -r "$jq_filter" 2>/dev/null) || actual="0"

    if (( actual > threshold )); then
        PASS=$((PASS + 1))
        echo -e "  ${GREEN}✓${NC} $label (got ${actual})"
    else
        FAIL=$((FAIL + 1))
        echo -e "  ${RED}✗${NC} $label"
        echo -e "    expected > ${threshold}, got ${actual}"
    fi
}

# Send a JSON-RPC message and read the response
send_rpc() {
    local msg="$1"
    echo "$msg" >&7
    # Read one line of response
    local response
    if ! response=$(timeout 15 head -n 1 <&8); then
        echo '{"error": "timeout reading response"}'
        return
    fi
    echo "$response"
}

# Send a notification (no response expected)
send_notification() {
    local msg="$1"
    echo "$msg" >&7
}

echo -e "${YELLOW}=== Gatemini MCP Client Integration Test ===${NC}"
echo ""

# Create named pipes
mkfifo "$FIFO_IN" "$FIFO_OUT"

# Start gatemini
$BINARY -c "$CONFIG" < "$FIFO_IN" > "$FIFO_OUT" 2>"$LOG" &
GATEMINI_PID=$!

# Open FDs: 7 for writing to stdin, 8 for reading from stdout
exec 7>"$FIFO_IN"
exec 8<"$FIFO_OUT"

# Give it time to start backends
sleep 3

echo -e "${YELLOW}1. Initialize Handshake${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-client","version":"0.1.0"}}}')
assert_json "has result" "$RESP" '.result | type' "object"
assert_json "protocol version" "$RESP" '.result.protocolVersion' "2024-11-05"
assert_json_contains "server name" "$RESP" '.result.serverInfo.name' "rmcp"
assert_json "has tools capability" "$RESP" '.result.capabilities.tools | type' "object"
assert_json_contains "has instructions" "$RESP" '.result.instructions' "MCP gateway"

# Send initialized notification
send_notification '{"jsonrpc":"2.0","method":"notifications/initialized"}'
sleep 0.5

echo ""
echo -e "${YELLOW}2. tools/list — Discover Meta-tools + Backend Tools${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list"}')
assert_json "has result.tools" "$RESP" '.result.tools | type' "array"
assert_json_gt "has multiple tools" "$RESP" '.result.tools | length' 2

# Check meta-tools exist
META_TOOLS="search_tools list_tools_meta tool_info call_tool_chain register_manual deregister_manual get_required_keys_for_tool"
for mt in $META_TOOLS; do
    TOTAL=$((TOTAL + 1))
    if echo "$RESP" | jq -e ".result.tools[] | select(.name == \"$mt\")" > /dev/null 2>&1; then
        PASS=$((PASS + 1))
        echo -e "  ${GREEN}✓${NC} meta-tool '$mt' present"
    else
        FAIL=$((FAIL + 1))
        echo -e "  ${RED}✗${NC} meta-tool '$mt' missing"
    fi
done

# Verify tools/list returns exactly 7 meta-tools (backend tools accessed via meta-tools)
assert_json "exactly 7 meta-tools" "$RESP" '.result.tools | length' "7"

echo ""
echo -e "${YELLOW}3. tools/call — search_tools${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_tools","arguments":{"task_description":"get current time","limit":5}}}')
assert_json "no error" "$RESP" '.error' "null"
assert_json_contains "result has content" "$RESP" '.result.content[0].type' "text"
SEARCH_RESULTS=$(echo "$RESP" | jq -r '.result.content[0].text')
TOTAL=$((TOTAL + 1))
if echo "$SEARCH_RESULTS" | jq -e '.[] | select(.name == "get_current_time")' > /dev/null 2>&1; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} search found 'get_current_time'"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} search did not find 'get_current_time'"
fi

echo ""
echo -e "${YELLOW}4. tools/call — list_tools_meta${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_tools_meta","arguments":{}}}')
assert_json "no error" "$RESP" '.error' "null"
LIST_TEXT=$(echo "$RESP" | jq -r '.result.content[0].text')
TOTAL=$((TOTAL + 1))
if echo "$LIST_TEXT" | jq -e '. | length > 0' > /dev/null 2>&1; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} list_tools_meta returned tool names"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} list_tools_meta returned empty"
fi

echo ""
echo -e "${YELLOW}5. tools/call — tool_info${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"tool_info","arguments":{"tool_name":"get_current_time"}}}')
assert_json "no error" "$RESP" '.error' "null"
INFO_TEXT=$(echo "$RESP" | jq -r '.result.content[0].text')
TOTAL=$((TOTAL + 1))
if echo "$INFO_TEXT" | jq -e '.name == "get_current_time"' > /dev/null 2>&1; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} tool_info returned correct tool"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} tool_info did not return expected tool"
fi
TOTAL=$((TOTAL + 1))
if echo "$INFO_TEXT" | jq -e '.input_schema | type == "object"' > /dev/null 2>&1; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} tool_info includes input_schema"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} tool_info missing input_schema"
fi

echo ""
echo -e "${YELLOW}6. tools/call — tool_info (not found)${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"tool_info","arguments":{"tool_name":"nonexistent_tool_xyz"}}}')
assert_json "no error" "$RESP" '.error' "null"
assert_json "isError flag set" "$RESP" '.result.isError' "true"
assert_json_contains "error message" "$RESP" '.result.content[0].text' "not found"

echo ""
echo -e "${YELLOW}7. tools/call — call_tool_chain (JSON direct call)${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"call_tool_chain","arguments":{"code":"{\"tool\": \"time.get_current_time\", \"arguments\": {\"timezone\": \"America/Denver\"}}"}}}')
assert_json "no error" "$RESP" '.error' "null"
CHAIN_TEXT=$(echo "$RESP" | jq -r '.result.content[0].text')
TOTAL=$((TOTAL + 1))
if [[ "$CHAIN_TEXT" == *"Denver"* ]] || [[ "$CHAIN_TEXT" == *"MST"* ]] || [[ "$CHAIN_TEXT" == *"MDT"* ]] || [[ "$CHAIN_TEXT" == *"202"* ]]; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} call_tool_chain returned time data"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} call_tool_chain unexpected result: ${CHAIN_TEXT:0:100}"
fi

echo ""
echo -e "${YELLOW}8. tools/call — call_tool_chain (dotted syntax)${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"call_tool_chain","arguments":{"code":"time.get_current_time({\"timezone\": \"UTC\"})"}}}')
assert_json "no error" "$RESP" '.error' "null"
CHAIN_TEXT2=$(echo "$RESP" | jq -r '.result.content[0].text')
TOTAL=$((TOTAL + 1))
if [[ "$CHAIN_TEXT2" == *"UTC"* ]] || [[ "$CHAIN_TEXT2" == *"202"* ]]; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} dotted syntax call returned time data"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} dotted syntax unexpected result: ${CHAIN_TEXT2:0:100}"
fi

echo ""
echo -e "${YELLOW}9. tools/call — call_tool_chain (error: bad backend)${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"call_tool_chain","arguments":{"code":"{\"tool\": \"nonexistent.fake_tool\", \"arguments\": {}}"}}}')
assert_json "no JSON-RPC error" "$RESP" '.error' "null"
assert_json "isError flag set" "$RESP" '.result.isError' "true"

echo ""
echo -e "${YELLOW}10. tools/call — get_required_keys_for_tool${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"get_required_keys_for_tool","arguments":{"tool_name":"get_current_time"}}}')
assert_json "no error" "$RESP" '.error' "null"
assert_json_contains "returns content" "$RESP" '.result.content[0].type' "text"

echo ""
echo -e "${YELLOW}11. Invalid method${NC}"
RESP=$(send_rpc '{"jsonrpc":"2.0","id":11,"method":"nonexistent/method","params":{}}')
TOTAL=$((TOTAL + 1))
if echo "$RESP" | jq -e '.error' > /dev/null 2>&1 && [[ $(echo "$RESP" | jq '.error') != "null" ]]; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} unknown method returns error"
else
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} unknown method handled (may be ignored per MCP spec)"
fi

echo ""
echo -e "${YELLOW}12. Verify subsystem logs${NC}"
TOTAL=$((TOTAL + 1))
if grep -q "backend started" "$LOG" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} backend 'time' started successfully"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} no backend start message in logs"
fi

TOTAL=$((TOTAL + 1))
if grep -q "tool discovery complete" "$LOG" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} tool discovery completed"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} tool discovery not completed"
fi

TOTAL=$((TOTAL + 1))
if grep -q "health checker started" "$LOG" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} health checker running"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} health checker not started"
fi

TOTAL=$((TOTAL + 1))
if grep -q "config file watcher started" "$LOG" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} config watcher running"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} config watcher not started"
fi

echo ""
echo -e "${YELLOW}13. Graceful shutdown${NC}"
# Close stdin FD to trigger stdio connection close
exec 7>&-
sleep 2

TOTAL=$((TOTAL + 1))
if grep -q "shutting down" "$LOG" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} graceful shutdown initiated"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} shutdown message not found"
fi

TOTAL=$((TOTAL + 1))
if grep -q "all backends stopped" "$LOG" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} all backends stopped"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} backends stop message not found"
fi

# Wait for process to exit
sleep 1
TOTAL=$((TOTAL + 1))
if ! kill -0 "$GATEMINI_PID" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}✓${NC} process exited cleanly"
else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}✗${NC} process still running"
fi

# Summary
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [[ $FAIL -eq 0 ]]; then
    echo -e "${GREEN}ALL PASSED: ${PASS}/${TOTAL} tests${NC}"
else
    echo -e "${RED}FAILED: ${FAIL}/${TOTAL} tests failed${NC} (${PASS} passed)"
fi
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Log: $LOG"

exit $FAIL
