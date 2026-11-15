param(
    [Parameter(Mandatory = $true)][string]$PackRoot,
    [Parameter(Mandatory = $true)][string]$TargetCodexHome
)

$ErrorActionPreference = 'Stop'
# The manager parses stdout as UTF-8 JSON, including non-ASCII user paths.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

$packFull = [IO.Path]::GetFullPath($PackRoot)
$codexHome = [IO.Path]::GetFullPath($TargetCodexHome)
$pluginName = 'product-design'
$marketplaceName = 'usb-product-design-offline'
$bundleRoot = Join-Path $packFull 'plugins\product-design-offline'
$pluginSource = Join-Path $bundleRoot "plugins\$pluginName"
$catalogSource = Join-Path $bundleRoot '.agents\plugins\marketplace.json'
$dependenciesArchive = Join-Path $packFull 'runtime\bundled\product-design-dependencies-win-x64.zip'
$manifestPath = Join-Path $pluginSource '.codex-plugin\plugin.json'

foreach ($required in @($pluginSource, $catalogSource, $manifestPath, $dependenciesArchive)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf) -and
        -not (Test-Path -LiteralPath $required -PathType Container)) {
        throw "Product Design offline bundle is incomplete: $required"
    }
}

$pluginManifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ([string]$pluginManifest.name -ne $pluginName -or [string]$pluginManifest.version -notmatch '^\d+\.\d+\.\d+$') {
    throw 'Product Design plugin manifest identity or version is invalid.'
}
$version = [string]$pluginManifest.version
$catalog = Get-Content -LiteralPath $catalogSource -Raw -Encoding UTF8 | ConvertFrom-Json
if ([string]$catalog.name -ne $marketplaceName -or
    @($catalog.plugins | Where-Object { [string]$_.name -eq $pluginName }).Count -ne 1) {
    throw 'Product Design offline marketplace manifest is invalid.'
}

$marketplaceRoot = Join-Path $codexHome "plugins\marketplaces\$marketplaceName"
$marketplacePlugin = Join-Path $marketplaceRoot "plugins\$pluginName"
$cachePlugin = Join-Path $codexHome "plugins\cache\$marketplaceName\$pluginName\$version"
$configPath = Join-Path $codexHome 'config.toml'
$beginMarker = '# BEGIN CODEX USB OFFLINE PRODUCT DESIGN v1'
$endMarker = '# END CODEX USB OFFLINE PRODUCT DESIGN v1'

function Assert-NoReparsePath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $cursor = [IO.Path]::GetPathRoot($full)
    $relative = $full.Substring($cursor.Length).TrimStart('\')
    foreach ($segment in @($relative.Split('\') | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })) {
        $cursor = Join-Path $cursor $segment
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Refusing to install Product Design through a symbolic link or junction: $cursor"
            }
        }
    }
}

Assert-NoReparsePath $pluginSource
Assert-NoReparsePath $codexHome
Assert-NoReparsePath $marketplaceRoot
Assert-NoReparsePath $cachePlugin
Assert-NoReparsePath $dependenciesArchive

