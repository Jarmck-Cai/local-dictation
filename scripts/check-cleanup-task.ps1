param([string]$MakeNsis)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path -Parent $PSScriptRoot
$source = Join-Path $projectRoot 'desktop\src-tauri\resources\cleanup-task.ps1'
$hooks = Join-Path $projectRoot 'desktop\src-tauri\nsis\local-dictation-hooks.nsh'
$outputs = Join-Path $projectRoot 'outputs'
$fixtureRoot = Join-Path $outputs ('.cleanup-task-check-' + [guid]::NewGuid().ToString('N'))
$global:CleanupTaskCheckState = @{ Tasks = @{} }
$global:CleanupTaskCheckState.Exports = @{}
$global:CleanupTaskCheckState.RegisterCalls = @()
$global:CleanupTaskCheckState.RemoveCalls = @()
$global:CleanupTaskCheckState.Failure = ''
$global:CleanupTaskCheckState.Passed = 0

function Assert-Check([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Check([string]$Name, [scriptblock]$Body) {
    & $Body
    $global:CleanupTaskCheckState.Passed++
    Write-Output ('PASS ' + $Name)
}

# Only these in-memory doubles may touch the scheduled-task boundary. The
# production script is executed unmodified; no real ScheduledTasks cmdlet runs.
function Get-ScheduledTask {
    [CmdletBinding()]
    param([string]$TaskPath)
    Assert-Check ($TaskPath -eq '\') 'Task lookup escaped the root namespace'
    if ($global:CleanupTaskCheckState.Failure -eq 'lookup') { throw 'Simulated task lookup denied' }
    @($global:CleanupTaskCheckState.Tasks.Values)
}

function Export-ScheduledTask {
    [CmdletBinding()]
    param([string]$TaskName, [string]$TaskPath)
    Assert-Check ($TaskPath -eq '\') 'Task export escaped the root namespace'
    Assert-Check ($global:CleanupTaskCheckState.Tasks.ContainsKey($TaskName)) 'Export requested a different task identity'
    if ($global:CleanupTaskCheckState.Failure -eq 'export') { throw 'Simulated task export denied' }
    Assert-Check ($global:CleanupTaskCheckState.Exports.ContainsKey($TaskName)) 'Fixture has no canonical task export'
    $global:CleanupTaskCheckState.Exports[$TaskName]
}

function Register-ScheduledTask {
    [CmdletBinding()]
    param([string]$TaskName, [string]$TaskPath, [string]$Xml, [switch]$Force)
    if ($global:CleanupTaskCheckState.Failure -eq 'register') { throw 'Simulated task registration denied' }
    Assert-Check ($TaskPath -eq '\') 'Task registration escaped the root namespace'
    if ($global:CleanupTaskCheckState.Tasks.ContainsKey($TaskName) -and -not $Force) { throw 'Task already exists' }
    [xml]$document = $Xml
    $ns = New-Object System.Xml.XmlNamespaceManager($document.NameTable)
    $ns.AddNamespace('t', 'http://schemas.microsoft.com/windows/2004/02/mit/task')
    $action = $document.SelectSingleNode('/t:Task/t:Actions/t:Exec', $ns)
    $principal = $document.SelectSingleNode('/t:Task/t:Principals/t:Principal', $ns)
    $task = [pscustomobject]@{
        TaskName = $TaskName
        TaskPath = $TaskPath
        Description = $document.Task.RegistrationInfo.Description
        Actions = @([pscustomobject]@{
            Execute = $action.Command
            Arguments = $action.Arguments
            WorkingDirectory = $action.WorkingDirectory
        })
        Principal = [pscustomobject]@{
            UserId = $principal.UserId
            LogonType = 'Interactive'
            RunLevel = 'Limited'
        }
    }
    $global:CleanupTaskCheckState.RegisterCalls += [pscustomobject]@{ Name = $TaskName; Force = [bool]$Force; Xml = $document }
    $global:CleanupTaskCheckState.Tasks[$TaskName] = $task
    $global:CleanupTaskCheckState.Exports[$TaskName] = $document.OuterXml
    $task
}

function Unregister-ScheduledTask {
    [CmdletBinding(SupportsShouldProcess = $true)]
    param([string]$TaskName, [string]$TaskPath)
    if ($global:CleanupTaskCheckState.Failure -eq 'remove') { throw 'Simulated task removal denied' }
    Assert-Check ($TaskPath -eq '\') 'Task removal escaped the root namespace'
    $global:CleanupTaskCheckState.RemoveCalls += $TaskName
    $global:CleanupTaskCheckState.Tasks.Remove($TaskName)
    $global:CleanupTaskCheckState.Exports.Remove($TaskName)
}

function New-Fixture([string]$Name) {
    $installDir = Join-Path $fixtureRoot $Name
    $resourceDir = Join-Path $installDir 'resources'
    [IO.Directory]::CreateDirectory($resourceDir) | Out-Null
    Copy-Item -LiteralPath $source -Destination (Join-Path $resourceDir 'cleanup-task.ps1')
    [IO.File]::WriteAllText((Join-Path $installDir 'local-dictation.exe'), 'never execute this test fixture')
    $installDir
}

function Invoke-Fixture([string]$InstallDir, [string]$Mode, [int]$WantExit = 0, [string]$TaskScript, [string]$InstallExecutable, [string]$DiagnosticPattern) {
    $scriptPath = if ($TaskScript) { $TaskScript } else { Join-Path $InstallDir 'resources\cleanup-task.ps1' }
    $arguments = @{ Mode = $Mode }
    if ($InstallExecutable) { $arguments.InstallExecutable = $InstallExecutable }
    $global:LASTEXITCODE = 99
    # Failed cases intentionally exercise script error output as well as exit status.
    $messages = @(& $scriptPath @arguments 2>&1)
    Assert-Check ($LASTEXITCODE -eq $WantExit) "Expected $Mode exit $WantExit, received $LASTEXITCODE`: $messages"
    if ($WantExit -ne 0) {
        Assert-Check ($messages.Count -gt 0) 'Failure gave the installer no diagnostic'
        if ($DiagnosticPattern) {
            Assert-Check (($messages -join "`n") -match $DiagnosticPattern) 'Failure did not explain the rejected installation path'
        }
    }
}

try {
    Check 'cleanup task resource exists' {
        Assert-Check (Test-Path -LiteralPath $source -PathType Leaf) 'Missing cleanup-task.ps1 installer resource'
    }
    $installA = New-Fixture 'Local Dictation 中文'
    $installB = New-Fixture 'Another install'
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value

    Check 'registers a current-user daily cleanup action' {
        Invoke-Fixture $installA 'Install'
        Assert-Check ($global:CleanupTaskCheckState.Tasks.Count -eq 1) 'Install did not create exactly one task'
        $task = @($global:CleanupTaskCheckState.Tasks.Values)[0]
        $xml = $global:CleanupTaskCheckState.RegisterCalls[-1].Xml
        Assert-Check ($task.Principal.UserId -eq $sid) 'Principal does not identify the current user'
        Assert-Check ($xml.Task.Principals.Principal.LogonType -eq 'InteractiveToken') 'Task requests noninteractive credentials'
        Assert-Check ($xml.Task.Principals.Principal.RunLevel -eq 'LeastPrivilege') 'Task requests elevated permissions'
        Assert-Check ($task.Actions.Count -eq 1) 'Task includes extra actions'
        Assert-Check ($task.Actions[0].Execute -eq (Join-Path $installA 'local-dictation.exe')) 'Task does not directly execute the installed app'
        Assert-Check ($task.Actions[0].Arguments -ceq '--cleanup-only') 'Task arguments can start recording or another mode'
        Assert-Check ($task.Actions[0].WorkingDirectory -eq $installA) 'Working directory differs from this install'
        Assert-Check ($xml.Task.Triggers.TimeTrigger.Repetition.Interval -eq 'P1D') 'Cleanup does not repeat daily'
        Assert-Check ($null -eq $xml.Task.Triggers.TimeTrigger.Repetition.SelectSingleNode('*[local-name()="Duration"]')) 'Cleanup repetition expires'
        Assert-Check ([datetime]$xml.Task.Triggers.TimeTrigger.StartBoundary -gt (Get-Date).AddMinutes(-1)) 'First run is not scheduled in the future'
        Assert-Check ($xml.Task.Settings.StartWhenAvailable -eq 'true') 'Missed cleanup cannot catch up'
        Assert-Check ($xml.Task.Settings.WakeToRun -eq 'false') 'Task can wake the computer'
        Assert-Check ($xml.Task.Settings.RunOnlyIfNetworkAvailable -eq 'false') 'Task depends on networking'
        Assert-Check ($xml.Task.Settings.DisallowStartIfOnBatteries -eq 'false') 'Task cannot clean on battery'
        Assert-Check ($xml.Task.Settings.StopIfGoingOnBatteries -eq 'false') 'Cleanup can stop on battery'
        Assert-Check ($xml.Task.Settings.MultipleInstancesPolicy -eq 'IgnoreNew') 'Concurrent cleanups are allowed'
        Assert-Check ($xml.Task.Settings.ExecutionTimeLimit -eq 'PT5M') 'Cleanup is not time bounded'
        Assert-Check (-not $global:CleanupTaskCheckState.RegisterCalls[-1].Force) 'New task creation can overwrite a name collision'
    }

    $taskA = @($global:CleanupTaskCheckState.Tasks.Values)[0]
    $nameA = $taskA.TaskName
    Check 'trigger payload follows the Microsoft schema element sequence' {
        $triggerElements = @($global:CleanupTaskCheckState.RegisterCalls[-1].Xml.Task.Triggers.TimeTrigger.ChildNodes | ForEach-Object { $_.LocalName })
        Assert-Check (($triggerElements -join ',') -ceq 'Enabled,StartBoundary,Repetition') 'Time trigger elements violate the documented XSD sequence'
    }
    Check 'reinstall updates only the matching owned task' {
        Invoke-Fixture $installA 'Install'
        Assert-Check ($global:CleanupTaskCheckState.Tasks.Count -eq 1) 'Reinstall duplicated the task'
        Assert-Check ($global:CleanupTaskCheckState.RegisterCalls[-1].Name -eq $nameA -and $global:CleanupTaskCheckState.RegisterCalls[-1].Force) 'Reinstall did not update the existing owned task'
    }
    Check 'accepts current-user principal normalized to a qualified account name' {
        $accountName = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        $global:CleanupTaskCheckState.Tasks[$nameA].Principal.UserId = $accountName
        Invoke-Fixture $installA 'Install'
        $global:CleanupTaskCheckState.Tasks[$nameA].Principal.UserId = $accountName
        Invoke-Fixture $installA 'Uninstall'
        Assert-Check (-not $global:CleanupTaskCheckState.Tasks.ContainsKey($nameA)) 'Qualified current-user principal could not be removed'
        Invoke-Fixture $installA 'Install'
    }
    Check 'accepts a short display name only when the canonical exported SID is current' {
        $shortName = ([Security.Principal.WindowsIdentity]::GetCurrent().Name -split '\\')[-1]
        $global:CleanupTaskCheckState.Tasks[$nameA].Principal.UserId = $shortName
        Invoke-Fixture $installA 'Install'
        $global:CleanupTaskCheckState.Tasks[$nameA].Principal.UserId = $shortName
        Invoke-Fixture $installA 'Uninstall'
        Assert-Check (-not $global:CleanupTaskCheckState.Tasks.ContainsKey($nameA)) 'Current canonical SID could not be removed after short-name normalization'
        Invoke-Fixture $installA 'Install'
    }
    Check 'rejects a misleading current display name with a foreign canonical SID' {
        $originalExport = $global:CleanupTaskCheckState.Exports[$nameA]
        $originalDisplay = $global:CleanupTaskCheckState.Tasks[$nameA].Principal.UserId
        $global:CleanupTaskCheckState.Tasks[$nameA].Principal.UserId = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        [xml]$foreignExport = $originalExport
        $foreignExport.Task.Principals.Principal.UserId = 'S-1-5-21-111-222-333-9999'
        $global:CleanupTaskCheckState.Exports[$nameA] = $foreignExport.OuterXml
        $registerCount = $global:CleanupTaskCheckState.RegisterCalls.Count
        $removeCount = $global:CleanupTaskCheckState.RemoveCalls.Count
        Invoke-Fixture $installA 'Install' 1
        Invoke-Fixture $installA 'Uninstall' 1
        Assert-Check ($global:CleanupTaskCheckState.RegisterCalls.Count -eq $registerCount) 'Display name overrode a foreign canonical SID during update'
        Assert-Check ($global:CleanupTaskCheckState.RemoveCalls.Count -eq $removeCount) 'Display name overrode a foreign canonical SID during uninstall'
        $global:CleanupTaskCheckState.Exports[$nameA] = $originalExport
        $global:CleanupTaskCheckState.Tasks[$nameA].Principal.UserId = $originalDisplay
    }
    Check 'task export errors reject both update and uninstall without mutation' {
        $registerCount = $global:CleanupTaskCheckState.RegisterCalls.Count
        $removeCount = $global:CleanupTaskCheckState.RemoveCalls.Count
        $global:CleanupTaskCheckState.Failure = 'export'
        Invoke-Fixture $installA 'Install' 1
        Invoke-Fixture $installA 'Uninstall' 1
        $global:CleanupTaskCheckState.Failure = ''
        Assert-Check ($global:CleanupTaskCheckState.RegisterCalls.Count -eq $registerCount) 'Failed export still updated a task'
        Assert-Check ($global:CleanupTaskCheckState.RemoveCalls.Count -eq $removeCount) 'Failed export still removed a task'
    }
    Check 'malformed or ambiguous canonical XML rejects all task mutation' {
        $originalExport = $global:CleanupTaskCheckState.Exports[$nameA]
        $invalidExports = @(
            '<Task>',
            ('<Task><Principals><Principal><UserId>' + $sid + '</UserId></Principal></Principals></Task>'),
            '<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Principals><Principal /></Principals></Task>',
            ('<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Principals><Principal><UserId>' + $sid + '</UserId><UserId>' + $sid + '</UserId></Principal></Principals></Task>'),
            ('<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Principals><Principal><UserId>' + $sid + '</UserId></Principal><Principal /></Principals></Task>'),
            '<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Principals><Principal><UserId>local-account</UserId></Principal></Principals></Task>',
            ('<!DOCTYPE Task [<!ENTITY account "' + $sid + '">]><Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Principals><Principal><UserId>&account;</UserId></Principal></Principals></Task>')
        )
        Assert-Check ($invalidExports.Count -eq 7) 'Canonical XML rejection fixtures are incomplete'
        foreach ($invalidExport in $invalidExports) {
            $global:CleanupTaskCheckState.Exports[$nameA] = $invalidExport
            $registerCount = $global:CleanupTaskCheckState.RegisterCalls.Count
            $removeCount = $global:CleanupTaskCheckState.RemoveCalls.Count
            Invoke-Fixture $installA 'Install' 1
            Invoke-Fixture $installA 'Uninstall' 1
            Assert-Check ($global:CleanupTaskCheckState.RegisterCalls.Count -eq $registerCount) 'Invalid canonical XML still updated a task'
            Assert-Check ($global:CleanupTaskCheckState.RemoveCalls.Count -eq $removeCount) 'Invalid canonical XML still removed a task'
        }
        $global:CleanupTaskCheckState.Exports[$nameA] = $originalExport
    }
    Check 'copied uninstaller resource targets the original install after file removal' {
        $uninstallScript = Join-Path $fixtureRoot 'copied-cleanup-task.ps1'
        Copy-Item -LiteralPath $source -Destination $uninstallScript
        $installedExecutable = Join-Path $installA 'local-dictation.exe'
        Remove-Item -LiteralPath $installedExecutable
        Invoke-Fixture $installA 'Uninstall' 0 $uninstallScript $installedExecutable
        Assert-Check (-not $global:CleanupTaskCheckState.Tasks.ContainsKey($nameA)) 'Copied resource did not remove the original install task'
        [IO.File]::WriteAllText($installedExecutable, 'never execute this test fixture')
        Invoke-Fixture $installA 'Install'
    }
    Check 'different install paths receive separate task identities' {
        Invoke-Fixture $installB 'Install'
        Assert-Check ($global:CleanupTaskCheckState.Tasks.Count -eq 2) 'Second install overwrote another path'
        Assert-Check ($global:CleanupTaskCheckState.RegisterCalls[-1].Name -ne $nameA) 'Task identity does not include install location'
    }
    Check 'special path characters remain literal action data' {
        $special = New-Fixture '路径 & ${notCode}; (test) [x] '' quote ` tick'
        Invoke-Fixture $special 'Install'
        $specialTask = $global:CleanupTaskCheckState.Tasks[$global:CleanupTaskCheckState.RegisterCalls[-1].Name]
        Assert-Check ($specialTask.Actions[0].Execute -ceq (Join-Path $special 'local-dictation.exe')) 'Special characters changed the executable path'
        Assert-Check ($specialTask.Actions[0].Arguments -ceq '--cleanup-only') 'Path contents became action arguments'
        Invoke-Fixture $special 'Uninstall'
    }
    Check 'installation rejects every percent path before task registration' {
        foreach ($directoryName in @('Literal %USERNAME% install', 'Unpaired % install', 'EndPercent%')) {
            $percentInstall = New-Fixture $directoryName
            $registerCount = $global:CleanupTaskCheckState.RegisterCalls.Count
            $taskCount = $global:CleanupTaskCheckState.Tasks.Count
            Invoke-Fixture $percentInstall 'Install' 1 -DiagnosticPattern 'containing.*%.*Task Scheduler'
            Assert-Check ($global:CleanupTaskCheckState.RegisterCalls.Count -eq $registerCount) 'A percent path reached task registration'
            Assert-Check ($global:CleanupTaskCheckState.Tasks.Count -eq $taskCount) 'Rejected install changed the scheduled tasks'
        }
    }
    Check 'uninstall can remove a legacy literal percent path task' {
        $legacyInstall = New-Fixture 'Legacy %USERNAME% install'
        $legacyExecutable = Join-Path $legacyInstall 'local-dictation.exe'
        $legacyHasher = [Security.Cryptography.SHA256]::Create()
        try {
            $legacyDigest = $legacyHasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($legacyExecutable.ToLowerInvariant()))
            $legacyPathId = ([BitConverter]::ToString($legacyDigest)).Replace('-', '').ToLowerInvariant()
        }
        finally { $legacyHasher.Dispose() }
        $legacyName = 'LocalDictation.AudioCleanup.' + $sid + '.' + $legacyPathId
        # Seed a pre-fix task fixture by its raw-path identity; no task provider
        # or registration runs to create this historical installation state.
        $global:CleanupTaskCheckState.Tasks[$legacyName] = [pscustomobject]@{
            TaskName = $legacyName
            TaskPath = '\'
            Description = 'Local Dictation audio cleanup v1; owner=' + $sid + '; install=' + $legacyPathId
            Actions = @([pscustomobject]@{
                Execute = $legacyExecutable
                Arguments = '--cleanup-only'
                WorkingDirectory = $legacyInstall
            })
            Principal = [pscustomobject]@{ UserId = $sid }
        }
        $global:CleanupTaskCheckState.Exports[$legacyName] = '<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Principals><Principal><UserId>' + $sid + '</UserId></Principal></Principals></Task>'
        $legacyUninstallCopy = Join-Path $fixtureRoot 'legacy-uninstall-copy.ps1'
        Copy-Item -LiteralPath $source -Destination $legacyUninstallCopy
        Remove-Item -LiteralPath $legacyExecutable
        $registerCount = $global:CleanupTaskCheckState.RegisterCalls.Count
        $removeCount = $global:CleanupTaskCheckState.RemoveCalls.Count
        Invoke-Fixture $legacyInstall 'Uninstall' 0 $legacyUninstallCopy $legacyExecutable
        Assert-Check (-not $global:CleanupTaskCheckState.Tasks.ContainsKey($legacyName)) 'Legacy raw-path task was not removed'
        Assert-Check ($global:CleanupTaskCheckState.RemoveCalls.Count -eq $removeCount + 1 -and $global:CleanupTaskCheckState.RemoveCalls[-1] -ceq $legacyName) 'Legacy uninstall targeted another identity'
        Assert-Check ($global:CleanupTaskCheckState.RegisterCalls.Count -eq $registerCount) 'Uninstall unexpectedly registered a task'
    }
    Check 'uninstall removes only this install and is idempotent' {
        Invoke-Fixture $installB 'Uninstall'
        Assert-Check ($global:CleanupTaskCheckState.Tasks.Count -eq 1 -and $global:CleanupTaskCheckState.Tasks.ContainsKey($nameA)) 'Uninstall removed a different install'
        $removeCount = $global:CleanupTaskCheckState.RemoveCalls.Count
        Invoke-Fixture $installB 'Uninstall'
        Assert-Check ($global:CleanupTaskCheckState.RemoveCalls.Count -eq $removeCount) 'Missing-task uninstall attempted removal'
    }

    foreach ($field in @('Description', 'UserId', 'Execute', 'Arguments', 'WorkingDirectory', 'ExtraAction')) {
        Check ("rejects a same-name task with foreign $field") {
            $original = $global:CleanupTaskCheckState.Tasks[$nameA]
            $originalExport = $global:CleanupTaskCheckState.Exports[$nameA]
            $foreign = $original | ConvertTo-Json -Depth 10 | ConvertFrom-Json
            switch ($field) {
                'Description' { $foreign.Description = 'Another application task' }
                'UserId' {
                    $foreign.Principal.UserId = 'S-1-5-21-111-222-333-9999'
                    [xml]$foreignExport = $originalExport
                    $foreignExport.Task.Principals.Principal.UserId = $foreign.Principal.UserId
                    $global:CleanupTaskCheckState.Exports[$nameA] = $foreignExport.OuterXml
                }
                'Execute' { $foreign.Actions[0].Execute = 'C:\Other\local-dictation.exe' }
                'Arguments' { $foreign.Actions[0].Arguments = '--start-hidden' }
                'WorkingDirectory' { $foreign.Actions[0].WorkingDirectory = 'C:\Other' }
                'ExtraAction' { $foreign.Actions += $foreign.Actions[0] }
            }
            $global:CleanupTaskCheckState.Tasks[$nameA] = $foreign
            $registerCount = $global:CleanupTaskCheckState.RegisterCalls.Count
            $removeCount = $global:CleanupTaskCheckState.RemoveCalls.Count
            Invoke-Fixture $installA 'Install' 1
            Invoke-Fixture $installA 'Uninstall' 1
            Assert-Check ($global:CleanupTaskCheckState.RegisterCalls.Count -eq $registerCount) 'Foreign task was overwritten'
            Assert-Check ($global:CleanupTaskCheckState.RemoveCalls.Count -eq $removeCount) 'Foreign task was deleted'
            $global:CleanupTaskCheckState.Tasks[$nameA] = $original
            $global:CleanupTaskCheckState.Exports[$nameA] = $originalExport
        }
    }
    foreach ($fault in @('lookup', 'register', 'remove')) {
        Check ("surfaces ScheduledTasks $fault failure") {
            $global:CleanupTaskCheckState.Failure = $fault
            $mode = if ($fault -eq 'remove') { 'Uninstall' } else { 'Install' }
            Invoke-Fixture $installA $mode 1
            $global:CleanupTaskCheckState.Failure = ''
            Assert-Check ($global:CleanupTaskCheckState.Tasks.ContainsKey($nameA)) 'Failure removed the existing task'
        }
    }
    Check 'missing application never registers a task' {
        $missing = New-Fixture 'Missing executable'
        Remove-Item -LiteralPath (Join-Path $missing 'local-dictation.exe')
        $registerCount = $global:CleanupTaskCheckState.RegisterCalls.Count
        Invoke-Fixture $missing 'Install' 1
        Assert-Check ($global:CleanupTaskCheckState.RegisterCalls.Count -eq $registerCount) 'Task registered without its executable'
    }

    Check 'NSIS hooks compile with install and uninstall entry points' {
        Assert-Check (Test-Path -LiteralPath $hooks -PathType Leaf) 'Missing installer hooks'
        if (-not $MakeNsis) { $MakeNsis = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe' }
        Assert-Check (Test-Path -LiteralPath $MakeNsis -PathType Leaf) 'Set -MakeNsis to the installed NSIS compiler'
        $harness = Join-Path $fixtureRoot 'hooks-check.nsi'
        $compiled = Join-Path $fixtureRoot 'hooks-check.exe'
        $harnessText = @"
Unicode true
Name "Cleanup hook compile check"
OutFile "$compiled"
RequestExecutionLevel user
!include "$hooks"
!include "MUI2.nsh"
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"
Section
!insertmacro NSIS_HOOK_POSTINSTALL
WriteUninstaller "`$INSTDIR\uninstall.exe"
SectionEnd
Section Uninstall
!insertmacro NSIS_HOOK_PREUNINSTALL
DetailPrint "Cancellable app-running check"
!insertmacro NSIS_HOOK_POSTUNINSTALL
SectionEnd
"@
        [IO.File]::WriteAllText($harness, $harnessText, (New-Object Text.UTF8Encoding($true)))
        $compilerOutput = @(& $MakeNsis '-V2' $harness 2>&1)
        Assert-Check ($LASTEXITCODE -eq 0) "NSIS hooks did not compile: $compilerOutput"
        Assert-Check (Test-Path -LiteralPath $compiled -PathType Leaf) 'NSIS compiler emitted no check artifact'
        $expanded = @(& $MakeNsis '-PPO' $harness 2>&1) -join "`n"
        Assert-Check ($LASTEXITCODE -eq 0) 'Cannot inspect expanded NSIS hook instructions'
        $uninstallStart = $expanded.IndexOf('Section Uninstall', [StringComparison]::Ordinal)
        $uninstallInstructions = $expanded.Substring($uninstallStart)
        $cancellationCheck = $uninstallInstructions.IndexOf('Cancellable app-running check', [StringComparison]::Ordinal)
        $taskMutation = $uninstallInstructions.IndexOf('nsExec::ExecToStack', [StringComparison]::Ordinal)
        Assert-Check ($cancellationCheck -ge 0 -and $taskMutation -gt $cancellationCheck) 'Task removal happens before the user can cancel uninstall'
        # Never run the compiled installer, even this minimal harness.
    }
    Check 'Tauri config binds hooks and bundles the script resource' {
        $config = Get-Content -LiteralPath (Join-Path $projectRoot 'desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
        Assert-Check ($config.bundle.windows.nsis.installMode -eq 'currentUser') 'Installer no longer uses current-user mode'
        Assert-Check ($config.bundle.windows.nsis.installerHooks -eq 'nsis/local-dictation-hooks.nsh') 'Installer hook is not configured'
        Assert-Check (@($config.bundle.resources) -contains 'resources/**/*') 'Default bundle no longer includes cleanup script'
        $windowsConfig = Get-Content -LiteralPath (Join-Path $projectRoot 'desktop\src-tauri\tauri.windows.conf.json') -Raw | ConvertFrom-Json
        Assert-Check ($windowsConfig.bundle.resources.resources -eq 'resources') 'Windows bundle relocates cleanup script unexpectedly'
    }
    Write-Output ("All $($global:CleanupTaskCheckState.Passed) cleanup installer checks passed; ScheduledTasks were simulated and no installer was executed.")
}
finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        $resolved = [IO.Path]::GetFullPath($fixtureRoot)
        $allowedPrefix = [IO.Path]::GetFullPath($outputs).TrimEnd('\') + '\.cleanup-task-check-'
        if (-not $resolved.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Refusing to clean fixture files outside the designated outputs directory'
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
