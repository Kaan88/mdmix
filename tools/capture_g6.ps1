<#
.SYNOPSIS
  Record USB traffic of the SoundBlaster X G6 with USBPcapCMD (needs admin + USBPcap installed).

.DESCRIPTION
  Finds the USBPcap root hub the G6 is on, records only the G6's own USB address on that hub (never other devices, e.g. keyboards) to -Out (libpcap format, readable by
  g6-probe and Wireshark), and stops when the file named by -StopFile appears, or after -MaxSeconds.
  Writes "<Out>.started" once recording runs, so a non-admin caller can wait for it.

.EXAMPLE
  Start-Process powershell -Verb RunAs -ArgumentList '-ExecutionPolicy Bypass -File tools\capture_g6.ps1 -Out payloads\raw\g6-sbc-startup-a.pcap'
  # ... do the thing to record ..., then:
  New-Item payloads\raw\g6-sbc-startup-a.pcap.stop
#>
param(
    [Parameter(Mandatory)] [string] $Out,
    [string] $StopFile = "$Out.stop",
    [int] $MaxSeconds = 300,
    [string] $Hub,     # e.g. \\.\USBPcap1 (default: auto-detect)
    [string] $Device   # the G6's USB address on that hub, e.g. 12 (default: auto-detect)
)
$ErrorActionPreference = 'Stop'
$log = "$Out.log"
function Write-Log($msg) { "$(Get-Date -Format HH:mm:ss) $msg" | Tee-Object -FilePath $log -Append }

try {
    $cmd = @("$env:ProgramFiles\USBPcap\USBPcapCMD.exe", "${env:ProgramFiles(x86)}\USBPcap\USBPcapCMD.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $cmd) { throw 'USBPcapCMD.exe not found. Install USBPcap (winget install desowin.USBPcap) and reboot.' }

    # find the root hub the G6 is attached to, and its USB address on that hub
    $hub = $Hub; $addr = $Device
    $ifaces = @()
    if (-not ($hub -and $addr)) {
        $ifaces = & $cmd --extcap-interfaces | Select-String -Pattern 'value=(\\\\\.\\USBPcap\d+)' -AllMatches |
            ForEach-Object { $_.Matches.Groups[1].Value } | Select-Object -Unique
        foreach ($i in $ifaces) {
            $cfg = (& $cmd --extcap-interface $i --extcap-config 2>&1) -join "`n"
            $m = [regex]::Match($cfg, '\{value=(\d+)_\d+\}\{display=Sound ?BlasterX G6\}')
            if ($m.Success) { $hub = $i; $addr = $m.Groups[1].Value; break }
        }
    }
    if (-not ($hub -and $addr)) { throw "G6 not found on any USBPcap hub ($($ifaces -join ', ')). Is it plugged in?" }
    Write-Log "recording $hub device $addr (G6 only) -> $Out"

    Remove-Item -ErrorAction SilentlyContinue $Out, $StopFile, "$Out.started"
    $Out = [IO.Path]::GetFullPath($Out)
    # one pre-quoted string: Start-Process does not quote array items, so a path with spaces would be split
    $p = Start-Process -FilePath $cmd -ArgumentList "-d $hub -o `"$Out`" --devices $addr" -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
    if ($p.HasExited) { throw "USBPcapCMD exited right away (code $($p.ExitCode))" }
    New-Item -ItemType File "$Out.started" -Force | Out-Null

    $deadline = (Get-Date).AddSeconds($MaxSeconds)
    while (-not (Test-Path $StopFile) -and (Get-Date) -lt $deadline -and -not $p.HasExited) { Start-Sleep -Milliseconds 300 }
    Start-Sleep -Seconds 2  # let the last packets reach the file
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    Write-Log "stopped; $((Get-Item $Out).Length) bytes"
}
catch {
    Write-Log "ERROR: $_"
    exit 1
}
finally {
    Remove-Item -ErrorAction SilentlyContinue $StopFile
}
