[CmdletBinding()]
param(
    [switch]$IncludeNonDefault,
    [switch]$IncludeUnsupported,
    [switch]$Overwrite,
    [string]$PackRoot,
    [string]$TargetUserProfile,
    [ValidateSet('tenant','oauth','user')][string]$FeishuAuthMode = 'tenant',
    [string]$FeishuDomain = 'https://open.feishu.cn',
    [string[]]$EnableSkills = @(),
    [string[]]$EnabledApiServices = @(),
    [switch]$EnableFeishu,
    [switch]$DisableFeishu
)

$ErrorActionPreference = 'Stop'
if ($IncludeUnsupported -and -not $IncludeNonDefault) { throw '-IncludeUnsupported requires -IncludeNonDefault.' }
if ($EnableFeishu -and $DisableFeishu) { throw '-EnableFeishu and -DisableFeishu are mutually exclusive.' }
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Write-JsonNoBom([string]$Path, [object]$Value, [int]$Depth = 8) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $json = $Value | ConvertTo-Json -Depth $Depth
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $utf8NoBom)
}

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($PackRoot)) {
    if (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'pack.json') -PathType Leaf) {
        $PackRoot = $PSScriptRoot
    } else {
        $PackRoot = Join-Path $repoRoot 'installer-pack'
    }
}
$PackRoot = (Resolve-Path -LiteralPath $PackRoot).Path
$packFile = Join-Path $PackRoot 'pack.json'
$dependencyFile = Join-Path $PackRoot 'dependencies.json'
$fileManifest = Join-Path $PackRoot 'file-manifest.json'
$checksumFile = Join-Path $PackRoot 'checksums.sha256'
foreach ($requiredFile in @($packFile, $dependencyFile, $fileManifest, $checksumFile)) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) { throw "Invalid capability pack: missing $requiredFile" }
}

