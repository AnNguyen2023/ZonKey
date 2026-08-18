Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$packageDir = Join-Path $repoRoot "release\zonkey-query-reject-beta-0.0.1"
$cliPath = Join-Path $packageDir "zonkey-cli.exe"
$vsixPath = Join-Path $packageDir "zonkey-vscode-spike-0.0.1.vsix"
$operatorPath = Join-Path $packageDir "OPERATOR.md"
$manifestPath = Join-Path $packageDir "release-manifest.json"
$checksumsPath = Join-Path $packageDir "SHA256SUMS.txt"
$smokeRunner = Join-Path $repoRoot "hosts\vscode-spike\scripts\run-m3d37-smoke.mjs"
$extensionRoot = Join-Path $repoRoot "hosts\vscode-spike"

function Stop-WithMessage([string]$Message) {
    Write-Host "ZONKEY_BETA_SMOKE_FAIL: $Message" -ForegroundColor Red
    exit 1
}

try {
    $required = @($cliPath, $vsixPath, $operatorPath, $manifestPath, $checksumsPath)
    $missing = @($required | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
    if ($missing.Count -gt 0) {
        Stop-WithMessage "beta package missing; run the release packaging step first."
    }

    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.git_worktree_clean -ne $true) {
        Stop-WithMessage "release manifest is not from a clean committed HEAD."
    }
    if ($manifest.platform -ne "Windows 11 x64 / x86_64-pc-windows-msvc") {
        Stop-WithMessage "release manifest platform is not Windows 11 x64."
    }

    $checksums = @{}
    foreach ($line in Get-Content -LiteralPath $checksumsPath) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ($line -notmatch '^(?<hash>[0-9A-Fa-f]{64})\s{2}(?<file>.+)$') {
            Stop-WithMessage "malformed SHA256SUMS.txt entry."
        }
        $checksums[$Matches.file.Trim()] = $Matches.hash.ToLowerInvariant()
    }
    foreach ($name in @("zonkey-cli.exe", "zonkey-vscode-spike-0.0.1.vsix", "OPERATOR.md", "release-manifest.json")) {
        if (-not $checksums.ContainsKey($name)) {
            Stop-WithMessage "checksum missing for $name."
        }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $packageDir $name)).Hash.ToLowerInvariant()
        if ($actual -ne $checksums[$name]) {
            Stop-WithMessage "checksum mismatch for $name."
        }
    }

    if (-not (Get-Command node -ErrorAction SilentlyContinue) -or -not (Get-Command npm -ErrorAction SilentlyContinue)) {
        Stop-WithMessage "Node.js and npm are required for the approved isolated VS Code smoke harness."
    }
    if (-not (Test-Path -LiteralPath $smokeRunner -PathType Leaf)) {
        Stop-WithMessage "approved M3D-37 smoke runner is missing."
    }
    Write-Host "Preparing the approved smoke entry..." -ForegroundColor DarkGray
    Push-Location -LiteralPath $extensionRoot
    try {
        & npm run compile:m3d37
        if ($LASTEXITCODE -ne 0) {
            Stop-WithMessage "could not prepare the packaged smoke harness."
        }
    } finally {
        Pop-Location
    }

    Write-Host "==============================================" -ForegroundColor Cyan
    Write-Host " ZONKEY INTERNAL BETA SMOKE - PHYSICAL KEYBOARD" -ForegroundColor Cyan
    Write-Host "==============================================" -ForegroundColor Cyan
    Write-Host "Verified package: $($manifest.git_commit)" -ForegroundColor Green
    Write-Host "VSIX is installed only in an isolated temporary test profile." -ForegroundColor Yellow
    Write-Host "Manual one-time install (only if using VS Code outside this runner):" -ForegroundColor DarkGray
    Write-Host "  code --install-extension `"$vsixPath`" --force" -ForegroundColor DarkGray
    Write-Host ""
    Write-Host "STEP 1 - Type physically:  dungf  then  Space" -ForegroundColor Yellow
    Write-Host "        Wait for the no-current-handoff state." -ForegroundColor Yellow
    Write-Host "STEP 2 - Type physically:  resume  then  Space" -ForegroundColor Yellow
    Write-Host "        Stop typing immediately after Space." -ForegroundColor Yellow
    Write-Host "STEP 3 - The packaged command runs automatically:" -ForegroundColor Yellow
    Write-Host "        Zonkey spike: check current handoff" -ForegroundColor Yellow
    Write-Host "Do not use SendInput, paste, macros, or scripted input." -ForegroundColor Red
    Write-Host ""

    $oldCliPath = $env:ZONKEY_M3D37_CLI_PATH
    $oldVsixPath = $env:ZONKEY_M3D37_VSIX_PATH
    try {
        $env:ZONKEY_M3D37_CLI_PATH = $cliPath
        $env:ZONKEY_M3D37_VSIX_PATH = $vsixPath
        Push-Location -LiteralPath $extensionRoot
        try {
            & node $smokeRunner
            $runnerExit = $LASTEXITCODE
        } finally {
            Pop-Location
        }
    } finally {
        if ($null -eq $oldCliPath) { Remove-Item Env:ZONKEY_M3D37_CLI_PATH -ErrorAction SilentlyContinue }
        else { $env:ZONKEY_M3D37_CLI_PATH = $oldCliPath }
        if ($null -eq $oldVsixPath) { Remove-Item Env:ZONKEY_M3D37_VSIX_PATH -ErrorAction SilentlyContinue }
        else { $env:ZONKEY_M3D37_VSIX_PATH = $oldVsixPath }
    }

    if ($runnerExit -ne 0) {
        Stop-WithMessage "smoke runner failed with exit code $runnerExit; see the typed failure above."
    }

    Write-Host ""
    Write-Host "ZONKEY_BETA_SMOKE_OK" -ForegroundColor Green
    Write-Host "Expected result: Rejected(CompositionUnknown); document unchanged; no Applied/mutation." -ForegroundColor Green
} catch {
    if ($_.Exception.Message -notlike "ZONKEY_BETA_SMOKE_FAIL:*") {
        Write-Host "ZONKEY_BETA_SMOKE_FAIL: $($_.Exception.Message)" -ForegroundColor Red
    }
    exit 1
}
