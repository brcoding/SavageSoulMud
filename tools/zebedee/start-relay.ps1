param([string]$Python = '')
$ErrorActionPreference = 'Stop'
if (-not $Python) { $Python = (Get-Command python -ErrorAction Stop).Source }
$grimRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$stateDir = Join-Path $env:LOCALAPPDATA 'SavageSoul/Zebedee'
New-Item -ItemType Directory -Force -Path $stateDir | Out-Null
$keyFile = Join-Path $stateDir 'relay.key'
if (-not (Test-Path -LiteralPath $keyFile)) {
    $random = New-Object byte[] 32
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($random)
    [System.IO.File]::WriteAllText($keyFile, [Convert]::ToHexString($random).ToLowerInvariant())
}
$gateway = (& wsl -d Ubuntu-24.04 -- sh -c 'ip route show default' | Select-Object -First 1) -replace '^default via ([0-9.]+).*$', '$1'
if ($gateway -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Cannot determine the Windows WSL gateway address.' }
$relayScript = Join-Path $PSScriptRoot 'windows-relay.py'
$pidFile = Join-Path $stateDir 'relay.json'
if (Test-Path -LiteralPath $pidFile) {
    $previous = Get-Content -Raw -LiteralPath $pidFile | ConvertFrom-Json
    $process = Get-CimInstance Win32_Process -Filter "ProcessId=$($previous.pid)" -ErrorAction SilentlyContinue
    if ($process -and $process.CommandLine.Contains($relayScript) -and $process.CommandLine.Contains($keyFile)) {
        if ($previous.gateway -eq $gateway) { Write-Host 'Zebedee relay is already running.'; return }
        Stop-Process -Id $previous.pid
    }
}
$linuxKeySource = '/mnt/' + $keyFile.Substring(0,1).ToLowerInvariant() + $keyFile.Substring(2).Replace('\','/')
$linuxProvision = '/mnt/' + $relayScript.Substring(0,1).ToLowerInvariant() + $relayScript.Substring(2).Replace('\','/').Replace('windows-relay.py','provision-key.sh')
$wslKey = (& wsl -d Ubuntu-24.04 -- bash $linuxProvision $linuxKeySource | Select-Object -Last 1).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot provision the private WSL relay key.' }
$configFile = Join-Path $grimRoot 'data/zebedee.json'
$config = Get-Content -Raw -LiteralPath $configFile | ConvertFrom-Json
$config | Add-Member -NotePropertyName relay_host -NotePropertyValue $gateway -Force
$config | Add-Member -NotePropertyName relay_port -NotePropertyValue 4084 -Force
$config | Add-Member -NotePropertyName relay_key_file -NotePropertyValue $wslKey -Force
$config | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $configFile -Encoding utf8NoBOM
$arguments = '"{0}" --bind {1} --port 4084 --udp-port 4002 --key-file "{2}" --config "{3}"' -f $relayScript,$gateway,$keyFile,$configFile
$relayProcess = Start-Process -FilePath $Python -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $stateDir 'relay.log') -RedirectStandardError (Join-Path $stateDir 'relay-error.log')
@{pid=$relayProcess.Id;gateway=$gateway} | ConvertTo-Json | Set-Content -LiteralPath $pidFile
Write-Host "Zebedee relay started on WSL interface $gateway (PID $($relayProcess.Id))."
