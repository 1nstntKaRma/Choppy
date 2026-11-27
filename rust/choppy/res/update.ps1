<#
 Choppy's own update (2.02+). Choppy starts this script hidden when "Update to vX" is clicked, and it needs nothing
 more: the same install ChocoPlus's Choppy button runs (IDA_Choppy.ps1 -> Invoke-GolaNTool), for this one user.
   1. downloads Choppy-vX.zip and SHA256SUMS.txt of release vX from GitHub
   2. checks the zip and its bin\Choppy.exe against the release's SHA-256 sums, and the exe's version against vX
   3. runs the release's own build.ps1 -NoBuild -Shell Update: it quits the running Choppy, replaces the exe,
      registers the right-click entries again and starts the tray Choppy again when Default file manager is on -
      every setting stays as it was
   4. tells the new Choppy it is updated (--updated vX settings|tray), or the installed one what went wrong
      (--update-failed "why")
 Every step goes to -Log (Choppy's Logs\update.log); build.ps1 keeps its own %TEMP%\Choppy-setup.log.
#>
param(
    [Parameter(Mandatory)][ValidatePattern('^v\d+(\.\d+){0,3}$')][string]$Tag,
    [ValidateSet('tray', 'settings')][string]$Then = 'tray',
    [Parameter(Mandatory)][string]$Log,
    [switch]$DryRun     # download and check only, nothing installed (to try the script)
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Repo = '1nstntKaRma/Choppy'
$Installed = Join-Path $env:LOCALAPPDATA 'Programs\Choppy\Choppy.exe'
$PowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$work = Join-Path $env:TEMP ('Choppy-update-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))

function Note([string]$Text) {
    try { Add-Content -LiteralPath $Log -Value ('{0:yyyy-MM-dd HH:mm:ss}  {1}' -f (Get-Date), $Text) -Encoding UTF8 } catch { }
}

# a file of the release, tried 3 times (a dropped connection, a busy server)
function Get-ReleaseFile([string]$Name) {
    $to = Join-Path $work $Name
    for ($i = 1; ; $i++) {
        try {
            Invoke-WebRequest -Uri "https://github.com/$Repo/releases/download/$Tag/$Name" -OutFile $to -UseBasicParsing -UserAgent 'Choppy-update' -TimeoutSec 60
            return $to
        }
        catch {
            if ($i -ge 3) { throw "Couldn't download $Name from GitHub ($($_.Exception.Message))" }
            Start-Sleep -Seconds (3 * $i)
        }
    }
}

function Get-Sha256([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }

# the release's build.ps1 says what it did; its last "  > ..." line says why it stopped
function Get-SetupLines {
    $f = Join-Path $env:TEMP 'Choppy-setup.log'
    if (-not (Test-Path -LiteralPath $f)) { return @() }
    @(Get-Content -LiteralPath $f -ErrorAction SilentlyContinue | Where-Object { $_ -match '^\s*(>|\[)|^  \S' })
}

# the log keeps the last few updates
try {
    if ((Test-Path -LiteralPath $Log) -and (Get-Item -LiteralPath $Log).Length -gt 256KB) {
        $keep = @(Get-Content -LiteralPath $Log -Tail 300)
        Set-Content -LiteralPath $Log -Value $keep -Encoding UTF8
    }
} catch { }

$code = 0
try {
    $from = if (Test-Path -LiteralPath $Installed) { (Get-Item -LiteralPath $Installed).VersionInfo.FileVersion } else { 'not installed' }
    Note "Update to $Tag started (installed: $from)$(if ($DryRun) { ' - dry run, nothing is installed' })"
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    New-Item -ItemType Directory -Force $work | Out-Null

    # 1-2: download and check
    $sums = @(Get-Content -LiteralPath (Get-ReleaseFile 'SHA256SUMS.txt'))
    function Get-Want([string]$Name) {
        foreach ($line in $sums) {
            $h, $n = "$line".Trim() -split '\s+', 2
            if ($n -and $n.TrimStart('*') -eq $Name) { return $h.ToLowerInvariant() }
        }
        throw "The release's SHA256SUMS.txt has no line for $Name"
    }
    $zipName = "Choppy-$Tag.zip"
    $zip = Get-ReleaseFile $zipName
    if ((Get-Sha256 $zip) -ne (Get-Want $zipName)) { throw "The downloaded $zipName doesn't match the release's SHA-256 - not installed" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $dir = Join-Path $work 'release'
    [IO.Compression.ZipFile]::ExtractToDirectory($zip, $dir)
    $build = Join-Path $dir 'build.ps1'
    $exe = Join-Path $dir 'bin\Choppy.exe'
    if (-not (Test-Path -LiteralPath $build) -or -not (Test-Path -LiteralPath $exe)) { throw "$zipName has no build.ps1 or bin\Choppy.exe" }
    if ((Get-Sha256 $exe) -ne (Get-Want 'Choppy.exe')) { throw "The release's Choppy.exe doesn't match its SHA-256 - not installed" }
    # build.ps1's stamp: 2.02 -> 2.2.0.0
    $v = [version]$Tag.Substring(1)
    $stamp = "$($v.Major).$($v.Minor).$([Math]::Max(0, $v.Build)).0"
    $got = (Get-Item -LiteralPath $exe).VersionInfo.FileVersion
    if ($got -ne $stamp) { throw "The release's Choppy.exe is version $got, not $stamp" }
    Note "Downloaded $zipName, SHA-256 and version checked"

    if ($DryRun) { Note "Dry run: ready to install $Tag" }
    else {
        # 3: the release's own install, for this user (keeps every setting; an older build.ps1 knows only Install)
        $mode = if ((Get-Content -LiteralPath $build -Raw) -match "'Update'") { 'Update' } else { 'Install' }
        Note "Installing: build.ps1 -NoBuild -Shell $mode"
        # waits for build.ps1 alone (Start-Process -Wait would also wait for the tray Choppy it starts)
        $p = Start-Process -FilePath $PowerShell -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', "`"$build`"", '-NoBuild', '-Shell', $mode) -NoNewWindow -PassThru
        $null = $p.Handle   # Windows PowerShell keeps the exit code only of a process whose handle was taken
        $p.WaitForExit()
        $lines = Get-SetupLines
        foreach ($l in $lines) { Note "  build.ps1: $($l.Trim())" }
        if ($p.ExitCode -ne 0) {
            $why = @($lines | Where-Object { $_ -match '^\s*>\s' } | Select-Object -Last 1)
            throw "The install stopped$(if ($why) { ': ' + ($why[0].Trim() -replace '^>\s*', '') } else { " (code $($p.ExitCode))" })"
        }
        $now = (Get-Item -LiteralPath $Installed).VersionInfo.FileVersion
        if ($now -ne $stamp) { throw "After the install Choppy.exe is version $now, not $stamp" }
        Note "Updated to $Tag"
        # 4: the new Choppy says so (the tray one, when it runs, gets this through its pipe)
        Start-Process -FilePath $Installed -ArgumentList '--updated', $Tag, $Then
    }
}
catch {
    $code = 1
    $why = $_.Exception.Message
    Note "Failed: $why"
    if (-not $DryRun -and (Test-Path -LiteralPath $Installed)) {
        # one argument: no quotes inside, no backslash before the closing quote
        $arg = ($why -replace '"', "'").Trim()
        if ($arg.Length -gt 400) { $arg = $arg.Substring(0, 400) + '...' }
        try { Start-Process -FilePath $Installed -ArgumentList '--update-failed', ('"' + $arg.TrimEnd('\') + '"') } catch { }
    }
}
finally {
    for ($i = 0; $i -lt 20 -and (Test-Path -LiteralPath $work); $i++) {
        try { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction Stop } catch { Start-Sleep -Milliseconds 250 }
    }
}
exit $code
