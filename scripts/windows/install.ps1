# Ghal Bol - per-user Windows install (no admin rights). Started by Setup.bat.
# Copies this package to %LOCALAPPDATA%\Programs\<label>\ and adds Start Menu shortcuts,
# an "Apps & features" entry, sign-in autostart, and the ghalbol:// invite-link handler.
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

$src = $PSScriptRoot
$manifest = Read-Manifest (Join-Path $src 'install-manifest.env')
$label = $manifest['GHALBOL_APP_LABEL']
$appId = $manifest['GHALBOL_APP_ID']
$exeName = 'ghalbol-app.exe'
$installDir = Join-Path $env:LOCALAPPDATA "Programs\$label"
$exe = Join-Path $installDir $exeName

# Close a running copy of this install so its files can be replaced.
Get-Process -Name 'ghalbol-app' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $exe } |
    Stop-Process -Force

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
foreach ($item in @($exeName, 'ghalbol_app', 'makepad_widgets', 'install-manifest.env', 'uninstall.ps1', 'Uninstall.bat')) {
    $from = Join-Path $src $item
    $to = Join-Path $installDir $item
    if (Test-Path -LiteralPath $to -PathType Container) { Remove-Item -LiteralPath $to -Recurse -Force }
    Copy-Item -LiteralPath $from -Destination $to -Recurse -Force
}

# Start Menu: the app and its uninstaller.
$menuDir = Join-Path ([Environment]::GetFolderPath('Programs')) $label
New-Item -ItemType Directory -Force -Path $menuDir | Out-Null
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut((Join-Path $menuDir "$label.lnk"))
$link.TargetPath = $exe
$link.WorkingDirectory = $installDir
$link.IconLocation = "$exe,0"
$link.Description = 'Ghal Bol - end-to-end encrypted messenger'
$link.Save()
$unlink = $shell.CreateShortcut((Join-Path $menuDir "Uninstall $label.lnk"))
$unlink.TargetPath = Join-Path $installDir 'Uninstall.bat'
$unlink.Save()

# "Apps & features" entry for this user.
$key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appId"
New-Item -Path $key -Force | Out-Null
Set-ItemProperty -Path $key -Name 'DisplayName' -Value $label
Set-ItemProperty -Path $key -Name 'DisplayIcon' -Value "$exe,0"
Set-ItemProperty -Path $key -Name 'Publisher' -Value 'GhalBol'
Set-ItemProperty -Path $key -Name 'InstallLocation' -Value $installDir
Set-ItemProperty -Path $key -Name 'UninstallString' -Value "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$installDir\uninstall.ps1`""
Set-ItemProperty -Path $key -Name 'NoModify' -Value 1 -Type DWord
Set-ItemProperty -Path $key -Name 'NoRepair' -Value 1 -Type DWord

# Start at sign-in so messages and calls arrive (Linux does the same via ~/.config/autostart).
Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $appId -Value "`"$exe`""

# ghalbol:// invite links open this install (the last install wins).
$proto = 'HKCU:\Software\Classes\ghalbol'
New-Item -Path "$proto\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path $proto -Name '(default)' -Value 'URL:Ghal Bol invite'
Set-ItemProperty -Path $proto -Name 'URL Protocol' -Value ''
Set-ItemProperty -Path "$proto\shell\open\command" -Name '(default)' -Value "`"$exe`" `"%1`""

Write-Host "$label installed to $installDir"
Write-Host 'Start it from the Start Menu.'
