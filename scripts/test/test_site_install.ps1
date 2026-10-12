param([string]$InstallerPath = (Join-Path $PSScriptRoot '../../docs/site/install.ps1'))
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This regression test requires native Windows.' }
$InstallerPath = (Resolve-Path $InstallerPath).Path
$installerText = [IO.File]::ReadAllText($InstallerPath, [Text.Encoding]::UTF8)
$installerUrl = 'https://test.invalid/install.ps1'
$userEnvironment = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
if ($null -eq $userEnvironment) { throw 'HKCU Environment registry key is unavailable.' }
$hadUserPath = $userEnvironment.GetValueNames() -contains 'Path'
$savedUserPath = $userEnvironment.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$savedUserPathKind = if ($hadUserPath) { $userEnvironment.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
$savedEnvironment = @{}
foreach ($key in @('Path', 'TEMP', 'TMP', 'V8_RUNNER_INSTALL_DIR')) {
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
    if ($Uri -eq $installerUrl) { return $installerText }
    if ($Uri -eq 'https://github.com/IngvarConsulting/v8-runner-rust/releases/latest/download/v8-runner-assets.json') {
        return $manifest
    }
    throw "Unexpected REST URL: $Uri"
}
function Invoke-WebRequest {
    param([string]$Uri, [string]$OutFile)
    Assert-True ($Uri -eq 'https://github.com/IngvarConsulting/v8-runner-rust/releases/download/v1.0.0/v8-runner-windows-x86_64.zip') 'archive URL'
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
    New-Item -ItemType Directory -Path $root, $bin, $scratch | Out-Null
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
} finally {
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
