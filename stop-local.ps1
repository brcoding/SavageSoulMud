$ErrorActionPreference = 'Stop'
$grimLinuxRoot = '/mnt/' + $PSScriptRoot.Substring(0, 1).ToLowerInvariant() + $PSScriptRoot.Substring(2).Replace('\', '/')
wsl -d Ubuntu-24.04 -- bash "$grimLinuxRoot/stop-local.sh"
