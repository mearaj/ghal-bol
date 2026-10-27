# Ghal Bol - per-user Windows uninstall. Started by Uninstall.bat or "Apps & features".
# Removes the install, its shortcuts and registry entries, and - only if you type YES -
# this install's identity, contacts and chats.
# Keep this file ASCII: Windows PowerShell 5.1 reads BOM-less scripts as ANSI.
$ErrorActionPreference = 'Stop'

function Read-Manifest([string]$Path) {
    $manifest = @{}
    foreach ($line in Get-Content -LiteralPath $Path) {
        $t = $line.Trim()
        if ($t -eq '' -or $t.StartsWith('#') -or -not $t.Contains('=')) { continue }
        $key, $value = $t.Split('=', 2)
        $manifest[$key.Trim()] = $value.Trim()
    }
    return $manifest
}

$installDir = $PSScriptRoot
$manifest = Read-Manifest (Join-Path $installDir 'install-manifest.env')
$label = $manifest['GHALBOL_APP_LABEL']
$appId = $manifest['GHALBOL_APP_ID']
$exe = Join-Path $installDir 'ghalbol-app.exe'

Get-Process -Name 'ghalbol-app' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $exe } |
    Stop-Process -Force

# App data: release lives directly in %LOCALAPPDATA%\com.ghalbol\data, and the debug
# namespace (com.ghalbol.debug) is a subfolder there - never delete one with the other.
$dataRoot = Join-Path $env:LOCALAPPDATA 'com.ghalbol\data'
if ($appId -eq 'com.ghalbol') {
    $dataItems = @(Get-ChildItem -LiteralPath $dataRoot -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -notlike 'com.ghalbol.*' })
    $dataShown = $dataRoot
} else {
    $dataShown = Join-Path $dataRoot $appId
    $dataItems = @(Get-Item -LiteralPath $dataShown -Force -ErrorAction SilentlyContinue)
}
if ($dataItems.Count -gt 0) {
    Write-Host "Local data for $label is in:"
    Write-Host "  $dataShown"
    Write-Host 'It holds your identity, contacts and chats. Deleting it cannot be undone.'
    $answer = Read-Host 'Type YES to delete it too, or press Enter to keep it'
    if ($answer -ceq 'YES') {
        $dataItems | Remove-Item -Recurse -Force
        Write-Host 'Data deleted.'
    } else {
        Write-Host 'Data kept.'
    }
}

$menuDir = Join-Path ([Environment]::GetFolderPath('Programs')) $label
Remove-Item -LiteralPath $menuDir -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId" -Recurse -Force -ErrorAction SilentlyContinue
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $appId -ErrorAction SilentlyContinue

# Drop the ghalbol:// handler only when it still points at this install.
$proto = 'HKCU:\Software\Classes\ghalbol'
$command = (Get-ItemProperty -Path "$proto\shell\open\command" -ErrorAction SilentlyContinue).'(default)'
if ($command -and $command.Contains($exe)) {
    Remove-Item -Path $proto -Recurse -Force
}

Set-Location $env:TEMP
Remove-Item -LiteralPath $installDir -Recurse -Force
Write-Host "$label uninstalled."
