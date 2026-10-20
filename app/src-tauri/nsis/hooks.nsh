; The window (axelstra-app.exe) and the background daemon (axelstra.exe) can be running during an update:
; the daemon owns the terminals and agents. Windows will not let a running exe be overwritten or deleted,
; but it does let it be renamed. Move both aside so nothing has to be closed: the open window and the
; daemon keep running their old code, the new files are installed under the original names, and the next
; launch uses them. The installer's "is running" check looks for the original name, so it stays quiet.
; The new app reconnects to the old daemon, and replaces it only if its protocol is too old
; (see `connect` in app/src-tauri/src/main.rs).
!macro AXELSTRA_MOVE_ASIDE name
  ${If} ${FileExists} "$INSTDIR\${name}.exe"
    System::Call 'kernel32::GetTickCount()i.r0'
    Rename "$INSTDIR\${name}.exe" "$INSTDIR\${name}.old.$0.exe"
    ; deleted right away when that exe was not running
    Delete "$INSTDIR\${name}.old.$0.exe"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; close the old window (the daemon keeps the terminals) so two windows never coexist. The window asks
  ; for confirmation before it closes, so the polite request is followed by a forced one: only the UI
  ; process dies, the daemon (axelstra.exe) and the terminals it owns are not touched.
  nsExec::Exec 'taskkill /IM axelstra-app.exe'
  nsExec::Exec 'taskkill /IM axelstra-app.old.*.exe'
  Sleep 1500
  nsExec::Exec 'taskkill /F /IM axelstra-app.exe'
  nsExec::Exec 'taskkill /F /IM axelstra-app.old.*.exe'
  Sleep 500
  ; leftovers of earlier updates; the ones still running cannot be deleted and stay
  Delete "$INSTDIR\axelstra.old.*.exe"
  Delete "$INSTDIR\axelstra-app.old.*.exe"
  !insertmacro AXELSTRA_MOVE_ASIDE "axelstra"
  !insertmacro AXELSTRA_MOVE_ASIDE "axelstra-app"
!macroend

; Uninstalling ends every copy, the renamed ones included.
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /IM axelstra.exe'
  nsExec::Exec 'taskkill /F /IM axelstra.old.*.exe'
  nsExec::Exec 'taskkill /F /IM axelstra-app.old.*.exe'
  Sleep 500
  Delete "$INSTDIR\axelstra.old.*.exe"
  Delete "$INSTDIR\axelstra-app.old.*.exe"
!macroend
