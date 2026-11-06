; CopyPaste → Kept, for installations that are already out there.
;
; The updater runs this installer silently, so it is the only place that can take
; the old product away. NSIS identifies an installed product by its name, and the
; name has changed — so without this the update would install *beside* the old
; copy: two clipboard watchers, two autostart entries, and one of them still on the
; abandoned data folder. The new app carries the data across itself (see
; `src-tauri/src/legacy.rs`); this hook only clears the old program away.

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Removing the previous CopyPaste installation…"

  ; The old build is usually still running: it is the process that launched this
  ; installer. `nsExec` keeps its output out of the installer's own window.
  nsExec::Exec 'taskkill /IM copypaste.exe /F'
  Pop $0

  ; Its own uninstaller removes the shortcuts, the uninstall entry and the folder.
  IfFileExists "$LOCALAPPDATA\CopyPaste\uninstall.exe" 0 kept_no_old_uninstaller
    ExecWait '"$LOCALAPPDATA\CopyPaste\uninstall.exe" /S _?=$LOCALAPPDATA\CopyPaste'
    DetailPrint "The previous CopyPaste installation was uninstalled."
  kept_no_old_uninstaller:

  ; Whatever the uninstaller declined to remove, and the login entry it left behind.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "CopyPaste"
  RMDir /r "$LOCALAPPDATA\CopyPaste"
!macroend
