[CmdletBinding()]
param(
    [string]$SigningCertificateThumbprint = "",
    [string]$TimestampUrl = "",
    [switch]$RequireSignature
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$tauriConfigPath = Join-Path $projectRoot "src-tauri\tauri.conf.json"
$tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
$version = [string]$tauriConfig.version

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

$releaseBinary = Join-Path $projectRoot "src-tauri\target\release\kestrel-local.exe"
$bundleDirectory = Join-Path $projectRoot "src-tauri\target\release\bundle\nsis"
$installer = Get-ChildItem -LiteralPath $bundleDirectory -Filter "*.exe" -File |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if (-not (Test-Path -LiteralPath $releaseBinary -PathType Leaf) -or -not $installer) {
    throw "The release binary or NSIS installer was not produced."
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$outputDirectory = Join-Path $projectRoot "release\$version-$stamp"
New-Item -ItemType Directory -Path $outputDirectory -ErrorAction Stop | Out-Null
$portablePath = Join-Path $outputDirectory "Kestrel-Local-$version-portable.exe"
$installerPath = Join-Path $outputDirectory "Kestrel-Local-$version-offline-setup.exe"
Copy-Item -LiteralPath $releaseBinary -Destination $portablePath
Copy-Item -LiteralPath $installer.FullName -Destination $installerPath

$artifacts = @($portablePath, $installerPath) | ForEach-Object {
    $item = Get-Item -LiteralPath $_
    $signature = Get-AuthenticodeSignature -LiteralPath $_
    [ordered]@{
        file = $item.Name
        bytes = $item.Length
        sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash
        authenticode = [string]$signature.Status
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
