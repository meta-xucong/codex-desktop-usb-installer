# Read-only resolution of the exact package's system-store junction. This does
# not create/remove links, change ACLs, infer ownership, or accept a missing
# target as a staged package. AppX identity/volume/health gates remain separate.
function Get-CodexSystemPackageStore {
  $programFiles = $env:ProgramW6432
  if ([string]::IsNullOrWhiteSpace($programFiles)) { $programFiles = $env:ProgramFiles }
  if ([string]::IsNullOrWhiteSpace($programFiles)) { throw 'System Program Files is unavailable' }
  return Join-Path $programFiles 'WindowsApps'
}
function ConvertTo-CodexPackagePath([string]$Path) {
  $path = $Path.Trim().TrimEnd('\')
  if ($path.StartsWith('\\?\') -or $path.StartsWith('\??\')) { $path = $path.Substring(4) }
  if ($path -notmatch '^[A-Za-z]:\\' -or $path.Substring(2).Contains(':') -or
      $path -match '(^|\\)\.\.?($|\\)') { throw 'Package path is not an unambiguous local drive path' }
  return $path
}
function Get-CodexPackagePathInfo([string]$Path, [string]$FullName, [string]$ExpectedStore = '') {
  $info = @{state='direct'; originalPath=$Path; effectivePath=$null; target=$null; error=$null}
  try {
    $location = ConvertTo-CodexPackagePath $Path
    $info.effectivePath = $location
    if ($FullName -notmatch '^OpenAI\.Codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0$') { return $info }
    $systemEntry = Join-Path (Get-CodexSystemPackageStore) $FullName
    if ($location -ine $systemEntry) { return $info }
    $entry = Get-Item -LiteralPath $location -Force -ErrorAction Stop
    if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) { return $info }
    $info.effectivePath = $null
    $info.state = 'rejected-system-link'
    if (-not $entry.PSIsContainer -or [string]$entry.LinkType -ne 'Junction' -or @($entry.Target).Count -ne 1) { throw 'System package entry is not a single directory junction' }
    $target = ConvertTo-CodexPackagePath ([string]@($entry.Target)[0])
    $info.target = $target
    if ([string]::IsNullOrWhiteSpace($ExpectedStore)) {
      if ($target -notmatch '^[A-Za-z]:\\WindowsApps\\[^\\]+$') { throw 'Junction does not target an external AppX package store' }
      $ExpectedStore = Split-Path -Parent $target
    }
    $expected = Join-Path $ExpectedStore $FullName
    if ($target -ine $expected) { throw 'System package junction target differs from the exact expected package path' }
    # A single hop only. The validated drive/WindowsApps store and package leaf
    # must not redirect to another volume or unrelated package.
    foreach ($path in @($ExpectedStore, $target)) {
      $item = Get-Item -LiteralPath $path -Force -ErrorAction Stop
      if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Package junction target/store is not a plain directory' }
    }
    $again = Get-Item -LiteralPath $location -Force -ErrorAction Stop
    if ([string]$again.LinkType -ne 'Junction' -or @($again.Target).Count -ne 1 -or
        (ConvertTo-CodexPackagePath ([string]@($again.Target)[0])) -ine $target) { throw 'Package junction changed during observation' }
    $info.state = 'verified-system-junction'
    $info.effectivePath = $target
  } catch {
    $info.effectivePath = $null
    if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) {
      $info.state = if ($info.target) { 'dangling-system-junction' } else { 'path-absent' }
    } elseif ($info.state -eq 'direct') { $info.state = 'path-query-failed' }
    $info.error = [string]$_.Exception.Message
  }
  return $info
}

function Get-CodexPackageProcessRoots([string]$Path, [string]$FullName, [string]$ExpectedStore = '') {
  $info = Get-CodexPackagePathInfo $Path $FullName $ExpectedStore
  if ([string]::IsNullOrWhiteSpace([string]$info.effectivePath)) { throw ('Cannot resolve exact package process roots: ' + ($info | ConvertTo-Json -Compress)) }
  return @((ConvertTo-CodexPackagePath $Path), [string]$info.effectivePath) | Select-Object -Unique
}
function Test-CodexProcessPathUnderRoots([string]$Path, $Roots) {
  try { $path = ConvertTo-CodexPackagePath $Path } catch { return $false }
  foreach ($root in @($Roots)) {
    if ($path.Equals([string]$root, [StringComparison]::OrdinalIgnoreCase) -or
        $path.StartsWith(([string]$root + '\'), [StringComparison]::OrdinalIgnoreCase)) { return $true }
  }
  return $false
}

# A missing AppX registration/volume is not proof that Windows removed its
# exact system-store entry. Never recursively inspect or manually delete it.
function Get-CodexSystemPackageEntryAudit([string]$FullName) {
  if ($FullName -notmatch '^OpenAI\.Codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0$') { throw 'Invalid exact Codex package identity for system entry audit' }
  $path = Join-Path (Get-CodexSystemPackageStore) $FullName
  try { $entry = Get-Item -LiteralPath $path -Force -ErrorAction Stop }
  catch {
    if ($_.CategoryInfo.Category -eq [Management.Automation.ErrorCategory]::ObjectNotFound) {
      return @{status='absent'; message="Exact system package entry is absent: $path"}
    }
    return @{status='pending'; message="System package entry query failed; absence is unproven: $path; $($_.Exception.Message)"}
  }
  $evidence = @{path=$path; attributes=[string]$entry.Attributes; linkType=[string]$entry.LinkType; target=@($entry.Target)} | ConvertTo-Json -Compress
  return @{status='pending'; message="Windows-managed package entry remains after AppX cleanup; recovery evidence retained, no ACL change or manual link deletion: $evidence"}
}
