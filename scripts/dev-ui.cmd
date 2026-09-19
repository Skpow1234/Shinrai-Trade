@echo off
REM One-command paper trader for cmd.exe
setlocal
cd /d "%~dp0.."
if not defined SHINRAI_OG_DEV set SHINRAI_OG_DEV=1
if not defined SHINRAI_OG_BIND set SHINRAI_OG_BIND=127.0.0.1:8081
echo Starting paper order-gateway (dev defaults)...
echo   UI:    http://%SHINRAI_OG_BIND%/ui
echo   Token: dev
echo.
start "" "http://%SHINRAI_OG_BIND%/ui"
cargo run -p shinrai-order-gateway -- --dev
