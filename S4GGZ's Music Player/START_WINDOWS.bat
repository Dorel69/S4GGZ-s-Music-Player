@echo off
setlocal
set "APP_DIR=%~dp0target\release"
set "APP_EXE=%APP_DIR%\s4ggz.exe"

if not exist "%APP_EXE%" (
  echo [S4GGZ] Executabilul Windows lipseste:
  echo "%APP_EXE%"
  echo Restaureaza pachetul complet al aplicatiei si incearca din nou.
  pause
  exit /b 1
)

start "S4GGZ" /D "%APP_DIR%" "%APP_EXE%"
if errorlevel 1 (
  echo [S4GGZ] Programul nu a putut fi pornit.
  pause
  exit /b 1
)
exit /b 0
