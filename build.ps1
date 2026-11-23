# Builds Choppy -> bin\Choppy.exe, then installs it for the signed-in user.
#
# Choppy targets .NET Framework 4.8, which is part of every Windows 10 / 11 installation: one small exe (under 1 MB),
# nothing extra to install. Building from source needs the .NET SDK (WPF); without it build.ps1 downloads the
# ready-made exe of the latest release and checks it against the release's SHA-256.
#
#   .\build.ps1                       build, then ask what to do
#   .\build.ps1 -Shell Apply          install / update + "Default file manager" on (Ctrl+V of files in Explorer uses Choppy)
#   .\build.ps1 -Shell RightClick     install / update, Default file manager off (only right-click "Paste with Choppy")
#   .\build.ps1 -Shell Install        install / update, keep the user's choices (a first install turns Default file
#                                     manager on), then open Choppy's settings
#   .\build.ps1 -Shell Revert         remove Choppy (Ultracopier's startup entry comes back if Choppy had parked it)
#   .\build.ps1 -Shell Cancel         build only
#   .\build.ps1 -NoBuild -Shell ...   only install / remove (uses bin\Choppy.exe)
#
# Everything is per user - %LOCALAPPDATA%\Programs\Choppy, a Start menu shortcut, HKCU - no administrator rights.
# Started elevated (for example from a tool that runs as administrator), the install still runs as the signed-in
# user, through a one-time scheduled task: Choppy's Ctrl+V hook, clipboard and Explorer hand-off live in the
# user's own session.
param(
    [ValidateSet('Ask', 'Apply', 'RightClick', 'Install', 'Revert', 'Cancel', 'Abort', '1', '2', '3', '4')][string]$Shell = 'Ask',
    [switch]$NoBuild,
    [switch]$AsUserStep     # internal: this run is the signed-in user's part of an elevated run
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Repo = '1nstntKaRma/Choppy'
$root = $PSScriptRoot
$proj = Join-Path $root 'src\Choppy.csproj'
$bin = Join-Path $root 'bin'
$exe = Join-Path $bin 'Choppy.exe'
$version = if (Test-Path (Join-Path $root 'VERSION')) { (Get-Content (Join-Path $root 'VERSION') -TotalCount 1).Trim() } else { '0.0' }
$interactive = [Environment]::UserInteractive -and -not ([Environment]::GetCommandLineArgs() -contains '-NonInteractive')

$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\Choppy'
$Installed = Join-Path $InstallDir 'Choppy.exe'
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Choppy.lnk'

function Say([string]$Text, [string]$Color = 'Gray') {
    # the signed-in user's step runs hidden with its output in a file: plain text, so the first window can show it
    if ($AsUserStep) { [Console]::Out.WriteLine($Text) } else { Write-Host $Text -ForegroundColor $Color }
}

# Files from a downloaded zip carry Mark-of-the-Web: unblock them so SmartScreen / ExecutionPolicy never block the exe
try { Get-ChildItem -LiteralPath $root -Recurse -File -ErrorAction SilentlyContinue | Unblock-File -ErrorAction SilentlyContinue } catch { }
# test-mode switches of Choppy must never leak into a real install
Get-ChildItem Env: | Where-Object Name -like 'CHOPPY*' | ForEach-Object { Remove-Item "Env:$($_.Name)" }

# ================= build =================

function Get-Stamp { $p = [version]$version; "$($p.Major).$($p.Minor).$([Math]::Max(0, $p.Build))" }

function Get-Prebuilt {
    Say "> No .NET SDK here - downloading the ready-made Choppy.exe from the latest release" Magenta
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    New-Item -ItemType Directory -Force $bin | Out-Null
    $base = "https://github.com/$Repo/releases/latest/download"
    Invoke-WebRequest "$base/Choppy.exe" -OutFile $exe -UseBasicParsing
    $raw = (Invoke-WebRequest "$base/SHA256SUMS.txt" -UseBasicParsing).Content
    $sums = if ($raw -is [byte[]]) { [Text.Encoding]::UTF8.GetString($raw) } else { [string]$raw }
    $want = $sums -split "`r?`n" | Where-Object { $_ -match '\sChoppy\.exe\s*$' } | ForEach-Object { ($_ -split '\s+')[0].ToLowerInvariant() } | Select-Object -First 1
    $got = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not $want -or $want -ne $got) { Remove-Item $exe -Force; throw "The downloaded Choppy.exe doesn't match the release's SHA-256 - not used." }
    Say "  [Downloaded] bin\Choppy.exe (SHA-256 checked)" Green
}

