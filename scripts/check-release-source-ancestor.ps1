[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string] $Repo = "release-source",
    [Parameter(Position = 1)]
    [string] $ReleaseSourceSha = "",
    [Parameter(Position = 2)]
    [string] $DefaultBranch = ""
)

$ErrorActionPreference = "Stop"

if ($ReleaseSourceSha -notmatch '^[0-9a-f]{40}$') {
    Write-Error "release source must be a lowercase 40-character commit SHA"
    exit 1
}
if ([string]::IsNullOrWhiteSpace($DefaultBranch)) {
    Write-Error "default branch is missing"
    exit 1
}

& git -C $Repo check-ref-format --branch $DefaultBranch *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Error "invalid default branch name"
    exit 1
}

$remoteRef = "refs/remotes/origin/$DefaultBranch"
& git -C $Repo fetch --no-tags origin "+refs/heads/${DefaultBranch}:$remoteRef"
if ($LASTEXITCODE -ne 0) {
    Write-Error "could not fetch origin/$DefaultBranch"
    exit 1
}

& git -C $Repo cat-file -e "$ReleaseSourceSha^{commit}" 2> $null
if ($LASTEXITCODE -ne 0) {
    Write-Error "release source commit is missing after fetching the default branch"
    exit 1
}

& git -C $Repo merge-base --is-ancestor $ReleaseSourceSha $remoteRef
$status = $LASTEXITCODE
if ($status -eq 0) {
    Write-Output "Release source $ReleaseSourceSha is merged into origin/$DefaultBranch"
    exit 0
}
if ($status -eq 1) {
    Write-Error "release source $ReleaseSourceSha is not an ancestor of origin/$DefaultBranch"
} else {
    Write-Error "could not verify release source ancestry (git merge-base exit $status)"
}
exit 1
