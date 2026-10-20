# Create deploy/secrets/{postgres_password,ops_token} if missing.
param(
    [switch]$Random
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$Dir = Join-Path $Root "deploy\secrets"
New-Item -ItemType Directory -Force -Path $Dir | Out-Null

function New-UrlSafeToken {
    $bytes = New-Object byte[] 24
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    $b64 = [Convert]::ToBase64String($bytes)
    ($b64 -replace '[/+=]', '').Substring(0, [Math]::Min(32, ($b64 -replace '[/+=]', '').Length))
}

function Write-Secret([string]$Name, [string]$Default) {
    $path = Join-Path $Dir $Name
    if (Test-Path $path) {
        Write-Host "keep  $path"
        return
    }
    $value = if ($Random) { New-UrlSafeToken } else { $Default }
    # UTF-8 no BOM, single trailing newline
    [System.IO.File]::WriteAllText($path, ($value.Trim() + "`n"), [System.Text.UTF8Encoding]::new($false))
    Write-Host "wrote $path"
}

Write-Secret "postgres_password" "shinrai"
Write-Secret "ops_token" "compose-ops"

Write-Host "Done. Start stack with:"
Write-Host "  docker compose --profile full up -d --build"
Write-Host "TLS:"
Write-Host "  docker compose --profile full --profile tls -f compose.yaml -f compose.tls.yaml up -d --build"
