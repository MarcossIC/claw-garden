#!/bin/bash
# hooks/zone-router.sh
#
# PreToolUse — async: true
# Moves the agent to the correct zone BEFORE the tool executes.
# Does not block — purely a visual effect.
#
# Receives payload via stdin (JSON).
# Reads HOOK_TOOL_NAME from env for fast access without jq.

case $HOOK_TOOL_NAME in
  Bash|execute_command)
    echo '{"garden_event": {"move_to_zone": "terminal-zone", "animation": "ready"}}'
    ;;
  edit_file|write_file)
    echo '{"garden_event": {"move_to_zone": "desk-zone", "animation": "typing"}}'
    ;;
  read_file|glob_search|grep_search)
    echo '{"garden_event": {"move_to_zone": "bookshelf-zone", "animation": "searching"}}'
    ;;
  think|analyze)
    echo '{"garden_event": {"move_to_zone": "thinking-zone", "animation": "thinking"}}'
    ;;
esac

exit 0
