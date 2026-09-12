@echo off
setlocal
rem Prefer the Windows PowerShell 5.1 system modules. Some hosts prepend a
rem stripped PowerShell.Utility module that hides Get-FileHash from powershell.exe.
set "PSModulePath=%SystemRoot%\System32\WindowsPowerShell\v1.0\Modules;%ProgramFiles%\WindowsPowerShell\Modules;%USERPROFILE%\Documents\WindowsPowerShell\Modules"
if /I "%CODEX_STARTER_NONINTERACTIVE%"=="1" goto :noninteractive
if not "%~1"=="" goto :noninteractive
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-wizard.ps1"
exit /b %ERRORLEVEL%

:noninteractive
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-all.ps1" %*
exit /b %ERRORLEVEL%
