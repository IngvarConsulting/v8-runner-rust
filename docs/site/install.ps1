& {
    $ErrorActionPreference = 'Stop'
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    if ($env:OS -ne 'Windows_NT' -or $arch -ne 'X64') { throw 'Нужна Windows Intel/AMD 64.' }
    $base = 'https://github.com/IngvarConsulting/v8-runner-rust/releases'
    $manifest = Invoke-RestMethod "$base/latest/download/v8-runner-assets.json"
    $tag = $manifest.release.sourceTag
    if ($tag -notmatch '^v[A-Za-z0-9._-]+$') { throw 'Некорректный выпуск.' }
    if ($manifest.schemaVersion -ne 2) { throw 'Неизвестная схема манифеста.' }
    $asset = @($manifest.assets | Where-Object name -eq 'v8-runner-windows-x86_64.zip')
    if ($asset.Count -ne 1 -or $asset[0].sha256 -notmatch '^[a-f0-9]{64}$') { throw 'В манифесте нет SHA-256 архива.' }
    $temp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $archive = Join-Path $temp 'runner.zip'
        Invoke-WebRequest "$base/download/$tag/v8-runner-windows-x86_64.zip" -OutFile $archive
        if ((Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset[0].sha256) { throw 'SHA-256 не совпадает. Установка остановлена.' }
        Expand-Archive $archive -DestinationPath $temp
        $source = Join-Path $temp 'v8-runner-windows-x86_64\v8-runner.exe'
        if ((Get-FileHash $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset[0].binary.sha256) { throw 'SHA-256 бинарника не совпадает.' }
        & $source --version
        if ($LASTEXITCODE -ne 0) { throw 'Проверка запуска не прошла.' }
        $bin = Join-Path $env:LOCALAPPDATA 'Programs\v8-runner'
        if ($env:V8_RUNNER_INSTALL_DIR) { $bin = $env:V8_RUNNER_INSTALL_DIR }
        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        $target = Join-Path $bin 'v8-runner.exe'
        $stage = Join-Path $bin ([Guid]::NewGuid().ToString() + '.exe')
        try {
            Copy-Item $source $stage
            if (Test-Path $target) { [IO.File]::Replace($stage, $target, $null) }
            else { [IO.File]::Move($stage, $target) }
        } finally { if (Test-Path $stage) { Remove-Item $stage -Force } }
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (($userPath -split ';') -notcontains $bin) {
            [Environment]::SetEnvironmentVariable('Path', "$bin;$userPath", 'User')
        }
        $env:Path = "$bin;$env:Path"
        & (Join-Path $bin 'v8-runner.exe') --version
        if ($LASTEXITCODE -ne 0) { throw 'Проверка запуска не прошла.' }
        Write-Host "Установлено в $bin. Откройте новый терминал."
    } finally { Remove-Item $temp -Recurse -Force }
}
