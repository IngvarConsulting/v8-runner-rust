param([string]$InstallerPath = (Join-Path $PSScriptRoot '../../docs/site/install.ps1'), [switch]$TypeCollision)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This regression test requires native Windows.' }
$InstallerPath = (Resolve-Path $InstallerPath).Path
$installerText = [IO.File]::ReadAllText($InstallerPath, [Text.Encoding]::UTF8)
if ([IO.File]::ReadAllBytes($InstallerPath) | Where-Object { $_ -gt 127 }) { throw 'Installer source must be ASCII for charset-free HTTP.' }
$serverJob = $null
$accelerators = $null
$collisionInstalled = $false
$script:httpCalls = 0
$userEnvironment = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
if ($null -eq $userEnvironment) { throw 'HKCU Environment registry key is unavailable.' }
$hadUserPath = $userEnvironment.GetValueNames() -contains 'Path'
$savedUserPath = $userEnvironment.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$savedUserPathKind = if ($hadUserPath) { $userEnvironment.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
$savedEnvironment = @{}
foreach ($key in @('Path', 'TEMP', 'TMP', 'V8_RUNNER_INSTALL_DIR', 'OS')) {
    $savedEnvironment[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('runner ' + (-join @([char]0x442, [char]0x435, [char]0x441, [char]0x442)) + ' spaces ' + [Guid]::NewGuid())
$bin = Join-Path $root 'private bin'
$scratch = Join-Path $root 'installer temp'
$target = Join-Path $bin 'v8-runner.exe'

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw "Assertion failed: $Message" }
}
function Hash([string]$Path) { (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Invoke-RestMethod {
    param([string]$Uri)
    if ($Uri -eq $installerUrl) { return Microsoft.PowerShell.Utility\Invoke-RestMethod -Uri $Uri -TimeoutSec 10 }
    if ($Uri -eq 'https://github.com/IngvarConsulting/v8-runner-rust/releases/latest/download/v8-runner-assets.json') {
        $script:httpCalls++
        return $manifest
    }
    throw "Unexpected REST URL: $Uri"
}
function Invoke-WebRequest {
    param([string]$Uri, [string]$OutFile)
    Assert-True ($Uri -eq 'https://github.com/IngvarConsulting/v8-runner-rust/releases/download/v1.0.0/v8-runner-windows-x86_64.zip') 'archive URL'
    $script:httpCalls++
    Copy-Item $fixtureArchive $OutFile
}
function Select-Fixture([string]$Archive, [string]$Binary, [string]$BadHash = '') {
    $script:fixtureArchive = $Archive
    $archiveHash = Hash $Archive
    $binaryHash = Hash $Binary
    if ($BadHash -eq 'archive') { $archiveHash = '0' * 64 }
    if ($BadHash -eq 'binary') { $binaryHash = '0' * 64 }
    $script:manifest = [pscustomobject]@{
        schemaVersion = 2
        release = [pscustomobject]@{ sourceTag = 'v1.0.0' }
        assets = @([pscustomobject]@{
            name = 'v8-runner-windows-x86_64.zip'
            sha256 = $archiveHash
            binary = [pscustomobject]@{ sha256 = $binaryHash }
        })
    }
}
function Assert-Clean {
    Assert-True (@(Get-ChildItem $scratch -Force).Count -eq 0) 'temporary extraction directory removed'
    Assert-True (@(Get-ChildItem $bin -Force).Count -eq 1) 'no staged exe remains'
    Assert-True (Test-Path $target) 'installed target exists'
}
function Install([string]$ExpectedFailure = '') {
    $beforeHash = if (Test-Path $target) { Hash $target } else { $null }
    $caught = $null
    try { Invoke-RestMethod $installerUrl | Invoke-Expression }
    catch { $caught = $_ }
    if ($ExpectedFailure) {
        Assert-True ($null -ne $caught) 'installation must fail'
        Assert-True ($caught.Exception.Message.Contains($ExpectedFailure)) "expected checksum refusal: $caught"
        Assert-True ((Hash $target) -eq $beforeHash) 'failure preserves previous binary'
    } elseif ($caught) { throw $caught }
    Assert-Clean
}

try {
    if ($TypeCollision) {
        # Fresh process: register before any lookup fills the type resolver cache.
        $accelerators = [psobject].Assembly.GetType('System.Management.Automation.TypeAccelerators')
        $typeName = 'System.Runtime.InteropServices.RuntimeInformation'
        $previousAccelerator = $accelerators::Get[$typeName]
        if ($null -ne $previousAccelerator) { $accelerators::Remove($typeName) }
        $accelerators::Add($typeName, [object])
        $collisionInstalled = $true
        Write-Host 'Controlled collision case: RuntimeInformation accelerator points to Object.'
    } else { Write-Host 'Real PSReadLine case: no controlled resolver collision.' }
    if (-not $TypeCollision) { Import-Module PSReadLine }
    $resolved = Invoke-Expression '[System.Runtime.InteropServices.RuntimeInformation]'
    $observedArch = Invoke-Expression '[System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture'
    Write-Host "Unqualified type assembly: $($resolved.Assembly.FullName); OSArchitecture: <$observedArch>"
    if ($TypeCollision) { Assert-True ($null -eq $observedArch) 'controlled old unqualified lookup returns null' }
    Assert-True ((Invoke-Expression '[System.Runtime.InteropServices.RuntimeInformation, mscorlib]::OSArchitecture').ToString() -eq 'X64') 'qualified Windows architecture survives collision'
    New-Item -ItemType Directory -Path $root, $bin, $scratch | Out-Null
    $readyFile = Join-Path $root 'http-port.txt'
    $serverJob = Start-Job -ArgumentList $InstallerPath, $readyFile -ScriptBlock {
        param($Path, $Ready)
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
        try {
            $listener.Start()
            [IO.File]::WriteAllText($Ready, [string]$listener.LocalEndpoint.Port)
            $body = [IO.File]::ReadAllBytes($Path)
            while ($true) {
                # Yield while idle so Stop-Job can interrupt cleanup between requests.
                while (-not $listener.Pending()) { Start-Sleep -Milliseconds 100 }
                $client = $listener.AcceptTcpClient()
                try {
                    $client.ReceiveTimeout = 5000
                    $stream = $client.GetStream()
                    $reader = [IO.StreamReader]::new($stream)
                    do { $line = $reader.ReadLine() } while ($null -ne $line -and $line.Length -gt 0)
                    $header = [Text.Encoding]::ASCII.GetBytes("HTTP/1.1 200 OK`r`nContent-Type: application/octet-stream`r`nContent-Length: $($body.Length)`r`nConnection: close`r`n`r`n")
                    $stream.Write($header, 0, $header.Length)
                    $stream.Write($body, 0, $body.Length)
                } finally { $client.Dispose() }
            }
        } finally { $listener.Stop() }
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not (Test-Path $readyFile) -and [DateTime]::UtcNow -lt $deadline -and $serverJob.State -notin @('Failed', 'Completed')) { Start-Sleep -Milliseconds 100 }
    Assert-True (Test-Path $readyFile) 'HTTP fixture starts within 20 seconds'
    $installerUrl = 'http://127.0.0.1:' + [IO.File]::ReadAllText($readyFile) + '/install.ps1'
    $env:TEMP = $scratch
    $env:TMP = $scratch
    $env:V8_RUNNER_INSTALL_DIR = $bin
    # Avoid a persistent PATH write during installation; restore the exact original below.
    $userEnvironment.SetValue('Path', "$bin;$savedUserPath", $savedUserPathKind)
    $csc = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    Assert-True (Test-Path $csc) 'framework csc.exe is required'
    $fixtures = @()
    foreach ($version in @('1.0.0', '1.0.1')) {
        $directory = Join-Path $root ('fixture-' + $version)
        $package = Join-Path $directory 'v8-runner-windows-x86_64'
        New-Item -ItemType Directory -Path $package | Out-Null
        $source = Join-Path $directory 'Runner.cs'
        $exe = Join-Path $package 'v8-runner.exe'
        [IO.File]::WriteAllText($source, ('class Runner { static int Main(string[] args) { System.Console.WriteLine("v8-runner ' + $version + '"); return args.Length == 1 && args[0] == "--version" ? 0 : 2; } }'))
        & $csc /nologo /target:exe ("/out:" + $exe) $source
        Assert-True ($LASTEXITCODE -eq 0) 'fixture compilation succeeds'
        $archive = Join-Path $directory 'runner.zip'
        Compress-Archive -Path $package -DestinationPath $archive
        $fixtures += [pscustomobject]@{ Archive = $archive; Binary = $exe; Version = $version }
    }
    Select-Fixture $fixtures[0].Archive $fixtures[0].Binary
    Install
    Assert-True ((Hash $target) -eq (Hash $fixtures[0].Binary)) 'fresh install copies verified binary'
    Write-Host 'PASS fresh installation'
    Install
    Assert-True ((Hash $target) -eq (Hash $fixtures[0].Binary)) 'repeat preserves binary contents'
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    Assert-True (@($userPath -split ';' | Where-Object { $_ -eq $bin }).Count -eq 1) 'user PATH entry is not duplicated'
    Write-Host 'PASS repeat installation'
    Assert-True ((Hash $fixtures[0].Binary) -ne (Hash $fixtures[1].Binary)) 'update fixture is different'
    Select-Fixture $fixtures[1].Archive $fixtures[1].Binary
    Install
    Assert-True ((Hash $target) -eq (Hash $fixtures[1].Binary)) 'replacement installs new bytes'
    Assert-True ((& $target --version) -eq 'v8-runner 1.0.1') 'replacement executable runs'
    Write-Host 'PASS replacement with different binary'
    Select-Fixture $fixtures[0].Archive $fixtures[0].Binary 'archive'
    Install 'SHA-256'
    Write-Host 'PASS archive checksum failure preserves target and cleans staging'
    Select-Fixture $fixtures[0].Archive $fixtures[0].Binary 'binary'
    Install 'SHA-256'
    Write-Host 'PASS binary checksum failure preserves target and cleans staging'
    $beforeCalls = $script:httpCalls
    $env:OS = 'FixtureNonWindows'
    Install 'Windows'
    Assert-True ($script:httpCalls -eq $beforeCalls) 'OS refusal precedes manifest/archive HTTP'
    $env:OS = $savedEnvironment['OS']
    Write-Host 'PASS readable OS refusal before downloads'
} finally {
    if ($serverJob) { Stop-Job $serverJob -ErrorAction SilentlyContinue; Remove-Job $serverJob -Force -ErrorAction SilentlyContinue }
    if ($collisionInstalled) {
        $accelerators::Remove($typeName)
        if ($null -ne $previousAccelerator) { $accelerators::Add($typeName, $previousAccelerator) }
    }
    try {
        if ($hadUserPath) { $userEnvironment.SetValue('Path', $savedUserPath, $savedUserPathKind) }
        else { $userEnvironment.DeleteValue('Path', $false) }
    } finally {
        $userEnvironment.Dispose()
        foreach ($key in $savedEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key], 'Process')
        }
        if (Test-Path $root) { Remove-Item $root -Recurse -Force }
    }
}
if (-not $TypeCollision) {
    $shellName = if ($PSVersionTable.PSEdition -eq 'Desktop') { 'powershell.exe' } else { 'pwsh.exe' }
    & (Join-Path $PSHOME $shellName) -NoProfile -File $PSCommandPath -InstallerPath $InstallerPath -TypeCollision
    Assert-True ($LASTEXITCODE -eq 0) 'fresh-process controlled collision installation passes'
}
