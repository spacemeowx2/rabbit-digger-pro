# Elevated TUN test harness.
#
# Starts the watchdog and a daemon with a TUN config, waits for the split
# routes to appear, then keeps the setup alive for -TimeoutSeconds. On exit it
# removes the routes itself; the watchdog is the second line of defence if this
# script or the agent dies.

param(
    [Parameter(Mandatory = $true)][string]$Config,
    [Parameter(Mandatory = $true)][string]$Engine,
    [int]$TimeoutSeconds = 45,
    [int]$ApiPort = 9091,
    [string]$LogFile = "$env:TEMP\rdp-tun-test.log",
    [string]$ReadyFile = "$env:TEMP\rdp-tun-ready",
    [string]$ResultFile = "$env:TEMP\rdp-tun-result.txt"
)

$ErrorActionPreference = "Stop"
$tunName = "rabbit-digger-pro"
$prefixes = @("0.0.0.0/1", "128.0.0.0/1")
$watchdog = Join-Path $PSScriptRoot "tun-watchdog.ps1"

function Write-TestLog([string]$message) {
    $line = "{0} {1}" -f (Get-Date -Format o), $message
    Add-Content -Path $LogFile -Value $line -Encoding utf8
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-TestLog "not elevated; refusing to start"
    exit 2
}

function Get-TunAdapter {
    # Resolve the adapter from the split route instead of a fixed IP so a
    # customized TUN address still gets cleaned up.
    $route = Get-NetRoute -DestinationPrefix "0.0.0.0/1" -ErrorAction SilentlyContinue |
        Where-Object { $_.NextHop -eq "0.0.0.0" } |
        Select-Object -First 1
    if (-not $route) {
        $route = Get-NetRoute -DestinationPrefix "128.0.0.0/1" -ErrorAction SilentlyContinue |
            Where-Object { $_.NextHop -eq "0.0.0.0" } |
            Select-Object -First 1
    }
    if ($route) {
        $adapter = Get-NetAdapter -InterfaceIndex $route.InterfaceIndex -ErrorAction SilentlyContinue
        if ($adapter -and ($adapter.InterfaceDescription -eq "Wintun Userspace Tunnel" -or $adapter.Name -like "*rabbit*")) {
            return $adapter
        }
    }

    $address = Get-NetIPAddress -AddressFamily IPv4 -IPAddress "10.0.0.1" -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($address) {
        return Get-NetAdapter -InterfaceIndex $address.InterfaceIndex -ErrorAction SilentlyContinue
    }
    return Get-NetAdapter -ErrorAction SilentlyContinue |
        Where-Object { $_.InterfaceDescription -eq "Wintun Userspace Tunnel" } |
        Select-Object -First 1
}

function Restore-Tun {
    $adapter = Get-TunAdapter
    if ($adapter) {
        Get-NetRoute -InterfaceIndex $adapter.ifIndex -ErrorAction SilentlyContinue |
            Remove-NetRoute -Confirm:$false -ErrorAction SilentlyContinue
        Set-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -ResetServerAddresses -ErrorAction SilentlyContinue
        Set-NetIPInterface -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -InterfaceMetric 250 -ErrorAction SilentlyContinue
    }
    Write-TestLog "routes/DNS restored"
}

Remove-Item $ReadyFile -ErrorAction SilentlyContinue
Write-TestLog "starting test (timeout ${TimeoutSeconds}s)"
$started = Get-Date

$watchdogProcess = $null
$engineProcess = $null
try {
    $engineProcess = Start-Process -FilePath $Engine `
        -ArgumentList @("--config", $Config, "--bind", "127.0.0.1:$ApiPort") `
        -RedirectStandardOutput "$LogFile.engine.out" `
        -RedirectStandardError "$LogFile.engine.err" `
        -WindowStyle Hidden -PassThru

    Write-TestLog "engine pid $($engineProcess.Id)"

    # Start the watchdog as soon as the engine PID is known; if the engine dies
    # the routes are restored immediately instead of waiting for the timeout.
    $watchdogProcess = Start-Process -FilePath "powershell.exe" `
        -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $watchdog, "-TimeoutSeconds", $TimeoutSeconds, "-ProcessId", $engineProcess.Id, "-LogFile", "$LogFile.watchdog") `
        -WindowStyle Hidden -PassThru

    $deadline = (Get-Date).AddSeconds(15)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        if ($engineProcess.HasExited) {
            Write-TestLog "engine exited early with code $($engineProcess.ExitCode)"
            break
        }
        $route = Get-NetRoute -DestinationPrefix "0.0.0.0/1" -ErrorAction SilentlyContinue
        if ($route) {
            $ready = $true
            break
        }
        Start-Sleep -Milliseconds 500
    }

    if ($ready) {
        Write-TestLog "TUN split routes are active"
        Set-Content -Path $ReadyFile -Value "ready" -Encoding ascii

        $routeInfo = Get-NetRoute -DestinationPrefix "0.0.0.0/1", "128.0.0.0/1" -ErrorAction SilentlyContinue |
            Format-Table -AutoSize | Out-String
        $tunAdapter = Get-TunAdapter
        $dnsInfo = Get-DnsClientServerAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
            Where-Object { $tunAdapter -and $_.InterfaceIndex -eq $tunAdapter.ifIndex } | Format-Table -AutoSize | Out-String
        $trace = & curl.exe -4 -s --noproxy "*" --max-time 15 -w "`nHTTP=%{http_code} time=%{time_total}`n" "https://www.cloudflare.com/cdn-cgi/trace" 2>&1
        $direct = & curl.exe -4 -s --noproxy "*" --max-time 12 "https://myip.ipip.net" 2>&1
        $result = @(
            "routes:",
            $routeInfo,
            "dns:",
            $dnsInfo,
            "trace:",
            ($trace | Out-String),
            "direct (expect CN):",
            ($direct | Out-String)
        ) -join "`n"
        Set-Content -Path $ResultFile -Value $result -Encoding utf8
        Write-TestLog "probe finished; result written to $ResultFile"
    } else {
        Write-TestLog "TUN routes did not appear"
        Set-Content -Path $ReadyFile -Value "failed" -Encoding ascii
    }

    $remaining = $TimeoutSeconds - 8 - ((Get-Date) - $started).TotalSeconds
    if ($remaining -gt 0) {
        Start-Sleep -Seconds $remaining
    }
} finally {
    if ($engineProcess -and -not $engineProcess.HasExited) {
        Stop-Process -Id $engineProcess.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
    Restore-Tun
    if ($watchdogProcess -and -not $watchdogProcess.HasExited) {
        Stop-Process -Id $watchdogProcess.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-Item $ReadyFile -ErrorAction SilentlyContinue
    Write-TestLog "test finished"
}