$archiveHash = (Get-FileHash -LiteralPath $dependenciesArchive -Algorithm SHA256).Hash.ToLowerInvariant()
$runtimeRoot = Join-Path $codexHome "product-design-runtime\$version-$($archiveHash.Substring(0, 12))"
Assert-NoReparsePath $runtimeRoot
$installedRuntimeMarker = Join-Path $runtimeRoot '.installed-manifest.json'
if (Test-Path -LiteralPath $runtimeRoot) {
    if (-not (Test-Path -LiteralPath $installedRuntimeMarker -PathType Leaf)) {
        throw "Product Design runtime already exists without its installer marker: $runtimeRoot"
    }
    $runtimeMarker = Get-Content -LiteralPath $installedRuntimeMarker -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$runtimeMarker.pluginVersion -ne $version -or [string]$runtimeMarker.archiveSha256 -ne $archiveHash) {
        throw "Product Design runtime already exists with different contents: $runtimeRoot"
    }
    $installedDependencyManifestPath = Join-Path $runtimeRoot 'product-design-dependencies.json'
    if (-not (Test-Path -LiteralPath $installedDependencyManifestPath -PathType Leaf)) {
        throw "Product Design runtime manifest is missing: $runtimeRoot"
    }
    $installedDependencyManifest = Get-Content -LiteralPath $installedDependencyManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($templateName in @('prototype', 'mobile-app')) {
        $nodeModules = Join-Path $runtimeRoot "$templateName\node_modules"
        $expectedCount = [int]$installedDependencyManifest.templates.$templateName.nodeModulesFiles
        if (-not (Test-Path -LiteralPath $nodeModules -PathType Container) -or
            @(Get-ChildItem -LiteralPath $nodeModules -Recurse -File -Force).Count -ne $expectedCount) {
            throw "Product Design runtime dependencies are incomplete for template: $templateName"
        }
    }
} else {
    $runtimeStage = "$runtimeRoot.tmp-$([guid]::NewGuid().ToString('N'))"
    Assert-NoReparsePath $runtimeStage
    New-Item -ItemType Directory -Force -Path $runtimeStage | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($dependenciesArchive, $runtimeStage)

    $dependencyManifestPath = Join-Path $runtimeStage 'product-design-dependencies.json'
    if (-not (Test-Path -LiteralPath $dependencyManifestPath -PathType Leaf)) {
        throw 'Product Design dependency archive is missing its manifest.'
    }
    $dependencyManifest = Get-Content -LiteralPath $dependencyManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([int]$dependencyManifest.schemaVersion -ne 1 -or
        [string]$dependencyManifest.pluginVersion -ne $version -or
        [string]$dependencyManifest.platform -ne 'win32-x64' -or
        [string]$dependencyManifest.nodeVersion -ne '22.22.0') {
        throw 'Product Design dependency archive targets an incompatible plugin, platform, or Node runtime.'
    }

    foreach ($templateName in @('prototype', 'mobile-app')) {
        $templateSource = Join-Path $pluginSource "templates\$templateName"
        $nodeModules = Join-Path $runtimeStage "$templateName\node_modules"
        $expected = $dependencyManifest.templates.$templateName
        if (-not $expected -or -not (Test-Path -LiteralPath (Join-Path $templateSource 'package.json') -PathType Leaf) -or
            -not (Test-Path -LiteralPath (Join-Path $templateSource 'package-lock.json') -PathType Leaf) -or
            -not (Test-Path -LiteralPath $nodeModules -PathType Container)) {
            throw "Product Design dependency archive is incomplete for template: $templateName"
        }
        $packageHash = (Get-FileHash -LiteralPath (Join-Path $templateSource 'package.json') -Algorithm SHA256).Hash.ToLowerInvariant()
        $lockHash = (Get-FileHash -LiteralPath (Join-Path $templateSource 'package-lock.json') -Algorithm SHA256).Hash.ToLowerInvariant()
        $nodeFileCount = @(Get-ChildItem -LiteralPath $nodeModules -Recurse -File -Force).Count
        if ($packageHash -ne [string]$expected.packageJsonSha256 -or
            $lockHash -ne [string]$expected.packageLockSha256 -or
            $nodeFileCount -ne [int]$expected.nodeModulesFiles) {
            throw "Product Design bundled dependencies do not match the locked $templateName template."
        }
    }

    $stagedRuntimeMarker = Join-Path $runtimeStage '.installed-manifest.json'
    [ordered]@{
        schemaVersion = 1
        pluginVersion = $version
        platform = 'win32-x64'
        nodeVersion = '22.22.0'
        archiveSha256 = $archiveHash
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $stagedRuntimeMarker -Encoding UTF8
    Move-Item -LiteralPath $runtimeStage -Destination $runtimeRoot
}

New-Item -ItemType Directory -Force -Path $marketplacePlugin, $cachePlugin | Out-Null
New-Item -ItemType Directory -Force -Path $marketplaceRoot | Out-Null
foreach ($bundleItem in @(Get-ChildItem -LiteralPath $bundleRoot -Force)) {
    Copy-Item -LiteralPath $bundleItem.FullName -Destination $marketplaceRoot -Recurse -Force
}
foreach ($destination in @($marketplacePlugin, $cachePlugin)) {
    Assert-NoReparsePath $destination
    foreach ($file in @(Get-ChildItem -LiteralPath $pluginSource -Recurse -File -Force)) {
        $relative = $file.FullName.Substring($pluginSource.Length).TrimStart('\')
        $target = Join-Path $destination $relative
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $target -Force
        $sourceHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        $targetHash = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
        if ($sourceHash -ne $targetHash) { throw "Product Design plugin file verification failed: $relative" }
    }
}

$marketplaceManifest = Join-Path $marketplaceRoot '.agents\plugins\marketplace.json'
Assert-NoReparsePath $marketplaceManifest
if ((Get-FileHash -LiteralPath $catalogSource -Algorithm SHA256).Hash -ne
    (Get-FileHash -LiteralPath $marketplaceManifest -Algorithm SHA256).Hash) {
    throw 'Product Design offline marketplace verification failed.'
}

$quotedMarketplaceRoot = ConvertTo-Json -InputObject $marketplaceRoot -Compress
$managedConfig = @(
    $beginMarker,
    "[marketplaces.$marketplaceName]",
    'source_type = "local"',
    "source = $quotedMarketplaceRoot",
    "[plugins.`"$pluginName@$marketplaceName`"]",
    'enabled = true',
    $endMarker
) -join "`n"
$existingConfig = if (Test-Path -LiteralPath $configPath -PathType Leaf) {
    [IO.File]::ReadAllText($configPath, [Text.Encoding]::UTF8)
} else {
    ''
}
$beginIndex = $existingConfig.IndexOf($beginMarker, [StringComparison]::Ordinal)
$endIndex = $existingConfig.IndexOf($endMarker, [StringComparison]::Ordinal)
if (($beginIndex -ge 0) -xor ($endIndex -ge 0)) { throw 'Product Design managed Codex config markers are incomplete.' }
if ($beginIndex -ge 0 -and ($existingConfig.IndexOf($beginMarker, $beginIndex + $beginMarker.Length, [StringComparison]::Ordinal) -ge 0 -or
    $existingConfig.IndexOf($endMarker, $endIndex + $endMarker.Length, [StringComparison]::Ordinal) -ge 0)) {
    throw 'Product Design managed Codex config markers are duplicated.'
}
if ($beginIndex -ge 0) {
    if ($endIndex -lt $beginIndex) { throw 'Product Design managed Codex config markers are out of order.' }
    $endIndex += $endMarker.Length
    $prefix = $existingConfig.Substring(0, $beginIndex).TrimEnd("`r", "`n")
    $suffix = $existingConfig.Substring($endIndex).TrimStart("`r", "`n")
    $existingConfig = if ([string]::IsNullOrEmpty($suffix)) { "$prefix`n$managedConfig`n" } else { "$prefix`n$managedConfig`n`n$suffix" }
} else {
    if ($existingConfig -match '(?m)^\[marketplaces\.usb-product-design-offline\]\s*$' -or
        $existingConfig -match '(?m)^\[plugins\."product-design@usb-product-design-offline"\]\s*$') {
        throw 'Product Design Codex config already contains unmanaged marketplace settings.'
    }
    $existingConfig = if ([string]::IsNullOrWhiteSpace($existingConfig)) { "$managedConfig`n" } else { "$($existingConfig.TrimEnd("`r", "`n"))`n`n$managedConfig`n" }
}

New-Item -ItemType Directory -Force -Path $codexHome | Out-Null
Assert-NoReparsePath $configPath
$configTemp = "$configPath.tmp-$([guid]::NewGuid().ToString('N'))"
try {
    [IO.File]::WriteAllText($configTemp, $existingConfig, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $configTemp -Destination $configPath -Force
} finally {
    if (Test-Path -LiteralPath $configTemp -PathType Leaf) { Remove-Item -LiteralPath $configTemp -Force }
}

$installedFiles = @(Get-ChildItem -LiteralPath $cachePlugin -Recurse -File -Force)
$skillCount = @(Get-ChildItem -LiteralPath (Join-Path $cachePlugin 'skills') -Directory -Force | Where-Object {
    Test-Path -LiteralPath (Join-Path $_.FullName 'SKILL.md') -PathType Leaf
}).Count
if ($skillCount -ne 10 -or $installedFiles.Count -lt 1) {
    throw "Product Design plugin verification failed: expected 10 skills, found $skillCount."
}

[ordered]@{
    schemaVersion = 1
    status = 'ready'
    plugin = $pluginName
    version = $version
    marketplace = $marketplaceName
    enabled = $true
    skillCount = $skillCount
    fileCount = $installedFiles.Count
    runtimeReady = $true
    runtimePath = $runtimeRoot
    restartRequired = $true
    installedPath = $cachePlugin
} | ConvertTo-Json -Depth 4 -Compress
