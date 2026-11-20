<#
.SYNOPSIS
    Builds the SnipTone release binary and packages native Windows .msi and .exe installers using WiX Toolset v5.

.EXAMPLE
    .\scripts\build-installers.ps1
    .\scripts\build-installers.ps1 -SkipCargoBuild
#>
[CmdletBinding()]
param(
    [switch]$SkipCargoBuild,
    [string]$OutputDir = "dist"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $RepoRoot

try {
    # 1. Read package version from Cargo.toml
    $cargoToml = Get-Content (Join-Path $RepoRoot "Cargo.toml") -Raw
    if ($cargoToml -match '(?m)^version\s*=\s*"([^"]+)"') {
        $AppVersion = $Matches[1]
    } else {
        throw "Could not parse package version from Cargo.toml"
    }

    Write-Host "=== SnipTone Installer Builder (v$AppVersion) ===" -ForegroundColor Cyan

    # 2. Verify WiX Toolset CLI (v5+)
    $wixCmd = Get-Command wix -ErrorAction SilentlyContinue
    if (-not $wixCmd) {
        Write-Host "WiX CLI not found. Installing via 'dotnet tool install --global wix'..." -ForegroundColor Yellow
        dotnet tool install --global wix
        $wixCmd = Get-Command wix -ErrorAction Stop
    }

    $wixVerRaw = (& wix --version).Trim()
    $wixVer = ($wixVerRaw -split '\+')[0]
    Write-Host "Using WiX Toolset $wixVer ($($wixCmd.Source))" -ForegroundColor DarkGray

    # Ensure required WiX UI & BootstrapperApplications extensions are installed for the active WiX version
    $installedExts = (& wix extension list -g) -join "`n"
    foreach ($ext in @("WixToolset.UI.wixext", "WixToolset.BootstrapperApplications.wixext")) {
        $hasValid = ($installedExts -split "`r?`n") | Where-Object {
            $_ -match ("^" + [regex]::Escape($ext) + "\s+") -and $_ -notmatch "\(damaged\)"
        }
        if (-not $hasValid) {
            Write-Host "Installing WiX extension $ext/$wixVer..." -ForegroundColor Yellow
            & wix extension add -g "$ext/$wixVer"
        }
    }

    # 3. Build optimized Windows release binary (embeds assets/sniptone.ico via build.rs)
    if (-not $SkipCargoBuild) {
        $running = Get-Process -Name "sniptone" -ErrorAction SilentlyContinue
        if ($running) {
            Write-Host "Stopping running sniptone.exe instance(s) to unlock output binary..." -ForegroundColor Yellow
            $running | Stop-Process -Force
            Start-Sleep -Milliseconds 300
        }

        Write-Host "`n[1/3] Building release binary (cargo build --release)..." -ForegroundColor Cyan
        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release failed with exit code $LASTEXITCODE"
        }
    }

    $ExePath     = Join-Path $RepoRoot "target\release\sniptone.exe"
    $IconPath    = Join-Path $RepoRoot "assets\sniptone.ico"
    $LogoPath    = Join-Path $RepoRoot "assets\sniptone.png"
    $LicensePath = Join-Path $RepoRoot "installer\License.rtf"
    $PackageWxs  = Join-Path $RepoRoot "installer\Package.wxs"
    $BundleWxs   = Join-Path $RepoRoot "installer\Bundle.wxs"

    foreach ($req in @($ExePath, $IconPath, $LogoPath, $LicensePath, $PackageWxs, $BundleWxs)) {
        if (-not (Test-Path $req)) {
            throw "Required build input missing: $req"
        }
    }

    $DistPath = Join-Path $RepoRoot $OutputDir
    New-Item -ItemType Directory -Force -Path $DistPath | Out-Null

    $MsiOut = Join-Path $DistPath "SnipTone-$AppVersion-x64.msi"
    $ExeOut = Join-Path $DistPath "SnipTone-$AppVersion-x64-Setup.exe"

    # 4. Build .MSI installer
    Write-Host "`n[2/3] Building MSI installer -> $MsiOut" -ForegroundColor Cyan
    & wix build $PackageWxs `
        -arch x64 `
        -ext WixToolset.UI.wixext `
        -d "AppVersion=$AppVersion" `
        -d "ExePath=$ExePath" `
        -d "IconPath=$IconPath" `
        -d "LicensePath=$LicensePath" `
        -o $MsiOut
    if ($LASTEXITCODE -ne 0) {
        throw "WiX MSI build failed with exit code $LASTEXITCODE"
    }

    # 5. Build standalone .EXE Burn bootstrapper wrapping the .MSI
    Write-Host "`n[3/3] Building EXE Setup installer -> $ExeOut" -ForegroundColor Cyan
    & wix build $BundleWxs `
        -arch x64 `
        -ext WixToolset.BootstrapperApplications.wixext `
        -d "AppVersion=$AppVersion" `
        -d "MsiPath=$MsiOut" `
        -d "IconPath=$IconPath" `
        -d "LogoPath=$LogoPath" `
        -o $ExeOut
    if ($LASTEXITCODE -ne 0) {
        throw "WiX EXE bundle build failed with exit code $LASTEXITCODE"
    }

    # 6. Print final artifacts & SHA-256 checksums
    Write-Host "`n=== Build Complete ===" -ForegroundColor Green
    foreach ($artifact in @($MsiOut, $ExeOut)) {
        $item = Get-Item $artifact
        $hash = (Get-FileHash $artifact -Algorithm SHA256).Hash
        $mb   = [math]::Round($item.Length / 1MB, 2)
        Write-Host ("  {0,-36} {1,6} MB   SHA256: {2}" -f $item.Name, $mb, $hash)
    }
}
finally {
    Pop-Location
}
