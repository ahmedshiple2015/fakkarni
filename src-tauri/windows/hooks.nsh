!macro NSIS_HOOK_POSTINSTALL
  StrCpy $1 1
  ExecWait '"$INSTDIR\fakkarni.exe" --installer-register' $1
  ${If} $1 != 0
    DetailPrint "Browser registration failed. Quit Brave and Edge, then use Repair browser bridge in Fakkarni."
    IfSilent +2
    MessageBox MB_ICONEXCLAMATION "Could not register the browser bridge. Quit Brave and Edge, then use Repair browser bridge in Fakkarni."
    SetErrorLevel 1
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $1 1
  ExecWait '"$INSTDIR\fakkarni.exe" --installer-unregister' $1
  ${If} $1 != 0
    DetailPrint "Some browser registration files could not be removed."
    IfSilent +2
    MessageBox MB_ICONEXCLAMATION "Some browser bridge files could not be removed. Quit Brave and Edge before uninstalling again."
    SetErrorLevel 1
  ${EndIf}
!macroend
