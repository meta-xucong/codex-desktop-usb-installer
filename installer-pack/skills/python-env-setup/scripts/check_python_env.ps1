[CmdletBinding()]
param(
    [switch]$Json,
    [switch]$RepairPath,
    [switch]$Install
)

$ErrorActionPreference = 'Stop'
$checks = @()
# Keep the probe free of quote characters. Windows PowerShell 5.1 strips nested
# quotes when passing native-command arguments, so build the delimited output
# with chr(124) and pass only punctuation/identifiers to Python.
$probe = 'import platform,sys,sysconfig; v=sys.version_info; print(str(v[0])+chr(46)+str(v[1])+chr(46)+str(v[2])+chr(124)+chr(124).join(map(str,[sys.implementation.name,sys.implementation.cache_tag,sysconfig.get_platform(),platform.machine()])))'

function Test-PythonCommand([string]$Label, [string]$Command, [string[]]$Arguments) {
    $commandInfo = Get-Command $Command -ErrorAction SilentlyContinue
    if (-not $commandInfo) { return $null }
    try {
        $output = (& $commandInfo.Source @Arguments -c $probe 2>&1 | Out-String).Trim()
        $match = [regex]::Match($output, '(?m)^3\.12\.\d+\|cpython\|cpython-312\|win-amd64\|(AMD64|x86_64)$')
        if ($LASTEXITCODE -ne 0 -or -not $match.Success) { return $null }
        return [ordered]@{
            command = $Label
            path = $commandInfo.Source
            version = $output.Split('|')[0]
            abi = 'cp312'
            platform = 'win_amd64'
            architecture = 'x64'
        }
    } catch {
        return [ordered]@{ command = $Label; path = $commandInfo.Source; error = $_.Exception.Message }
    }
}

$checks += Test-PythonCommand 'python' 'python' @()
$checks += Test-PythonCommand 'py -3' 'py' @('-3')
$checks += Test-PythonCommand 'python3' 'python3' @()
$match = $checks | Where-Object { $_ -and $_.version -and $_.abi -eq 'cp312' -and $_.platform -eq 'win_amd64' } | Select-Object -First 1
$result = [ordered]@{
    available = [bool]$match
    required = [ordered]@{ majorMinor = '3.12'; abi = 'cp312'; platform = 'win_amd64'; architecture = 'x64' }
    match = $match
    checked = @($checks | Where-Object { $_ })
    pathChanged = $false
    message = if ($match) { 'CPython 3.12 win_amd64 (cp312) is available.' } else { 'No compatible CPython 3.12 win_amd64 (cp312) interpreter found; install the approved private 3.12.10 runtime.' }
}
if ($RepairPath) { $result.message += ' PATH repair is intentionally manual in this package.' }
if ($Install) { $result.message += ' This checker never downloads or executes an installer.' }
if ($Json) { $result | ConvertTo-Json -Depth 8 } else { $result.message }
if (-not $match) { exit 1 }
