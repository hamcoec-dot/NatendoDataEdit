; Natendo Data Edit setup hooks (included by the Tauri NSIS installer).
; After install: ask which file types to open with the app. The app does the
; registration itself (per user, never taking a type another app owns);
; Windows asks the user before the app becomes a default, so setup offers
; to open that Settings page. Silent installs register table files only.
; Before uninstall: the app removes everything it registered.

!macro NSIS_HOOK_POSTINSTALL
  Push $R9
  StrCpy $R9 "0"
  IfSilent natendo_silent
  MessageBox MB_YESNO|MB_ICONQUESTION "Open table files (.csv, .tsv, .tab, .psv, .ssv) with Natendo Data Edit?$\r$\n$\r$\nThe app is added to $\"Open with$\" for these types and never takes over a type another app already opens. You can change this later in File > Options." /SD IDYES IDNO natendo_text
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --register-file-types csv,tsv,tab,psv,ssv'
    StrCpy $R9 "1"
  natendo_text:
  MessageBox MB_YESNO|MB_ICONQUESTION "Also open plain text files (.txt, .dat, .log, .prn) with Natendo Data Edit?$\r$\n$\r$\nMost people open these in a text editor; choose No to leave them as they are." /SD IDNO IDNO natendo_ask
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --register-file-types txt,dat,log,prn'
    StrCpy $R9 "1"
  natendo_ask:
  StrCmp $R9 "1" 0 natendo_done
  MessageBox MB_YESNO|MB_ICONQUESTION "Windows asks you to confirm a new default app yourself.$\r$\n$\r$\nOpen Windows Settings now to make Natendo Data Edit the default for these files?" /SD IDNO IDNO natendo_done
    ExecShell "open" "ms-settings:defaultapps?registeredAppUser=Natendo%20Data%20Edit"
  Goto natendo_done
  natendo_silent:
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --register-file-types csv,tsv,tab,psv,ssv'
  natendo_done:
  Pop $R9
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --unregister-file-types'
!macroend

; After uninstall (not an update): remove recovery copies and earlier saves,
; which hold copies of the user's data in the app's private folder.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$LOCALAPPDATA\${BUNDLEID}\recovery"
    RmDir /r "$LOCALAPPDATA\${BUNDLEID}\versions"
  ${EndIf}
!macroend
