#!/bin/bash
# hooks/visual-update.sh
#
# PostToolUse — async: true
# Updates the agent's visual state after a tool executes.
# Never blocks — purely a UI effect.

if [ "$HOOK_TOOL_IS_ERROR" = "1" ]; then
  echo '{"garden_event": {"animation": "error", "notify": "Tool failed"}}'
else
  echo '{"garden_event": {"animation": "idle", "notify": "Done"}}'
fi

exit 0
