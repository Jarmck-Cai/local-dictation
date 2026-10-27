param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Install', 'Uninstall')]
    [string]$Mode,
    [string]$InstallExecutable
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

try {
    # Installation uses this resource's location. After uninstall has removed
    # resources, NSIS runs a private temporary copy with the original exe path.
    # Both paths stay data; neither is interpolated into source or a shell.
    if ($InstallExecutable) {
        if ($Mode -ne 'Uninstall' -or -not [IO.Path]::IsPathRooted($InstallExecutable) -or
            [IO.Path]::GetFileName($InstallExecutable) -ine 'local-dictation.exe') {
            throw 'An explicit executable path is only valid for uninstalling Local Dictation.'
        }
        $installDir = Split-Path -Parent ([IO.Path]::GetFullPath($InstallExecutable))
    }
    else { $installDir = Split-Path -Parent $PSScriptRoot }
    $installDir = [IO.Path]::GetFullPath($installDir).TrimEnd('\')
    if ($installDir -match '^[A-Za-z]:$') { $installDir += '\' }
    # The scheduler expands environment variables in Command/WorkingDirectory
    # after XML parsing. Reject all percent paths for registration; uninstall
    # still computes the original raw-path identity to remove older tasks.
    if ($Mode -eq 'Install' -and $installDir.Contains('%')) {
        throw "Cannot register cleanup for an installation path containing '%' because Task Scheduler expands environment variables; no task was registered."
    }
    $executable = Join-Path $installDir 'local-dictation.exe'
    if ($Mode -eq 'Uninstall') {
        # Runs after the cancellable uninstall phase. Login cleanup is independent
        # of Task Scheduler availability and only touches this installation.
        $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
        $loginValues = $null
        try { $loginValues = Get-ItemProperty -LiteralPath $runKey -ErrorAction Stop }
        catch [System.Management.Automation.ItemNotFoundException] { }
        $loginProperty = if ($null -ne $loginValues) { $loginValues.PSObject.Properties['Local Dictation'] } else { $null }
        $loginCommand = if ($null -ne $loginProperty) { $loginProperty.Value } else { $null }
        $ownedCommands = @(('"' + $executable + '" --start-hidden'), ('"' + $executable + '"'), $executable)
        if ($loginCommand -is [string] -and $ownedCommands -contains $loginCommand.Trim()) {
            Remove-ItemProperty -LiteralPath $runKey -Name 'Local Dictation' -ErrorAction Stop
        }
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $sid = $identity.User.Value
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($executable.ToLowerInvariant()))
        $pathId = ([BitConverter]::ToString($digest)).Replace('-', '').ToLowerInvariant()
    }
    finally { $hasher.Dispose() }

    $taskName = 'LocalDictation.AudioCleanup.' + $sid + '.' + $pathId
    $description = 'Local Dictation audio cleanup v1; owner=' + $sid + '; install=' + $pathId
    # Enumerate one known namespace so a missing task is distinguishable from
    # access/service errors. TaskName itself contains no wildcard characters.
    $existing = @(Get-ScheduledTask -TaskPath '\' -ErrorAction Stop | Where-Object {
        $_.TaskName -ceq $taskName -and $_.TaskPath -eq '\'
    })
    if ($existing.Count -gt 1) { throw 'Ambiguous cleanup task identity; no task was changed.' }
    if ($existing.Count -eq 1) {
        $task = $existing[0]
        $actions = @($task.Actions)
        # CIM may display a short account name. Trust only the canonical SID
        # exported for this exact task; never resolve or trust account aliases.
        $canonicalXml = Export-ScheduledTask -TaskName $taskName -TaskPath '\' -ErrorAction Stop
        $xmlSettings = New-Object System.Xml.XmlReaderSettings
        $xmlSettings.DtdProcessing = [System.Xml.DtdProcessing]::Prohibit
        $xmlSettings.XmlResolver = $null
        $xmlSettings.CloseInput = $true
        $xmlReader = [System.Xml.XmlReader]::Create([IO.StringReader]::new($canonicalXml), $xmlSettings)
        try {
            $canonicalTask = New-Object System.Xml.XmlDocument
            $canonicalTask.XmlResolver = $null
            $canonicalTask.Load($xmlReader)
        }
        finally { $xmlReader.Dispose() }
        $xmlNamespace = New-Object System.Xml.XmlNamespaceManager($canonicalTask.NameTable)
        $xmlNamespace.AddNamespace('t', 'http://schemas.microsoft.com/windows/2004/02/mit/task')
        $principals = @($canonicalTask.SelectNodes('/t:Task/t:Principals/t:Principal', $xmlNamespace))
        $userIds = @($canonicalTask.SelectNodes('/t:Task/t:Principals/t:Principal/t:UserId', $xmlNamespace))
        $sameUser = $principals.Count -eq 1 -and $userIds.Count -eq 1 -and $userIds[0].InnerText -ceq $sid
        $owned = $task.Description -ceq $description -and
            $sameUser -and
            $actions.Count -eq 1 -and
            $actions[0].Execute -eq $executable -and
            $actions[0].Arguments -ceq '--cleanup-only' -and
            $actions[0].WorkingDirectory -eq $installDir
        if (-not $owned) { throw 'Cleanup task name is already used by a different owner or action; no task was changed.' }
    }

    if ($Mode -eq 'Uninstall') {
        if ($existing.Count -eq 1) {
            Unregister-ScheduledTask -TaskName $taskName -TaskPath '\' -Confirm:$false -ErrorAction Stop
        }
        exit 0
    }

    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw 'The installed Local Dictation executable is missing; no cleanup task was registered.'
    }
    # XML-escape the supported path characters. The task executes the app
    # directly, with one fixed argument and no shell wrapper.
    $xmlExecutable = [Security.SecurityElement]::Escape($executable)
    $xmlDirectory = [Security.SecurityElement]::Escape($installDir)
    $xmlSid = [Security.SecurityElement]::Escape($sid)
    $xmlDescription = [Security.SecurityElement]::Escape($description)
    $start = (Get-Date).AddMinutes(5).ToString('s', [Globalization.CultureInfo]::InvariantCulture)
    $definition = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>$xmlDescription</Description></RegistrationInfo>
  <Triggers>
    <TimeTrigger>
      <Enabled>true</Enabled><StartBoundary>$start</StartBoundary>
      <Repetition><Interval>P1D</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="CurrentUser"><UserId>$xmlSid</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <Enabled>true</Enabled><Hidden>false</Hidden><WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT5M</ExecutionTimeLimit><Priority>7</Priority>
  </Settings>
  <Actions Context="CurrentUser"><Exec><Command>$xmlExecutable</Command><Arguments>--cleanup-only</Arguments><WorkingDirectory>$xmlDirectory</WorkingDirectory></Exec></Actions>
</Task>
"@
    # Force is only allowed after all ownership checks. A new registration must
    # fail if a conflicting task appears between lookup and registration.
    Register-ScheduledTask -TaskName $taskName -TaskPath '\' -Xml $definition -Force:($existing.Count -eq 1) -ErrorAction Stop | Out-Null
    exit 0
}
catch {
    Write-Error ('Local Dictation cleanup task ' + $Mode + ' failed: ' + $_.Exception.Message) -ErrorAction Continue
    exit 1
}