function Invoke-Build {
    if (-not (Test-Path $proj)) {
        # a release zip: the exe is already in bin\
        if (Test-Path $exe) { return }
        Get-Prebuilt; return
    }
    $sdk = $null
    if (Get-Command dotnet -ErrorAction SilentlyContinue) { $sdk = & dotnet --list-sdks 2>$null }
    if (-not $sdk) {
        if (Test-Path $exe) { Say "  No .NET SDK here - using the existing bin\Choppy.exe" DarkYellow; return }
        Get-Prebuilt; return
    }
    $stamp = Get-Stamp
    Say "> Building Choppy $version" Magenta
    & dotnet build $proj -c Release -nologo -v q "-p:Version=$stamp" | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "The build failed." }
    New-Item -ItemType Directory -Force $bin | Out-Null
    Copy-Item (Join-Path $root 'src\bin\Release\net48\Choppy.exe') $exe -Force
    $t = Join-Path $env:TEMP 'choppy-selftest.txt'
    $p = Start-Process $exe -ArgumentList '--selftest', "`"$t`"" -Wait -PassThru
    if ($p.ExitCode -ne 0) { Get-Content $t | Out-Host; throw "The self-test failed." }
    Say "  [Built] bin\Choppy.exe ($([int]((Get-Item $exe).Length / 1KB)) KB), self-test passed" Green
}

# ================= choice =================

function Get-Choice {
    if (-not $interactive) { return 'Cancel' }
    Write-Host ""
    Write-Host "  [1] Install + Default file manager   (Ctrl+V of files in Explorer uses Choppy)" -ForegroundColor White
    Write-Host "  [2] Install - right-click only       (Windows' own copy stays the default)" -ForegroundColor White
    Write-Host "  [3] Remove Choppy" -ForegroundColor White
    Write-Host "  [4] Cancel" -ForegroundColor White
    $c = Read-Host "Choice"
    switch ($c) { '' { 'Apply' } '1' { 'Apply' } '2' { 'RightClick' } '3' { 'Revert' } default { 'Cancel' } }
}

function Resolve-Mode([string]$m) {
    switch ($m) { '1' { 'Apply' } '2' { 'RightClick' } '3' { 'Revert' } '4' { 'Cancel' } 'Abort' { 'Cancel' } default { $m } }
}

# ================= install / remove (always as the signed-in user) =================

function Wait-Gone {
    for ($i = 0; $i -lt 50; $i++) {
        if (-not @(Get-Process Choppy -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Installed })) { return }
        Start-Sleep -Milliseconds 200
    }
}

function Install-ForUser([string]$Mode) {
    if (-not (Test-Path $exe)) { throw "bin\Choppy.exe is missing." }
    $isNew = -not (Test-Path 'HKCU:\Software\Choppy')
    $wasDefault = (Get-ItemProperty 'HKCU:\Software\Choppy' -Name ExplorerTakeover -ErrorAction SilentlyContinue).ExplorerTakeover -eq '1'
    if (Test-Path $Installed) { Start-Process $Installed -ArgumentList '--quit' -Wait; Wait-Gone }
    New-Item -ItemType Directory -Force $InstallDir | Out-Null
    Copy-Item $exe $Installed -Force
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut($Shortcut)
    $s.TargetPath = $Installed
    $s.WorkingDirectory = $InstallDir
    $s.Description = 'Choppy - fast, clear file copy'
    $s.Save()
    Start-Process $Installed -ArgumentList '--menu' -Wait
    Say "  [Installed] Choppy $version - right-click a folder: Paste with Choppy" Green
    $default = switch ($Mode) { 'Apply' { $true } 'RightClick' { $false } default { $isNew -or $wasDefault } }
    if ($default) {
        Start-Process $Installed -ArgumentList '--takeover', 'on'
        Say "  [On] Default file manager - Ctrl+V of files in Explorer uses Choppy" Green
    }
    elseif ($wasDefault) {
        Start-Process $Installed -ArgumentList '--takeover', 'off' -Wait
        Say "  [Off] Default file manager - Ctrl+V is Windows' own copy again" Green
    }
    if ($Mode -eq 'Install') {
        Start-Sleep -Milliseconds 800
        Start-Process $Installed   # no arguments = Choppy's settings
    }
}

function Remove-ForUser {
    if (Test-Path $Installed) { Start-Process $Installed -ArgumentList '--uninstall' -Wait; Wait-Gone }
    Remove-Item $Shortcut -Force -ErrorAction SilentlyContinue
    if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force }
    Remove-Item 'HKCU:\Software\Choppy' -Recurse -Force -ErrorAction SilentlyContinue
    Say "  [Removed] Choppy, its right-click entry and settings (logs stay in %LOCALAPPDATA%\Choppy)" Green
}

function Test-Admin {
    ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# Elevated: hand the install to the user who owns the desktop (a one-time scheduled task in their session)
function Invoke-AsDesktopUser([string]$Mode) {
    $user = $null
    try {
        $e = Get-CimInstance Win32_Process -Filter "Name='explorer.exe'" | Select-Object -First 1
        $o = Invoke-CimMethod -InputObject $e -MethodName GetOwner
        if ($o.User) { $user = if ($o.Domain) { "$($o.Domain)\$($o.User)" } else { $o.User } }
    } catch { }
    if (-not $user) { throw "Nobody is signed in to the desktop." }
    $stage = Join-Path $env:ProgramData 'Choppy-setup'
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force (Join-Path $stage 'bin') | Out-Null
    & icacls.exe $stage /grant '*S-1-5-32-545:(OI)(CI)M' /T /Q | Out-Null   # the user's step writes its log here
    Copy-Item (Join-Path $root 'build.ps1') $stage
    if (Test-Path (Join-Path $root 'VERSION')) { Copy-Item (Join-Path $root 'VERSION') $stage }
    if (Test-Path $exe) { Copy-Item $exe (Join-Path $stage 'bin') }
    $log = Join-Path $stage 'setup.log'
    $task = 'Choppy-setup-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $arg = "/c powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$stage\build.ps1`" -NoBuild -Shell $Mode -AsUserStep > `"$log`" 2>&1"
    $action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument $arg
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Force | Out-Null
    try {
        $started = Get-Date
        Start-ScheduledTask -TaskName $task
        $ran = $false
        do {
            Start-Sleep -Milliseconds 400
            $state = (Get-ScheduledTask -TaskName $task).State
            $info = Get-ScheduledTaskInfo -TaskName $task
            if ($state -eq 'Running' -or $info.LastRunTime -ge $started.AddSeconds(-2)) { $ran = $true }
        } while (($state -eq 'Running' -or -not $ran) -and (Get-Date) -lt $started.AddMinutes(3))
        if (Test-Path $log) { Get-Content $log | Where-Object { $_.Trim() } | ForEach-Object { Say $_ $(if ($_ -match '^\s*>') { 'Red' } elseif ($_ -match '\[') { 'Green' } else { 'Gray' }) } }
        if (-not $ran) { throw "The install didn't start in $user's session (is someone signed in?)." }
        if ($state -eq 'Running') { throw "The install in $user's session didn't finish in 3 minutes." }
        $code = [BitConverter]::ToInt32([BitConverter]::GetBytes([uint32]$info.LastTaskResult), 0)
        if ($code -ne 0) { Say "  > The step in $user's session ended with code $code" Red }
        return $code
    }
    finally {
        Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
        Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ================= main =================

# every run leaves its output in %TEMP%\Choppy-setup.log (the user's step writes into the first run's log)
if (-not $AsUserStep) { try { Start-Transcript -Path (Join-Path $env:TEMP 'Choppy-setup.log') -Force | Out-Null } catch { } }
$exitCode = 0
try {
    $mode = Resolve-Mode $Shell
    if ($AsUserStep) {
        if ($mode -eq 'Revert') { Remove-ForUser } else { Install-ForUser $mode }
    }
    else {
        if ($mode -ne 'Revert' -and (-not $NoBuild -or -not (Test-Path $exe))) { Invoke-Build }
        if ($mode -eq 'Ask') { $mode = Get-Choice }
        if ($mode -eq 'Cancel') { Say "  Nothing installed or changed." DarkGray }
        else {
            Say $(if ($mode -eq 'Revert') { "> Removing Choppy" } else { "> Installing Choppy $version for the signed-in user" }) Magenta
            if (Test-Admin) { $exitCode = [int](Invoke-AsDesktopUser $mode | Select-Object -Last 1) }
            elseif ($mode -eq 'Revert') { Remove-ForUser }
            else { Install-ForUser $mode }
        }
    }
}
catch {
    Say "  > $($_.Exception.Message)" Red
    $exitCode = 1
}
if (-not $AsUserStep) { try { Stop-Transcript | Out-Null } catch { } }
exit $exitCode
