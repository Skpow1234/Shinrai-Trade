# One-command paper trader for Windows PowerShell.
# Usage: .\scripts\dev-ui.ps1
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

if (-not $env:SHINRAI_OG_DEV) { $env:SHINRAI_OG_DEV = "1" }
if (-not $env:SHINRAI_OG_BIND) { $env:SHINRAI_OG_BIND = "127.0.0.1:8081" }

$Ui = "http://$($env:SHINRAI_OG_BIND)/ui"
Write-Host "Starting paper order-gateway (dev defaults)..."
Write-Host "  UI:    $Ui"
Write-Host "  Token: dev"
Write-Host ""

Start-Job -ScriptBlock {
    param($Url)
    Start-Sleep -Seconds 3
    Start-Process $Url
} -ArgumentList $Ui | Out-Null

cargo run -p shinrai-order-gateway -- --dev
