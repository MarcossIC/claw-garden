#!/bin/bash
# hooks/git-guard.sh
#
# PreToolUse — if: "Bash(git push*)", phase: enforce, on_timeout: continue
# Only reaches here when someone is doing `git push` — the runtime already
# filtered by the if_condition, no need to re-check.
#
# Blocks direct pushes to main/master.
# Injects pre-push git status into the LLM context.

PAYLOAD=$(cat -)
BRANCH=$(git branch --show-current 2>/dev/null)

if [[ "$BRANCH" == "main" || "$BRANCH" == "master" ]]; then
  echo '{"decision": "block", "reason": "Direct push to main/master is not allowed. Create a branch and open a PR."}'
  exit 2
fi

# Inject repo state as context for the LLM before the push executes.
STATUS=$(git status --short 2>/dev/null)
REMOTE=$(git remote -v 2>/dev/null | head -1)

echo "{\"additional_context\": \"Pre-push state:\\nBranch: $BRANCH\\nRemote: $REMOTE\\nStatus:\\n$STATUS\"}"
exit 0
