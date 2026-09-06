[CmdletBinding()]
param(
    [string]$ArtifactDir = $PSScriptRoot,
    [string]$ApiKey,
    [switch]$PromptForApiKey,
    [switch]$SkipNetwork
)

$ErrorActionPreference = "Stop"

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw "验证失败：$Message"
    }
}

$ArtifactDir = (Resolve-Path -LiteralPath $ArtifactDir).Path
$exePath = Join-Path $ArtifactDir "CodexSwitch.exe"
$manifestPath = Join-Path $ArtifactDir "manifest.json"
$catalogPath = Join-Path $ArtifactDir "providers.json"

Assert-Condition (Test-Path -LiteralPath $exePath) "缺少 CodexSwitch.exe"
Assert-Condition (Test-Path -LiteralPath $manifestPath) "缺少 manifest.json"
Assert-Condition (Test-Path -LiteralPath $catalogPath) "缺少 providers.json"

 # Windows PowerShell 5.1's Get-Content uses the active ANSI code page for
 # UTF-8 files without a BOM. Read release metadata explicitly as UTF-8 so a
 # Chinese provider name cannot corrupt JSON parsing on a blank machine.
 $utf8 = [System.Text.UTF8Encoding]::new($false, $true)
 $manifest = [System.IO.File]::ReadAllText($manifestPath, $utf8) | ConvertFrom-Json
 $catalog = [System.IO.File]::ReadAllText($catalogPath, $utf8) | ConvertFrom-Json
$provider = @($catalog.providers) | Where-Object { $_.id -eq "404token-sub2api" } | Select-Object -First 1

Assert-Condition ($null -ne $provider) "providers.json 中缺少 404token-sub2api"
Assert-Condition ($provider.baseUrl -eq "https://404token.xyz/v1") "Base URL 不正确"
Assert-Condition ($provider.modelsUrl -eq "https://404token.xyz/v1/models") "模型目录 URL 不正确"
Assert-Condition ($provider.protocol -eq "responses") "协议必须为 Responses"
Assert-Condition ($manifest.secretsIncluded -eq $false) "发布物不应包含密钥"

$sha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToUpperInvariant()
Assert-Condition ($sha256 -eq ([string]$manifest.artifact.sha256).ToUpperInvariant()) "EXE SHA256 与 manifest 不一致"

$binaryText = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($exePath))
Assert-Condition ($binaryText.Contains("404token.xyz/v1")) "EXE 未包含 404TOKEN 端点"
Assert-Condition (-not $binaryText.Contains("127.0.0.1:8080")) "EXE 不应包含旧本地代理地址"
# A release binary built without Tauri's custom-protocol feature falls back to
# devUrl (localhost:3000) on a blank machine.  The compiled renderer asset
# path is a stable marker that the offline frontend was embedded.
Assert-Condition ($binaryText.Contains("assets/index-")) "EXE 未包含打包后的 renderer 资源；请使用 --features custom-protocol 重新构建"

Write-Host "静态验证通过：$exePath"
Write-Host "SHA256: $sha256"

if (-not $SkipNetwork) {
    if ($PromptForApiKey) {
        $secureKey = Read-Host -Prompt "输入临时验证用 API Key（不会写入文件）" -AsSecureString
        $keyPtr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureKey)
        try {
            $ApiKey = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($keyPtr)
        } finally {
            [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($keyPtr)
        }
    }
    $health = Invoke-WebRequest -Uri "https://404token.xyz/health" -UseBasicParsing -TimeoutSec 20
    Assert-Condition ($health.StatusCode -eq 200) "404TOKEN health 检查未返回 200"
    Write-Host "网络验证通过：404TOKEN /health 返回 $($health.StatusCode)"

    if ($ApiKey) {
        $headers = @{ Authorization = "Bearer $ApiKey" }
        $models = Invoke-WebRequest -Uri "https://404token.xyz/v1/models" -Headers $headers -UseBasicParsing -TimeoutSec 30
        Assert-Condition ($models.StatusCode -eq 200) "携带 API Key 的 /v1/models 未返回 200"
        Write-Host "API Key 验证通过：/v1/models 返回 $($models.StatusCode)"
    } else {
        try {
            Invoke-WebRequest -Uri "https://404token.xyz/v1/models" -UseBasicParsing -TimeoutSec 20 | Out-Null
            throw "未携带 API Key 的 /v1/models 意外返回成功"
        } catch {
            if ($_.Exception.Response -and $_.Exception.Response.StatusCode.value__ -eq 401) {
                Write-Host "网络验证通过：未携带 API Key 的 /v1/models 正确返回 401"
            } else {
                throw
            }
        }
    }
}

Write-Host "全部验证通过。"
