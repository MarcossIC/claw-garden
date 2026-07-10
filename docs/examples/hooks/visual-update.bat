@echo off
REM hooks/visual-update.bat
REM
REM PostToolUse — async: true
REM Updates the agent's visual state after a tool executes.
REM Never blocks — purely a UI effect.

if "%HOOK_TOOL_IS_ERROR%"=="1" (
    echo {"garden_event": {"animation": "error", "notify": "Tool failed"}}
) else (
    echo {"garden_event": {"animation": "idle", "notify": "Done"}}
)
exit /b 0
