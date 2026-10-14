[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Root
)

$ErrorActionPreference = 'Stop'

if ($PSVersionTable.PSEdition -ne 'Desktop' -or $PSVersionTable.PSVersion -lt [version]'5.1') {
    throw 'This gate must run under Windows PowerShell 5.1 or newer Desktop edition.'
}

$resolvedRoot = (Resolve-Path -LiteralPath $Root -ErrorAction Stop).Path
$strictUtf8 = New-Object System.Text.UTF8Encoding($false, $true)
$extensions = @('.ps1', '.psm1', '.psd1')
$scripts = @(Get-ChildItem -LiteralPath $resolvedRoot -File -Recurse -Force | Where-Object {
    $extensions -contains $_.Extension.ToLowerInvariant()
})

if ($scripts.Count -eq 0) {
    throw "No PowerShell source files were found under: $resolvedRoot"
}

$failures = New-Object System.Collections.Generic.List[string]
foreach ($scriptFile in $scripts) {
    $bytes = [System.IO.File]::ReadAllBytes($scriptFile.FullName)
    $relativePath = $scriptFile.FullName.Substring($resolvedRoot.TrimEnd('\').Length).TrimStart('\')

    if ($bytes.Length -lt 3 -or $bytes[0] -ne 0xEF -or $bytes[1] -ne 0xBB -or $bytes[2] -ne 0xBF) {
        $failures.Add("$relativePath : missing UTF-8 BOM")
        continue
    }

    try {
        [void]$strictUtf8.GetString($bytes, 3, $bytes.Length - 3)
    } catch {
        $failures.Add("$relativePath : invalid UTF-8 byte sequence")
        continue
    }

    $tokens = $null
    $parseErrors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $scriptFile.FullName,
        [ref]$tokens,
        [ref]$parseErrors
    )
    foreach ($parseError in $parseErrors) {
        $failures.Add(("{0}:{1}:{2} : {3}" -f $relativePath, $parseError.Extent.StartLineNumber, $parseError.Extent.StartColumnNumber, $parseError.Message))
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Error $failure -ErrorAction Continue
    }
    exit 1
}

Write-Output ("PowerShell encoding and Windows PowerShell 5.1 parse check passed: {0} files" -f $scripts.Count)
