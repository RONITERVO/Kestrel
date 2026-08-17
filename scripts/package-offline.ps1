[CmdletBinding()]
param(
    [string]$SigningCertificateThumbprint = "",
    [string]$TimestampUrl = "",
    [switch]$RequireSignature,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$tauriConfigPath = Join-Path $projectRoot "src-tauri\tauri.conf.json"
$tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
$version = [string]$tauriConfig.version
$extract_version = {
    param([string]$value)
    if (-not $value) { return $null }
    $match = [regex]::Match($value, "\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?")
    if ($match.Success) { return $match.Value } else { return $null }
}

function Get-KestrelArtifactVersion([string]$TargetPath, [string]$Role) {
    $item = Get-Item -LiteralPath $TargetPath
    $candidates = @($item.VersionInfo.ProductVersion, $item.VersionInfo.FileVersion) | ForEach-Object -Process {
        & $extract_version $_
    } | Where-Object { $_ }
    $versionFromName = & $extract_version $item.Name
    if ($candidates.Count -gt 0) {
        return $candidates[0]
    }
    if ($versionFromName) {
        return $versionFromName
    }
    throw "Could not determine the version for ${Role}: $TargetPath"
}

$validate_version = {
    param([string]$Artifact, [string]$Path, [string]$ExpectedVersion)
    $actualVersion = Get-KestrelArtifactVersion -TargetPath $Path -Role $Artifact
    if ($actualVersion -ne $ExpectedVersion) {
        throw "$Artifact version mismatch for $Path. Expected $ExpectedVersion but found $actualVersion."
    }
}

if ($tauriConfig.bundle.windows.webviewInstallMode.type -ne "offlineInstaller") {
    throw "Release packaging requires bundle.windows.webviewInstallMode.type=offlineInstaller."
}
if ($tauriConfig.bundle.windows.nsis.installMode -ne "currentUser") {
    throw "Release packaging requires a current-user NSIS installer."
}
if ($RequireSignature -and -not $SigningCertificateThumbprint) {
    throw "-RequireSignature also requires -SigningCertificateThumbprint."
}
if ($SigningCertificateThumbprint -and $SigningCertificateThumbprint -notmatch "^[A-Fa-f0-9]{40}$") {
    throw "The signing certificate thumbprint must contain exactly 40 hexadecimal characters."
}

$signingConfigPath = $null
if (-not $SkipBuild) {
    Push-Location $projectRoot
    try {
        $buildArguments = @("run", "tauri", "--", "build", "--bundles", "nsis")
        if ($SigningCertificateThumbprint) {
            $windows = [ordered]@{
                certificateThumbprint = $SigningCertificateThumbprint
                digestAlgorithm = "sha256"
            }
            if ($TimestampUrl) {
                $windows["timestampUrl"] = $TimestampUrl
                $windows["tsp"] = $true
            }
            $signingConfig = [ordered]@{ bundle = [ordered]@{ windows = $windows } }
            $signingConfigPath = Join-Path ([System.IO.Path]::GetTempPath()) ("kestrel-signing-" + [guid]::NewGuid().ToString("N") + ".json")
            $signingConfig | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $signingConfigPath -Encoding utf8
            $buildArguments += @("--config", $signingConfigPath)
        }
        & npm.cmd @buildArguments
        if ($LASTEXITCODE -ne 0) { throw "Tauri release build failed with exit code $LASTEXITCODE." }
    } finally {
        Pop-Location
        if ($signingConfigPath -and (Test-Path -LiteralPath $signingConfigPath)) {
            Remove-Item -LiteralPath $signingConfigPath -Force
        }
    }
}

$releaseBinary = Join-Path $projectRoot "src-tauri\target\release\kestrel-local.exe"
$bundleDirectory = Join-Path $projectRoot "src-tauri\target\release\bundle\nsis"
$installer = Get-ChildItem -LiteralPath $bundleDirectory -Filter "*.exe" -File |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if (-not (Test-Path -LiteralPath $releaseBinary -PathType Leaf) -or -not $installer) {
    throw "The release binary or NSIS installer was not produced."
}
& $validate_version "release binary" $releaseBinary $version
& $validate_version "NSIS installer" $installer.FullName $version

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$outputDirectory = Join-Path $projectRoot "release\$version-$stamp"
New-Item -ItemType Directory -Path $outputDirectory -ErrorAction Stop | Out-Null
$portablePath = Join-Path $outputDirectory "Kestrel-Local-$version-portable.exe"
$installerPath = Join-Path $outputDirectory "Kestrel-Local-$version-offline-setup.exe"
Copy-Item -LiteralPath $releaseBinary -Destination $portablePath
Copy-Item -LiteralPath $installer.FullName -Destination $installerPath

function Get-KestrelAuthenticodeStatus([string]$TargetPath) {
    $authenticodeShell = Get-Command pwsh.exe -ErrorAction SilentlyContinue
    if (-not $authenticodeShell) {
        $authenticodeShell = Get-Command powershell.exe -ErrorAction Stop
    }
    $status = & $authenticodeShell.Source -NoProfile -NonInteractive -Command '& { param([string]$Target) (Get-AuthenticodeSignature -LiteralPath $Target).Status.ToString() }' $TargetPath
    if ($LASTEXITCODE -ne 0 -or -not $status) {
        throw "Could not inspect the Authenticode status of $TargetPath."
    }
    return [string]$status
}

function Get-KestrelSha256([string]$TargetPath) {
    $stream = [System.IO.File]::OpenRead($TargetPath)
    try {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        try { return ([System.BitConverter]::ToString($sha.ComputeHash($stream))).Replace("-", "") }
        finally { $sha.Dispose() }
    } finally { $stream.Dispose() }
}

$artifacts = @($portablePath, $installerPath) | ForEach-Object {
    $item = Get-Item -LiteralPath $_
    [ordered]@{
        file = $item.Name
        bytes = $item.Length
        sha256 = Get-KestrelSha256 $_
        authenticode = Get-KestrelAuthenticodeStatus $_
    }
}
$invalidArtifacts = @($artifacts | Where-Object authenticode -ne "Valid")
if ($RequireSignature -and $invalidArtifacts.Count -gt 0) {
    throw "A valid Authenticode signature is required, but at least one copied release artifact is unsigned or invalid."
}
$manifest = [ordered]@{
    product = "Kestrel Local"
    version = $version
    createdAt = (Get-Date).ToUniversalTime().ToString("o")
    webView2 = "embedded offline installer"
    installMode = "current user"
    artifacts = $artifacts
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputDirectory "release-manifest.json") -Encoding utf8
$artifacts | ForEach-Object { "$($_.sha256)  $($_.file)" } | Set-Content -LiteralPath (Join-Path $outputDirectory "SHA256SUMS.txt") -Encoding ascii

Write-Host "Offline release created at $outputDirectory"
if ($invalidArtifacts.Count -gt 0) {
    Write-Warning "Artifacts are unsigned. Public releases should pass -SigningCertificateThumbprint and -RequireSignature."
}
