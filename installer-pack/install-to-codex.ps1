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
    [string[]]$EnabledRemoteMcps = @(),
    [switch]$EnableFeishu,
    [switch]$DisableFeishu,
    [switch]$EnableMediaGeneration,
    [switch]$DisableMediaGeneration,
    [ValidateSet('not-checked','verified','failed')][string]$MediaGatewayCatalogStatus = 'not-checked',
    [ValidateRange(0,100000)][int]$MediaGatewayModelCount = 0,
    [string]$MediaGatewayCatalogVerifiedAt = ''
)

$ErrorActionPreference = 'Stop'
if ($IncludeUnsupported -and -not $IncludeNonDefault) { throw '-IncludeUnsupported requires -IncludeNonDefault.' }
if ($EnableFeishu -and $DisableFeishu) { throw '-EnableFeishu and -DisableFeishu are mutually exclusive.' }
if ($EnableMediaGeneration -and $DisableMediaGeneration) { throw '-EnableMediaGeneration and -DisableMediaGeneration are mutually exclusive.' }
if ($MediaGatewayCatalogStatus -eq 'verified' -and [string]::IsNullOrWhiteSpace($MediaGatewayCatalogVerifiedAt)) {
    throw 'A verified media gateway catalog must include its verification timestamp.'
}
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

