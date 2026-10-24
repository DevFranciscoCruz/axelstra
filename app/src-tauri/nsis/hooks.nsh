; The window (axelstra-app.exe) and the background daemon (axelstra.exe) can be running during an update:
; the daemon owns the terminals and agents. Windows will not let a running exe be overwritten or deleted,
; but it does let it be renamed. Move both aside so nothing has to be closed: the open window and the
; daemon keep running their old code, the new files are installed under the original names, and the next
; launch uses them. The installer's "is running" check looks for the original name, so it stays quiet.
; The new app reconnects to the old daemon, and replaces it only if its protocol is too old
; (see `connect` in app/src-tauri/src/main.rs).
; The processes whose exe lives in $INSTDIR are found and ended by PID. `taskkill /IM` goes by image name, so it would
; also end another user's copy or an unrelated program called the same. ($$ is a literal $ for PowerShell.)
!macro AXELSTRA_CLOSE_WINDOWS
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "$$d='$INSTDIR'; Get-Process | Where-Object { $$_.Path -and $$_.Path.StartsWith($$d, 'OrdinalIgnoreCase') -and $$_.ProcessName -like 'axelstra-app*' } | ForEach-Object { [void]$$_.CloseMainWindow() }"`
!macroend

!macro AXELSTRA_KILL_WINDOWS
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "$$d='$INSTDIR'; Get-Process | Where-Object { $$_.Path -and $$_.Path.StartsWith($$d, 'OrdinalIgnoreCase') -and $$_.ProcessName -like 'axelstra-app*' } | ForEach-Object { Stop-Process -Id $$_.Id -Force }"`
!macroend

!macro AXELSTRA_KILL_ALL
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "$$d='$INSTDIR'; Get-Process | Where-Object { $$_.Path -and $$_.Path.StartsWith($$d, 'OrdinalIgnoreCase') -and $$_.ProcessName -like 'axelstra*' } | ForEach-Object { Stop-Process -Id $$_.Id -Force }"`
!macroend

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
  !insertmacro AXELSTRA_CLOSE_WINDOWS
  Sleep 1500
  !insertmacro AXELSTRA_KILL_WINDOWS
  Sleep 500
  ; leftovers of earlier updates; the ones still running cannot be deleted and stay
  Delete "$INSTDIR\axelstra.old.*.exe"
  Delete "$INSTDIR\axelstra-app.old.*.exe"
  !insertmacro AXELSTRA_MOVE_ASIDE "axelstra"
  !insertmacro AXELSTRA_MOVE_ASIDE "axelstra-app"
!macroend

; Uninstalling ends every copy, the renamed ones included.
!macro NSIS_HOOK_PREUNINSTALL
  ; the daemon (axelstra, axelstra.old.*) and every window (axelstra-app*) of this install, by PID
  !insertmacro AXELSTRA_KILL_ALL
  Sleep 500
  Delete "$INSTDIR\axelstra.old.*.exe"
  Delete "$INSTDIR\axelstra-app.old.*.exe"
!macroend
