; Claw Garden installer hooks, merged into Tauri's NSIS template through
; bundle.windows.nsis.installerHooks (tauri.windows.conf.json).
;
; clawg.exe is bundled as a sidecar next to the app. Everything with logic in
; it lives in clawg itself; these hooks only call it:
;
;   - `clawg stop` for BOTH profiles before files are replaced or removed.
;     Tauri's own running-app check covers only the main binary, and a live
;     daemon keeps clawg.exe open. That includes a dev daemon: the release
;     binary may serve `--profile dev`, which is how phase 3 rehearses the
;     migration (verified: a dev daemon started from the installed clawg.exe
;     blocks a native write to it). `stop` returns once that process has
;     exited (or fails after 15 s); with no daemon running it just says so.
;     Stopping a dev daemon that runs from somewhere else is harmless: the
;     next `clawg mcp` starts it again.
;   - `clawg path add` / `clawg path remove` for the user PATH. Done in Rust,
;     not here: NSIS strings are capped at 1024 characters, and truncating a
;     long PATH on write is how installers wipe their users' PATH.
;
; Exit codes are popped and ignored on purpose: none of these failing is a
; reason to abort an install or leave a half-removed app behind.

!macro NSIS_HOOK_PREINSTALL
  ${If} ${FileExists} "$INSTDIR\clawg.exe"
    nsExec::ExecToLog '"$INSTDIR\clawg.exe" stop --profile prod'
    Pop $0
    nsExec::ExecToLog '"$INSTDIR\clawg.exe" stop --profile dev'
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog '"$INSTDIR\clawg.exe" path add'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\clawg.exe"
    nsExec::ExecToLog '"$INSTDIR\clawg.exe" stop --profile prod'
    Pop $0
    nsExec::ExecToLog '"$INSTDIR\clawg.exe" stop --profile dev'
    Pop $0
    nsExec::ExecToLog '"$INSTDIR\clawg.exe" path remove'
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
