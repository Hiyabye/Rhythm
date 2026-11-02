; Per-user protocol activation. Tauri assigns the shortcut AUMID from identifier.
!macro RHYTHM_CHECK_PROTOCOL_OWNER
  ReadRegStr $0 HKCU "Software\Classes\rhythm-win\shell\open\command" ""
  ${If} $0 != ""
  ${AndIf} $0 != '$\"$INSTDIR\rhythm.exe$\" $\"%1$\"'
    ${If} $RhythmPreviousInstallDir == ""
    ${OrIf} $0 != '$\"$RhythmPreviousInstallDir\rhythm.exe$\" $\"%1$\"'
      MessageBox MB_ICONSTOP "The rhythm-win link is owned by another installation. Resolve that installation before continuing."
      SetErrorLevel 1
      Abort
    ${EndIf}
  ${EndIf}
!macroend
; This per-user identity is used for Windows toast sender metadata. Never
; adopt a populated identity that lacks this installer's ownership marker.
!macro RHYTHM_CHECK_AUMID_OWNER
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "RhythmInstallPath"
  ${If} $0 != ""
    ${If} $0 != "$INSTDIR"
    ${AndIf} $0 != "$RhythmPreviousInstallDir"
      MessageBox MB_ICONSTOP "The Rhythm notification identity belongs to another installation." /SD IDOK
      SetErrorLevel 1
      Abort
    ${EndIf}
  ${Else}
    ClearErrors
    EnumRegValue $1 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" 0
    ${IfNot} ${Errors}
      MessageBox MB_ICONSTOP "The Rhythm notification identity is already registered by another program." /SD IDOK
      SetErrorLevel 1
      Abort
    ${EndIf}
    ClearErrors
    EnumRegKey $1 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" 0
    ${IfNot} ${Errors}
      MessageBox MB_ICONSTOP "The Rhythm notification identity is already registered by another program." /SD IDOK
      SetErrorLevel 1
      Abort
    ${EndIf}
  ${EndIf}
!macroend
!macro NSIS_HOOK_POSTINSTALL
  !insertmacro RHYTHM_CHECK_PROTOCOL_OWNER
  !insertmacro RHYTHM_CHECK_AUMID_OWNER
  ClearErrors
  WriteRegStr HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "RhythmInstallPath" "$INSTDIR"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the Rhythm notification identity. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "DisplayName" "Rhythm"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the Rhythm notification name. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "RhythmInstallPath"
  ${If} ${Errors}
  ${OrIf} $0 != "$INSTDIR"
    MessageBox MB_ICONSTOP "Could not verify the Rhythm notification identity. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "DisplayName"
  ${If} ${Errors}
  ${OrIf} $0 != "Rhythm"
    MessageBox MB_ICONSTOP "Could not verify the Rhythm notification name. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\rhythm-win" "" "URL:Rhythm Windows notification"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the Rhythm notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\rhythm-win" "URL Protocol" ""
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the Rhythm notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\rhythm-win\shell\open\command" "" '$\"$INSTDIR\rhythm.exe$\" $\"%1$\"'
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the Rhythm notification handler. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\rhythm-win" ""
  ${If} ${Errors}
  ${OrIf} $0 != "URL:Rhythm Windows notification"
    MessageBox MB_ICONSTOP "Could not verify the Rhythm notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\rhythm-win" "URL Protocol"
  ${If} ${Errors}
  ${OrIf} $0 != ""
    MessageBox MB_ICONSTOP "Could not verify the Rhythm notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\rhythm-win\shell\open\command" ""
  ${If} ${Errors}
  ${OrIf} $0 != '$\"$INSTDIR\rhythm.exe$\" $\"%1$\"'
    MessageBox MB_ICONSTOP "Could not verify the Rhythm notification handler. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ; Preserve opt-in and StartupApproved bytes when an upgrade moves the app.
  ${If} $RhythmPreviousInstallDir != ""
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Rhythm.app.rhythm.windows"
    ${If} $0 == '$\"$RhythmPreviousInstallDir\rhythm.exe$\" --autostart'
      ClearErrors
      WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Rhythm.app.rhythm.windows" '$\"$INSTDIR\rhythm.exe$\" --autostart'
      ${If} ${Errors}
        MessageBox MB_ICONSTOP "Could not update Rhythm startup registration. Installation cannot be completed." /SD IDOK
        SetErrorLevel 1
        Abort
      ${EndIf}
      ClearErrors
      ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Rhythm.app.rhythm.windows"
      ${If} ${Errors}
      ${OrIf} $0 != '$\"$INSTDIR\rhythm.exe$\" --autostart'
        MessageBox MB_ICONSTOP "Could not verify Rhythm startup registration. Installation cannot be completed." /SD IDOK
        SetErrorLevel 1
        Abort
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend
!macro NSIS_HOOK_POSTUNINSTALL
  ; A manual upgrade keeps this preference; an ordinary uninstall removes only
  ; the Run entry owned by this installation.
  ${If} $UpdateMode <> 1
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Rhythm.app.rhythm.windows"
    ${If} $0 == '$\"$INSTDIR\rhythm.exe$\" --autostart'
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Rhythm.app.rhythm.windows"
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "Rhythm.app.rhythm.windows"
    ${EndIf}
    ReadRegStr $0 HKCU "Software\Classes\rhythm-win\shell\open\command" ""
    StrCmp $0 '$\"$INSTDIR\rhythm.exe$\" $\"%1$\"' 0 +2
      DeleteRegKey HKCU "Software\Classes\rhythm-win"
    ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "RhythmInstallPath"
    ${If} $0 == "$INSTDIR"
      ReadRegStr $1 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "DisplayName"
      ${If} $1 == "Rhythm"
        DeleteRegValue HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "DisplayName"
      ${EndIf}
      DeleteRegValue HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" "RhythmInstallPath"
      ; /ifempty checks subkeys only, so check for other values ourselves too.
      ClearErrors
      EnumRegValue $2 HKCU "Software\Classes\AppUserModelId\app.rhythm.windows" 0
      ${If} ${Errors}
        DeleteRegKey /ifempty HKCU "Software\Classes\AppUserModelId\app.rhythm.windows"
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ; Do not remove LocalAppData: archives and models belong to the user.
!macroend
