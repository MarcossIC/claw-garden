@echo off
REM hooks/zone-router.bat
REM
REM PreToolUse — async: true
REM Moves the agent to the correct zone BEFORE the tool executes.
REM Does not block — purely a visual effect.
REM
REM Reads HOOK_TOOL_NAME from environment variable.

if "%HOOK_TOOL_NAME%"=="Bash" (
    echo {"garden_event": {"move_to_zone": "terminal-zone", "animation": "ready"}}
    exit /b 0
)
if "%HOOK_TOOL_NAME%"=="execute_command" (
    echo {"garden_event": {"move_to_zone": "terminal-zone", "animation": "ready"}}
    exit /b 0
)
if "%HOOK_TOOL_NAME%"=="edit_file" (
    echo {"garden_event": {"move_to_zone": "desk-zone", "animation": "typing"}}
    exit /b 0
)
if "%HOOK_TOOL_NAME%"=="write_file" (
    echo {"garden_event": {"move_to_zone": "desk-zone", "animation": "typing"}}
    exit /b 0
)
if "%HOOK_TOOL_NAME%"=="read_file" (
    echo {"garden_event": {"move_to_zone": "bookshelf-zone", "animation": "searching"}}
    exit /b 0
)
if "%HOOK_TOOL_NAME%"=="glob_search" (
    echo {"garden_event": {"move_to_zone": "bookshelf-zone", "animation": "searching"}}
    exit /b 0
)
if "%HOOK_TOOL_NAME%"=="grep_search" (
    echo {"garden_event": {"move_to_zone": "bookshelf-zone", "animation": "searching"}}
    exit /b 0
)

exit /b 0
