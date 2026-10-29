param(
    [Parameter(Mandatory = $true)][string]$MsixPath
)
$ErrorActionPreference = 'Stop'
$expectedMsixSha256 = '9085CF2E2EB5081F894A2CF334E1F36DBB9438ECB409D20195AA2FE1ECFCB4C8'
$expectedPackageVersion = '26.1016.8622.0'
$msix = Get-Item -LiteralPath $MsixPath
$actualHash = (Get-FileHash -LiteralPath $msix.FullName -Algorithm SHA256).Hash
if ($actualHash -ne $expectedMsixSha256) { throw 'MSIX SHA-256 does not match the audited package' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($msix.FullName)
$tempAsar = Join-Path $env:TEMP ('codex-msix-readonly-' + [guid]::NewGuid().ToString('N') + '.asar')
try {
    $manifestEntry = $archive.GetEntry('AppxManifest.xml')
    if (-not $manifestEntry) { throw 'AppxManifest.xml missing' }
    $reader = [System.IO.StreamReader]::new($manifestEntry.Open())
    try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
    $packageVersion = $manifest.Package.Identity.Version
    if ($packageVersion -ne $expectedPackageVersion) { throw 'MSIX package version does not match audited package' }
    $asarEntry = $archive.GetEntry('app/resources/app.asar')
    if (-not $asarEntry) { throw 'app/resources/app.asar missing' }
    $input = $asarEntry.Open()
    $output = [System.IO.File]::Open($tempAsar, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::None)
    try { $input.CopyTo($output) } finally { $output.Dispose(); $input.Dispose() }
    Write-Output ("MSIX package version: {0}; SHA-256: {1}; bytes: {2}" -f $packageVersion,$actualHash,$msix.Length)
    & node (Join-Path $PSScriptRoot 'verify-asar-renderer.mjs') $tempAsar
    if ($LASTEXITCODE -ne 0) { throw 'ASAR renderer verification failed' }
} finally {
    $archive.Dispose()
    if (Test-Path -LiteralPath $tempAsar) { Remove-Item -LiteralPath $tempAsar -Force }
}
