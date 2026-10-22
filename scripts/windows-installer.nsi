; Installer for Tapetum. Needs NSIS 3.
;
;   makensis /DVERSION=0.1.0 /DARCH=x64 scripts\windows-installer.nsi
;
; Expects target\release\tapetum.exe and writes target\bundle\Tapetum-windows-<arch>-setup.exe.
; Installs for the current user, so it needs no administrator rights.
; (Paths below are relative to this file's folder.)

Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef ARCH
  !define ARCH "x64"
!endif
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Tapetum"

Name "Tapetum"
OutFile "..\target\bundle\Tapetum-windows-${ARCH}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Tapetum"
InstallDirRegKey HKCU "Software\Tapetum" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "..\crates\tapetum\assets\icon.ico"
!define MUI_UNICON "..\crates\tapetum\assets\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\Tapetum.exe"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "German"

Section "Tapetum"
  SetOutPath "$INSTDIR"
  File "/oname=Tapetum.exe" "..\target\release\tapetum.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Tapetum.lnk" "$INSTDIR\Tapetum.exe"

  WriteRegStr HKCU "Software\Tapetum" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Tapetum"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Julius B. Müller"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\Tapetum.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

; Settings (%APPDATA%\Tapetum) and show files stay where they are.
Section "Uninstall"
  Delete "$INSTDIR\Tapetum.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Tapetum.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\Tapetum"
SectionEnd
