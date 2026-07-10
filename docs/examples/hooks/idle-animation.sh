#!/bin/bash
# hooks/idle-animation.sh
#
# Stop — async: true
# Fires when the agent finishes its entire turn (end_turn).
# The agent returned a text response — no more tool calls pending.

echo '{"garden_event": {"animation": "idle", "notify": "Waiting..."}}'
exit 0
