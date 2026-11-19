Unicode true
RequestExecutionLevel user
SilentInstall silent
CRCCheck force
SetCompressor /SOLID lzma
Name "BitNet Portable"
OutFile "${OUTPUT}"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

Var Base
Var Payload
Var Stage
Var Options
Var Args
Var Path

!macro RequireRegularPath path
  Push "${path}"
  Call RequireRegularPath
!macroend

; Reject reparse points in launcher-controlled paths.
Function RequireRegularPath
  Exch $Path
  System::Call 'kernel32::GetFileAttributesW(w "$Path") i .r0'
  IntOp $1 $0 & 0x400
  ${If} $0 == -1
  ${OrIf} $1 <> 0
    MessageBox MB_ICONSTOP "BitNet portable files are missing or redirected." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  Pop $Path
FunctionEnd

Section
  StrCpy $Base "$LOCALAPPDATA\BitNet.Portable"
  StrCpy $Payload "$Base\${PAYLOAD_ID}"
  CreateDirectory "$Base"
  !insertmacro RequireRegularPath "$Base"
  IfFileExists "$Payload\.ready" launch
  ClearErrors
  GetTempFileName $Stage "$Base"
  IfErrors failed
  Delete "$Stage"
  ClearErrors
  CreateDirectory "$Stage"
  IfErrors failed
  !insertmacro RequireRegularPath "$Stage"
  SetOutPath "$Stage"
  File "${STAGE}\bitnet-gui.exe"
  File "${STAGE}\bitnet-tui.exe"
  File "${STAGE}\bitnet-web.exe"
  File "${STAGE}\bitengine.exe"
  File "${STAGE}\bitctl.exe"
  File "${STAGE}\bitmaint.exe"
  SetOutPath "$Stage\licenses"
  File "${STAGE}\licenses\*"
  IfErrors releaseStage
  SetOutPath "$Base"
  FileOpen $3 "$Stage\.ready" w
  IfErrors releaseStage
  FileClose $3
  Rename "$Stage" "$Payload"
  IfErrors releaseStage launch

releaseStage:
  Delete "$Stage\bitnet-gui.exe"
  Delete "$Stage\bitnet-tui.exe"
  Delete "$Stage\bitnet-web.exe"
  Delete "$Stage\bitengine.exe"
  Delete "$Stage\bitctl.exe"
  Delete "$Stage\bitmaint.exe"
  Delete "$Stage\.ready"
  Delete "$Stage\licenses\*"
  RMDir "$Stage\licenses"
  RMDir "$Stage"
  IfFileExists "$Payload\.ready" launch failed

launch:
  !insertmacro RequireRegularPath "$Payload"
  !insertmacro RequireRegularPath "$Payload\bitnet-gui.exe"
  !insertmacro RequireRegularPath "$Payload\bitnet-tui.exe"
  !insertmacro RequireRegularPath "$Payload\bitnet-web.exe"
  !insertmacro RequireRegularPath "$Payload\bitengine.exe"
  !insertmacro RequireRegularPath "$Payload\bitctl.exe"
  !insertmacro RequireRegularPath "$Payload\bitmaint.exe"
  ${GetParameters} $Args
  ${GetOptions} $Args "--autostart" $0
  ${IfNot} ${Errors}
    StrCpy $Options "--autostart"
  ${EndIf}
  ${GetOptions} $Args "--silent" $0
  ${IfNot} ${Errors}
    StrCpy $Options "--silent"
  ${EndIf}
  ${GetOptions} $Args "--hidden" $0
  ${IfNot} ${Errors}
    StrCpy $Options "--hidden"
  ${EndIf}
  ${GetOptions} $Args "--show" $0
  ${IfNot} ${Errors}
    StrCpy $Options "--show"
  ${EndIf}
  SetOutPath "$Payload"
  ClearErrors
  Exec '"$Payload\bitnet-gui.exe" $Options'
  IfErrors failed done

failed:
  MessageBox MB_ICONSTOP "Could not launch BitNet Portable." /SD IDOK
  SetErrorLevel 1
done:
SectionEnd
