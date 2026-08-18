param(
  [string]$ShortcutPath = "",
  [switch]$SkipBuild,
  [switch]$NoShortcut
)

$ErrorActionPreference = "Stop"

function Invoke-Step {
  param(
    [string]$Name,
    [string[]]$Arguments
  )

  Write-Host "Running: npm $($Arguments -join ' ')"
  & npm.cmd @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "$Name failed with exit code $LASTEXITCODE."
  }
}

function Resolve-KestrelShortcutPath {
  param([string]$InputPath)

  if ($InputPath) {
    return $InputPath
  }

  $oneDrivePath = $env:OneDrive
  if ($oneDrivePath) {
    foreach ($subFolder in @("Työpöytä", "Desktop")) {
      $candidate = Join-Path $oneDrivePath "$subFolder\Kestrel Local.lnk"
      if (Test-Path -LiteralPath (Split-Path -LiteralPath $candidate)) {
        return $candidate
      }
    }
  }

  return Join-Path ([Environment]::GetFolderPath("Desktop")) "Kestrel Local.lnk"
}

function Write-KestrelShortcut {
  param(
    [string]$TargetPath,
    [string]$ProjectRoot,
    [string]$ScriptPath
  )

  $shortcutParent = Split-Path -Parent $TargetPath
  if (-not (Test-Path -LiteralPath $shortcutParent)) {
    New-Item -ItemType Directory -Path $shortcutParent | Out-Null
  }

  $powershellPath = (Get-Command powershell.exe -ErrorAction Stop).Source
  $iconPath = Join-Path $ProjectRoot "src-tauri\icons\icon.ico"

  $shortcut = New-Object -ComObject WScript.Shell
  $link = $shortcut.CreateShortcut($TargetPath)
  $link.TargetPath = $powershellPath
  $link.Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$ScriptPath`""
  $link.WorkingDirectory = $ProjectRoot
  if (Test-Path -LiteralPath $iconPath) {
    $link.IconLocation = $iconPath
  }
  $link.Description = "Kestrel Local launcher"
  $link.Save()

  Write-Host "Updated shortcut: $TargetPath"
}

$projectRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
$resolvedShortcutPath = Resolve-KestrelShortcutPath -InputPath $ShortcutPath

Write-Host "Kestrel launcher root: $projectRoot"
Write-Host "Shortcut target: $resolvedShortcutPath"
if (-not $NoShortcut) {
  Write-KestrelShortcut -TargetPath $resolvedShortcutPath -ProjectRoot $projectRoot -ScriptPath $PSCommandPath
}

if (-not (Test-Path -LiteralPath (Join-Path $projectRoot "node_modules"))) {
  Invoke-Step "npm install" @("install")
}

if (-not $SkipBuild) {
  $running = Get-Process -Name "kestrel-local" -ErrorAction SilentlyContinue
  if ($running) {
    Write-Host "Stopping running Kestrel instance(s)..."
    & taskkill.exe /F /IM "kestrel-local.exe" /T *>$null
    Start-Sleep -Seconds 1
  }
  Invoke-Step "npm run build" @("run", "build")
  Invoke-Step "npm run tauri build" @("run", "tauri", "--", "build", "--no-bundle")
}

$binaryPath = Join-Path $projectRoot "src-tauri\target\release\kestrel-local.exe"
if (-not (Test-Path -LiteralPath $binaryPath)) {
  throw "Release executable not found: $binaryPath"
}

Start-Process -FilePath $binaryPath
