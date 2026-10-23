$ErrorActionPreference = 'Stop'
$grimLinuxRoot = '/mnt/' + $PSScriptRoot.Substring(0, 1).ToLowerInvariant() + $PSScriptRoot.Substring(2).Replace('\', '/')
wsl -d Ubuntu-24.04 -- python3 "$grimLinuxRoot/tools/grapevine/configure.py"
if ($LASTEXITCODE -ne 0) { throw 'Grapevine configuration was not completed.' }
