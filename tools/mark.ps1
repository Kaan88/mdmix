<#
.SYNOPSIS
  Append a timestamped marker line to a capture's marker file, to split a long capture into steps later.
.EXAMPLE
  tools\mark.ps1 payloads\raw\g6-sbc-session-c.pcap "scout mode on"
#>
param([Parameter(Mandatory)] [string] $Capture, [Parameter(Mandatory)] [string] $Label)
"$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0)`t$Label" | Add-Content -Path "$Capture.markers"
