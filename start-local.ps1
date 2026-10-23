$ErrorActionPreference = 'Stop'
if ((Test-Path (Join-Path $PSScriptRoot 'data/zebedee.json')) -and ((Get-Content -Raw (Join-Path $PSScriptRoot 'data/zebedee.json') | ConvertFrom-Json).relay_host)) {
    & (Join-Path $PSScriptRoot 'tools/zebedee/start-relay.ps1')
}
$grimLinuxRoot = '/mnt/' + $PSScriptRoot.Substring(0, 1).ToLowerInvariant() + $PSScriptRoot.Substring(2).Replace('\', '/')
wsl -d Ubuntu-24.04 -- systemd-run --user --unit=savsoul-grim --collect --property=Type=notify --property=NotifyAccess=all --property=Restart=on-failure --property=RestartSec=3s --property=WorkingDirectory="$grimLinuxRoot" --setenv=GRIM_BIND_HOST=127.0.0.1 bash "$grimLinuxRoot/start-local.sh"
if ($LASTEXITCODE -ne 0) { throw 'Could not start Grim. Check: wsl -d Ubuntu-24.04 -- systemctl --user status savsoul-grim' }
Write-Host 'Grim is running in WSL. Connect to localhost port 4000.'
