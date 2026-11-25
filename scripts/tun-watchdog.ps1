# Restores Windows TUN routing/DNS after a test, even if the test process dies
# or the agent loses network access. Must run elevated.
#
# The script does NOT set up TUN; it only snapshots the system enough to undo
# the split default routes and TUN DNS. It restores when -ProcessId exits, or
# after -TimeoutSeconds unconditionally, so a short test should always finish
# before the deadline.

param(
    [int]$TimeoutSeconds = 60,
    [int]$ProcessId = 0,
    [string]$LogFile = "$env:TEMP\rdp-tun-watchdog.log"
)

$ErrorActionPreference = "Stop"
$tunName = "rabbit-digger-pro"
$prefixes = @("0.0.0.0/1", "128.0.0.0/1")

function Write-WatchdogLog([string]$message) {
    $line = "{0} {1}" -f (Get-Date -Format o), $message
    Add-Content -Path $LogFile -Value $line -Encoding utf8
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-WatchdogLog "not elevated; refusing to start"
    exit 2
}

function Get-TunAdapter {
    # The split route is the thing that must be cleaned, so resolve the adapter
    # from the route itself. This also works when the TUN IP is customized.
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
        # Remove every route owned by the TUN interface (split routes, on-link
        # routes and any test bypass route that happened to land there).
        Get-NetRoute -InterfaceIndex $adapter.ifIndex -ErrorAction SilentlyContinue |
            Remove-NetRoute -Confirm:$false -ErrorAction SilentlyContinue
        Set-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -ResetServerAddresses -ErrorAction SilentlyContinue
        Set-NetIPInterface -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -InterfaceMetric 250 -ErrorAction SilentlyContinue
        Write-WatchdogLog "restored routes/DNS on $tunName (ifIndex $($adapter.ifIndex))"
    } else {
        # The adapter may have disappeared; at least remove our prefixes on any
        # Wintun-looking interface.
        foreach ($prefix in $prefixes) {
            Get-NetRoute -DestinationPrefix $prefix -ErrorAction SilentlyContinue | ForEach-Object {
                if ($_.InterfaceAlias -like "*rabbit*" -or $_.InterfaceAlias -like "*wintun*") {
                    Remove-NetRoute -DestinationPrefix $prefix -InterfaceIndex $_.ifIndex -Confirm:$false -ErrorAction SilentlyContinue
                }
            }
        }
        Write-WatchdogLog "TUN adapter not present; removed any matching split routes"
    }
}

Write-WatchdogLog "watchdog started (timeout ${TimeoutSeconds}s, pid $PID, watching ${ProcessId})"
try {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        if ($ProcessId -gt 0 -and -not (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)) {
            Write-WatchdogLog "watched process $ProcessId exited; restoring"
            break
        }
        Start-Sleep -Milliseconds 500
    }
    if ((Get-Date) -ge $deadline) {
        Write-WatchdogLog "timeout reached; restoring"
    }
} finally {
    Restore-Tun
}
Write-WatchdogLog "watchdog exiting"
