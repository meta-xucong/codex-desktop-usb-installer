[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$GeneratedFragment,
    [Parameter(Mandatory = $true)][string]$TargetUserProfile,
    [Parameter(Mandatory = $true)][string]$KitHome,
    [string]$PythonExe
)

$ErrorActionPreference = 'Stop'
$profileFull = [IO.Path]::GetFullPath($TargetUserProfile).TrimEnd('\')
$actualProfile = [IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('USERPROFILE')).TrimEnd('\')
$configRoot = Join-Path $profileFull '.codex'
$configPath = Join-Path $configRoot 'config.toml'
$backupRoot = Join-Path $KitHome 'backups'
$beginMarker = '# BEGIN CODEX-STARTER-KIT MANAGED CONFIG v1'
$endMarker = '# END CODEX-STARTER-KIT MANAGED CONFIG v1'

if (-not (Test-Path -LiteralPath $GeneratedFragment -PathType Leaf)) { throw "Generated Codex fragment is missing: $GeneratedFragment" }
$fragment = Get-Content -LiteralPath $GeneratedFragment -Raw -Encoding UTF8
if ($fragment -match '(?i)(app[_-]?secret\s*=|FEISHU_APP_SECRET\s*=\s*["''][^"'']+)') { throw 'Generated Codex fragment contains a literal secret.' }
if ($fragment -match '__[A-Z0-9_]+__') { throw 'Generated Codex fragment contains unresolved placeholders.' }

New-Item -ItemType Directory -Force -Path $configRoot, $backupRoot | Out-Null
$existing = if (Test-Path -LiteralPath $configPath -PathType Leaf) { Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 } else { '' }
$managedPattern = '(?ms)^' + [regex]::Escape($beginMarker) + '\r?\n.*?^' + [regex]::Escape($endMarker) + '\r?\n?'
$withoutManaged = [regex]::Replace($existing, $managedPattern, '')

function Get-FeishuSections([string]$Text) {
    $lines = @($Text -split '\r?\n', -1)
    $sections = New-Object System.Collections.Generic.List[object]
    $start = -1
    for ($index = 0; $index -lt $lines.Count; $index++) {
        $header = ([string]$lines[$index]).Trim()
        if ($header -notmatch '^\[\[?.*\]\]?$') { continue }

        $isRoot = $header -eq '[mcp_servers.feishu]'
        $isChild = $header -match '^\[\[?mcp_servers\.feishu\.'
        if ($start -ge 0 -and -not $isChild) {
            [void]$sections.Add([pscustomobject]@{
                Start = $start
                End = $index
                Text = (@($lines[$start..($index - 1)]) -join "`n")
            })
            $start = -1
        }
        if ($isRoot) { $start = $index }
    }
    if ($start -ge 0) {
        [void]$sections.Add([pscustomobject]@{
            Start = $start
            End = $lines.Count
            Text = (@($lines[$start..($lines.Count - 1)]) -join "`n")
        })
    }
    return $sections.ToArray()
}

function Remove-FeishuSection([string]$Text, [pscustomobject]$Section) {
    $lines = @($Text -split '\r?\n', -1)
    $kept = New-Object System.Collections.Generic.List[string]
    if ($Section.Start -gt 0) {
        foreach ($line in @($lines[0..($Section.Start - 1)])) { [void]$kept.Add([string]$line) }
    }
    if ($Section.End -lt $lines.Count) {
        foreach ($line in @($lines[$Section.End..($lines.Count - 1)])) { [void]$kept.Add([string]$line) }
    }
    return ($kept -join "`n")
}

function Normalize-FeishuSection([string]$Text) {
    $normalizedLines = New-Object System.Collections.Generic.List[string]
    foreach ($line in @($Text.Replace("`r`n", "`n").Replace("`r", "`n") -split "`n")) {
        $trimmed = ([string]$line).Trim()
        if ($trimmed -and -not $trimmed.StartsWith('#', [StringComparison]::Ordinal)) {
            [void]$normalizedLines.Add($trimmed)
        }
    }
    return ($normalizedLines -join "`n").Trim()
}

$generatedFeishu = @(Get-FeishuSections $fragment)
$legacyFeishu = @(Get-FeishuSections $withoutManaged)
$legacyMigration = $null
if ($generatedFeishu.Count -eq 1 -and $legacyFeishu.Count -eq 1) {
    $generatedFeishuText = Normalize-FeishuSection $generatedFeishu[0].Text
    $legacyFeishuText = Normalize-FeishuSection $legacyFeishu[0].Text
    if ($legacyFeishuText -ceq $generatedFeishuText) {
        $legacyMigration = $legacyFeishu[0]
        $withoutManaged = Remove-FeishuSection $withoutManaged $legacyMigration
    }
}
$hasUnmanagedFeishu = [regex]::IsMatch($withoutManaged, '(?m)^\s*\[mcp_servers\.feishu\][ \t]*(?:#.*)?$')

if ($hasUnmanagedFeishu) {
    $status = [ordered]@{ merged = $false; status = 'conflict'; configPath = $configPath; reason = 'Existing unmanaged [mcp_servers.feishu] was preserved; review the generated fragment manually.'; backup = $null }
    $status | ConvertTo-Json -Depth 6
    exit 0
}

$newContent = $withoutManaged.TrimEnd() + [Environment]::NewLine + [Environment]::NewLine + $beginMarker + [Environment]::NewLine + $fragment.Trim() + [Environment]::NewLine + $endMarker + [Environment]::NewLine
$backupPath = $null
if (Test-Path -LiteralPath $configPath -PathType Leaf) {
    $backupPath = Join-Path $backupRoot ('config.toml.' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '.bak')
    Copy-Item -LiteralPath $configPath -Destination $backupPath -Force
}
$tempPath = $configPath + '.tmp-' + [guid]::NewGuid().ToString('N')
try {
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($tempPath, $newContent, $utf8NoBom)
    if ([string]::IsNullOrWhiteSpace($PythonExe)) {
        $bundledPython = Join-Path $KitHome 'runtime\python-env\Scripts\python.exe'
        if (Test-Path -LiteralPath $bundledPython -PathType Leaf) { $PythonExe = $bundledPython }
    }
    if ([string]::IsNullOrWhiteSpace($PythonExe)) {
        $candidate = Get-Command python -ErrorAction SilentlyContinue
        if ($candidate) { $PythonExe = $candidate.Source }
    }
    if ([string]::IsNullOrWhiteSpace($PythonExe) -or -not (Test-Path -LiteralPath $PythonExe -PathType Leaf)) {
        throw 'Cannot safely merge Codex config without a Python TOML validator; the existing config was preserved.'
    }
    & $PythonExe -c "import pathlib,sys,tomllib; data=pathlib.Path(sys.argv[1]).read_bytes(); assert not data.startswith(bytes.fromhex('efbbbf')), 'UTF-8 BOM is not allowed'; tomllib.loads(data.decode('utf-8'))" $tempPath
    if ($LASTEXITCODE -ne 0) { throw 'Merged Codex config failed strict UTF-8/TOML validation; the existing config was preserved.' }
    if (Test-Path -LiteralPath $configPath -PathType Leaf) {
        $atomicBackupPath = $configPath + '.replace-' + [guid]::NewGuid().ToString('N')
        [System.IO.File]::Replace($tempPath, $configPath, $atomicBackupPath, $true)
        Remove-Item -LiteralPath $atomicBackupPath -Force -ErrorAction SilentlyContinue
    } else {
        [System.IO.File]::Move($tempPath, $configPath)
    }
}
finally {
    if (Test-Path -LiteralPath $tempPath -PathType Leaf) { Remove-Item -LiteralPath $tempPath -Force }
}

[ordered]@{ merged = $true; status = 'merged'; configPath = $configPath; backup = $backupPath; targetIsCurrentUser = $profileFull.Equals($actualProfile, [StringComparison]::OrdinalIgnoreCase) } | ConvertTo-Json -Depth 6
