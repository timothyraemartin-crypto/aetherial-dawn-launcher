; One-click install: a plain double-click on the installer shows no pages.
; When the installer was started without /P (passive) or /S (silent), it
; starts itself again as "/P /R" and closes before any window appears. Passive
; mode skips the Welcome, folder and Finish pages, shows only the progress bar,
; closes itself, and makes the desktop shortcut; /R starts the launcher when the
; install is done. The launcher's own updates already run with /P, so they are
; unchanged. Silent installs never reach .onGUIInit, so /S is unchanged too.
!define MUI_CUSTOMFUNCTION_GUIINIT AdOneClickGuiInit

Function AdOneClickGuiInit
  ClearErrors
  ${GetOptions} $CMDLINE "/P" $R0
  ${If} ${Errors}
    Exec '"$EXEPATH" /P /R'
    ${IfNot} ${Errors}
      Quit
    ${EndIf}
  ${EndIf}
FunctionEnd
