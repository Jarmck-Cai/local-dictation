!ifndef LOCAL_DICTATION_CLEANUP_HOOKS
!define LOCAL_DICTATION_CLEANUP_HOOKS
!include "LogicLib.nsh"

LangString LocalDictationCleanupInstallFailure 1033 "The daily audio cleanup task could not be configured. Local Dictation will still clean expired audio when opened and while running. See installation details for the error."
LangString LocalDictationCleanupInstallFailure 2052 "无法配置每天录音清理任务。Local Dictation 仍会在启动和运行时清理过期录音。错误原因见安装详情。"
LangString LocalDictationCleanupUninstallFailure 1033 "Audio cleanup or login startup settings could not be removed. See uninstall details. Only entries owned by this user and installation can be removed."
LangString LocalDictationCleanupUninstallFailure 2052 "无法移除录音清理或登录启动设置。错误原因见卸载详情；只会移除属于当前用户和本安装目录的项目。"

; ExecToStack starts PowerShell without a console window. -File binds the path
; as data: never use -Command, cmd.exe, or source interpolation for $INSTDIR.
; The uninstall copy gets the original .exe path as a separate data parameter.
!macro LocalDictationCleanupTask MODE SCRIPT_PATH FAILURE_MESSAGE
  Push $0
  Push $1
  !if "${MODE}" == "Uninstall"
    nsExec::ExecToStack /TIMEOUT=30000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "${SCRIPT_PATH}" -Mode Uninstall -InstallExecutable "$INSTDIR\local-dictation.exe"'
  !else
    nsExec::ExecToStack /TIMEOUT=30000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "${SCRIPT_PATH}" -Mode Install'
  !endif
  Pop $0
  Pop $1
  ${If} $0 != "0"
    DetailPrint "Local Dictation cleanup task ${MODE} failed ($0): $1"
    MessageBox MB_OK|MB_ICONEXCLAMATION "${FAILURE_MESSAGE}" /SD IDOK
  ${Else}
    DetailPrint "Local Dictation cleanup task ${MODE}: completed."
  ${EndIf}
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro LocalDictationCleanupTask Install "$INSTDIR\resources\cleanup-task.ps1" "$(LocalDictationCleanupInstallFailure)"
!macroend

; PREUNINSTALL precedes Tauri's cancellable app-running check. Only preserve
; the resource here; changing the task now would break a cancelled uninstall.
!macro NSIS_HOOK_PREUNINSTALL
  InitPluginsDir
  CreateDirectory "$PLUGINSDIR\LocalDictationCleanup"
  CopyFiles /SILENT "$INSTDIR\resources\cleanup-task.ps1" "$PLUGINSDIR\LocalDictationCleanup"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro LocalDictationCleanupTask Uninstall "$PLUGINSDIR\LocalDictationCleanup\cleanup-task.ps1" "$(LocalDictationCleanupUninstallFailure)"
!macroend

!endif
