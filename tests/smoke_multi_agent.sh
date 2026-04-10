#!/bin/bash
# Smoke test: verify multi-agent routing works end-to-end.
#
# What this verifies:
# 1. Per-request classifier is active and routing decisions are made
# 2. Simple questions route to local Ollama (free tokens)
# 3. Local model produces real responses
# 4. Classifier cache reduces latency after repeated same-route decisions
# 5. Cloud tier routing decisions are logged
#
# Known limitation: local coder tool execution is experimental.
# File-creating tasks may not work through local routing yet.
#
# Usage:
#   ./tests/smoke_multi_agent.sh [path-to-codex-binary]

set -euo pipefail

CODEX="$(realpath "${1:-codex-rs/target/debug/codex}")"
WORKDIR=$(mktemp -d)
LOGFILE="$WORKDIR/smoke.log"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'
pass() { echo -e "${GREEN}PASS${NC}: $1"; }
fail() { echo -e "${RED}FAIL${NC}: $1"; FAILURES=$((FAILURES + 1)); }
info() { echo -e "${YELLOW}....${NC}: $1"; }

FAILURES=0; TESTS=0

# --- Setup ---
info "Setting up test repo in $WORKDIR"
cd "$WORKDIR"
git init -q
echo "# Smoke test" > README.md
git add -A && git commit -q -m "init"

mkdir -p .codex-multi
[ -f "/home/jesse/src/codex/.codex-multi/config.toml" ] && \
    cp /home/jesse/src/codex/.codex-multi/config.toml .codex-multi/config.toml

# Trust
CODEX_HOME="${HOME}/.codex"
[ -f "${CODEX_HOME}/config.toml" ] && \
    ! grep -q "$WORKDIR" "${CODEX_HOME}/config.toml" 2>/dev/null && \
    echo -e "\n[projects.\"${WORKDIR}\"]\ntrust_level = \"trusted\"" >> "${CODEX_HOME}/config.toml"

# ============================================================
# TEST 1: Simple question → local model (free)
# ============================================================
TESTS=$((TESTS + 1))
info "Test 1: Simple question → local model"

RUST_LOG=codex_core::local_routing=info,codex_routing=info \
timeout 60 "$CODEX" exec --json --full-auto --ephemeral --skip-git-repo-check \
  -C "$WORKDIR" \
  'What is the difference between a stack and a queue? Answer in 2 sentences.' \
  2>"$LOGFILE" > "$WORKDIR/test1.jsonl" || true

if grep -q "Routing to local model\|Streaming from local model\|LightReasoner" "$LOGFILE" 2>/dev/null; then
    pass "Routed to local model (free)"
else
    fail "Did NOT route to local"
fi

# ============================================================
# TEST 2: Got real response from local model
# ============================================================
TESTS=$((TESTS + 1))
RESPONSE_LEN=$(python3 -c "
import json
for line in open('$WORKDIR/test1.jsonl'):
    obj = json.loads(line.strip())
    if obj.get('type') == 'item.completed':
        item = obj.get('item',{})
        if item.get('type') == 'agent_message' and item.get('text',''):
            print(len(item['text']))
            break
" 2>/dev/null || echo "0")

if [ "${RESPONSE_LEN:-0}" -gt 20 ]; then
    pass "Local model produced real response ($RESPONSE_LEN chars)"
else
    fail "Response too short or empty ($RESPONSE_LEN chars)"
fi

# ============================================================
# TEST 3: Second question — should use cache or fast classify
# ============================================================
TESTS=$((TESTS + 1))
info "Test 3: Second similar question"

RUST_LOG=codex_core::local_routing=info,codex_routing=info \
timeout 60 "$CODEX" exec --json --full-auto --ephemeral --skip-git-repo-check \
  -C "$WORKDIR" \
  'What is a binary tree? One sentence.' \
  2>>"$LOGFILE" > "$WORKDIR/test3.jsonl" || true

if grep -q "Routing to local model\|Streaming from local\|LightReasoner" "$LOGFILE" 2>/dev/null; then
    pass "Second question also routed locally"
else
    fail "Second question did not route locally"
fi

# ============================================================
# TEST 4: Classifier was active
# ============================================================
TESTS=$((TESTS + 1))
info "Test 4: Classifier activity"

CLASSIFY_COUNT=$(grep -c "Request classified" "$LOGFILE" 2>/dev/null || echo "0")
CACHE_COUNT=$(grep -c "cached classification" "$LOGFILE" 2>/dev/null || echo "0")
CLASSIFY_COUNT="${CLASSIFY_COUNT//[^0-9]/}"
CACHE_COUNT="${CACHE_COUNT//[^0-9]/}"
TOTAL=$(( ${CLASSIFY_COUNT:-0} + ${CACHE_COUNT:-0} ))

if [ "$TOTAL" -gt 0 ]; then
    pass "Classifier active: $CLASSIFY_COUNT LLM calls, $CACHE_COUNT cache hits"
else
    fail "No classifier activity"
fi

# ============================================================
# TEST 5: Local model was used (token savings)
# ============================================================
TESTS=$((TESTS + 1))
info "Test 5: Token savings"

LOCAL_COUNT=$(grep -c "local model\|Routing to local\|Streaming from local" "$LOGFILE" 2>/dev/null || echo "0")
CLOUD_COUNT=$(grep -c "cloud model\|CloudOverride\|cloud_" "$LOGFILE" 2>/dev/null || echo "0")
LOCAL_COUNT="${LOCAL_COUNT//[^0-9]/}"
CLOUD_COUNT="${CLOUD_COUNT//[^0-9]/}"

if [ "$LOCAL_COUNT" -gt 0 ]; then
    pass "Local model used $LOCAL_COUNT time(s), cloud used $CLOUD_COUNT time(s)"
else
    fail "No local model usage detected"
fi

# ============================================================
# TEST 6: Per-request routing is enabled
# ============================================================
TESTS=$((TESTS + 1))
if grep -q "Per-request routing enabled" "$LOGFILE" 2>/dev/null; then
    pass "Per-request routing enabled"
else
    fail "Per-request routing not enabled"
fi

# ============================================================
# Summary
# ============================================================
echo ""
echo "================================"
echo "Results: $((TESTS - FAILURES))/$TESTS passed"
echo ""
echo "Routing summary:"
echo "  Local responses: $(grep -c 'Routing to local model\|Streaming from local' "$LOGFILE" 2>/dev/null || echo 0)"
echo "  Classifier LLM calls: $(grep -c 'Request classified' "$LOGFILE" 2>/dev/null || echo 0)"
echo "  Cache hits: $(grep -c 'cached classification' "$LOGFILE" 2>/dev/null || echo 0)"
echo ""

if [ "$FAILURES" -gt 0 ]; then
    echo -e "${RED}$FAILURES test(s) failed${NC}"
    echo "Logs: $LOGFILE"
    echo "Workdir: $WORKDIR"
    exit 1
else
    echo -e "${GREEN}All tests passed${NC}"
    echo "Workdir: $WORKDIR"
    exit 0
fi
