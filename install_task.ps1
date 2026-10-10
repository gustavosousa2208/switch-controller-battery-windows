# Install/uninstall the battery publisher as a hidden logon task.
#
#   Install   (as admin): powershell -ExecutionPolicy Bypass -File install_task.ps1
#   Uninstall (as admin): powershell -ExecutionPolicy Bypass -File install_task.ps1 -Remove
#
# The task runs pythonw.exe (no console window) as the current user with
# highest privileges, starts at logon, and retries every minute while the
# publisher is not running (watchdog; single instance enforced). The publisher
# logs to logs\publisher.log. Install verifies the watchdog by killing the
# process once (skip with -SkipRestartTest).

param(
    [switch]$Remove,
    [switch]$SkipRestartTest
)

$ErrorActionPreference = 'Stop'
$TaskName = 'SwitchControllerBattery'
$ProjectDir = $PSScriptRoot
$ScriptPath = Join-Path $ProjectDir 'battery_publisher.py'
$UserId = "$env:USERDOMAIN\$env:USERNAME"
$TranscriptPath = Join-Path $ProjectDir ('install_task-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')

# resolve pythonw.exe (the windowless Python host)
$Pythonw = $null
$cmd = Get-Command pythonw.exe -ErrorAction SilentlyContinue
if ($cmd) { $Pythonw = $cmd.Source }
if (-not $Pythonw) {
    $candidates = Get-ChildItem "$env:LOCALAPPDATA\Programs\Python\Python*\pythonw.exe" `
        -ErrorAction SilentlyContinue | Sort-Object FullName -Descending
    if ($candidates) { $Pythonw = $candidates[0].FullName }
}
if (-not $Pythonw) {
    throw ("pythonw.exe not found. Install Python 3.8+ from python.org " +
           "(enable 'Add python.exe to PATH'), then run: " +
           "python -m pip install hidapi")
}

function Get-PublisherProcess {
    Get-CimInstance Win32_Process -Filter "Name = 'python.exe' OR Name = 'pythonw.exe'" |
        Where-Object { $_.CommandLine -like '*battery_publisher*' }
}

function Stop-Publishers {
    foreach ($proc in Get-PublisherProcess) {
        Write-Host "stopping publisher pid $($proc.ProcessId)"
        Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

# recover from stale installer runs (e.g. interrupted)
foreach ($stale in Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'") {
    if ($stale.ProcessId -ne $PID -and $stale.CommandLine -like '*install_task.ps1*') {
        Write-Host "stopping stale installer pid $($stale.ProcessId)"
        Stop-Process -Id $stale.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

Start-Transcript -Path $TranscriptPath -Force | Out-Null
try {
    if ($Remove) {
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false `
            -ErrorAction SilentlyContinue
        Stop-Publishers
        Write-Host "task '$TaskName' removed"
        return
    }

    foreach ($path in @($ScriptPath, $Pythonw)) {
        if (-not (Test-Path $path)) { throw "missing file: $path" }
    }

    Stop-Publishers
    Start-Sleep -Milliseconds 800

    $action = New-ScheduledTaskAction -Execute $Pythonw `
        -Argument ('"' + $ScriptPath + '"') -WorkingDirectory $ProjectDir
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $UserId
    $trigger.Delay = 'PT30S'
    # Watchdog: a once-trigger repeating every minute. If the publisher is
    # already running, MultipleInstances=IgnoreNew skips the tick; if it died
    # (for any reason), a fresh instance starts within ~1 min.
    $watchdog = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) `
        -RepetitionInterval (New-TimeSpan -Minutes 1) `
        -RepetitionDuration (New-TimeSpan -Days 3650)
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries -StartWhenAvailable `
        -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1)
    $principal = New-ScheduledTaskPrincipal -UserId $UserId `
        -LogonType Interactive -RunLevel Highest

    Register-ScheduledTask -TaskName $TaskName -Action $action `
        -Trigger @($trigger, $watchdog) -Settings $settings `
        -Principal $principal -Force | Out-Null
    Write-Host "task '$TaskName' registered"

    Start-ScheduledTask -TaskName $TaskName
    Write-Host "task started; waiting for the hidden publisher process..."
    $proc = $null
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        $proc = Get-PublisherProcess |
            Where-Object { $_.Name -eq 'pythonw.exe' } |
            Select-Object -First 1
        if ($proc) { break }
    }
    if (-not $proc) { throw "hidden publisher process did not appear" }
    $firstPid = $proc.ProcessId
    Write-Host "hidden publisher running as pid $firstPid (pythonw: $Pythonw)"

    if (-not $SkipRestartTest) {
        Write-Host "watchdog test: killing pid $firstPid (expect a new instance within ~1-2 min)..."
        Stop-Process -Id $firstPid -Force
        $newProc = $null
        for ($i = 0; $i -lt 150; $i++) {
            Start-Sleep -Seconds 1
            $newProc = Get-PublisherProcess |
                Where-Object { $_.Name -eq 'pythonw.exe' } |
                Select-Object -First 1
            if ($newProc) { break }
            if ($i % 15 -eq 14) {
                Write-Host "  waiting for restart... $($i + 1)s"
            }
        }
        if ($newProc) {
            Write-Host ("watchdog test PASSED: new pid {0} after ~{1}s" -f `
                $newProc.ProcessId, ($i + 1))
        } else {
            Write-Host "watchdog test FAILED (no restart within 150 s); starting task manually"
            Start-ScheduledTask -TaskName $TaskName
        }
    }

    Get-ScheduledTask -TaskName $TaskName |
        Select-Object TaskName, State | Format-List
    Get-ScheduledTaskInfo -TaskName $TaskName |
        Select-Object LastRunTime, LastTaskResult, NextRunTime | Format-List
} finally {
    Stop-Transcript | Out-Null
}
