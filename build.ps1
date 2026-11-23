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

# the installed Choppy has quit (it asks first when a copy is running: up to a minute for that answer)
function Wait-Gone {
    for ($i = 0; $i -lt 300; $i++) {
        if (-not @(Get-Process Choppy -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Installed })) { return }
        Start-Sleep -Milliseconds 200
    }
    throw "Choppy is still running (a copy in progress?) - let it finish or quit it from the tray, then try again."
}

# a Choppy that just quit, or a virus scanner looking at the file, can hold it a moment longer
function Copy-Patiently([string]$From, [string]$To) {
    for ($i = 0; ; $i++) {
        try { Copy-Item $From $To -Force -ErrorAction Stop; return }
        catch { if ($i -ge 40) { throw "Couldn't replace ${To}: $($_.Exception.Message)" }; Start-Sleep -Milliseconds 250 }
    }
}

function Install-ForUser([string]$Mode) {
    if (-not (Test-Path $exe)) { throw "bin\Choppy.exe is missing." }
    $isNew = -not (Test-Path 'HKCU:\Software\Choppy')
    $wasDefault = (Get-ItemProperty 'HKCU:\Software\Choppy' -Name ExplorerTakeover -ErrorAction SilentlyContinue).ExplorerTakeover -eq '1'
    # an installed exe that can't start any more (damaged, blocked) must not stop the update that replaces it
    if (Test-Path $Installed) { try { Start-Process $Installed -ArgumentList '--quit' -Wait } catch { }; Wait-Gone }
    New-Item -ItemType Directory -Force $InstallDir | Out-Null
    Copy-Patiently $exe $Installed
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
    if (Test-Path $Installed) { try { Start-Process $Installed -ArgumentList '--uninstall' -Wait } catch { }; Wait-Gone }
    Remove-Item $Shortcut -Force -ErrorAction SilentlyContinue
    if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force }
    Remove-Item 'HKCU:\Software\Choppy' -Recurse -Force -ErrorAction SilentlyContinue
    Say "  [Removed] Choppy, its right-click entry and settings (logs stay in %LOCALAPPDATA%\Choppy)" Green
}

