Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!ifndef BIN_DIR
  !error "Pass /DBIN_DIR with the absolute directory containing both release executables"
!endif
Name "Papamo SMS"
OutFile "${BIN_DIR}\psms-windows-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Papamo SMS"
RequestExecutionLevel user
Icon "..\..\assets\psms.ico"
UninstallIcon "..\..\assets\psms.ico"
!define MUI_ICON "..\..\assets\psms.ico"
!define MUI_UNICON "..\..\assets\psms.ico"
!define ARP "Software\Microsoft\Windows\CurrentVersion\Uninstall\PapamoSMS"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "pSMS requires 64-bit Windows."
    Abort
  ${EndIf}
FunctionEnd

Section "pSMS"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  IfFileExists "$INSTDIR\psms.exe" 0 +2
    nsExec::ExecToLog '"$INSTDIR\psms.exe" stop'
  File "${BIN_DIR}\psms.exe"
  File "${BIN_DIR}\psms-desktop.exe"
  File "..\..\LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\pSMS.lnk" "$INSTDIR\psms-desktop.exe"
  CreateDirectory "$SMPROGRAMS\Papamo SMS"
  CreateShortcut "$SMPROGRAMS\Papamo SMS\pSMS.lnk" "$INSTDIR\psms-desktop.exe"
  WriteRegStr HKCU "${ARP}" "DisplayName" "Papamo SMS"
  WriteRegStr HKCU "${ARP}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${ARP}" "Publisher" "Papamo Software"
  WriteRegStr HKCU "${ARP}" "DisplayIcon" "$INSTDIR\psms-desktop.exe"
  WriteRegStr HKCU "${ARP}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  nsExec::ExecToLog '"$INSTDIR\psms.exe" stop'
  Delete "$DESKTOP\pSMS.lnk"
  Delete "$SMPROGRAMS\Papamo SMS\pSMS.lnk"
  RMDir "$SMPROGRAMS\Papamo SMS"
  Delete "$INSTDIR\psms-desktop.exe"
  Delete "$INSTDIR\psms.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${ARP}"
SectionEnd
