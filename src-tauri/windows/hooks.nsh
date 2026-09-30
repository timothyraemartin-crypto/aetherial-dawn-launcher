; One-click install: a plain double-click on the installer shows no pages.
; When the installer was started with no arguments at all, it starts itself
; again as "/P /R" and closes before any window appears (with exit code 0, so
; anything waiting on it sees success). Passive mode skips the Welcome, folder
; and Finish pages, shows only the progress bar, closes itself, and makes the
; desktop shortcut; /R starts the launcher when the install is done. Any
; argument (/P for the launcher's own updates, /D=, /NS and the rest) keeps the
; installer as it is, so nothing given on the command line is lost. Silent
; installs (/S) never reach .onGUIInit.
!define MUI_CUSTOMFUNCTION_GUIINIT AdOneClickGuiInit

Function AdOneClickGuiInit
  ${GetParameters} $R0
  ${If} $R0 == ""
    ClearErrors
    Exec '"$EXEPATH" /P /R'
    ${IfNot} ${Errors}
      SetErrorLevel 0
      Quit
    ${EndIf}
  ${EndIf}
FunctionEnd

; Uninstall: put back what the launcher changed on the PC before its files go
; (launcher_core::uninstall; docs/qa/PLAN-D18-UNINSTALL.md). Only a real
; uninstall runs it: a launcher update runs the old uninstaller with /UPDATE,
; and a one-click reinstall over the same version runs it with /P (passive),
; and neither should turn Steam updates back on for a player who keeps playing.
; The cleanup never fails the uninstall; its log is %TEMP%\aetherial-dawn-uninstall.log.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-cleanup'
  ${EndIf}
!macroend
