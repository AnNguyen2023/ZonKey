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
$transcriptPath = Join-Path $env:TEMP ("zonkey-m3d40-pilot-" + [Guid]::NewGuid().ToString("N") + ".log")
Set-Content -LiteralPath $transcriptPath -Value "# sanitized ZonKey M3D-40 pilot transcript" -Encoding utf8

function Write-Pilot-Marker([string]$Stage, [string]$Details = "") {
    $suffix = if ([string]::IsNullOrEmpty($Details)) { "" } else { " $Details" }
    $line = "{0} stage={1}{2}" -f [DateTime]::UtcNow.ToString("o"), $Stage, $suffix
    Add-Content -LiteralPath $transcriptPath -Value $line -Encoding utf8
}

Write-Host "Sanitized pilot transcript: $transcriptPath" -ForegroundColor Cyan

function Stop-WithMessage([string]$Message, [string]$Stage = "OTHER_TYPED_FAILURE") {
    Write-Pilot-Marker "PILOT_SMOKE_FAIL:$Stage"
    Write-Pilot-Marker "exit_code=1 document_unchanged=unknown"
    Write-Host "PILOT_SMOKE_FAIL:$Stage" -ForegroundColor Red
    Write-Host "ZONKEY_BETA_SMOKE_FAIL: $Message" -ForegroundColor Red
    exit 1
}

try {
    $required = @($cliPath, $vsixPath, $operatorPath, $manifestPath, $checksumsPath)
    $missing = @($required | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
    if ($missing.Count -gt 0) {
        Stop-WithMessage "beta package missing; run the release packaging step first." "PACKAGE_OR_CHECKSUM"
    }

    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.git_worktree_clean -ne $true) {
        Stop-WithMessage "release manifest is not from a clean committed HEAD." "PACKAGE_OR_CHECKSUM"
    }
    if ($manifest.platform -ne "Windows 11 x64 / x86_64-pc-windows-msvc") {
        Stop-WithMessage "release manifest platform is not Windows 11 x64." "PACKAGE_OR_CHECKSUM"
    }

    $checksums = @{}
    foreach ($line in Get-Content -LiteralPath $checksumsPath) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ($line -notmatch '^(?<hash>[0-9A-Fa-f]{64})\s{2}(?<file>.+)$') {
            Stop-WithMessage "malformed SHA256SUMS.txt entry." "PACKAGE_OR_CHECKSUM"
        }
        $checksums[$Matches.file.Trim()] = $Matches.hash.ToLowerInvariant()
    }
    foreach ($name in @("zonkey-cli.exe", "zonkey-vscode-spike-0.0.1.vsix", "OPERATOR.md", "release-manifest.json")) {
        if (-not $checksums.ContainsKey($name)) {
            Stop-WithMessage "checksum missing for $name." "PACKAGE_OR_CHECKSUM"
        }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $packageDir $name)).Hash.ToLowerInvariant()
        if ($actual -ne $checksums[$name]) {
            Stop-WithMessage "checksum mismatch for $name." "PACKAGE_OR_CHECKSUM"
        }
    }
    Write-Pilot-Marker "PILOT_PACKAGE_OK"

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
        $oldTranscriptPath = $env:ZONKEY_PILOT_TRANSCRIPT
        $env:ZONKEY_PILOT_TRANSCRIPT = $transcriptPath
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
        if ($null -eq $oldTranscriptPath) { Remove-Item Env:ZONKEY_PILOT_TRANSCRIPT -ErrorAction SilentlyContinue }
        else { $env:ZONKEY_PILOT_TRANSCRIPT = $oldTranscriptPath }
    }

    if ($runnerExit -ne 0) {
        $failureMatch = Select-String -LiteralPath $transcriptPath -Pattern "stage=PILOT_SMOKE_FAIL:(?<stage>[A-Z_]+)" | Select-Object -First 1
        if ($null -eq $failureMatch) {
            Write-Pilot-Marker "PILOT_SMOKE_FAIL:OTHER_TYPED_FAILURE"
            Write-Pilot-Marker "exit_code=$runnerExit document_unchanged=unknown"
            $failureStage = "OTHER_TYPED_FAILURE"
        } else {
            $failureStage = $failureMatch.Matches[0].Groups["stage"].Value
        }
        Write-Host "PILOT_SMOKE_FAIL:$failureStage" -ForegroundColor Red
        Write-Host "ZONKEY_BETA_SMOKE_FAIL: smoke runner failed with exit code $runnerExit; see the sanitized transcript." -ForegroundColor Red
        exit 1
    }

    Write-Pilot-Marker "exit_code=0 document_unchanged=true"
    Write-Host ""
    Write-Host "PILOT_SMOKE_OK" -ForegroundColor Green
    Write-Host "ZONKEY_BETA_SMOKE_OK" -ForegroundColor Green
    Write-Host "Expected result: Rejected(CompositionUnknown); document unchanged; no Applied/mutation." -ForegroundColor Green
} catch {
    if ($_.Exception.Message -notlike "ZONKEY_BETA_SMOKE_FAIL:*") {
        Write-Pilot-Marker "PILOT_SMOKE_FAIL:OTHER_TYPED_FAILURE"
        Write-Pilot-Marker "exit_code=1 document_unchanged=unknown"
        Write-Host "PILOT_SMOKE_FAIL:OTHER_TYPED_FAILURE" -ForegroundColor Red
        Write-Host "ZONKEY_BETA_SMOKE_FAIL: $($_.Exception.Message)" -ForegroundColor Red
    }
    exit 1
}
