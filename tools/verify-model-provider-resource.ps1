[CmdletBinding()]
param(
    [string]$SourcePath = '',
    [string]$ArtifactPath = ''
)
$ErrorActionPreference = 'Stop'
if (-not $PSBoundParameters.ContainsKey('SourcePath')) {
    $SourcePath = Join-Path $PSScriptRoot '..\src-tauri\resources\ccswitch\providers.json'
}
$utf8 = [System.Text.UTF8Encoding]::new($false, $true)
function Read-Catalog([string]$Path) {
    $file = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($file.PSIsContainer -or $file.Length -gt 65536 -or
        ($file.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
        throw 'Provider catalog must be a bounded regular file.'
    }
    return [System.IO.File]::ReadAllText($file.FullName, $utf8) | ConvertFrom-Json
}
$source = Read-Catalog $SourcePath
if ($source.schemaVersion -ne 1 -or $source.product -ne 'codex-usb') {
    throw 'Unexpected provider catalog schema/product.'
}
foreach ($provider in @($source.providers)) {
    if ($null -ne $provider.defaultModel -or @($provider.fallbackModels).Count -ne 0) {
        throw 'Unverified static default/fallback models must not enter the USB resource bundle.'
    }
    if ($provider.modelSource -ne 'runtime') { throw 'Model discovery must remain runtime-only.' }
}
if (@($source.providers).Count -eq 0) { throw 'Provider discovery configuration is empty.' }
if (-not [string]::IsNullOrWhiteSpace($ArtifactPath)) {
    $null = Read-Catalog $ArtifactPath
    $expected = (Get-FileHash -LiteralPath $SourcePath -Algorithm SHA256).Hash
    $actual = (Get-FileHash -LiteralPath $ArtifactPath -Algorithm SHA256).Hash
    if ($expected -ne $actual) { throw 'Packaged provider resource differs from canonical source; rebuild resources.' }
}
Write-Output 'Provider resource code gate passed; this does not certify model compatibility.'
