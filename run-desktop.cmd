@echo off
set "app=%~dp0installed\Local Dictation\local-dictation.exe"
if not exist "%app%" set "app=%LOCALAPPDATA%\Local Dictation\local-dictation.exe"
if not exist "%app%" set "app=%~dp0outputs\desktop\Local Dictation\local-dictation.exe"
if not exist "%app%" (
  echo Windows desktop build is not available yet. See docs/desktop-usage.md.
  pause
  exit /b 1
)
rem Delegate launch to the Windows desktop shell, including when this script
rem is invoked by a development tool whose child processes end with that tool.
"%SystemRoot%\explorer.exe" "%app%"
