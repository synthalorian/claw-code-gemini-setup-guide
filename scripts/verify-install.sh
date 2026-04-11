#!/usr/bin/env bash
# verify-install.sh — sanity-check that claw-code is wired to Google Code Assist
#
# Checks:
#   1. ~/.gemini/oauth_creds.json exists and has the expected fields
#   2. The `claw` binary is on PATH
#   3. A simple prompt round-trips
#   4. A multi-turn tool call round-trips (the real test — exercises thoughtSignature replay)

set -euo pipefail

# Colors
RED=$'\e[31m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; CYAN=$'\e[36m'; RESET=$'\e[0m'

ok()    { echo "${GREEN}✓${RESET} $*"; }
fail()  { echo "${RED}✗${RESET} $*" >&2; exit 1; }
info()  { echo "${CYAN}ℹ${RESET} $*"; }
warn()  { echo "${YELLOW}!${RESET} $*"; }

CREDS="${HOME}/.gemini/oauth_creds.json"

# 1. Credentials file
info "Checking ${CREDS}..."
[[ -f "$CREDS" ]] || fail "Missing $CREDS — run \`gemini\` once to mint OAuth tokens."

PERMS="$(stat -c '%a' "$CREDS" 2>/dev/null || stat -f '%A' "$CREDS")"
[[ "$PERMS" == "600" ]] || warn "$CREDS perms are $PERMS (should be 600). Run: chmod 600 $CREDS"

if command -v jq >/dev/null 2>&1; then
    for field in access_token refresh_token expiry_date; do
        if jq -e "has(\"$field\")" "$CREDS" >/dev/null; then
            ok "credentials have field: $field"
        else
            fail "credentials missing field: $field"
        fi
    done
else
    warn "jq not installed — skipping credential field checks"
fi

# 2. claw binary
info "Looking for the \`claw\` binary..."
if command -v claw >/dev/null 2>&1; then
    CLAW="$(command -v claw)"
    ok "found claw at $CLAW"
elif [[ -x "${HOME}/claw-code/rust/target/release/claw" ]]; then
    CLAW="${HOME}/claw-code/rust/target/release/claw"
    ok "found claw at $CLAW (release build)"
else
    fail "claw binary not found. Build with: cd ~/claw-code/rust && cargo build --release"
fi

# 3. Simple prompt
info "Running simple prompt..."
if "$CLAW" "say 'hello synthwave grid' in exactly four words" 2>&1 | tee /tmp/claw-verify-1.txt | tail -3; then
    ok "simple prompt round-tripped"
else
    fail "simple prompt failed — see /tmp/claw-verify-1.txt"
fi

# 4. Tool call round-trip (the real test)
info "Running multi-turn tool call (the thoughtSignature test)..."
if "$CLAW" "use the bash tool to run \`date +%s\` then tell me whether the number is even or odd" \
    2>&1 | tee /tmp/claw-verify-2.txt | tail -10; then
    if grep -qiE "(missing|thought_signature|400)" /tmp/claw-verify-2.txt; then
        fail "looks like a thoughtSignature 400 — check the cache wiring in providers/google_codeassist.rs"
    fi
    ok "multi-turn tool call round-tripped — thoughtSignature replay is working"
else
    fail "multi-turn tool call failed — see /tmp/claw-verify-2.txt"
fi

echo
ok "All checks passed. claw-code is wired to Google Code Assist correctly."
