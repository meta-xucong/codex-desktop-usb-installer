param()

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repoRoot 'src-tauri/Cargo.toml'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Tauri manifest not found: $manifestPath"
}

$testKey = '0EqyMnQrtKs6E2i9RhXk5tAiSrcaAWuvhSCjMsl3hzc='
$testKeyring = '{"keys":{"test-revocation-key":"0EqyMnQrtKs6E2i9RhXk5tAiSrcaAWuvhSCjMsl3hzc="}}'
$extraTopLevelKeyring = '{"keys":{"test-revocation-key":"0EqyMnQrtKs6E2i9RhXk5tAiSrcaAWuvhSCjMsl3hzc="},"unexpected":true}'
$invalidPublicKey = 'AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
$invalidPointKeyring = '{"keys":{"test-revocation-key":"' + $invalidPublicKey + '"}}'
$names = @(
    'CODEX_USB_GUARD_POLICY',
    'CODEX_USB_GUARD_PUBLIC_KEY_B64',
    'CODEX_USB_GUARD_PUBLIC_KEY_ID',
    'CODEX_USB_GUARD_KEYRING_JSON',
    'CODEX_USB_REVOCATION_URL',
    'CODEX_USB_REVOCATION_KEYRING_JSON',
    'CARGO_FEATURE_SIGNED_REQUIRED'
)
$original = @{}
foreach ($name in $names) {
    $original[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

function Invoke-OfflineCargoCheck {
    $output = & cargo check --offline --locked --color never --manifest-path $manifestPath --no-default-features 2>&1
    $exitCode = $LASTEXITCODE
    [pscustomobject]@{
        ExitCode = $exitCode
        Output = ($output | Out-String)
    }
}

function Assert-SignedRequiredRejected {
    param(
        [string]$Keyring,
        [string]$ExpectedMessage,
        [string]$Scenario
    )

    $env:CODEX_USB_GUARD_POLICY = 'SignedRequired'
    $env:CODEX_USB_GUARD_PUBLIC_KEY_B64 = $testKey
    $env:CODEX_USB_GUARD_PUBLIC_KEY_ID = 'test-license-key'
    Remove-Item Env:CODEX_USB_GUARD_KEYRING_JSON -ErrorAction SilentlyContinue
    $env:CODEX_USB_REVOCATION_URL = 'https://example.com/'
    $env:CODEX_USB_REVOCATION_KEYRING_JSON = $Keyring

    $result = Invoke-OfflineCargoCheck
    if ($result.ExitCode -eq 0) {
        throw "SignedRequired unexpectedly accepted input for: $Scenario"
    }
    if ($result.Output -notmatch $ExpectedMessage) {
        throw "SignedRequired rejected $Scenario for an unexpected reason:`n$($result.Output)"
    }
    Write-Output "PASS: SignedRequired rejects $Scenario."
}

try {
    Remove-Item Env:CARGO_FEATURE_SIGNED_REQUIRED -ErrorAction SilentlyContinue
    Assert-SignedRequiredRejected `
        -Keyring $extraTopLevelKeyring `
        -ExpectedMessage '只能包含 keys 字段' `
        -Scenario 'a revocation keyring with an extra top-level field'
    Assert-SignedRequiredRejected `
        -Keyring $invalidPointKeyring `
        -ExpectedMessage '不是可解析的 Ed25519 公钥' `
        -Scenario 'a 32-byte value that is not an Ed25519 public key'
    Assert-SignedRequiredRejected `
        -Keyring $testKeyring `
        -ExpectedMessage '固定的生产 HTTPS host' `
        -Scenario 'a non-production HTTPS host even when the status keyring is structurally valid'

    $env:CODEX_USB_GUARD_POLICY = 'DevelopmentAutoEnroll'
    Remove-Item Env:CODEX_USB_GUARD_PUBLIC_KEY_B64 -ErrorAction SilentlyContinue
    Remove-Item Env:CODEX_USB_GUARD_PUBLIC_KEY_ID -ErrorAction SilentlyContinue
    Remove-Item Env:CODEX_USB_GUARD_KEYRING_JSON -ErrorAction SilentlyContinue
    Remove-Item Env:CODEX_USB_REVOCATION_URL -ErrorAction SilentlyContinue
    Remove-Item Env:CODEX_USB_REVOCATION_KEYRING_JSON -ErrorAction SilentlyContinue

    $development = Invoke-OfflineCargoCheck
    if ($development.ExitCode -ne 0) {
        throw "DevelopmentAutoEnroll build failed:`n$($development.Output)"
    }
    Write-Output 'PASS: DevelopmentAutoEnroll builds without production release pins.'
}
finally {
    foreach ($name in $names) {
        if ($null -eq $original[$name]) {
            Remove-Item "Env:$name" -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($name, $original[$name], 'Process')
        }
    }
}

$restoredFeatureFlag = [Environment]::GetEnvironmentVariable('CARGO_FEATURE_SIGNED_REQUIRED', 'Process')
if ($restoredFeatureFlag -cne $original['CARGO_FEATURE_SIGNED_REQUIRED']) {
    throw 'CARGO_FEATURE_SIGNED_REQUIRED was not restored exactly after the isolated build checks.'
}
Write-Output 'PASS: CARGO_FEATURE_SIGNED_REQUIRED was isolated during checks and restored afterward.'
