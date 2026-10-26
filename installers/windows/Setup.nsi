Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma
Name "BitNet"
OutFile "${OUTPUT}"
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "Sections.nsh"
Var Mode
Var Options
Var HelperExit
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$LOCALAPPDATA\Programs\BitNet\bitnet-gui.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch BitNet"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"
Section "Desktop + CLI (current user)" ClientSection
  SectionIn RO
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=Client.msi "${CLIENT_MSI}"
  ExecWait '"$SYSDIR\msiexec.exe" /i "$PLUGINSDIR\Client.msi" /qn /norestart' $HelperExit
  ${If} $HelperExit != 0
  ${AndIf} $HelperExit != 3010
    SetErrorLevel $HelperExit
    Abort "Client installation failed."
  ${EndIf}
  ${If} $HelperExit == 3010
    SetRebootFlag true
    SetErrorLevel 3010
  ${EndIf}
SectionEnd
Section /o "Unattended background service (administrator approval)" ServiceSection
  StrCpy $Options "--interactive"
  IfSilent 0 +2
    StrCpy $Options ""
  ExecWait '"$LOCALAPPDATA\Programs\BitNet\bitmaint.exe" --scope system --action service-install $Options' $HelperExit
  ${If} $HelperExit != 0
    IfSilent +2
      MessageBox MB_ICONINFORMATION "The desktop client is installed. Background service was not enabled; you can enable it later from Settings."
    SetErrorLevel $HelperExit
  ${EndIf}
SectionEnd
Function .onInit
  ${GetParameters} $Options
  ${GetOptions} $Options "/MODE=" $Mode
  ${If} $Mode == "service"
    !insertmacro SelectSection ${ServiceSection}
  ${ElseIf} $Mode != ""
  ${AndIf} $Mode != "user"
    SetErrorLevel 2
    Abort "Use /MODE=user or /MODE=service."
  ${EndIf}
FunctionEnd