$pack = Get-Content -LiteralPath $packFile -Raw -Encoding UTF8 | ConvertFrom-Json
if ([int]$pack.schemaVersion -ne 2) { throw "Unsupported capability pack schema: $($pack.schemaVersion)" }
$manifest = Get-Content -LiteralPath $fileManifest -Raw -Encoding UTF8 | ConvertFrom-Json
$packReparsePoints = @(Get-ChildItem -LiteralPath $PackRoot -Recurse -Force | Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 })
if ($packReparsePoints.Count -gt 0) { throw "Capability pack contains a symbolic link or junction: $($packReparsePoints[0].FullName)" }
function Resolve-PackOwnedFile([string]$RelativePath) {
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [IO.Path]::IsPathRooted($RelativePath) -or $RelativePath -match '(^|[\/])\.\.([\/]|$)') { throw "Unsafe pack path: $RelativePath" }
    $packRootFull = [IO.Path]::GetFullPath($PackRoot).TrimEnd('\') + '\'
    $candidate = [IO.Path]::GetFullPath((Join-Path $PackRoot ($RelativePath -replace '/', '\')))
    if (-not $candidate.StartsWith($packRootFull, [StringComparison]::OrdinalIgnoreCase)) { throw "Pack path escapes root: $RelativePath" }
    return $candidate
}
function Test-PackTransientPath([string]$RelativePath) {
    $segments = @($RelativePath -split '[\\/]')
    foreach ($segment in $segments) {
        if ($segment -in @('.git', '.pytest_cache', '__pycache__')) { return $true }
    }
    return ([IO.Path]::GetExtension($RelativePath) -ieq '.pyc')
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
$actualPackFiles = @(Get-ChildItem -LiteralPath $PackRoot -Recurse -File -Force | ForEach-Object { $_.FullName.Substring($PackRoot.Length).TrimStart('\').Replace('\','/') } | Where-Object { $_ -ne 'checksums.sha256' -and -not (Test-PackTransientPath $_) })
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
        # The ownership manifest contains the absolute user profile path.  It
        # can therefore contain non-ASCII characters (for example a Chinese
        # Windows account/folder name); Windows PowerShell 5.1 otherwise reads
        # the UTF-8 JSON using the legacy code page and rejects valid escapes.
        $previousRecord = Get-Content -LiteralPath $ownershipFile -Raw -Encoding UTF8 | ConvertFrom-Json
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

# The native USB installer selects API services directly, bypassing the PowerShell
# connection wizard. Keep each selected API usable by enabling its companion Skill.
$apiServicesFile = Join-Path $PackRoot 'manifest\api-services.json'
if (@($EnabledApiServices).Count -gt 0 -and (Test-Path -LiteralPath $apiServicesFile -PathType Leaf)) {
    $apiServiceManifest = Get-Content -LiteralPath $apiServicesFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $selectedApiSkills = @(
        $apiServiceManifest.services |
            Where-Object { @($EnabledApiServices) -contains [string]$_.id } |
            ForEach-Object { @($_.relatedSkills) }
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) }
    $skillNames = @(@($skillNames) + @($selectedApiSkills) | Select-Object -Unique)
}
if ($EnableMediaGeneration) {
    $mcpServicesFile = Join-Path $PackRoot 'manifest\mcp-servers.json'
    if (-not (Test-Path -LiteralPath $mcpServicesFile -PathType Leaf)) { throw "Missing MCP manifest in pack: $mcpServicesFile" }
    $mediaService = Get-Content -LiteralPath $mcpServicesFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $mediaSkills = @($mediaService.servers | Where-Object { [string]$_.id -eq 'media-generation' } | ForEach-Object { @($_.relatedSkills) })
    $skillNames = @(@($skillNames) + @($mediaSkills) | Select-Object -Unique)
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
        if (Test-PackTransientPath $relativeFile) { continue }
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
    @{ source = 'mcp/start-media-generation-mcp.ps1'; destination = (Join-Path $ownershipRoot 'bin\start-media-generation-mcp.ps1'); kind = 'mcp-wrapper' },
    @{ source = 'mcp/media-generation-mcp.py'; destination = (Join-Path $ownershipRoot 'bin\media-generation-mcp.py'); kind = 'mcp-server' },
    @{ source = 'sync-model-catalog.ps1'; destination = (Join-Path $ownershipRoot 'bin\sync-model-catalog.ps1'); kind = 'model-sync' },
    @{ source = 'model-catalog.json'; destination = (Join-Path $ownershipRoot 'model-catalog.json'); kind = 'model-catalog' },
    @{ source = 'render-codex-config.ps1'; destination = (Join-Path $ownershipRoot 'bin\render-codex-config.ps1'); kind = 'config-renderer' },
    @{ source = 'configure-codex.ps1'; destination = (Join-Path $ownershipRoot 'bin\configure-codex.ps1'); kind = 'config-merger' },
    @{ source = 'audit-codex-compatibility.py'; destination = (Join-Path $ownershipRoot 'bin\audit-codex-compatibility.py'); kind = 'compatibility-auditor' },
    @{ source = 'manifest/feishu-tools.json'; destination = (Join-Path $ownershipRoot 'manifest\feishu-tools.json'); kind = 'tool-contract' },
    @{ source = 'manifest/media-generation-tools.json'; destination = (Join-Path $ownershipRoot 'manifest\media-generation-tools.json'); kind = 'tool-contract' },
    @{ source = 'manifest/mcp-servers.json'; destination = (Join-Path $ownershipRoot 'manifest\mcp-servers.json'); kind = 'mcp-catalog' },
    @{ source = 'manifest/skill-audit.json'; destination = (Join-Path $ownershipRoot 'manifest\skill-audit.json'); kind = 'audit-contract' },
    @{ source = 'config-fragments/feishu-official-stdio.template.toml'; destination = (Join-Path $ownershipRoot 'config-fragments\feishu-official-stdio.template.toml'); kind = 'config-template' },
    @{ source = 'config-fragments/media-generation-stdio.template.toml'; destination = (Join-Path $ownershipRoot 'config-fragments\media-generation-stdio.template.toml'); kind = 'config-template' },
    @{ source = 'mcp/dashscope-web-search.template.env'; destination = (Join-Path $ownershipRoot 'connections\dashscope-web-search.template.env'); kind = 'connection-template' },
    @{ source = 'mcp/media-generation.template.env'; destination = (Join-Path $ownershipRoot 'connections\media-generation.template.env'); kind = 'connection-template' }
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
$mediaGenerationEnabled = -not [bool]$DisableMediaGeneration
if ($mediaGenerationEnabled) {
    $mediaProfile = [Environment]::GetEnvironmentVariable('MEDIA_GENERATION_API_PROFILE', 'Process')
    if ([string]::IsNullOrWhiteSpace($mediaProfile)) {
        [Environment]::SetEnvironmentVariable('MEDIA_GENERATION_API_PROFILE', 'responses', 'Process')
        $mediaProfile = 'responses'
    }
    if ($mediaProfile -notin @('auto','legacy','wokey','responses')) { throw 'Unsupported media API profile.' }
    # Installing the MCP server is independent from whether this upstream has
    # usable media routes today. Missing models / route metadata are reported by
    # media_capabilities at runtime; they must not block registration, because
    # the model catalog is allowed to be empty when the gateway has no match.
}

function Redact-Diagnostic([string]$Text) {
    $result = if ($null -eq $Text) { '' } else { [string]$Text }
    foreach ($name in @('FEISHU_APP_SECRET','FEISHU_USER_ACCESS_TOKEN','DASHSCOPE_API_KEY','MEDIA_GENERATION_API_KEY','MINIMAX_API_KEY')) {
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
    if ([string]$State.status -notin @('ready','partial') -or [string]$State.python.health -ne 'passed') { return $false }
    foreach ($key in @('manim','node','ffmpeg')) {
        $component = $State.PSObject.Properties[$key]
        if (-not $component -or [string]$component.Value.health -ne 'passed') { return $false }
    }
    $hyperframes = $State.PSObject.Properties['hyperframes']
    if (-not $hyperframes -or [string]$hyperframes.Value.cliHealth -notin @('passed','requires-network')) { return $false }
    if ([string]$hyperframes.Value.cliHealth -eq 'passed' -and [string]$hyperframes.Value.browserHealth -notin @('passed','requires-network')) { return $false }
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
    mediaGeneration = [ordered]@{
        health = if ($mediaGenerationEnabled) { 'unknown' } else { 'disabled' }
        server = 'media-generation-mcp.py'
        serverVersion = ''
        protocol = 'not-checked'
        gatewayCatalog = if ($mediaGenerationEnabled) { $MediaGatewayCatalogStatus } else { 'not-checked' }
        gatewayCatalogVerifiedAt = $null
        gatewayModelCount = 0
        generationCapability = 'not-tested'
    }
    paths = [ordered]@{ kitHome = $ownershipRoot; runtimeRoot = (Join-Path $ownershipRoot 'runtime'); binRoot = (Join-Path $ownershipRoot 'bin') }
    secretsStored = $false
}
try {
    $runtimeArguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', $runtimeScript, '-PackRoot', $PackRoot, '-TargetUserProfile', $userProfile)
if ($feishuEnabled) { $runtimeArguments += '-EnableFeishu' }
    $runtimeOutput = (& $powershellCommand.Source @runtimeArguments 2>&1 | Out-String)
    $runtimeExitCode = $LASTEXITCODE
    $runtimeState = $runtimeOutput | ConvertFrom-Json
    if ($runtimeExitCode -ne 0) { throw "Bundled runtime provisioning failed with exit code $runtimeExitCode." }
    if (-not (Test-RuntimeReady $runtimeState)) { throw 'Bundled runtime provisioning returned a partial or unhealthy state.' }
    $initialMediaHealth = if ($mediaGenerationEnabled) { 'unknown' } else { 'disabled' }
    $mediaRuntimeDefaults = [ordered]@{
        health = $initialMediaHealth
        server = 'media-generation-mcp.py'
        serverVersion = ''
        protocol = 'not-checked'
        gatewayCatalog = if ($mediaGenerationEnabled) { $MediaGatewayCatalogStatus } else { 'not-checked' }
        gatewayCatalogVerifiedAt = $null
        gatewayModelCount = 0
        generationCapability = 'not-tested'
    }
    if (-not $runtimeState.PSObject.Properties['mediaGeneration']) {
        $runtimeState | Add-Member -NotePropertyName mediaGeneration -NotePropertyValue ([pscustomobject]$mediaRuntimeDefaults)
    } else {
        foreach ($property in $mediaRuntimeDefaults.GetEnumerator()) {
            if (-not $runtimeState.mediaGeneration.PSObject.Properties[$property.Key]) {
                $runtimeState.mediaGeneration | Add-Member -NotePropertyName $property.Key -NotePropertyValue $property.Value
            }
        }
    }
    if (-not $runtimeState.PSObject.Properties['error']) { $runtimeState | Add-Member -NotePropertyName error -NotePropertyValue '' }
} catch {
    $runtimeState.error = Redact-Diagnostic (($runtimeOutput + "`n" + $_.Exception.Message).Trim())
}
$runtimeDiagnostic = Write-Diagnostic $runtimeDiagnosticPath ($runtimeOutput + "`nstatus=" + [string]$runtimeState.status + "`nerror=" + [string]$runtimeState.error)
$runtimeStatePath = Join-Path $ownershipRoot 'runtime-state.json'
Write-JsonNoBom $runtimeStatePath $runtimeState 8

$pdfSmokeScript = Join-Path $PackRoot 'skills\pdf-processing-toolkit\scripts\pdf_tool.py'
$pdfSmokeDiagnosticPath = Join-Path $diagnosticsRoot 'pdf-smoke.log'
$pdfSmokeOutput = ''
$pdfSmokeStatus = [ordered]@{ status = 'skipped'; script = $pdfSmokeScript; reason = 'Bundled Python runtime is not healthy.' }
if ([string]$runtimeState.python.health -eq 'passed' -and (Test-Path -LiteralPath $pdfSmokeScript -PathType Leaf)) {
    try {
        $pdfSmokeOutput = & ([string]$runtimeState.python.executable) $pdfSmokeScript smoke 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0 -or $pdfSmokeOutput -notmatch 'pdf smoke: passed') { throw 'PDF smoke test did not report passed.' }
        $pdfSmokeStatus = [ordered]@{ status = 'passed'; script = $pdfSmokeScript; reason = '' }
    } catch {
        $pdfSmokeStatus = [ordered]@{ status = 'failed'; script = $pdfSmokeScript; reason = Redact-Diagnostic (($pdfSmokeOutput + "`n" + $_.Exception.Message).Trim()) }
    }
} elseif (-not (Test-Path -LiteralPath $pdfSmokeScript -PathType Leaf)) {
    $pdfSmokeStatus = [ordered]@{ status = 'failed'; script = $pdfSmokeScript; reason = 'Bundled PDF smoke-test script is missing.' }
}
$pdfSmokeDiagnostic = Write-Diagnostic $pdfSmokeDiagnosticPath ($pdfSmokeOutput + "`nstatus=" + [string]$pdfSmokeStatus.status + "`nreason=" + [string]$pdfSmokeStatus.reason)

$runtimeBinFiles = @()
if ([string]$runtimeState.python.health -eq 'passed') {
    $newline = [Environment]::NewLine
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\python.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" %*' + $newline)
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\pip.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\python-env\Scripts\python.exe" -m pip %*' + $newline)
    $runtimeBinFiles += @((Join-Path $ownershipRoot 'bin\python.cmd'), (Join-Path $ownershipRoot 'bin\pip.cmd'))
}
if ([string]$runtimeState.node.health -eq 'passed') {
    $newline = [Environment]::NewLine
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\node.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v22.22.0-win-x64\node.exe" %*' + $newline)
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\npm.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v22.22.0-win-x64\node.exe" "%~dp0..\runtime\node-v22.22.0-win-x64\node_modules\npm\bin\npm-cli.js" %*' + $newline)
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\npx.cmd') ('@echo off' + $newline + '"%~dp0..\runtime\node-v22.22.0-win-x64\node.exe" "%~dp0..\runtime\node-v22.22.0-win-x64\node_modules\npm\bin\npx-cli.js" %*' + $newline)
    $runtimeBinFiles += @((Join-Path $ownershipRoot 'bin\node.cmd'), (Join-Path $ownershipRoot 'bin\npm.cmd'), (Join-Path $ownershipRoot 'bin\npx.cmd'))
}
if ([string]$runtimeState.manim.health -eq 'passed') {
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\manim.cmd') ('@echo off' + [Environment]::NewLine + '"%~dp0..\runtime\manim-env\Scripts\manim.exe" %*' + [Environment]::NewLine)
    $runtimeBinFiles += (Join-Path $ownershipRoot 'bin\manim.cmd')
}
if ([string]$runtimeState.ffmpeg.health -eq 'passed') {
    $newline = [Environment]::NewLine
    $ffmpegRel = 'ffmpeg\ffmpeg-n9.0-latest-win64-lgpl-shared-9.0\bin'
    foreach ($tool in @('ffmpeg','ffprobe','ffplay')) {
        Write-AsciiFile (Join-Path $ownershipRoot ("bin\{0}.cmd" -f $tool)) ('@echo off' + $newline + ('"%~dp0..\runtime\{0}\{1}.exe" %*' -f $ffmpegRel,$tool) + $newline)
        $runtimeBinFiles += (Join-Path $ownershipRoot ("bin\{0}.cmd" -f $tool))
    }
}
if ([string]$runtimeState.hyperframes.cliHealth -eq 'passed') {
    Write-AsciiFile (Join-Path $ownershipRoot 'bin\hyperframes.cmd') ('@echo off' + [Environment]::NewLine + 'call "%~dp0..\runtime\hyperframes\hyperframes.cmd" %*' + [Environment]::NewLine)
    $runtimeBinFiles += (Join-Path $ownershipRoot 'bin\hyperframes.cmd')
}
foreach ($runtimeOwnedPath in @($runtimeStatePath, $pdfSmokeDiagnosticPath) + $runtimeBinFiles) {
    if (Test-Path -LiteralPath $runtimeOwnedPath -PathType Leaf) {
        $runtimeHash = (Get-FileHash -LiteralPath $runtimeOwnedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if (-not (Test-CurrentOwnership $runtimeOwnedPath 'runtime')) { $installed.Add([pscustomobject]@{ kind = 'runtime'; path = $runtimeOwnedPath; sha256 = $runtimeHash }) }
    }
}

$wrapperTarget = Join-Path $ownershipRoot 'bin\start-feishu-mcp.ps1'
$mediaWrapperTarget = Join-Path $ownershipRoot 'bin\start-media-generation-mcp.ps1'
$mediaHealthStatus = [ordered]@{ status = if ($mediaGenerationEnabled) { 'not-checked' } else { 'disabled' }; runtime = 'not-checked'; protocol = 'not-checked'; enabled = [bool]$mediaGenerationEnabled; gatewayCatalog = if ($mediaGenerationEnabled) { $MediaGatewayCatalogStatus } else { 'not-checked' }; generationCapability = 'not-tested'; python = ''; serverVersion = ''; catalogVerifiedAt = if (-not [string]::IsNullOrWhiteSpace($MediaGatewayCatalogVerifiedAt)) { $MediaGatewayCatalogVerifiedAt } else { $null }; modelCount = if ($MediaGatewayCatalogStatus -eq 'verified') { $MediaGatewayModelCount } else { 0 } }
$mediaRegistrationReady = -not $mediaGenerationEnabled
$mediaStageLog = New-Object System.Collections.Generic.List[string]
if ($mediaGenerationEnabled) {
    $mediaServerTarget = Join-Path $ownershipRoot 'bin\media-generation-mcp.py'
    if (-not (Test-Path -LiteralPath $mediaWrapperTarget -PathType Leaf) -or -not (Test-Path -LiteralPath $mediaServerTarget -PathType Leaf)) {
        $mediaStageLog.Add('deploy=failed;reason=wrapper-or-server-missing')
        throw '媒体 MCP 部署失败：本机 bin 缺少启动脚本或服务文件。'
    }
    $mediaStageLog.Add('deploy=passed')
}
if ($mediaGenerationEnabled) {
    $mediaHealthOutput = ''
    try {
        # Parse only stdout: the wrapper's healthcheck emits one JSON document
        # there, while diagnostics belong on stderr and must never corrupt it.
        $mediaHealthOutput = & $powershellCommand.Source -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File $mediaWrapperTarget -HealthCheck | Out-String
        if ($LASTEXITCODE -ne 0) { throw 'The preinstalled Python media MCP health check failed.' }
        $mediaHealth = $mediaHealthOutput | ConvertFrom-Json
        if (-not $mediaHealth.ready -or [string]$mediaHealth.serverVersion -ne '1.3.9' -or [string]$mediaHealth.healthScope -ne 'local-runtime-protocol' -or [string]$mediaHealth.runtimeStatus -ne 'passed' -or [string]$mediaHealth.protocolStatus -ne 'passed' -or [int]$mediaHealth.toolCount -ne 6) { throw 'The preinstalled media MCP local runtime/protocol check failed.' }
        $mediaHealthStatus = [ordered]@{ status = 'protocol-passed'; runtime = [string]$mediaHealth.runtimeStatus; protocol = [string]$mediaHealth.protocolStatus; enabled = [bool]$mediaGenerationEnabled; gatewayCatalog = $MediaGatewayCatalogStatus; generationCapability = 'not-tested'; python = [string]$mediaHealth.python; serverVersion = [string]$mediaHealth.serverVersion; catalogVerifiedAt = if (-not [string]::IsNullOrWhiteSpace($MediaGatewayCatalogVerifiedAt)) { $MediaGatewayCatalogVerifiedAt } else { $null }; modelCount = if ($MediaGatewayCatalogStatus -eq 'verified') { $MediaGatewayModelCount } else { 0 } }
        $runtimeState.mediaGeneration.health = 'protocol-passed'
        $runtimeState.mediaGeneration.protocol = 'passed'
        $runtimeState.mediaGeneration.gatewayCatalog = $MediaGatewayCatalogStatus
        $runtimeState.mediaGeneration.gatewayCatalogVerifiedAt = $mediaHealthStatus.catalogVerifiedAt
        $runtimeState.mediaGeneration.gatewayModelCount = $mediaHealthStatus.modelCount
        $runtimeState.mediaGeneration.generationCapability = 'not-tested'
        $runtimeState.mediaGeneration.serverVersion = [string]$mediaHealth.serverVersion
        $mediaStageLog.Add('protocol_health=passed')
        Write-JsonNoBom $runtimeStatePath $runtimeState 8
    } catch {
        $mediaHealthStatus.status = 'failed'
        $mediaHealthOutput = Redact-Diagnostic (($mediaHealthOutput + "`n" + $_.Exception.Message).Trim())
        $mediaHealthDiagnosticPath = Join-Path $diagnosticsRoot 'media-generation-health.log'
        [void](Write-Diagnostic $mediaHealthDiagnosticPath ($mediaHealthOutput + "`nstatus=failed"))
        throw "Bundled Python media MCP is not ready: $mediaHealthOutput"
    }
}
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
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', (Join-Path $PackRoot 'render-codex-config.ps1'),
    '-OutputPath', $generatedConfig,
    '-SkillRoot', $skillTarget,
    '-FeishuWrapperPath', $wrapperTarget,
    '-MediaMcpWrapperPath', $mediaWrapperTarget,
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
if ($mediaGenerationEnabled) { $renderArguments += '-EnableMediaGeneration' }
if (@($EnabledRemoteMcps).Count -gt 0) {
    $renderArguments += '-EnableRemoteMcps'
    $renderArguments += @($EnabledRemoteMcps)
}
if (@($EnableSkills).Count -gt 0) {
    $renderArguments += '-EnableSkills'
    $renderArguments += @($EnableSkills)
}
$configMerger = Join-Path $PackRoot 'configure-codex.ps1'
$configDiagnosticPath = Join-Path $diagnosticsRoot 'config-merge.log'
$configOutput = ''
$renderStatus = [ordered]@{ status = 'not-run'; totalSkills = 0; enabledSkillCount = 0; disabledSkillCount = 0; enabledSkills = @(); disabledSkills = @(); reason = 'Configuration renderer did not complete.' }
$configStatus = [ordered]@{ status = 'partial'; merged = $false; reason = 'Configuration stage did not complete.' }
try {
    $renderOutput = & $powershellCommand.Source @renderArguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Codex configuration rendering failed." }
    $renderStatus = $renderOutput | ConvertFrom-Json
    $configOutput = $renderOutput
    $configArguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', $configMerger,
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
    if ($mediaGenerationEnabled) {
        if ([string]$configStatus.status -ne 'merged') { throw "媒体 MCP 配置未合并：$([string]$configStatus.status)" }
        $codexConfigPath = Join-Path $userProfile '.codex\config.toml'
        if (-not (Test-Path -LiteralPath $codexConfigPath -PathType Leaf)) { throw 'Codex config.toml 不存在。' }
        $configText = Get-Content -LiteralPath $codexConfigPath -Raw -Encoding UTF8
        $mediaTable = [regex]::Match($configText, '(?ms)^\[mcp_servers\.media_generation\][ \t]*\r?\n(?<body>.*?)(?=^\[[^\[]|\z)')
        $expectedWrapperPath = [IO.Path]::GetFullPath($mediaWrapperTarget)
        $expectedWrapperToml = '"' + $expectedWrapperPath.Replace('\', '\\').Replace('"', '\"') + '"'
        if (-not $mediaTable.Success -or
            $mediaTable.Groups['body'].Value -notmatch '(?m)^enabled\s*=\s*true\s*$' -or
            -not $mediaTable.Groups['body'].Value.Contains($expectedWrapperToml)) {
            throw 'Codex config.toml 中未找到启用状态正确且指向本机包装器的 media_generation 配置。'
        }
        $mediaRegistrationReady = $true
        $mediaStageLog.Add('codex_registration=passed')
    }
} catch {
    if ([string]$renderStatus.status -eq 'not-run') {
        $renderStatus.status = 'failed'
        $renderStatus.reason = Redact-Diagnostic (($configOutput + "`n" + $_.Exception.Message).Trim())
    }
    $configStatus.reason = Redact-Diagnostic (($configOutput + "`n" + $_.Exception.Message).Trim())
    if ($mediaGenerationEnabled) {
        $mediaRegistrationReady = $false
        $mediaStageLog.Add(('codex_registration=failed;reason=' + (Redact-Diagnostic $_.Exception.Message)))
    }
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
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', $modelSyncScript,
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
$mediaReady = -not $mediaGenerationEnabled -or ($mediaRegistrationReady -and [string]$mediaHealthStatus.runtime -eq 'passed' -and [string]$mediaHealthStatus.protocol -eq 'passed' -and (Test-Path -LiteralPath $mediaWrapperTarget -PathType Leaf) -and (Test-Path -LiteralPath (Join-Path $ownershipRoot 'bin\media-generation-mcp.py') -PathType Leaf))
$mediaRegistered = [bool]($mediaGenerationEnabled -and $mediaReady)
if ($mediaGenerationEnabled -and -not $mediaReady) { $mediaStageLog.Add('acceptance=failed') }
$codexRunning = @(Get-Process -Name 'Codex','codex' -ErrorAction SilentlyContinue).Count -gt 0
$localMcpsEnabled = @()
if ($feishuEnabled) { $localMcpsEnabled += 'feishu' }
if ($mediaGenerationEnabled) { $localMcpsEnabled += 'media-generation' }
$connectionStatus = [ordered]@{
    schemaVersion = 1
    feishu = if ($feishuEnabled) { 'enabled-awaiting-tenant-health' } else { 'disabled-awaiting-credentials' }
    mediaGeneration = [ordered]@{
        enabled = [bool]$mediaGenerationEnabled
        runtime = [string]$mediaHealthStatus.runtime
        protocol = [string]$mediaHealthStatus.protocol
        registered = $mediaRegistered
        gatewayCatalog = [string]$mediaHealthStatus.gatewayCatalog
        generationCapability = [string]$mediaHealthStatus.generationCapability
        serverVersion = [string]$mediaHealthStatus.serverVersion
        gatewayCatalogVerifiedAt = $mediaHealthStatus.catalogVerifiedAt
        gatewayModelCount = [int]$mediaHealthStatus.modelCount
    }
    codex = [ordered]@{ restartRequired = [bool]$mediaGenerationEnabled; running = [bool]$codexRunning }
    codexConfig = [string]$configStatus.status
    apiServices = [ordered]@{
        templatesInstalled = $true
        enabled = [ordered]@{
            'dashscope-web-search' = @($EnabledApiServices) -contains 'dashscope-web-search'
        }
        credentialsStored = $false
        remoteMcpsEnabled = @($EnabledRemoteMcps)
        localMcpsEnabled = $localMcpsEnabled
        note = 'The media MCP receives the signed USB license credential only in the Codex launch environment; media credentials are not written to user environment variables or Codex config. Remote MCP OAuth authorization is handled by Codex.'
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
$pdfReady = [string]$pdfSmokeStatus.status -eq 'passed'
$configReady = [string]$configStatus.status -notin @('failed', 'partial', 'conflict')
$modelReady = [string]$modelSyncStatus.status -eq 'ready'
$overallStatus = if (-not $coreReady -or ($mediaGenerationEnabled -and -not $mediaReady)) { 'failed' } elseif ($runtimeReady -and [string]$runtimeState.status -eq 'ready' -and $pdfReady -and $configReady -and $modelReady) { 'ready' } else { 'partial' }
$diagnostics = [ordered]@{
    path = (Join-Path $diagnosticsRoot 'capability-pack.log')
    runtime = $runtimeDiagnosticPath
    pdfSmoke = $pdfSmokeDiagnosticPath
    config = $configDiagnosticPath
    modelSync = $modelSyncDiagnosticPath
}
$diagnosticText = @(
    "runtime status=$([string]$runtimeState.status)",
    "runtime health=python:$([string]$runtimeState.python.health),manim:$([string]$runtimeState.manim.health),node:$([string]$runtimeState.node.health),ffmpeg:$([string]$runtimeState.ffmpeg.health),hyperframes-cli:$([string]$runtimeState.hyperframes.cliHealth),hyperframes-browser:$([string]$runtimeState.hyperframes.browserHealth),feishu:$([string]$runtimeState.feishu.health)",
    "pdf smoke status=$([string]$pdfSmokeStatus.status)",
    "config status=$([string]$configStatus.status)",
    "model sync status=$([string]$modelSyncStatus.status) preferred=$([string]$modelSyncStatus.preferredModel) migration=$([string]$modelSyncStatus.migration)",
    "runtime log=$runtimeDiagnosticPath",
    "config log=$configDiagnosticPath",
    "media registration=$mediaRegistrationReady",
    "media stages=$($mediaStageLog -join ',')"
) -join [Environment]::NewLine
$diagnosticPath = Write-Diagnostic ([string]$diagnostics.path) $diagnosticText
foreach ($diagnosticFile in @($runtimeDiagnosticPath, $pdfSmokeDiagnosticPath, $configDiagnosticPath, $modelSyncDiagnosticPath, [string]$diagnostics.path)) {
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
    pdf = $pdfSmokeStatus
    config = $configStatus
    skillActivation = $renderStatus
    modelSync = $modelSyncStatus
    overallStatus = $overallStatus
    diagnostics = $diagnostics
    connections = $connectionStatus
    codex = [ordered]@{ restartRequired = [bool]$mediaGenerationEnabled; running = [bool]$codexRunning }
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
if ($mediaGenerationEnabled -and -not $mediaReady) {
    throw "媒体 MCP 预装未通过验收；安装已标记失败。诊断文件：$($diagnostics.path)"
}
Write-Host 'Done. Restart Codex or open a new task so the new agents and skills are reloaded.' -ForegroundColor Green
