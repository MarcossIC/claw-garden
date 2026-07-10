@echo off
REM hooks/idle-animation.bat
REM
REM Stop — async: true
REM Fires when the agent finishes its entire turn (end_turn).

echo {"garden_event": {"animation": "idle", "notify": "Waiting..."}}
exit /b 0
