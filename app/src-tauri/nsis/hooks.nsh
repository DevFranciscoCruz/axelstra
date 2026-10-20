; The background daemon (axelstra.exe) keeps running after the window closes, and a running exe
; cannot be overwritten. Stop it before files are copied. Terminals and agents it owns end with it.
!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'taskkill /F /IM axelstra.exe'
  Sleep 500
!macroend
