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
!define MUI_FINISHPAGE_RUN_TEXT "$(LaunchBitNet)"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"
LangString LaunchBitNet ${LANG_ENGLISH} "Launch BitNet"
LangString LaunchBitNet ${LANG_SIMPCHINESE} "启动 BitNet"
LangString ClientName ${LANG_ENGLISH} "Desktop and command line"
LangString ClientName ${LANG_SIMPCHINESE} "桌面与命令行"
LangString ServiceName ${LANG_ENGLISH} "Background service"
LangString ServiceName ${LANG_SIMPCHINESE} "系统后台服务"
LangString ClientDescription ${LANG_ENGLISH} "Install the desktop app and command line for the current user."
LangString ClientDescription ${LANG_SIMPCHINESE} "为当前用户安装桌面应用与命令行工具。"
LangString ServiceDescription ${LANG_ENGLISH} "Keep campus authentication running before sign-in. Requires administrator approval."
LangString ServiceDescription ${LANG_SIMPCHINESE} "系统登录前即可保持校园网认证，需要管理员确认。"
LangString ClientInstallFailed ${LANG_ENGLISH} "Client installation failed."
LangString ClientInstallFailed ${LANG_SIMPCHINESE} "桌面客户端安装失败。"
LangString ServiceInstallFailed ${LANG_ENGLISH} "The desktop app is installed. The background service could not be enabled. You can enable it later in Background service."
LangString ServiceInstallFailed ${LANG_SIMPCHINESE} "桌面应用已安装。系统后台服务未启用，可稍后在「后台服务」中启用。"
Section "$(ClientName)" ClientSection
  SectionIn RO
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=Client.msi "${CLIENT_MSI}"
  ExecWait '"$SYSDIR\msiexec.exe" /i "$PLUGINSDIR\Client.msi" /qn /norestart' $HelperExit
  ${If} $HelperExit != 0
  ${AndIf} $HelperExit != 3010
    SetErrorLevel $HelperExit
    Abort "$(ClientInstallFailed)"
  ${EndIf}
  ${If} $HelperExit == 3010
    SetRebootFlag true
    SetErrorLevel 3010
  ${EndIf}
SectionEnd
Section /o "$(ServiceName)" ServiceSection
  StrCpy $Options "--interactive"
  IfSilent 0 +2
    StrCpy $Options ""
  ExecWait '"$LOCALAPPDATA\Programs\BitNet\bitmaint.exe" --scope system --action service-install $Options' $HelperExit
  ${If} $HelperExit == 3010
    SetRebootFlag true
    SetErrorLevel 3010
  ${ElseIf} $HelperExit != 0
    IfSilent +2
      MessageBox MB_ICONINFORMATION "$(ServiceInstallFailed)"
    SetErrorLevel $HelperExit
  ${EndIf}
SectionEnd
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${ClientSection} "$(ClientDescription)"
  !insertmacro MUI_DESCRIPTION_TEXT ${ServiceSection} "$(ServiceDescription)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END
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
