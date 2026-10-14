# Claude Desktop MCP Configuration Script
# Purpose: Configure Filesystem + Shell MCP to enable Windows native environment access

[CmdletBinding()]
param(
    [switch]$ShellOnly,
    [switch]$FilesystemOnly,
    [string[]]$AdditionalPaths
)

$ErrorActionPreference = "Stop"

Write-Host "================================================" -ForegroundColor Cyan
Write-Host "Claude Desktop MCP Configuration Script" -ForegroundColor Cyan
Write-Host "================================================" -ForegroundColor Cyan
Write-Host ""

# Config file path
$configDir = "$env:APPDATA\Claude"
$configPath = "$configDir\claude_desktop_config.json"

# Ensure config directory exists
if (-not (Test-Path $configDir)) {
    Write-Host "[Step 1/4] Creating config directory..." -ForegroundColor Green
    New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    Write-Host "✓ Config directory created: $configDir" -ForegroundColor Green
} else {
    Write-Host "[Step 1/4] Config directory exists" -ForegroundColor Yellow
}
Write-Host ""

# Check existing config
$existingConfig = $null
$backupCreated = $false

if (Test-Path $configPath) {
    Write-Host "[Step 2/4] Checking existing config..." -ForegroundColor Green
    try {
        $existingConfig = Get-Content $configPath -Raw | ConvertFrom-Json
        Write-Host "✓ Found existing config file" -ForegroundColor Yellow

        # Backup existing config
        $backupPath = "$configPath.backup.$(Get-Date -Format 'yyyyMMdd-HHmmss')"
        Copy-Item $configPath $backupPath
        Write-Host "✓ Backed up to: $backupPath" -ForegroundColor Green
        $backupCreated = $true
    } catch {
        Write-Warning "Existing config file is malformed, will be overwritten"
        $existingConfig = $null
    }
} else {
    Write-Host "[Step 2/4] No existing config found" -ForegroundColor Yellow
}
Write-Host ""

# Prepare new config
Write-Host "[Step 3/4] Generating new config..." -ForegroundColor Green

# Default filesystem paths
$defaultPaths = @(
    "D:\\AI\\AGENT_U盘\\codex-desktop-usb-installer",
    "D:\\AI\\AGENT_U盘\\release",
    "C:\\Users\\T14S\\Desktop",
    "C:\\Users\\T14S\\Documents",
    "C:\\Users\\T14S\\Downloads"
)

# Merge additional paths
if ($AdditionalPaths) {
    $allPaths = $defaultPaths + ($AdditionalPaths | ForEach-Object { $_.Replace('\', '\\') })
} else {
    $allPaths = $defaultPaths
}

# Build config object
$newConfig = @{
    mcpServers = @{}
}

# Add Filesystem MCP
if (-not $ShellOnly) {
    $newConfig.mcpServers["filesystem"] = @{
        command = "npx"
        args = @("-y", "@modelcontextprotocol/server-filesystem") + $allPaths
    }
    Write-Host "✓ Configured Filesystem MCP" -ForegroundColor Green
    Write-Host "  Access paths:" -ForegroundColor Gray
    foreach ($path in $allPaths) {
        Write-Host "    - $($path.Replace('\\', '\'))" -ForegroundColor Gray
    }
}

# Add Shell MCP
if (-not $FilesystemOnly) {
    $newConfig.mcpServers["shell"] = @{
        command = "npx"
        args = @("-y", "mcp-shell-server")
    }
    Write-Host "✓ Configured Shell MCP (PowerShell/CMD execution)" -ForegroundColor Green
}

Write-Host ""

# Write config file
Write-Host "[Step 4/4] Writing config file..." -ForegroundColor Green

try {
    $newConfig | ConvertTo-Json -Depth 10 | Set-Content -Path $configPath -Encoding UTF8
    Write-Host "✓ Config file written: $configPath" -ForegroundColor Green
} catch {
    Write-Error "Failed to write config file: $_"
    if ($backupCreated) {
        Write-Host "You can restore from backup: $backupPath" -ForegroundColor Yellow
    }
    exit 1
}

Write-Host ""
Write-Host "================================================" -ForegroundColor Cyan
Write-Host "Configuration Complete!" -ForegroundColor Green
Write-Host "================================================" -ForegroundColor Cyan
Write-Host ""

# Display config content
Write-Host "Config content preview:" -ForegroundColor Cyan
Write-Host "----------------------------------------" -ForegroundColor Gray
Get-Content $configPath | Write-Host -ForegroundColor Gray
Write-Host "----------------------------------------" -ForegroundColor Gray
Write-Host ""

# Next steps
Write-Host "Next Steps:" -ForegroundColor Cyan
Write-Host ""
Write-Host "1. Completely exit Claude Desktop" -ForegroundColor White
Write-Host "   - Close all windows" -ForegroundColor Gray
Write-Host "   - Verify in Task Manager that Claude process has exited" -ForegroundColor Gray
Write-Host ""
Write-Host "2. Restart Claude Desktop" -ForegroundColor White
Write-Host ""
Write-Host "3. Verify configuration in new session:" -ForegroundColor White
Write-Host "   Test 1: List D:\AI\AGENT_U盘\codex-desktop-usb-installer directory" -ForegroundColor Gray
Write-Host "   Test 2: Run cargo --version command" -ForegroundColor Gray
Write-Host "   Test 3: Execute git status in project directory" -ForegroundColor Gray
Write-Host ""

# Security notice
Write-Host "Security Notice:" -ForegroundColor Yellow
Write-Host "- Shell MCP executes commands with your Windows user privileges" -ForegroundColor Gray
Write-Host "- Filesystem MCP can only access the specified directories above" -ForegroundColor Gray
Write-Host "- You can edit $configPath anytime to modify configuration" -ForegroundColor Gray
Write-Host ""

if ($backupCreated) {
    Write-Host "Backup file: $backupPath" -ForegroundColor Cyan
    Write-Host ""
}
