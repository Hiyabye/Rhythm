; Per-user protocol activation and notification identity, derived from Tauri's
; ${BUNDLEID} and ${PRODUCTNAME} so non-production installers cannot collide
; with production Rhythm (#70). Keep in sync with core::windows_identity.
!macro RHYTHM_RESOLVE_IDENTITY
  !if "${BUNDLEID}" != "io.github.hiyabye.rhythm"
    !if "${PRODUCTNAME}" == "Rhythm"
      !error "Non-production Windows installers must set a distinct productName (not 'Rhythm') so they cannot overwrite a production installation."
    !endif
  !endif
  !ifndef RHYTHM_PROTOCOL_SCHEME
    !if "${BUNDLEID}" == "io.github.hiyabye.rhythm"
      !define RHYTHM_PROTOCOL_SCHEME "rhythm-win"
      !define RHYTHM_AUTOSTART_NAME "Rhythm.io.github.hiyabye.rhythm"
    !else
      !define RHYTHM_PROTOCOL_SCHEME "rhythm-win.${BUNDLEID}"
      !define RHYTHM_AUTOSTART_NAME "${BUNDLEID}"
    !endif
  !endif
!macroend
!macro RHYTHM_CHECK_PROTOCOL_OWNER
  !insertmacro RHYTHM_RESOLVE_IDENTITY
  ReadRegStr $0 HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}\shell\open\command" ""
  ${If} $0 != ""
  ${AndIf} $0 != '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\"'
    ${If} $RhythmPreviousInstallDir == ""
    ${OrIf} $0 != '$\"$RhythmPreviousInstallDir\${MAINBINARYNAME}.exe$\" $\"%1$\"'
      MessageBox MB_ICONSTOP "The ${RHYTHM_PROTOCOL_SCHEME} link is owned by another installation. Resolve that installation before continuing."
      SetErrorLevel 1
      Abort
    ${EndIf}
  ${EndIf}
!macroend
; This per-user identity is used for Windows toast sender metadata. Never
; adopt a populated identity that lacks this installer's ownership marker.
!macro RHYTHM_CHECK_AUMID_OWNER
  !insertmacro RHYTHM_RESOLVE_IDENTITY
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "RhythmInstallPath"
  ${If} $0 != ""
    ${If} $0 != "$INSTDIR"
    ${AndIf} $0 != "$RhythmPreviousInstallDir"
      MessageBox MB_ICONSTOP "The ${PRODUCTNAME} notification identity belongs to another installation." /SD IDOK
      SetErrorLevel 1
      Abort
    ${EndIf}
  ${Else}
    ClearErrors
    EnumRegValue $1 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" 0
    ${IfNot} ${Errors}
      MessageBox MB_ICONSTOP "The ${PRODUCTNAME} notification identity is already registered by another program." /SD IDOK
      SetErrorLevel 1
      Abort
    ${EndIf}
    ClearErrors
    EnumRegKey $1 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" 0
    ${IfNot} ${Errors}
      MessageBox MB_ICONSTOP "The ${PRODUCTNAME} notification identity is already registered by another program." /SD IDOK
      SetErrorLevel 1
      Abort
    ${EndIf}
  ${EndIf}
!macroend
!macro NSIS_HOOK_POSTINSTALL
  !insertmacro RHYTHM_RESOLVE_IDENTITY
  !insertmacro RHYTHM_CHECK_PROTOCOL_OWNER
  !insertmacro RHYTHM_CHECK_AUMID_OWNER
  ClearErrors
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "RhythmInstallPath" "$INSTDIR"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the ${PRODUCTNAME} notification identity. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName" "${PRODUCTNAME}"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the ${PRODUCTNAME} notification name. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "RhythmInstallPath"
  ${If} ${Errors}
  ${OrIf} $0 != "$INSTDIR"
    MessageBox MB_ICONSTOP "Could not verify the ${PRODUCTNAME} notification identity. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName"
  ${If} ${Errors}
  ${OrIf} $0 != "${PRODUCTNAME}"
    MessageBox MB_ICONSTOP "Could not verify the ${PRODUCTNAME} notification name. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}" "" "URL:${PRODUCTNAME} Windows notification"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the ${PRODUCTNAME} notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}" "URL Protocol" ""
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the ${PRODUCTNAME} notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}\shell\open\command" "" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\"'
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "Could not register the ${PRODUCTNAME} notification handler. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}" ""
  ${If} ${Errors}
  ${OrIf} $0 != "URL:${PRODUCTNAME} Windows notification"
    MessageBox MB_ICONSTOP "Could not verify the ${PRODUCTNAME} notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}" "URL Protocol"
  ${If} ${Errors}
  ${OrIf} $0 != ""
    MessageBox MB_ICONSTOP "Could not verify the ${PRODUCTNAME} notification protocol. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}\shell\open\command" ""
  ${If} ${Errors}
  ${OrIf} $0 != '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\"'
    MessageBox MB_ICONSTOP "Could not verify the ${PRODUCTNAME} notification handler. Installation cannot be completed." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ; Preserve opt-in and StartupApproved bytes when an upgrade moves the app.
  ${If} $RhythmPreviousInstallDir != ""
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${RHYTHM_AUTOSTART_NAME}"
    ${If} $0 == '$\"$RhythmPreviousInstallDir\${MAINBINARYNAME}.exe$\" --autostart'
      ClearErrors
      WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${RHYTHM_AUTOSTART_NAME}" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --autostart'
      ${If} ${Errors}
        MessageBox MB_ICONSTOP "Could not update ${PRODUCTNAME} startup registration. Installation cannot be completed." /SD IDOK
        SetErrorLevel 1
        Abort
      ${EndIf}
      ClearErrors
      ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${RHYTHM_AUTOSTART_NAME}"
      ${If} ${Errors}
      ${OrIf} $0 != '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --autostart'
        MessageBox MB_ICONSTOP "Could not verify ${PRODUCTNAME} startup registration. Installation cannot be completed." /SD IDOK
        SetErrorLevel 1
        Abort
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ; Releases before 2.0 also shipped a Vulkan build. A passive update skips the
  ; old uninstaller, so remove that build's files here.
  RMDir /r "$INSTDIR\llama\vulkan"
!macroend
!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro RHYTHM_RESOLVE_IDENTITY
  ; A manual upgrade keeps this preference; an ordinary uninstall removes only
  ; the Run entry owned by this installation.
  ${If} $UpdateMode <> 1
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${RHYTHM_AUTOSTART_NAME}"
    ${If} $0 == '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --autostart'
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${RHYTHM_AUTOSTART_NAME}"
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${RHYTHM_AUTOSTART_NAME}"
    ${EndIf}
    ReadRegStr $0 HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}\shell\open\command" ""
    StrCmp $0 '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\"' 0 +2
      DeleteRegKey HKCU "Software\Classes\${RHYTHM_PROTOCOL_SCHEME}"
    ReadRegStr $0 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "RhythmInstallPath"
    ${If} $0 == "$INSTDIR"
      ReadRegStr $1 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName"
      ${If} $1 == "${PRODUCTNAME}"
        DeleteRegValue HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName"
      ${EndIf}
      DeleteRegValue HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "RhythmInstallPath"
      ; /ifempty checks subkeys only, so check for other values ourselves too.
      ClearErrors
      EnumRegValue $2 HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" 0
      ${If} ${Errors}
        DeleteRegKey /ifempty HKCU "Software\Classes\AppUserModelId\${BUNDLEID}"
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ; Do not remove LocalAppData: archives and models belong to the user.
!macroend
