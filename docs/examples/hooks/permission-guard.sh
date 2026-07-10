#!/bin/bash
# hooks/permission-guard.sh
#
# PreToolUse — phase: enforce, timeout_secs: 5
# Blocks dangerous tools in restricted zones.
#
# Exit codes:
#   0  → Allow (JSON output processed if present)
#   2  → Deny  (stderr goes to the MODEL so it understands why it was blocked)
#   1  → NonBlockingError (stderr goes to the USER as a warning; tool still executes)

PAYLOAD=$(cat -)
ZONE=$(echo "$PAYLOAD" | jq -r '.agent_zone // empty')

# Deny: junior agents in readonly-zone cannot run shell commands.
if [[ "$HOOK_TOOL_NAME" == "Bash" && "$ZONE" == "readonly-zone" ]]; then
  echo "Junior agents in readonly-zone cannot execute shell commands." >&2
  echo '{"decision": "block", "reason": "Shell execution is prohibited in readonly-zone."}'
  exit 2
fi

# NonBlockingError: warn the user but let the tool run.
if [[ "$HOOK_TOOL_NAME" == "Bash" && "$ZONE" == "unstable-zone" ]]; then
  echo "Warning: executing in unstable-zone — proceed with caution." >&2
  exit 1
fi

exit 0
