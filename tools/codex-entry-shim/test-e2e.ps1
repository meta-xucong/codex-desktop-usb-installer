[CmdletBinding()]
param(
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$shimManifest = Join-Path $PSScriptRoot "Cargo.toml"
$managerManifest = Join-Path $repo "src-tauri\Cargo.toml"
$verifier = Join-Path $repo "tools\verify-usb-launcher.ps1"

function Invoke-CapturedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$OutputBase
    )
    $stdoutPath = "$OutputBase.stdout"
    $stderrPath = "$OutputBase.stderr"
    $argumentLine = ($Arguments | ForEach-Object {
        '"' + ([string]$_).Replace('"', '\"') + '"'
    }) -join ' '
    $process = Start-Process -FilePath $FilePath -ArgumentList $argumentLine `
        -WorkingDirectory $WorkingDirectory -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $stdout = if (Test-Path -LiteralPath $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw } else { "" }
    $stderr = if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw } else { "" }
    [pscustomobject]@{
        ExitCode = $process.ExitCode
        Output = ($stdout + $stderr)
    }
}
$sdkRoots = @(
    (Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"),
    (Join-Path $env:ProgramFiles "Windows Kits\10\bin")
) | Where-Object { Test-Path -LiteralPath $_ }
$mt = $sdkRoots |
    ForEach-Object { Get-ChildItem -LiteralPath $_ -Recurse -Filter mt.exe -File -ErrorAction SilentlyContinue } |
    Where-Object { $_.FullName -match "\\x64\\mt\.exe$" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if ($null -eq $mt) { throw "Windows SDK mt.exe not found" }

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("codex-usb-entry-e2e-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $temp | Out-Null
$managerTests = @(
    "codex_entry_ipc::tests::compiled_shim_named_pipe_active_revoked_and_offline_e2e",
    "codex_entry_ipc::tests::compiled_shim_uac_cancel_path_fails_closed_without_core"
)
$testListPath = Join-Path $temp "manager-entry-tests.txt"
try {
    $testList = Invoke-CapturedProcess -FilePath "cargo.exe" -WorkingDirectory $repo -OutputBase $testListPath -Arguments @(
        "test", "--manifest-path", $managerManifest, "--lib", "codex_entry_ipc::tests", "-j", "1", "--", "--list"
    )
    if ($testList.ExitCode -ne 0) {
        throw "Could not enumerate manager entry integration tests: $($testList.Output)"
    }
    foreach ($testName in $managerTests) {
        if ($testList.Output -notmatch [regex]::Escape("${testName}: test")) {
            throw "Required USB direct-entry integration test is not compiled into the manager: $testName"
        }
    }
}
catch {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
    throw
}
$envNames = @(
    "CODEX_USB_ENTRY_SHIM_PATH",
    "CODEX_USB_ENTRY_CANCEL_SHIM_PATH",
    "CODEX_USB_ENTRY_TEST_QUIET"
)
$saved = @{}
foreach ($name in $envNames) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, "Process") }

try {
    $releaseBuild = Join-Path $repo "tools\codex-entry-shim\target\$Target\release\codex-entry-shim.exe"
    $debugBuild = Join-Path $repo "tools\codex-entry-shim\target\$Target\debug\codex-entry-shim.exe"
    $releaseBuildResult = Invoke-CapturedProcess -FilePath "cargo.exe" -WorkingDirectory $repo -OutputBase (Join-Path $temp "shim-release-build") -Arguments @(
        "build", "--release", "--manifest-path", $shimManifest, "--target", $Target
    )
    if ($releaseBuildResult.ExitCode -ne 0) { throw "release shim build failed: $($releaseBuildResult.Output)" }
    $debugBuildResult = Invoke-CapturedProcess -FilePath "cargo.exe" -WorkingDirectory $repo -OutputBase (Join-Path $temp "shim-debug-build") -Arguments @(
        "build", "--manifest-path", $shimManifest, "--target", $Target
    )
    if ($debugBuildResult.ExitCode -ne 0) { throw "debug shim build failed: $($debugBuildResult.Output)" }

    $releaseFixture = Join-Path $temp "CodexEntryShim.exe"
    $debugFixture = Join-Path $temp "CodexEntryShim-debug.exe"
    Copy-Item -LiteralPath $releaseBuild -Destination $releaseFixture
    Copy-Item -LiteralPath $debugBuild -Destination $debugFixture
    $manifestResult = Invoke-CapturedProcess -FilePath $mt.FullName -WorkingDirectory $repo -OutputBase (Join-Path $temp "shim-manifest") -Arguments @(
        "-manifest", (Join-Path $PSScriptRoot "as-invoker.manifest"), ("-outputresource:{0};#1" -f $releaseFixture)
    )
    if ($manifestResult.ExitCode -ne 0) { throw "test shim manifest embed failed: $($manifestResult.Output)" }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $verifier -Path $releaseFixture -ExpectedLevel asInvoker
    if ($LASTEXITCODE -ne 0) { throw "test shim asInvoker manifest verification failed: $LASTEXITCODE" }

    $env:CODEX_USB_ENTRY_SHIM_PATH = $releaseFixture
    $env:CODEX_USB_ENTRY_CANCEL_SHIM_PATH = $debugFixture
    $env:CODEX_USB_ENTRY_TEST_QUIET = "1"
    $ipcResult = Invoke-CapturedProcess -FilePath "cargo.exe" -WorkingDirectory $repo -OutputBase (Join-Path $temp "shim-ipc-test") -Arguments @(
        "test", "--manifest-path", $managerManifest, "--lib", "codex_entry_ipc::tests::compiled_shim_named_pipe_active_revoked_and_offline_e2e", "-j", "1", "--", "--ignored", "--nocapture"
    )
    if ($ipcResult.Output) { Write-Output $ipcResult.Output }
    if ($ipcResult.ExitCode -ne 0) { throw "compiled shim named-pipe IPC E2E failed: $($ipcResult.ExitCode)" }
    $uacResult = Invoke-CapturedProcess -FilePath "cargo.exe" -WorkingDirectory $repo -OutputBase (Join-Path $temp "shim-uac-test") -Arguments @(
        "test", "--manifest-path", $managerManifest, "--lib", "codex_entry_ipc::tests::compiled_shim_uac_cancel_path_fails_closed_without_core", "-j", "1", "--", "--ignored", "--nocapture"
    )
    if ($uacResult.Output) { Write-Output $uacResult.Output }
    if ($uacResult.ExitCode -ne 0) { throw "shim UAC-cancel fail-closed test failed: $($uacResult.ExitCode)" }
} finally {
    foreach ($name in $envNames) { [Environment]::SetEnvironmentVariable($name, $saved[$name], "Process") }
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