function Test-Admin {
    ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# ---------- elevated: the install runs in the desktop user's own session ----------
# Choppy is per user (its Ctrl+V hook, tray and clipboard live in the user's session), so an elevated run hands the
# install to that user: a one-time scheduled task in their session, with their normal (not elevated) rights.

# The user whose desktop this run belongs to: the shell of this run's own session (its oldest explorer.exe - later
# ones are folder windows, or elevated), else the console's user (a run from a service), else whoever has a desktop.
# Found by SID: a Microsoft or work account's name can fail to resolve, its SID can't.
function Get-DesktopUser {
    $mine = (Get-Process -Id $PID).SessionId
    $console = try { (Get-CimInstance Win32_ComputerSystem).UserName } catch { $null }
    $found = @(foreach ($p in @(Get-CimInstance Win32_Process -Filter "Name='explorer.exe'" | Sort-Object CreationDate)) {
        $sid = try { (Invoke-CimMethod -InputObject $p -MethodName GetOwnerSid).Sid } catch { $null }
        if ($sid -notmatch '^S-1-(5-21|12-1)-') { continue }   # a person's account: local, domain, Microsoft (5-21) or Entra ID (12-1)
        $name = try { ([Security.Principal.SecurityIdentifier]$sid).Translate([Security.Principal.NTAccount]).Value } catch { $sid }
        [pscustomobject]@{ Sid = $sid; Name = $name; Session = $p.SessionId }
    })
    @($found | Where-Object { $mine -ne 0 -and $_.Session -eq $mine }) + @($found | Where-Object { $console -and $_.Name -eq $console }) + $found |
        Select-Object -First 1
}

# What Task Scheduler's last result means while the step hasn't said anything itself
$TaskNotRunYet = 0x41303, 0x41301, 0x41325   # has not run yet, is running, is queued

function Invoke-AsDesktopUser([string]$Mode) {
    $u = Get-DesktopUser
    if (-not $u) { throw "Nobody is signed in to the desktop." }
    $who = $u.Name
    # staged in the user's own temp folder: only they and administrators can reach it
    $profileDir = (Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$($u.Sid)" -ErrorAction SilentlyContinue).ProfileImagePath
    if (-not $profileDir -or -not (Test-Path $profileDir)) { throw "$who has no profile folder on this PC." }
    $task = 'Choppy-setup-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $stage = Join-Path $profileDir "AppData\Local\Temp\$task"
    New-Item -ItemType Directory -Force (Join-Path $stage 'bin') | Out-Null
    try {
        Copy-Item (Join-Path $root 'build.ps1') $stage
        if (Test-Path (Join-Path $root 'VERSION')) { Copy-Item (Join-Path $root 'VERSION') $stage }
        if (Test-Path $exe) { Copy-Item $exe (Join-Path $stage 'bin') }
        $log = Join-Path $stage 'setup.log'
        $began = Join-Path $stage 'started.txt'
        $result = Join-Path $stage 'result.txt'
        $arg = "/c powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$stage\build.ps1`" -NoBuild -Shell $Mode -AsUserStep > `"$log`" 2>&1"
        # Task Scheduler's defaults would start it only on mains power, stop it if the laptop is unplugged, and run it -
        # and the Choppy it starts in the tray - below normal priority: none of that here
        $esc = [Security.SecurityElement]::Escape($arg)
        $xml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Principals><Principal id="Author"><UserId>USER-ID</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT15M</ExecutionTimeLimit><Priority>4</Priority><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
  </Settings>
  <Actions Context="Author"><Exec><Command>cmd.exe</Command><Arguments>$esc</Arguments></Exec></Actions>
</Task>
"@
        # by SID; by name if this Windows wants one
        try { Register-ScheduledTask -TaskName $task -Xml $xml.Replace('USER-ID', $u.Sid) -Force -ErrorAction Stop | Out-Null }
        catch {
            $bySid = $_.Exception.Message
            try { Register-ScheduledTask -TaskName $task -Xml $xml.Replace('USER-ID', [Security.SecurityElement]::Escape($who)) -Force -ErrorAction Stop | Out-Null }
            catch { throw "Windows wouldn't set up the install for ${who}: $bySid / $($_.Exception.Message)" }
        }
        try { Start-ScheduledTask -TaskName $task -ErrorAction Stop }
        catch { throw "Windows wouldn't start the install in ${who}'s session: $($_.Exception.Message)" }

        # the step says when it starts and how it ended (started.txt, result.txt): no guessing from the task's times
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $code = $null; $why = $null
        while ($null -eq $code) {
            Start-Sleep -Milliseconds 250
            if (Test-Path $result) {
                $lines = @(Get-Content $result -ErrorAction SilentlyContinue)
                if ($lines.Count -ge 2 -and $lines[-1] -eq 'end') { $code = [int]$lines[0]; break }
            }
            $state = (Get-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue).State
            $last = [uint32](Get-ScheduledTaskInfo -TaskName $task -ErrorAction SilentlyContinue).LastTaskResult
            $ran = Test-Path $began
            if ($state -eq 'Ready' -and ($ran -or $TaskNotRunYet -notcontains $last) -and $clock.Elapsed.TotalSeconds -gt 3) {
                # it ended without saying how (its result file is written last): a moment for the file, then the task's own code
                Start-Sleep -Seconds 2
                if (Test-Path $result) { continue }
                $code = [BitConverter]::ToInt32([BitConverter]::GetBytes($last), 0)
                $why = if ($ran) { "The step in ${who}'s session stopped before it finished" } else { "The step in ${who}'s session couldn't start" }
                $why += " (code 0x{0:X})." -f $last
            }
            elseif (-not $ran -and $clock.Elapsed.TotalMinutes -ge 2) {
                $code = 1; $why = "The install didn't start in ${who}'s session in 2 minutes (task $state, 0x{0:X})." -f $last
            }
            elseif ($clock.Elapsed.TotalMinutes -ge 14) {
                $code = 1; $why = "The install in ${who}'s session didn't finish in 14 minutes."
            }
        }
        # its log is complete once cmd.exe has closed it
        for ($i = 0; $i -lt 40 -and (Get-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue).State -eq 'Running'; $i++) { Start-Sleep -Milliseconds 250 }
        if (Test-Path $log) { Get-Content $log | Where-Object { $_.Trim() } | ForEach-Object { Say $_ $(if ($_ -match '^\s*>') { 'Red' } elseif ($_ -match '\[') { 'Green' } else { 'Gray' }) } }
        if ($why) { Say "  > $why" Red }
        elseif ($code -ne 0) { Say "  > The step in ${who}'s session ended with code $code" Red }
        return $code
    }
    finally {
        Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
        # .NET deletes a link inside the folder, never what it points to
        for ($i = 0; $i -lt 20 -and (Test-Path $stage); $i++) { try { [IO.Directory]::Delete($stage, $true) } catch { Start-Sleep -Milliseconds 250 } }
    }
}

# ================= main =================

# every run leaves its output in %TEMP%\Choppy-setup.log (the user's step writes into the first run's log)
if (-not $AsUserStep) { try { Start-Transcript -Path (Join-Path $env:TEMP 'Choppy-setup.log') -Force | Out-Null } catch { } }
$exitCode = 0
try {
    $mode = Resolve-Mode $Shell
    if ($AsUserStep) {
        Set-Content (Join-Path $root 'started.txt') $PID   # the elevated run waits for this, then for result.txt
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
if ($AsUserStep) { try { Set-Content (Join-Path $root 'result.txt') "$exitCode`r`nend" } catch { } }
if (-not $AsUserStep) { try { Stop-Transcript | Out-Null } catch { } }
exit $exitCode
