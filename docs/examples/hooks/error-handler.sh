#!/bin/bash
# hooks/error-handler.sh
#
# PostToolUseFailure — sync (simple string form, no options)
# Fires whenever a tool call returns an error.
# Moves the agent to the error zone and shows a notification.

PAYLOAD=$(cat -)
TOOL=$(echo "$PAYLOAD" | jq -r '.tool_name // "unknown"')

echo "{\"garden_event\": {\"animation\": \"error\", \"notify\": \"$TOOL failed\", \"move_to_zone\": \"error-zone\"}}"
exit 0
