@echo off
setlocal
rem ============================================================
rem agentdash sidebar launcher (W1-009). ASCII-only on purpose:
rem cmd.exe parses .bat files in the OEM codepage, so non-ASCII
rem comments turn into garbage commands on non-UTF-8 consoles.
rem Usage: dash-side.bat <project-dir>
rem   left pane  = cmd, working directory = %1
rem   right pane = agentdash watch (live TUI; refresh interval is
rem                the built-in 5s constant MODEL_INTERVAL)
rem Note: agentdash W1 CLI is `watch [--once] [PATH]` -- there is
rem no interval positional, so the pane passes the directory, not "5".
rem ============================================================
set "AD=D:\agentdash\target\debug\agentdash.exe"
if "%~1"=="" (set "DIR=%CD%") else set "DIR=%~f1"
if not exist "%AD%" (
    echo [dash-side] agentdash.exe not found: %AD%
    exit /b 1
)
if not exist "%DIR%\" (
    echo [dash-side] not a directory: %DIR%
    exit /b 1
)
start "agentdash sidebar" wt.exe --title "dash:cmd" -d "%DIR%" cmd /k "echo [dash-side] left pane: %DIR%" ; split-pane -V --title "dash:watch" -d "%DIR%" "%AD%" watch "%DIR%"
endlocal
