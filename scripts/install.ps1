# install.ps1 — First-time installer, Start Menu shortcut creator & updater
# for RepoHarbor on Windows (x86_64).
#
# Usage (PowerShell):
#   irm https://raw.githubusercontent.com/azmykn/RepoHarbor/main/scripts/install.ps1 | iex
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1 [-FromSource] [-Autostart] [-Uninstall] [-Purge]

param(
    [switch]$FromSource,
    [switch]$Autostart,
    [switch]$Uninstall,
    [switch]$Purge
)

$ErrorActionPreference = "Stop"
$RepoSlug = "azmykn/RepoHarbor"
$BinDir = Join-Path $env:LOCALAPPDATA "Programs\RepoHarbor"
$BinDst = Join-Path $BinDir "repoharbor.exe"
$DataDir = Join-Path $env:APPDATA "repoharbor"
$StartMenuLnk = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\RepoHarbor.lnk"
$StartupCmd = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Startup\RepoHarbor.cmd"

function Write-Step($msg) { Write-Host "`n> $msg" -ForegroundColor Cyan }
function Write-Ok($msg)   { Write-Host "  [OK] $msg" -ForegroundColor Green }
function Write-Warn($msg) { Write-Host "  [!] $msg" -ForegroundColor Yellow }

if ($Uninstall) {
    Write-Step "Uninstalling RepoHarbor..."
    Remove-Item -Path $StartMenuLnk -Force -ErrorAction SilentlyContinue
    Remove-Item -Path $StartupCmd -Force -ErrorAction SilentlyContinue
    Remove-Item -Path $BinDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -Path (Join-Path $DataDir "install-meta.toml") -Force -ErrorAction SilentlyContinue
    if ($Purge) {
        Remove-Item -Path $DataDir -Recurse -Force -ErrorAction SilentlyContinue
        Write-Ok "Removed user config and cache ($DataDir)"
    }
    Write-Ok "RepoHarbor uninstalled."
    exit 0
}

New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

function Write-InstallMeta($kind, $srcDir, $ver) {
    $now = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    $lines = @("kind = `"$kind`"")
    if ($srcDir) {
        $escapedSrc = $srcDir.Replace("\", "\\")
        $lines += "sourceDir = `"$escapedSrc`""
    }
    $lines += "version = `"$ver`""
    $lines += "installedAt = `"$now`""
    Set-Content -Path (Join-Path $DataDir "install-meta.toml") -Value ($lines -join "`n") -Encoding UTF8
}

function Add-ToUserPath($dir) {
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not $currentPath) { $currentPath = "" }
    $parts = $currentPath -split ";"
    if ($parts -notcontains $dir) {
        $newPath = if ($currentPath) { "$currentPath;$dir" } else { $dir }
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        $env:Path = "$env:Path;$dir"
        Write-Ok "Added $dir to User PATH"
    }
}

