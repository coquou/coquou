; Hooks for the Windows NSIS installer (tauri.conf.json bundle.windows.nsis.installerHooks).
;
; Up to 0.0.3 the presets shipped as files in $INSTDIR\presets\; they are now
; compiled into Coquou.exe (src/core/presets.rs). The installer never deletes
; files the new version doesn't ship: in-app updates (/UPDATE) skip running
; the old uninstaller, and a later uninstall only deletes what its own version
; installed. So the old folder (where 0.0.3 also left the renamed tuantuan)
; would stay on disk forever. Delete it on install.
;
; Remove this hook once updating from 0.0.3 or earlier is no longer supported.

!macro NSIS_HOOK_PREINSTALL
  ; This hook runs before the template's own running-app check; check first so
  ; that cancelling at that prompt cannot leave a running old Coquou, which
  ; still reads presets\, without its presets.
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  RMDir /r "$INSTDIR\presets"
!macroend