$pack = Get-Content -LiteralPath $packFile -Raw | ConvertFrom-Json
if ([int]$pack.schemaVersion -ne 2) { throw "Unsupported capability pack schema: $($pack.schemaVersion)" }
$manifest = Get-Content -LiteralPath $fileManifest -Raw | ConvertFrom-Json
$packReparsePoints = @(Get-ChildItem -LiteralPath $PackRoot -Recurse -Force | Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 })
if ($packReparsePoints.Count -gt 0) { throw "Capability pack contains a symbolic link or junction: $($packReparsePoints[0].FullName)" }
function Resolve-PackOwnedFile([string]$RelativePath) {
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [IO.Path]::IsPathRooted($RelativePath) -or $RelativePath -match '(^|[\/])\.\.([\/]|$)') { throw "Unsafe pack path: $RelativePath" }
    $packRootFull = [IO.Path]::GetFullPath($PackRoot).TrimEnd('\') + '\'
    $candidate = [IO.Path]::GetFullPath((Join-Path $PackRoot ($RelativePath -replace '/', '\')))
    if (-not $candidate.StartsWith($packRootFull, [StringComparison]::OrdinalIgnoreCase)) { throw "Pack path escapes root: $RelativePath" }
    return $candidate
}
$canonicalTextExtensions = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($extension in @('.cmd', '.env', '.in', '.json', '.lock', '.md', '.ps1', '.py', '.sha256', '.toml', '.txt', '.yaml', '.yml')) {
    [void]$canonicalTextExtensions.Add($extension)
}
function Get-CanonicalFileHash([string]$Path) {
    $extension = [IO.Path]::GetExtension($Path)
    if (-not $canonicalTextExtensions.Contains($extension)) {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $stream = New-Object System.IO.MemoryStream
    for ($index = 0; $index -lt $bytes.Length; $index++) {
        if ($bytes[$index] -eq 13) {
            if (($index + 1) -lt $bytes.Length -and $bytes[$index + 1] -eq 10) { $index++ }
            $stream.WriteByte([byte]10)
        } else {
            $stream.WriteByte($bytes[$index])
        }
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($stream.ToArray())).Replace('-', '')).ToLowerInvariant()
    } finally {
        $sha.Dispose()
        $stream.Dispose()
    }
}
$manifestPaths = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in $manifest.files) {
    $relativeManifestPath = ([string]$entry.path).Replace('\','/')
    if (-not $manifestPaths.Add($relativeManifestPath)) { throw "Duplicate pack manifest path: $relativeManifestPath" }
    $source = Resolve-PackOwnedFile ([string]$entry.path)
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Pack file missing: $($entry.path)" }
    $hash = Get-CanonicalFileHash $source
    if ($hash -ne ([string]$entry.sha256).ToLowerInvariant()) { throw "Pack hash mismatch: $($entry.path)" }
}
$checksumPaths = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($line in Get-Content -LiteralPath $checksumFile) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $match = [regex]::Match($line, '^(?<hash>[0-9a-fA-F]{64}) \*(?<path>.+)$')
    if (-not $match.Success) { throw "Invalid pack checksum line: $line" }
    $relative = $match.Groups['path'].Value.Trim()
    if (-not $checksumPaths.Add($relative)) { throw "Duplicate pack checksum path: $relative" }
    $source = Resolve-PackOwnedFile $relative
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Checksum target missing: $relative" }
    $hash = Get-CanonicalFileHash $source
    if ($hash -ne $match.Groups['hash'].Value.ToLowerInvariant()) { throw "Pack checksum mismatch: $relative" }
}
$expectedChecksumPaths = [System.Collections.Generic.HashSet[string]]::new($manifestPaths, [StringComparer]::OrdinalIgnoreCase)
$null = $expectedChecksumPaths.Add('file-manifest.json')
if ($checksumPaths.Count -ne $expectedChecksumPaths.Count) { throw 'Pack checksum and file manifest scopes differ.' }
foreach ($relative in $expectedChecksumPaths) {
    if (-not $checksumPaths.Contains($relative)) { throw "Pack checksum is missing a manifest target: $relative" }
}
$actualPackFiles = @(Get-ChildItem -LiteralPath $PackRoot -Recurse -File -Force | ForEach-Object { $_.FullName.Substring($PackRoot.Length).TrimStart('\').Replace('\','/') } | Where-Object { $_ -ne 'checksums.sha256' })
if ($actualPackFiles.Count -ne $checksumPaths.Count) { throw 'Capability pack contains missing or unlisted files.' }
foreach ($relative in $actualPackFiles) {
    if (-not $checksumPaths.Contains($relative)) { throw "Capability pack contains an unlisted file: $relative" }
}

$userProfile = $TargetUserProfile
if ([string]::IsNullOrWhiteSpace($userProfile)) { $userProfile = [Environment]::GetEnvironmentVariable('USERPROFILE') }
if ([string]::IsNullOrWhiteSpace($userProfile)) { throw 'USERPROFILE is required for a user-scoped installation.' }
$userProfile = [IO.Path]::GetFullPath($userProfile)
$agentTarget = Join-Path $userProfile '.codex\agents'
$skillTarget = Join-Path $userProfile '.agents\skills'
$ownershipRoot = Join-Path $userProfile '.codex\codex-agent-kit'
$ownershipFile = Join-Path $ownershipRoot 'install-manifest.json'
New-Item -ItemType Directory -Force -Path $agentTarget, $skillTarget, $ownershipRoot | Out-Null

function Test-PathWithin([string]$Path, [string]$RootPath, [switch]$AllowEqual) {
    $pathFull = [IO.Path]::GetFullPath($Path)
    $rootFull = [IO.Path]::GetFullPath($RootPath).TrimEnd('\')
    if ($AllowEqual -and $pathFull.Equals($rootFull, [StringComparison]::OrdinalIgnoreCase)) { return $true }
    return $pathFull.StartsWith(($rootFull + '\'), [StringComparison]::OrdinalIgnoreCase)
}

foreach ($targetRoot in @($agentTarget, $skillTarget, $ownershipRoot)) {
    $relativeTarget = $targetRoot.Substring($userProfile.TrimEnd('\').Length).TrimStart('\')
    $cursor = $userProfile.TrimEnd('\')
    foreach ($segment in $relativeTarget.Split('\')) {
        $cursor = Join-Path $cursor $segment
        $item = Get-Item -LiteralPath $cursor -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Refusing to install through a symbolic link or junction: $cursor" }
    }
}

$previousFiles = @()
if (Test-Path -LiteralPath $ownershipFile -PathType Leaf) {
    try {
        $previousRecord = Get-Content -LiteralPath $ownershipFile -Raw | ConvertFrom-Json
        if ([int]$previousRecord.schemaVersion -notin @(3,4) -or [string]$previousRecord.packId -ne [string]$pack.id) { throw 'Ownership record identity is invalid.' }
        if (-not ([IO.Path]::GetFullPath([string]$previousRecord.agentRoot)).Equals([IO.Path]::GetFullPath($agentTarget), [StringComparison]::OrdinalIgnoreCase) -or
            -not ([IO.Path]::GetFullPath([string]$previousRecord.skillRoot)).Equals([IO.Path]::GetFullPath($skillTarget), [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Ownership record target roots do not match this installation.'
        }
        $seenOwnership = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in @($previousRecord.files)) {
            $entryPath = [string]$entry.path
            $entryKind = [string]$entry.kind
            if ([string]::IsNullOrWhiteSpace($entryPath) -or [string]::IsNullOrWhiteSpace($entryKind)) { throw 'Ownership record contains an incomplete entry.' }
            if (-not $seenOwnership.Add($entryKind + '|' + [IO.Path]::GetFullPath($entryPath))) { throw "Ownership record contains a duplicate entry: $entryPath" }
            $allowed = if ($entryKind -in @('skill','skill-tree')) {
                Test-PathWithin $entryPath $skillTarget -AllowEqual
            } elseif ($entryKind -eq 'agent') {
                Test-PathWithin $entryPath $agentTarget
            } else {
                Test-PathWithin $entryPath $ownershipRoot
            }
            if (-not $allowed) { throw "Ownership record path is outside its allowed root: $entryPath" }
        }
        $previousFiles = @($previousRecord.files)
    } catch {
        Write-Warning "Could not read the previous ownership record; existing files will not be overwritten without a matching ownership entry."
    }
}
$installed = [System.Collections.Generic.List[object]]::new()
foreach ($entry in $previousFiles) {
    if ($entry.path -and (Test-Path -LiteralPath ([string]$entry.path))) { $installed.Add($entry) }
}

function Remove-RecordsUnder([string]$RootPath) {
    $prefix = [IO.Path]::GetFullPath($RootPath).TrimEnd('\') + '\'
    $keep = [System.Collections.Generic.List[object]]::new()
    foreach ($entry in @($installed)) {
        $path = if ($entry.path) { [IO.Path]::GetFullPath([string]$entry.path) } else { '' }
        if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -and -not $path.Equals([IO.Path]::GetFullPath($RootPath), [StringComparison]::OrdinalIgnoreCase)) { $keep.Add($entry) }
    }
    $installed.Clear()
    foreach ($entry in $keep) { $installed.Add($entry) }
}

function Get-PreviousOwnership([string]$Path, [string[]]$Kinds) {
    foreach ($entry in $previousFiles) {
        if ($entry.path -and ([string]$entry.path).Equals($Path, [StringComparison]::OrdinalIgnoreCase) -and $Kinds -contains ([string]$entry.kind)) { return $entry }
    }
    return $null
}

function Add-OwnedRecord([string]$Kind, [string]$Path, [string]$Hash) {
    $old = Get-PreviousOwnership $Path @($Kind)
    if ($old) { Remove-RecordsUnder $Path }
    $installed.Add([pscustomobject]@{ kind = $Kind; path = $Path; sha256 = $Hash })
}

function Copy-OwnedFile([string]$Source, [string]$Destination, [string]$Kind) {
    $existing = Test-Path -LiteralPath $Destination -PathType Leaf
    if ((Test-Path -LiteralPath $Destination) -and -not $existing) { throw "Expected a file but found another item type: $Destination" }
    if ($existing -and -not $Overwrite) { Write-Warning "Skipped existing ${Kind}: $Destination (use -Overwrite to update)."; return $false }
    if ($existing -and $Overwrite) {
        $previousOwnership = Get-PreviousOwnership $Destination @($Kind)
        if (-not $previousOwnership) {
            Write-Warning "Skipped unowned existing ${Kind}: $Destination (the installer only overwrites files recorded in its ownership manifest)."
            return $false
        }
        $currentHash = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace([string]$previousOwnership.sha256) -or $currentHash -ne ([string]$previousOwnership.sha256).ToLowerInvariant()) {
            Write-Warning "Skipped locally modified ${Kind}: $Destination (preserving user changes)."
            return $false
        }
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Destination) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
    $hash = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
    Add-OwnedRecord $Kind $Destination $hash
    Write-Host "Installed ${Kind}: $Destination"
    return $true
}

function Remove-OwnedSkillTree([string]$Destination) {
    $targetRoot = [IO.Path]::GetFullPath($skillTarget).TrimEnd('\') + '\'
    $destinationFull = [IO.Path]::GetFullPath($Destination)
    if (-not $destinationFull.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) { throw "Refusing to mutate a path outside the Skill root: $Destination" }
    $ownedTree = Get-PreviousOwnership $Destination @('skill-tree','skill')
    if (-not $ownedTree) { return $false }
    $destinationItem = Get-Item -LiteralPath $Destination -Force
    if (($destinationItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { Write-Warning "Skipped linked Skill tree: $Destination"; return $false }
    $treeItems = @(Get-ChildItem -LiteralPath $Destination -Recurse -Force)
    $reparsePoint = @($treeItems | Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 } | Select-Object -First 1)
    if ($reparsePoint.Count -gt 0) { Write-Warning "Skipped Skill tree containing a link or junction: $($reparsePoint[0].FullName)"; return $false }
    $ownedFiles = @($previousFiles | Where-Object { ([string]$_.kind) -eq 'skill' -and (Test-PathWithin ([string]$_.path) $Destination) })
    $ownedByPath = @{}
    foreach ($entry in $ownedFiles) { $ownedByPath[[IO.Path]::GetFullPath([string]$entry.path).ToLowerInvariant()] = $entry }
    $currentFiles = @($treeItems | Where-Object { -not $_.PSIsContainer })
    foreach ($file in $currentFiles) {
        $key = [IO.Path]::GetFullPath($file.FullName).ToLowerInvariant()
        if (-not $ownedByPath.ContainsKey($key)) { Write-Warning "Skipped Skill tree with a user-added file: $($file.FullName)"; return $false }
        $expectedHash = [string]$ownedByPath[$key].sha256
        $currentHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace($expectedHash) -or $currentHash -ne $expectedHash.ToLowerInvariant()) {
            Write-Warning "Skipped locally modified Skill tree: $($file.FullName)"
            return $false
        }
    }
    foreach ($file in $currentFiles) { Remove-Item -LiteralPath $file.FullName -Force }
    Remove-RecordsUnder $Destination
    return $true
}

$agentFiles = @($pack.defaultEnabled.agents)
$skillNames = @($pack.defaultEnabled.skills)
if ($IncludeNonDefault) {
    $agentFiles = @(Get-ChildItem -LiteralPath (Join-Path $PackRoot 'agents') -Filter '*.toml' -File | ForEach-Object { "agents/$($_.Name)" })
    $skillAuditFile = Join-Path $PackRoot 'manifest\skill-audit.json'
    if (-not (Test-Path -LiteralPath $skillAuditFile -PathType Leaf)) { throw "Missing Skill audit in pack: $skillAuditFile" }
    $skillAudit = Get-Content -LiteralPath $skillAuditFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $skillNames = @($skillAudit.skills | Where-Object { $IncludeUnsupported -or ([string]$_.afterRemediationStatus) -ne 'unsupported' } | ForEach-Object { [string]$_.id })
}

Write-Host "Agents target: $agentTarget"
foreach ($relativeAgent in $agentFiles) {
    $relative = ([string]$relativeAgent) -replace '/', '\'
    if (-not $relative.StartsWith('agents\')) { $relative = Join-Path 'agents' $relative }
    $source = Resolve-PackOwnedFile $relative
    $destination = Join-Path $agentTarget (Split-Path -Leaf $relative)
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing agent in pack: $relative" }
    Copy-OwnedFile $source $destination 'agent' | Out-Null
}

Write-Host "Skills target: $skillTarget"
foreach ($skillName in $skillNames) {
    $name = ([string]$skillName).Trim()
    if ($name -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') { throw "Invalid skill name in pack: $name" }
    $source = Join-Path (Join-Path $PackRoot 'skills') $name
    $destination = Join-Path $skillTarget $name
    if (-not (Test-Path -LiteralPath (Join-Path $source 'SKILL.md') -PathType Leaf)) { throw "Missing SKILL.md in pack: $name" }
    if (Test-Path -LiteralPath $destination) {
        if (-not $Overwrite) { Write-Warning "Skipped existing skill: $destination (use -Overwrite to update)."; continue }
        if (-not (Remove-OwnedSkillTree $destination)) {
            Write-Warning "Skipped unowned existing skill: $destination (the installer only overwrites a tree recorded in its ownership manifest)."
            continue
        }
    }
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    $installed.Add([pscustomobject]@{ kind = 'skill-tree'; path = $destination; sha256 = $null })
    foreach ($file in @(Get-ChildItem -LiteralPath $source -Recurse -File -Force)) {
        $relativeFile = $file.FullName.Substring($source.Length).TrimStart('\')
        $targetFile = Join-Path $destination $relativeFile
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $targetFile) | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $targetFile -Force
        $hash = (Get-FileHash -LiteralPath $targetFile -Algorithm SHA256).Hash.ToLowerInvariant()
        $installed.Add([pscustomobject]@{ kind = 'skill'; path = $targetFile; sha256 = $hash })
    }
    Write-Host "Installed skill tree: $destination"
}

$supportFiles = @(
    @{ source = 'mcp/start-feishu-mcp.ps1'; destination = (Join-Path $ownershipRoot 'bin\start-feishu-mcp.ps1'); kind = 'mcp-wrapper' },
    @{ source = 'sync-model-catalog.ps1'; destination = (Join-Path $ownershipRoot 'bin\sync-model-catalog.ps1'); kind = 'model-sync' },
    @{ source = 'model-catalog.json'; destination = (Join-Path $ownershipRoot 'model-catalog.json'); kind = 'model-catalog' },
    @{ source = 'render-codex-config.ps1'; destination = (Join-Path $ownershipRoot 'bin\render-codex-config.ps1'); kind = 'config-renderer' },
    @{ source = 'configure-codex.ps1'; destination = (Join-Path $ownershipRoot 'bin\configure-codex.ps1'); kind = 'config-merger' },
    @{ source = 'audit-codex-compatibility.py'; destination = (Join-Path $ownershipRoot 'bin\audit-codex-compatibility.py'); kind = 'compatibility-auditor' },
    @{ source = 'manifest/feishu-tools.json'; destination = (Join-Path $ownershipRoot 'manifest\feishu-tools.json'); kind = 'tool-contract' },
    @{ source = 'manifest/skill-audit.json'; destination = (Join-Path $ownershipRoot 'manifest\skill-audit.json'); kind = 'audit-contract' },
    @{ source = 'config-fragments/feishu-official-stdio.template.toml'; destination = (Join-Path $ownershipRoot 'config-fragments\feishu-official-stdio.template.toml'); kind = 'config-template' },
    @{ source = 'mcp/dashscope-web-search.template.env'; destination = (Join-Path $ownershipRoot 'connections\dashscope-web-search.template.env'); kind = 'connection-template' },
    @{ source = 'mcp/image-2.template.env'; destination = (Join-Path $ownershipRoot 'connections\image-2.template.env'); kind = 'connection-template' },
    @{ source = 'mcp/seedance.template.env'; destination = (Join-Path $ownershipRoot 'connections\seedance.template.env'); kind = 'connection-template' }
)
foreach ($supportFile in $supportFiles) {
    $source = Resolve-PackOwnedFile ([string]$supportFile.source)
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing support file in pack: $($supportFile.source)" }
    Copy-OwnedFile $source ([string]$supportFile.destination) ([string]$supportFile.kind) | Out-Null
}

function Test-CurrentOwnership([string]$Path, [string]$Kind) {
    foreach ($entry in $installed) {
        if ($entry.path -and ([string]$entry.path).Equals($Path, [StringComparison]::OrdinalIgnoreCase) -and ([string]$entry.kind) -eq $Kind) { return $true }
    }
    return $false
}

$feishuAppId = [Environment]::GetEnvironmentVariable('FEISHU_APP_ID', 'Process')
$feishuAppSecret = [Environment]::GetEnvironmentVariable('FEISHU_APP_SECRET', 'Process')
$feishuEnabled = if ($DisableFeishu) { $false } elseif ($EnableFeishu) { $true } else { (-not [string]::IsNullOrWhiteSpace($feishuAppId) -and $feishuAppId -match '^cli_[A-Za-z0-9_-]+$' -and -not [string]::IsNullOrWhiteSpace($feishuAppSecret) -and $feishuAppSecret.Length -ge 8) }

function Redact-Diagnostic([string]$Text) {
    $result = if ($null -eq $Text) { '' } else { [string]$Text }
    foreach ($name in @('FEISHU_APP_SECRET','FEISHU_USER_ACCESS_TOKEN','DASHSCOPE_API_KEY','IMAGE_2_API_KEY','SEEDANCE_API_KEY')) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (-not [string]::IsNullOrWhiteSpace($value)) { $result = $result.Replace($value, '<redacted>') }
    }
    $result = [regex]::Replace($result, '(?im)(app[_-]?secret|access[_-]?token|api[_-]?key)\s*[:=]\s*[^\r\n]+', '$1=<redacted>')
    $chars = $result.ToCharArray()
    if ($chars.Length -gt 16000) { $result = -join $chars[($chars.Length - 16000)..($chars.Length - 1)] }
    return $result
}

function Write-Diagnostic([string]$Path, [string]$Text) {
    $safe = Redact-Diagnostic $Text
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
    [IO.File]::WriteAllText($Path, $safe + [Environment]::NewLine, $utf8NoBom)
    return $safe
}

function Write-AsciiFile([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    # These files are command wrappers, not user-authored text. Use an
    # explicit encoding so Windows PowerShell 5.1 does not emit UTF-16 or a
    # malformed first line that cmd.exe interprets as an executable path.
    [IO.File]::WriteAllText($Path, $Text, [Text.Encoding]::ASCII)
}

function Test-RuntimeReady([object]$State) {
    if ([string]$State.status -ne 'ready' -or [string]$State.python.health -ne 'passed') { return $false }
    if ([string]$State.node.health -notin @('passed','skipped')) { return $false }
    return [string]$State.feishu.health -in @('passed','disabled','skipped')
}

$runtimeScript = Join-Path $PackRoot 'runtime\provision-runtime.ps1'
if (-not (Test-Path -LiteralPath $runtimeScript -PathType Leaf)) { throw "Runtime provisioner is missing from the pack: $runtimeScript" }
$powershellCommand = Get-Command powershell.exe -ErrorAction SilentlyContinue
if (-not $powershellCommand) { throw 'Windows PowerShell is required to provision the bundled Windows runtimes.' }
$diagnosticsRoot = Join-Path $ownershipRoot 'diagnostics'
$runtimeDiagnosticPath = Join-Path $diagnosticsRoot 'runtime-provision.log'
$runtimeOutput = ''
$runtimeState = [ordered]@{
    schemaVersion = 1
    status = 'failed'
    targetUserProfile = $userProfile
    python = [ordered]@{ executable = ''; baseInterpreter = ''; version = ''; wheelCount = 0; health = 'unknown' }
    node = [ordered]@{ executable = ''; version = ''; health = 'disabled' }
    feishu = [ordered]@{ packageRoot = ''; package = '@larksuiteoapi/lark-mcp@0.5.1'; health = if ($feishuEnabled) { 'unknown' } else { 'disabled' } }
    paths = [ordered]@{ kitHome = $ownershipRoot; runtimeRoot = (Join-Path $ownershipRoot 'runtime'); binRoot = (Join-Path $ownershipRoot 'bin') }
    secretsStored = $false
}
try {
    $runtimeArguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $runtimeScript, '-PackRoot', $PackRoot, '-TargetUserProfile', $userProfile)
    if ($feishuEnabled) { $runtimeArguments += '-EnableFeishu' }
    $runtimeOutput = (& $powershellCommand.Source @runtimeArguments 2>&1 | Out-String)
    $runtimeExitCode = $LASTEXITCODE
    $runtimeState = $runtimeOutput | ConvertFrom-Json
    if ($runtimeExitCode -ne 0) { throw "Bundled runtime provisioning failed with exit code $runtimeExitCode." }
    if (-not (Test-RuntimeReady $runtimeState)) { throw 'Bundled runtime provisioning returned a partial or unhealthy state.' }
} catch {
    $runtimeState.error = Redact-Diagnostic (($runtimeOutput + "`n" + $_.Exception.Message).Trim())
}
$runtimeDiagnostic = Write-Diagnostic $runtimeDiagnosticPath ($runtimeOutput + "`nstatus=" + [string]$runtimeState.status + "`nerror=" + [string]$runtimeState.error)
$runtimeStatePath = Join-Path $ownershipRoot 'runtime-state.json'
Write-JsonNoBom $runtimeStatePath $runtimeState 8

$runtimeBinFiles = @()
if ([string]$runtimeState.python.health -eq 'passed') {
    $newline = [Environment]::NewLine
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\python.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" %*' + $newline)
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\pip.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" -m pip %*' + $newline)
    $runtimeBinFiles += @((Join-Path $ownershipRoot 'bin\python.cmd'), (Join-Path $ownershipRoot 'bin\pip.cmd'))
}
if ([string]$runtimeState.node.health -eq 'passed') {
    $newline = [Environment]::NewLine
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\node.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v20.20.0-win-x64\node.exe" %*' + $newline)
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\npm.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v20.20.0-win-x64\node.exe" "%~dp0..\runtime\node-v20.20.0-win-x64\node_modules\npm\bin\npm-cli.js" %*' + $newline)
    $runtimeBinFiles += @((Join-Path $ownershipRoot 'bin\node.cmd'), (Join-Path $ownershipRoot 'bin\npm.cmd'))
}
foreach ($runtimeOwnedPath in @($runtimeStatePath) + $runtimeBinFiles) {
    if (Test-Path -LiteralPath $runtimeOwnedPath -PathType Leaf) {
        $runtimeHash = (Get-FileHash -LiteralPath $runtimeOwnedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if (-not (Test-CurrentOwnership $runtimeOwnedPath 'runtime')) { $installed.Add([pscustomobject]@{ kind = 'runtime'; path = $runtimeOwnedPath; sha256 = $runtimeHash }) }
    }
}

$wrapperTarget = Join-Path $ownershipRoot 'bin\start-feishu-mcp.ps1'
$rendererTarget = Join-Path $ownershipRoot 'bin\render-codex-config.ps1'
$generatedConfig = Join-Path $ownershipRoot 'config\codex-starter.generated.toml'
$feishuPackageCacheForRender = [string]$runtimeState.feishu.packageRoot
if ([string]::IsNullOrWhiteSpace($feishuPackageCacheForRender)) {
    # Windows PowerShell drops an empty item while expanding an argument array,
    # turning the following switch into a missing-argument error. The renderer
    # ignores this path while Feishu is disabled, but it still needs a stable,
    # non-empty placeholder for parameter binding.
    $feishuPackageCacheForRender = Join-Path $ownershipRoot 'runtime\feishu'
}
$renderArguments = @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PackRoot 'render-codex-config.ps1'),
    '-OutputPath', $generatedConfig,
    '-SkillRoot', $skillTarget,
    '-FeishuWrapperPath', $wrapperTarget,
    '-FeishuPackageCache', $feishuPackageCacheForRender,
    '-TargetUserProfile', $userProfile,
    '-FeishuAuthMode', $FeishuAuthMode,
    '-FeishuDomain', $FeishuDomain,
    '-Overwrite'
)
if (-not [string]::IsNullOrWhiteSpace([string]$runtimeState.python.executable)) {
    $renderArguments += @('-PythonExe', [string]$runtimeState.python.executable)
}
if ($feishuEnabled) { $renderArguments += '-EnableFeishu' }
if (@($EnableSkills).Count -gt 0) {
    $renderArguments += '-EnableSkills'
    $renderArguments += @($EnableSkills)
}
$configMerger = Join-Path $PackRoot 'configure-codex.ps1'
$configDiagnosticPath = Join-Path $diagnosticsRoot 'config-merge.log'
$configOutput = ''
$configStatus = [ordered]@{ status = 'partial'; merged = $false; reason = 'Configuration stage did not complete.' }
try {
    $renderOutput = & $powershellCommand.Source @renderArguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Codex configuration rendering failed." }
    $configOutput = $renderOutput
    $configArguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $configMerger,
        '-GeneratedFragment', $generatedConfig,
        '-TargetUserProfile', $userProfile,
        '-KitHome', $ownershipRoot
    )
    if (-not [string]::IsNullOrWhiteSpace([string]$runtimeState.python.executable)) {
        $configArguments += @('-PythonExe', [string]$runtimeState.python.executable)
    }
    $configOutput = & $powershellCommand.Source @configArguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Codex configuration merge failed." }
    $configStatus = $configOutput | ConvertFrom-Json
} catch {
    $configStatus.reason = Redact-Diagnostic (($configOutput + "`n" + $_.Exception.Message).Trim())
}
$configDiagnostic = Write-Diagnostic $configDiagnosticPath ($configOutput + "`nstatus=" + [string]$configStatus.status + "`nreason=" + [string]$configStatus.reason)
if (Test-Path -LiteralPath $generatedConfig -PathType Leaf) {
    $generatedHash = (Get-FileHash -LiteralPath $generatedConfig -Algorithm SHA256).Hash.ToLowerInvariant()
    if (-not (Test-CurrentOwnership $generatedConfig 'config')) { $installed.Add([pscustomobject]@{ kind = 'config'; path = $generatedConfig; sha256 = $generatedHash }) }
}
$modelSyncScript = Join-Path $PackRoot 'sync-model-catalog.ps1'
$modelSyncDiagnosticPath = Join-Path $diagnosticsRoot 'model-sync.log'
$modelSyncOutput = ''
$modelSyncStatus = [ordered]@{ schemaVersion = 1; status = 'failed'; migration = 'not-run'; preferredModel = ''; currentModel = $null; configChanged = $false; reason = 'Model sync did not complete.' }
try {
    $modelSyncArguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $modelSyncScript,
        '-PackRoot', $PackRoot,
        '-TargetUserProfile', $userProfile,
        '-KitHome', $ownershipRoot
    )
    if (-not [string]::IsNullOrWhiteSpace([string]$runtimeState.python.executable)) {
        $modelSyncArguments += @('-PythonExe', [string]$runtimeState.python.executable)
    }
    $modelSyncOutput = & $powershellCommand.Source @modelSyncArguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw 'Model catalog synchronization failed.' }
    $modelSyncStatus = $modelSyncOutput | ConvertFrom-Json
} catch {
    $modelSyncStatus.reason = Redact-Diagnostic (($modelSyncOutput + "`n" + $_.Exception.Message).Trim())
}
if ([string]$modelSyncStatus.status -ne 'ready' -and [string]$configStatus.status -eq 'merged') {
    $configStatus.status = 'partial'
    $configStatus.reason = 'Model catalog synchronization did not complete; core Skills remain installed.'
}
$modelSyncDiagnostic = Write-Diagnostic $modelSyncDiagnosticPath ($modelSyncOutput + "`nstatus=" + [string]$modelSyncStatus.status + "`nreason=" + [string]$modelSyncStatus.reason)
foreach ($modelOwnedPath in @((Join-Path $ownershipRoot 'model-catalog.json'), (Join-Path $ownershipRoot 'bin\sync-model-catalog.ps1'), $modelSyncDiagnosticPath)) {
    if (Test-Path -LiteralPath $modelOwnedPath -PathType Leaf) {
        $modelHash = (Get-FileHash -LiteralPath $modelOwnedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $modelKind = if ($modelOwnedPath -like '*model-catalog.json') { 'model-catalog' } elseif ($modelOwnedPath -like '*sync-model-catalog.ps1') { 'model-sync' } else { 'diagnostic' }
        if (-not (Test-CurrentOwnership $modelOwnedPath $modelKind)) { $installed.Add([pscustomobject]@{ kind = $modelKind; path = $modelOwnedPath; sha256 = $modelHash }) }
    }
}
$connectionStatusPath = Join-Path $ownershipRoot 'connection-status.json'
$connectionStatus = [ordered]@{
    schemaVersion = 1
    feishu = if ($feishuEnabled) { 'enabled-awaiting-tenant-health' } else { 'disabled-awaiting-credentials' }
    codexConfig = [string]$configStatus.status
    apiServices = [ordered]@{
        templatesInstalled = $true
        enabled = [ordered]@{
            'dashscope-web-search' = @($EnabledApiServices) -contains 'dashscope-web-search'
            'image-2' = @($EnabledApiServices) -contains 'image-2'
            'seedance' = @($EnabledApiServices) -contains 'seedance'
        }
        credentialsStored = $false
        note = 'DashScope, Image-2 and Seedance values remain user-provided environment variables.'
    }
    secretsStored = $false
}
Write-JsonNoBom $connectionStatusPath $connectionStatus 8
$connectionHash = (Get-FileHash -LiteralPath $connectionStatusPath -Algorithm SHA256).Hash.ToLowerInvariant()
if (-not (Test-CurrentOwnership $connectionStatusPath 'connection-status')) { $installed.Add([pscustomobject]@{ kind = 'connection-status'; path = $connectionStatusPath; sha256 = $connectionHash }) }

$installedAtUtc = [DateTime]::UtcNow.ToString('o')
$readinessFile = Join-Path $ownershipRoot 'readiness.json'
$installedSkillCount = @($installed | Where-Object {
        ([string]$_.kind) -eq 'skill-tree' -and
        ([string]$_.path).StartsWith(([IO.Path]::GetFullPath($skillTarget).TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)
    }).Count
$installedAgentCount = @($installed | Where-Object {
        ([string]$_.kind) -eq 'agent' -and
        ([string]$_.path).StartsWith(([IO.Path]::GetFullPath($agentTarget).TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)
    }).Count
$expectedSkillCount = @($skillNames).Count
$expectedAgentCount = @($agentFiles).Count
$coreReady = $installedSkillCount -ge $expectedSkillCount -and $installedAgentCount -ge $expectedAgentCount
$coreStatus = if ($coreReady) { 'ready' } else { 'partial' }
$runtimeReady = Test-RuntimeReady $runtimeState
$configReady = [string]$configStatus.status -notin @('failed', 'partial', 'conflict')
$modelReady = [string]$modelSyncStatus.status -eq 'ready'
$overallStatus = if (-not $coreReady) { 'failed' } elseif ($runtimeReady -and $configReady -and $modelReady) { 'ready' } else { 'partial' }
$diagnostics = [ordered]@{
    path = (Join-Path $diagnosticsRoot 'capability-pack.log')
    runtime = $runtimeDiagnosticPath
    config = $configDiagnosticPath
    modelSync = $modelSyncDiagnosticPath
}
$diagnosticText = @(
    "runtime status=$([string]$runtimeState.status)",
    "runtime health=$([string]$runtimeState.python.health)/$([string]$runtimeState.node.health)/$([string]$runtimeState.feishu.health)",
    "config status=$([string]$configStatus.status)",
    "model sync status=$([string]$modelSyncStatus.status) preferred=$([string]$modelSyncStatus.preferredModel) migration=$([string]$modelSyncStatus.migration)",
    "runtime log=$runtimeDiagnosticPath",
    "config log=$configDiagnosticPath"
) -join [Environment]::NewLine
$diagnosticPath = Write-Diagnostic ([string]$diagnostics.path) $diagnosticText
foreach ($diagnosticFile in @($runtimeDiagnosticPath, $configDiagnosticPath, $modelSyncDiagnosticPath, [string]$diagnostics.path)) {
    if (Test-Path -LiteralPath $diagnosticFile -PathType Leaf) {
        $diagnosticHash = (Get-FileHash -LiteralPath $diagnosticFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if (-not (Test-CurrentOwnership $diagnosticFile 'diagnostic')) { $installed.Add([pscustomobject]@{ kind = 'diagnostic'; path = $diagnosticFile; sha256 = $diagnosticHash }) }
    }
}
$readiness = [ordered]@{
    schemaVersion = 1
    pack = [ordered]@{
        id = [string]$pack.id
        version = [string]$pack.version
        sourceCommit = [string]$pack.sourceCommit
        sourceCommitRole = [string]$pack.sourceCommitRole
        sourceTreeSha256 = [string]$pack.sourceTreeSha256
    }
    installedAtUtc = $installedAtUtc
    targetUserProfile = $userProfile
    requested = [ordered]@{ includeNonDefault = [bool]$IncludeNonDefault; includeUnsupported = [bool]$IncludeUnsupported; overwrite = [bool]$Overwrite }
    core = [ordered]@{
        status = $coreStatus
        expectedSkills = $expectedSkillCount
        installedSkills = $installedSkillCount
        expectedAgents = $expectedAgentCount
        installedAgents = $installedAgentCount
    }
    installed = [ordered]@{
        skills = $installedSkillCount
        agents = $installedAgentCount
        feishuWrapper = Test-CurrentOwnership $wrapperTarget 'mcp-wrapper'
        configRenderer = Test-CurrentOwnership $rendererTarget 'config-renderer'
        modelCatalog = Test-CurrentOwnership (Join-Path $ownershipRoot 'model-catalog.json') 'model-catalog'
        modelSync = Test-CurrentOwnership (Join-Path $ownershipRoot 'bin\sync-model-catalog.ps1') 'model-sync'
    }
    runtime = $runtimeState
    config = $configStatus
    modelSync = $modelSyncStatus
    overallStatus = $overallStatus
    diagnostics = $diagnostics
    connections = $connectionStatus
    secretsStored = $false
}
$readinessTemp = Join-Path ([IO.Path]::GetTempPath()) ('codex-starter-readiness-' + [guid]::NewGuid().ToString('N') + '.json')
try {
    Write-JsonNoBom $readinessTemp $readiness 8
    Copy-OwnedFile $readinessTemp $readinessFile 'readiness' | Out-Null
} finally {
    if (Test-Path -LiteralPath $readinessTemp -PathType Leaf) { Remove-Item -LiteralPath $readinessTemp -Force }
}

$record = [ordered]@{
    schemaVersion = 4
    packId = [string]$pack.id
    packVersion = [string]$pack.version
    sourceCommit = [string]$pack.sourceCommit
    installedAtUtc = $installedAtUtc
    agentRoot = $agentTarget
    skillRoot = $skillTarget
    readinessFile = $readinessFile
    overwrite = [bool]$Overwrite
    files = @($installed)
    note = 'Ownership record contains package-owned paths and hashes only; it contains no credentials or user content.'
}
Write-JsonNoBom $ownershipFile $record 8
Write-Host "Ownership record: $ownershipFile"
Write-Host 'Done. Restart Codex or open a new task so the new agents and skills are reloaded.' -ForegroundColor Green
