& {
    $ErrorActionPreference = 'Stop'
    # Qualify the framework assembly: PSReadLine can expose a namesake without OSArchitecture.
    $arch = [System.Runtime.InteropServices.RuntimeInformation, mscorlib]::OSArchitecture
    if ($env:OS -ne 'Windows_NT' -or $arch -ne 'X64') { throw "Windows x64 (Intel/AMD) is required. Detected OS: $env:OS; architecture: $arch." }
    $base = 'https://github.com/IngvarConsulting/v8-runner-rust/releases'
    $manifest = Invoke-RestMethod "$base/latest/download/v8-runner-assets.json"
    $tag = $manifest.release.sourceTag
    if ($tag -notmatch '^v[A-Za-z0-9._-]+$') { throw 'Invalid release tag.' }
    if ($manifest.schemaVersion -ne 2) { throw 'Unsupported manifest schema.' }
    $asset = @($manifest.assets | Where-Object name -eq 'v8-runner-windows-x86_64.zip')
    if ($asset.Count -ne 1 -or $asset[0].sha256 -notmatch '^[a-f0-9]{64}$') { throw 'The manifest must contain exactly one archive with a valid SHA-256.' }
    $temp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $archive = Join-Path $temp 'runner.zip'
        Invoke-WebRequest "$base/download/$tag/v8-runner-windows-x86_64.zip" -OutFile $archive
        if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset[0].sha256) { throw 'Archive SHA-256 does not match. Installation stopped.' }
        Expand-Archive $archive -DestinationPath $temp
        $source = Join-Path $temp 'v8-runner-windows-x86_64\v8-runner.exe'
        if ((Get-FileHash $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset[0].binary.sha256) { throw 'Binary SHA-256 does not match.' }
        & $source --version
        if ($LASTEXITCODE -ne 0) { throw 'The executable version check failed.' }
        $bin = Join-Path $env:LOCALAPPDATA 'Programs\v8-runner'
        if ($env:V8_RUNNER_INSTALL_DIR) { $bin = $env:V8_RUNNER_INSTALL_DIR }
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        $target = Join-Path $bin 'v8-runner.exe'
        $stage = Join-Path $bin ([Guid]::NewGuid().ToString() + '.exe')
        try {
            Copy-Item $source $stage
            # PowerShell coerces $null to an empty string for a .NET string parameter.
            if (Test-Path $target) { [IO.File]::Replace($stage, $target, [System.Management.Automation.Language.NullString]::Value) }
            else { [IO.File]::Move($stage, $target) }
        } finally { if (Test-Path $stage) { Remove-Item $stage -Force } }
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (($userPath -split ';') -notcontains $bin) {
            [Environment]::SetEnvironmentVariable('Path', "$bin;$userPath", 'User')
        }
        $env:Path = "$bin;$env:Path"
        & (Join-Path $bin 'v8-runner.exe') --version
        if ($LASTEXITCODE -ne 0) { throw 'The executable version check failed.' }
        Write-Host "Installed in $bin. Open a new terminal."
    } finally { Remove-Item $temp -Recurse -Force }
}
