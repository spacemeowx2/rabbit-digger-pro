param(
    [string]$Gui = "C:\Users\space\shared_target\release\rdp-gui.exe",
    [string]$Probe = "C:\Users\space\shared_target\debug\examples\service_probe.exe",
    [string]$DataDir = "$env:TEMP\opencode\rdp-service-data",
    [string]$TunConfig = "$env:TEMP\opencode\rdp-tun-test.yaml",
    [string]$LogFile = "$env:TEMP\rdp-service-test.log"
)

$ErrorActionPreference = "Stop"
if (-not $PSBoundParameters.ContainsKey("DataDir")) {
    $DataDir = Join-Path $env:TEMP ("opencode\rdp-service-data-" + [guid]::NewGuid().ToString("N"))
}
New-Item -ItemType Directory -Path $DataDir -Force | Out-Null
$watchdog = Join-Path $PSScriptRoot "tun-watchdog.ps1"
$watchdogProcess = $null

function Write-TestLog([string]$message) {
    $line = "{0} {1}" -f (Get-Date -Format o), $message
    Add-Content -Path $LogFile -Value $line -Encoding utf8
}

function Invoke-Probe([string[]]$Arguments) {
    $process = Start-Process -FilePath $Probe -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "service probe failed with $($process.ExitCode): $($Arguments -join ' ')"
    }
}

function Wait-NoSplitRoutes {
    for ($i = 0; $i -lt 60; $i++) {
        $routes = Get-NetRoute -DestinationPrefix "0.0.0.0/1", "128.0.0.0/1" -ErrorAction SilentlyContinue
        if (-not $routes) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Stop-Watchdog {
    if ($watchdogProcess -and -not $watchdogProcess.HasExited) {
        Stop-Process -Id $watchdogProcess.Id -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-Gui([string[]]$Arguments) {
    $process = Start-Process -FilePath $Gui -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
    return $process.ExitCode
}

Remove-Item $LogFile -ErrorAction SilentlyContinue
Write-TestLog "data dir: $DataDir"
Write-TestLog "installing service from $Gui"
try {
    $code = Invoke-Gui @("--install-service", "--service-data-dir", $DataDir)
    if ($code -ne 0) {
        throw "install-service exited with $code; see $DataDir\service-install.log"
    }
    Write-TestLog "service installed"

    $service = Get-CimInstance Win32_Service -Filter "Name='RabbitDiggerPro'"
    if (-not $service) { throw "service not found after install" }
    Write-TestLog "service pid $($service.ProcessId)"

    # Watchdog first: if the service dies while TUN is up, routes are restored.
    $watchdogProcess = Start-Process -FilePath "powershell.exe" `
        -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $watchdog, "-TimeoutSeconds", "120", "-ProcessId", $service.ProcessId, "-LogFile", "$LogFile.watchdog") `
        -WindowStyle Hidden -PassThru

    $env:RDP_SERVICE_TOKEN_FILE = Join-Path $DataDir "service.token"
    Invoke-Probe @($TunConfig, "--apply-only")
    Write-TestLog "config applied"

    $ready = $false
    for ($i = 0; $i -lt 60; $i++) {
        $routes = Get-NetRoute -DestinationPrefix "0.0.0.0/1", "128.0.0.0/1" -ErrorAction SilentlyContinue
        if ($routes) { $ready = $true; break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw "TUN split routes did not appear" }

    $routeInfo = Get-NetRoute -DestinationPrefix "0.0.0.0/1", "128.0.0.0/1" -ErrorAction SilentlyContinue |
        Format-Table -AutoSize | Out-String
    $dns = & ipconfig.exe /all | Select-String -Pattern "10.0.0.1"
    $trace = & curl.exe -4 -s --noproxy "*" --max-time 20 -w "`nHTTP=%{http_code} time=%{time_total}`n" "https://www.cloudflare.com/cdn-cgi/trace" 2>&1
    $result = @("routes:", $routeInfo, "dns:", ($dns | Out-String), "trace:", ($trace | Out-String)) -join "`n"
    Set-Content -Path "$LogFile.result" -Value $result -Encoding utf8
    Write-TestLog "probe finished"

    Invoke-Probe @("--stop")
    Write-TestLog "engine.stop"
    if (-not (Wait-NoSplitRoutes)) { throw "TUN routes still present after engine.stop" }
    Write-TestLog "routes removed after engine.stop"

    Stop-Watchdog
    $code = Invoke-Gui @("--uninstall-service", "--service-data-dir", $DataDir)
    Write-TestLog "service uninstalled (exit $code)"
} catch {
    Write-TestLog "ERROR: $($_.Exception.GetBaseException().Message)"
    throw
} finally {
    Stop-Watchdog
    # Best effort: make sure the service is gone and no split routes remain.
    try {
        $service = Get-CimInstance Win32_Service -Filter "Name='RabbitDiggerPro'" -ErrorAction SilentlyContinue
        if ($service) {
            Invoke-Gui @("--uninstall-service", "--service-data-dir", $DataDir) | Out-Null
        }
    } catch {}
    $routes = Get-NetRoute -DestinationPrefix "0.0.0.0/1", "128.0.0.0/1" -ErrorAction SilentlyContinue
    if ($routes) {
        Write-TestLog "routes still present in finally; running watchdog cleanup"
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $watchdog -TimeoutSeconds 1 -LogFile "$LogFile.final-watchdog" | Out-Null
    }
    Write-TestLog "test finished"
}