function Install-FromRelease {
    Write-Step "Checking latest GitHub Release ($RepoSlug)..."
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$RepoSlug/releases/latest" -Headers @{ Accept = "application/vnd.github+json" }
    } catch {
        return $false
    }
    $asset = $release.assets | Where-Object { $_.name -match "windows-x86_64\.tar\.gz$" } | Select-Object -First 1
    if (-not $asset) {
        return $false
    }
    $sumsAsset = $release.assets | Where-Object { $_.name -eq "SHA256SUMS.txt" } | Select-Object -First 1

    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("repoharbor-install-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    try {
        $archivePath = Join-Path $tmp $asset.name
        Write-Step "Downloading $($asset.name) ($($release.tag_name))..."
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archivePath

        if ($sumsAsset) {
            $sumsPath = Join-Path $tmp "SHA256SUMS.txt"
            Invoke-WebRequest -Uri $sumsAsset.browser_download_url -OutFile $sumsPath
            $expectedLine = Get-Content $sumsPath | Where-Object { $_ -match [regex]::Escape($asset.name) } | Select-Object -First 1
            if ($expectedLine) {
                $expectedHash = ($expectedLine -split "\s+")[0].ToLower()
                $actualHash = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLower()
                if ($expectedHash -ne $actualHash) {
                    throw "SHA-256 checksum mismatch for $($asset.name) (expected $expectedHash, got $actualHash)"
                }
                Write-Ok "SHA-256 checksum verified"
            }
        }

        tar -xzf $archivePath -C $tmp
        $unpackedExe = Get-ChildItem -Path $tmp -Recurse -Filter "repoharbor.exe" | Select-Object -First 1
        if (-not $unpackedExe) {
            throw "Release archive did not contain repoharbor.exe"
        }

        $oldBackup = "$BinDst.old"
        Remove-Item -Path $oldBackup -Force -ErrorAction SilentlyContinue
        if (Test-Path $BinDst) {
            try {
                Copy-Item -Path $unpackedExe.FullName -Destination $BinDst -Force
            } catch {
                Move-Item -Path $BinDst -Destination $oldBackup -Force
                Copy-Item -Path $unpackedExe.FullName -Destination $BinDst -Force
                Remove-Item -Path $oldBackup -Force -ErrorAction SilentlyContinue
            }
        } else {
            Copy-Item -Path $unpackedExe.FullName -Destination $BinDst -Force
        }
        Write-Ok "Installed $BinDst"

        $ver = $release.tag_name.TrimStart("v")
        Write-InstallMeta "binary" "" $ver
        Add-ToUserPath $BinDir

        if ($Autostart) {
            & $BinDst install-desktop --autostart
        } else {
            & $BinDst install-desktop
        }
        return $true
    } finally {
        Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Install-FromSource {
    $workRepo = if ($PSScriptRoot -and (Test-Path (Join-Path $PSScriptRoot "..\Cargo.toml"))) {
        (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    } else {
        $srcClone = Join-Path $DataDir "src"
        if (Test-Path (Join-Path $srcClone ".git")) {
            Write-Step "Updating source checkout ($srcClone)..."
            git -C $srcClone pull --ff-only
        } else {
            Write-Step "Cloning $RepoSlug into $srcClone..."
            git clone "https://github.com/$RepoSlug.git" $srcClone
        }
        $srcClone
    }

    Push-Location $workRepo
    try {
        Write-Step "Building release binary (cargo build --release -p repoharbor)..."
        cargo build --release -p repoharbor
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

        $builtExe = Join-Path $workRepo "target\release\repoharbor.exe"
        Copy-Item -Path $builtExe -Destination $BinDst -Force
        Write-Ok "Installed $BinDst"

        $cargoToml = Get-Content (Join-Path $workRepo "crates\repoharbor\Cargo.toml")
        $verLine = $cargoToml | Where-Object { $_ -match '^version\s*=\s*"([^"]+)"' } | Select-Object -First 1
        $ver = if ($verLine -match '"([^"]+)"') { $Matches[1] } else { "0.0.0" }
        Write-InstallMeta "source" $workRepo $ver
        Add-ToUserPath $BinDir

        if ($Autostart) {
            & $BinDst install-desktop --autostart
        } else {
            & $BinDst install-desktop
        }
    } finally {
        Pop-Location
    }
}

if ($FromSource) {
    Install-FromSource
} else {
    if (-not (Install-FromRelease)) {
        Write-Warn "No Windows release archive found on GitHub Releases yet — falling back to source build..."
        Install-FromSource
    }
}

Write-Step "Done!"
Write-Ok "Launch RepoHarbor from the Start Menu or run: repoharbor"
Write-Ok "Update anytime with: repoharbor update (or click Update inside the app)"

$running = Get-Process repoharbor -ErrorAction SilentlyContinue
if ($running) {
    Write-Ok "Restarting running RepoHarbor application..."
    $pids = ($running | Select-Object -ExpandProperty Id) -join ","
    Start-Process powershell -WindowStyle Hidden -ArgumentList "-NoProfile", "-Command", "Start-Sleep -Milliseconds 300; Stop-Process -Id $pids -Force -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 300; Start-Process '$BinDst'"
}
