; Hooks for the Windows NSIS installer (tauri.conf.json bundle.windows.nsis.installerHooks).
;
; The installer only writes this version's files over the old ones, and the
; uninstaller only deletes the files its own version shipped. In-app updates
; (/UPDATE) also skip running the old uninstaller first. So a preset that a
; newer version renamed or removed would stay in presets\ and still be listed,
; and no later uninstall would know to delete it. Clearing the whole folder
; keeps presets\ exactly what this version ships.

!macro NSIS_HOOK_PREINSTALL
  ; This hook runs before the template's own running-app check; check first so
  ; that cancelling at that prompt cannot leave a running Coquou without presets.
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  RMDir /r "$INSTDIR\presets"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  RMDir /r "$INSTDIR\presets"
  ; The template's non-recursive RMDir ran while presets\ was still there.
  RMDir "$INSTDIR"
!macroend
