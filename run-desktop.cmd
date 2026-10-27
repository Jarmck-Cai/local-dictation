@echo off
set "app=%~dp0installed\Local Dictation\local-dictation.exe"
if not exist "%app%" set "app=%LOCALAPPDATA%\Local Dictation\local-dictation.exe"
if not exist "%app%" set "app=%~dp0outputs\desktop\Local Dictation\local-dictation.exe"
if not exist "%app%" (
  echo Windows desktop build is not available yet. See docs/desktop-usage.md.
  pause
  exit /b 1
)
start "" "%app%"
