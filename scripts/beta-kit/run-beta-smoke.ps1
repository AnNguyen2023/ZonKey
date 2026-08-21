# ZonKey standalone INTERNAL beta tester kit (TB-002).
# Requires on the tester machine ONLY: Windows 11 x64 + PowerShell +
# physical keyboard. No Node.js, no npm, no Rust, no source tree, no
# pre-existing .vscode-test cache, no internet (VS Code 1.133.0 win32-x64
# is bundled as a zip and extracted locally on first run).
#
# One command:  .\run-beta-smoke.ps1
# Owner types physically:  dungf + Space, then resume + Space, then stops.
# Expected: Rejected(CompositionUnknown), document unchanged, no Applied
# or mutation of any kind, final marker ZONKEY_BETA_SMOKE_OK.
#
# Fail-closed typed stages: PACKAGE_OR_CHECKSUM, BOOTSTRAP_VSCODE_EXTRACT,
# BOOTSTRAP_VSIX_INSTALL, ENDPOINT_STARTUP, or the entry's own typed stage
# (e.g. LIVE_HANDOFF_TIMEOUT / DOCUMENT_CHANGED); anything unexpected is
# OTHER_TYPED_FAILURE.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$kitRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$cliPath = Join-Path $kitRoot "zonkey-cli.exe"
$vsixPath = Join-Path $kitRoot "zonkey-vscode-spike-0.0.1.vsix"
$entryPath = Join-Path $kitRoot "m3d37-physical-smoke.cjs"
$zipPath = Join-Path $kitRoot "vscode-1.133.0-win32-x64-archive.zip"
$operatorPath = Join-Path $kitRoot "OPERATOR.md"
$manifestPath = Join-Path $kitRoot "release-manifest.json"
$checksumsPath = Join-Path $kitRoot "SHA256SUMS.txt"
$vscodeDir = Join-Path $kitRoot "vscode"
$codeExe = Join-Path $vscodeDir "Code.exe"
$codeCli = Join-Path $vscodeDir "bin\code.cmd"

$transcriptPath = Join-Path $env:TEMP ("zonkey-tb002-pilot-" + [Guid]::NewGuid().ToString("N") + ".log")
Set-Content -LiteralPath $transcriptPath -Value "# sanitized ZonKey TB-002 pilot transcript" -Encoding utf8

function Write-Pilot-Marker([string]$Stage, [string]$Details = "") {
    $suffix = if ([string]::IsNullOrEmpty($Details)) { "" } else { " $Details" }
    $line = "{0} stage={1}{2}" -f [DateTime]::UtcNow.ToString("o"), $Stage, $suffix
    Add-Content -LiteralPath $transcriptPath -Value $line -Encoding utf8
}

function Stop-WithMessage([string]$Message, [string]$Stage = "OTHER_TYPED_FAILURE") {
    Write-Pilot-Marker "PILOT_SMOKE_FAIL:$Stage"
    Write-Pilot-Marker "exit_code=1 document_unchanged=unknown"
    Write-Host "PILOT_SMOKE_FAIL:$Stage" -ForegroundColor Red
    Write-Host "ZONKEY_BETA_SMOKE_FAIL: $Message" -ForegroundColor Red
    Write-Host "Sanitized transcript: $transcriptPath" -ForegroundColor Cyan
    exit 1
}

$endpointProcess = $null
$oldEndpointDir = $env:ZONKEY_ENDPOINT_DIR
$tempDirs = @()

