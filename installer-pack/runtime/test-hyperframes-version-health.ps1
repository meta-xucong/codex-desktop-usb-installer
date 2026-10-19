$ErrorActionPreference = 'Stop'
$sourcePath = Join-Path $PSScriptRoot 'provision-runtime.ps1'
$source = [IO.File]::ReadAllText($sourcePath)
$matches = [regex]::Matches($source, '(?s)# BEGIN Test-HyperframesPackageHealth\r?\n(.*?)\r?\n# END Test-HyperframesPackageHealth')
if ($matches.Count -ne 1) { throw 'Expected exactly one marked HyperFrames health helper in provision-runtime.ps1.' }
Invoke-Expression $matches[0].Groups[1].Value

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ('hyperframes-health-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempRoot | Out-Null
try {
    $packagePath = Join-Path $tempRoot 'package.json'
    function Write-TestPackage([string]$Name, [string]$Version) {
        $json = @{ name = $Name; version = $Version } | ConvertTo-Json
        [IO.File]::WriteAllText($packagePath, $json, [Text.Encoding]::UTF8)
    }
    function Assert-Test([bool]$Condition, [string]$Message) {
        if (-not $Condition) { throw $Message }
    }

    Write-TestPackage 'hyperframes' '0.8.174'
    $valid = Test-HyperframesPackageHealth -PackageJsonPath $packagePath -CliOutput '0.8.174' -CliExitCode 0 -ExpectedVersion '0.8.174'
    Assert-Test $valid.IsValid 'Expected locked package and exact CLI version to pass.'
    Assert-Test ($valid.Version -eq '0.8.174') 'Expected returned installed version to be normalized.'

    $validWithPrefix = Test-HyperframesPackageHealth -PackageJsonPath $packagePath -CliOutput 'v0.8.174' -CliExitCode 0 -ExpectedVersion '0.8.174'
    Assert-Test $validWithPrefix.IsValid 'Expected a v-prefixed exact CLI version to pass.'

    Write-TestPackage 'hyperframes' '0.8.173'
    $wrongPackageVersion = Test-HyperframesPackageHealth -PackageJsonPath $packagePath -CliOutput '0.8.174' -CliExitCode 0 -ExpectedVersion '0.8.174'
    Assert-Test (-not $wrongPackageVersion.IsValid) 'Unexpected package version must be rejected.'
    Assert-Test ($wrongPackageVersion.Error -match 'package version') 'Wrong package version should explain the rejection.'

    Write-TestPackage 'other-package' '0.8.174'
    $wrongPackageName = Test-HyperframesPackageHealth -PackageJsonPath $packagePath -CliOutput '0.8.174' -CliExitCode 0 -ExpectedVersion '0.8.174'
    Assert-Test (-not $wrongPackageName.IsValid) 'Unexpected package name must be rejected.'

    Write-TestPackage 'hyperframes' '0.8.174'
    $wrongCliVersion = Test-HyperframesPackageHealth -PackageJsonPath $packagePath -CliOutput '0.8.173' -CliExitCode 0 -ExpectedVersion '0.8.174'
    Assert-Test (-not $wrongCliVersion.IsValid) 'Unexpected CLI version must be rejected.'
    $extraCliText = Test-HyperframesPackageHealth -PackageJsonPath $packagePath -CliOutput "HyperFrames 0.8.174`n0.8.174" -CliExitCode 0 -ExpectedVersion '0.8.174'
    Assert-Test (-not $extraCliText.IsValid) 'Additional CLI output must not be mistaken for an exact version.'
    $failedCli = Test-HyperframesPackageHealth -PackageJsonPath $packagePath -CliOutput '0.8.174' -CliExitCode 1 -ExpectedVersion '0.8.174'
    Assert-Test (-not $failedCli.IsValid) 'Nonzero CLI exit must be rejected.'

    $missingPath = Join-Path $tempRoot 'missing.json'
    $missingPackage = Test-HyperframesPackageHealth -PackageJsonPath $missingPath -CliOutput '0.8.174' -CliExitCode 0 -ExpectedVersion '0.8.174'
    Assert-Test (-not $missingPackage.IsValid) 'Missing package metadata must be rejected.'

    # Protect the installer control flow: stale markers trigger npm ci when package metadata differs,
    # and the lock marker is written only after the complete package+CLI check passes.
    $installGate = '(?s)if \(\$installedLockHash -ne \$expectedHyperframesLockHash.*?-not \$installedPackageMatchesLock\) \{'
    Assert-Test ([regex]::IsMatch($source, $installGate)) 'Installer must reinstall when installed package metadata mismatches the lock.'
    Assert-Test ([regex]::IsMatch($source, '(?s)if \(\$packageHealth\.IsValid -and \$hyperframesInstallSucceeded\).*?WriteAllText\(\$hyperframesMarker')) 'Installer may write the healthy marker only after package and CLI validation.'
    Assert-Test ([regex]::IsMatch($source, '(?s)if \(\$packageHealth\.IsValid -and \$hyperframesInstallSucceeded\)')) 'Installer must not certify an old package when a required npm ci failed.'
    Assert-Test ([regex]::IsMatch($source, '(?s)browser.*?ensure.*?ExitCode -eq 0')) 'Browser health should remain based on the existing browser ensure exit code.'

    Write-Output 'HyperFrames package/version health focused tests passed (8 scenarios).'
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