try {
    Write-Host "Sanitized pilot transcript: $transcriptPath" -ForegroundColor Cyan

    # ---- 1. Package and checksum gate (fail closed on any drift) --------
    $required = @($cliPath, $vsixPath, $entryPath, $zipPath, $operatorPath, $manifestPath, $checksumsPath)
    $missing = @($required | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
    if ($missing.Count -gt 0) {
        Stop-WithMessage "kit artifact missing; the kit is incomplete." "PACKAGE_OR_CHECKSUM"
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.kit_kind -ne "standalone-beta-tester") {
        Stop-WithMessage "release manifest is not a standalone beta tester kit." "PACKAGE_OR_CHECKSUM"
    }
    if ($manifest.platform -ne "Windows 11 x64 / x86_64-pc-windows-msvc") {
        Stop-WithMessage "kit platform is not Windows 11 x64." "PACKAGE_OR_CHECKSUM"
    }
    if ($manifest.vscode_version -ne "1.133.0") {
        Stop-WithMessage "kit VS Code version is not the validated 1.133.0 pin." "PACKAGE_OR_CHECKSUM"
    }
    $checksums = @{}
    foreach ($line in Get-Content -LiteralPath $checksumsPath) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ($line -notmatch '^(?<hash>[0-9A-Fa-f]{64})\s{2}(?<file>.+)$') {
            Stop-WithMessage "malformed SHA256SUMS.txt entry." "PACKAGE_OR_CHECKSUM"
        }
        $checksums[$Matches.file.Trim()] = $Matches.hash.ToLowerInvariant()
    }
    foreach ($name in @("zonkey-cli.exe", "zonkey-vscode-spike-0.0.1.vsix", "m3d37-physical-smoke.cjs", "vscode-1.133.0-win32-x64-archive.zip", "OPERATOR.md", "release-manifest.json")) {
        if (-not $checksums.ContainsKey($name)) {
            Stop-WithMessage "checksum missing for $name." "PACKAGE_OR_CHECKSUM"
        }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $kitRoot $name)).Hash.ToLowerInvariant()
        if ($actual -ne $checksums[$name]) {
            Stop-WithMessage "checksum mismatch for $name." "PACKAGE_OR_CHECKSUM"
        }
    }
    Write-Pilot-Marker "PILOT_PACKAGE_OK"

    # ---- 2. Bundled VS Code runtime (offline; extracted once) -----------
    if (-not (Test-Path -LiteralPath $codeExe -PathType Leaf)) {
        if (Test-Path -LiteralPath $vscodeDir) {
            Remove-Item -LiteralPath $vscodeDir -Recurse -Force
        }
        try {
            Expand-Archive -LiteralPath $zipPath -DestinationPath $vscodeDir -Force
        } catch {
            Stop-WithMessage "bundled VS Code archive extraction failed." "BOOTSTRAP_VSCODE_EXTRACT"
        }
    }
    if (-not (Test-Path -LiteralPath $codeExe -PathType Leaf) -or -not (Test-Path -LiteralPath $codeCli -PathType Leaf)) {
        Stop-WithMessage "extracted VS Code runtime is incomplete (Code.exe or code.cmd missing)." "BOOTSTRAP_VSCODE_EXTRACT"
    }
    Write-Pilot-Marker "PILOT_VSCODE_RUNTIME_OK"

    # ---- 3. Isolated profile + packaged VSIX install ---------------------
    $profileDir = Join-Path $env:TEMP ("zonkey-tb002-userdata-" + [Guid]::NewGuid().ToString("N"))
    $extensionsDir = Join-Path $env:TEMP ("zonkey-tb002-exts-" + [Guid]::NewGuid().ToString("N"))
    $discoveryDir = Join-Path $env:TEMP ("zonkey-tb002-discovery-" + [Guid]::NewGuid().ToString("N"))
    $workspaceDir = Join-Path $env:TEMP ("zonkey-tb002-workspace-" + [Guid]::NewGuid().ToString("N"))
    $dummyDevDir = Join-Path $env:TEMP ("zonkey-tb002-devdummy-" + [Guid]::NewGuid().ToString("N"))
    foreach ($dir in @($profileDir, $extensionsDir, $discoveryDir, $workspaceDir, $dummyDevDir)) {
        New-Item -ItemType Directory -Path $dir | Out-Null
        $tempDirs += $dir
    }
    @{ name = "zonkey-tb002-test-runner"; publisher = "zonkey-spike"; version = "0.0.1"; engines = @{ vscode = "^1.90.0" } } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $dummyDevDir "package.json") -Encoding utf8

    try {
        & $codeCli "--user-data-dir=$profileDir" "--extensions-dir=$extensionsDir" --install-extension $vsixPath --force
        if ($LASTEXITCODE -ne 0) {
            Stop-WithMessage "packaged VSIX installation into the isolated profile failed." "BOOTSTRAP_VSIX_INSTALL"
        }
    } catch {
        Stop-WithMessage "packaged VSIX installation into the isolated profile failed." "BOOTSTRAP_VSIX_INSTALL"
    }
    Write-Pilot-Marker "PILOT_VSIX_INSTALLED"

    # ---- 4. Endpoint (handoff-live, per-lifecycle auto pipe) -------------
    $env:ZONKEY_ENDPOINT_DIR = $discoveryDir
    $endpointProcess = Start-Process -FilePath $cliPath -ArgumentList @("handoff-live", "--pipe", "auto") -PassThru -WindowStyle Hidden
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $discoveryFile = Join-Path $discoveryDir "endpoint.txt"
    $endpointReady = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 200
        if ($endpointProcess.HasExited) { break }
        if (Test-Path -LiteralPath $discoveryFile) {
            $fields = @{}
            foreach ($line in Get-Content -LiteralPath $discoveryFile) {
                $separator = $line.IndexOf("=")
                if ($separator -gt 0) { $fields[$line.Substring(0, $separator)] = $line.Substring($separator + 1) }
            }
            if ($fields["protocol"] -eq "zonkey.host-transport/1" -and $fields["pid"] -eq [string]$endpointProcess.Id -and $fields["pipe"].Length -gt 0) {
                $endpointReady = $true
                break
            }
        }
    }
    if (-not $endpointReady) {
        Stop-WithMessage "live endpoint discovery failed within 20s." "ENDPOINT_STARTUP"
    }
    Write-Pilot-Marker "PILOT_ENDPOINT_STARTED"

    # ---- 5. Physical smoke instructions -----------------------------------
    Write-Host "==============================================" -ForegroundColor Cyan
    Write-Host " ZONKEY STANDALONE BETA SMOKE - PHYSICAL KEYBOARD" -ForegroundColor Cyan
    Write-Host "==============================================" -ForegroundColor Cyan
    Write-Host "Kit: $($manifest.kit_version) from commit $($manifest.generated_from_git_commit)" -ForegroundColor Green
    Write-Host "VSIX runs only in an isolated temporary profile." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "STEP 1 - Type physically:  dungf  then  Space" -ForegroundColor Yellow
    Write-Host "        Wait for the no-current-handoff state." -ForegroundColor Yellow
    Write-Host "STEP 2 - Type physically:  resume  then  Space" -ForegroundColor Yellow
    Write-Host "        Stop typing immediately after Space." -ForegroundColor Yellow
    Write-Host "STEP 3 - The packaged command runs automatically:" -ForegroundColor Yellow
    Write-Host "        Zonkey spike: check current handoff" -ForegroundColor Yellow
    Write-Host "Do not use SendInput, paste, macros, or scripted input." -ForegroundColor Red
    Write-Host ""

    # ---- 6. Launch bundled VS Code with the prebuilt entry ----------------
    $waitSeconds = if ($null -ne $env:ZONKEY_M3D37_WAIT_SECONDS) { $env:ZONKEY_M3D37_WAIT_SECONDS } else { "300" }
    $env:ZONKEY_M3D37_WAIT_SECONDS = $waitSeconds
    $env:ZONKEY_PILOT_TRANSCRIPT = $transcriptPath
    Remove-Item Env:ELECTRON_RUN_AS_NODE -ErrorAction SilentlyContinue

    $launchArgs = @(
        "--extensionDevelopmentPath=`"$dummyDevDir`"",
        "--extensionTestsPath=`"$entryPath`"",
        "`"$workspaceDir`"",
        "--disable-workspace-trust",
        "--user-data-dir=`"$profileDir`"",
        "--extensions-dir=`"$extensionsDir`""
    )
    $code = Start-Process -FilePath $codeExe -ArgumentList ($launchArgs -join " ") -Wait -PassThru
    $runnerExit = $code.ExitCode

    if ($runnerExit -ne 0) {
        $failureMatch = Select-String -LiteralPath $transcriptPath -Pattern "stage=PILOT_SMOKE_FAIL:(?<stage>[A-Z_]+)" | Select-Object -First 1
        $failureStage = if ($null -eq $failureMatch) { "OTHER_TYPED_FAILURE" } else { $failureMatch.Matches[0].Groups["stage"].Value }
        Write-Pilot-Marker "PILOT_SMOKE_FAIL:$failureStage"
        Write-Pilot-Marker "exit_code=$runnerExit document_unchanged=unknown"
        Write-Host "PILOT_SMOKE_FAIL:$failureStage" -ForegroundColor Red
        Write-Host "ZONKEY_BETA_SMOKE_FAIL: smoke entry failed with exit code $runnerExit; see the sanitized transcript." -ForegroundColor Red
        exit 1
    }

    Write-Pilot-Marker "exit_code=0 document_unchanged=true"
    Write-Host ""
    Write-Host "PILOT_SMOKE_OK" -ForegroundColor Green
    Write-Host "ZONKEY_BETA_SMOKE_OK" -ForegroundColor Green
    Write-Host "Expected result: Rejected(CompositionUnknown); document unchanged; no Applied/mutation." -ForegroundColor Green
    exit 0
} catch {
    if ($_.Exception.Message -notlike "ZONKEY_BETA_SMOKE_FAIL:*") {
        Write-Pilot-Marker "PILOT_SMOKE_FAIL:OTHER_TYPED_FAILURE"
        Write-Pilot-Marker "exit_code=1 document_unchanged=unknown"
        Write-Host "PILOT_SMOKE_FAIL:OTHER_TYPED_FAILURE" -ForegroundColor Red
        Write-Host "ZONKEY_BETA_SMOKE_FAIL: $($_.Exception.Message)" -ForegroundColor Red
    }
    exit 1
} finally {
    if ($null -ne $endpointProcess -and -not $endpointProcess.HasExited) {
        & taskkill /PID $endpointProcess.Id /T /F | Out-Null
    }
    if ($null -eq $oldEndpointDir) { Remove-Item Env:ZONKEY_ENDPOINT_DIR -ErrorAction SilentlyContinue }
    else { $env:ZONKEY_ENDPOINT_DIR = $oldEndpointDir }
    Remove-Item Env:ZONKEY_M3D37_WAIT_SECONDS -ErrorAction SilentlyContinue
    Remove-Item Env:ZONKEY_PILOT_TRANSCRIPT -ErrorAction SilentlyContinue
    foreach ($dir in $tempDirs) {
        Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
    }
    Write-Pilot-Marker "PILOT_TRANSCRIPT_FLUSHED_BEFORE_CLEANUP"
}
