[CmdletBinding()]
param(
    [switch]$ValidateOnly
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Write-JsonNoBom([string]$Path, [object]$Value, [int]$Depth = 8) {
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $json = $Value | ConvertTo-Json -Depth $Depth
    [IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $utf8NoBom)
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$kitRoot = if (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'pack.json') -PathType Leaf) {
    $PSScriptRoot
} elseif (Test-Path -LiteralPath (Join-Path $repoRoot 'manifest\connection-fields.json') -PathType Leaf) {
    $repoRoot
} else {
    throw '无法定位 Codex Starter Kit 根目录。'
}

function Read-KitJson([string]$RelativePath) {
    $path = Join-Path $kitRoot ($RelativePath -replace '/', [IO.Path]::DirectorySeparatorChar)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "缺少安装向导清单: $path" }
    return Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
}

$connectionManifest = Read-KitJson 'manifest/connection-fields.json'
$mcpManifest = Read-KitJson 'manifest/mcp-servers.json'
$apiManifest = Read-KitJson 'manifest/api-services.json'
$fieldById = @{}
foreach ($field in @($connectionManifest.fields)) {
    $fieldById[[string]$field.id] = $field
}

function Get-UniqueFieldIds([object[]]$Ids) {
    $result = New-Object System.Collections.Generic.List[string]
    foreach ($id in @($Ids)) {
        $text = [string]$id
        if (-not [string]::IsNullOrWhiteSpace($text) -and -not $result.Contains($text)) {
            $result.Add($text)
        }
    }
    return @($result.ToArray())
}

$serviceSpecs = New-Object System.Collections.Generic.List[object]
$feishu = @($mcpManifest.servers | Where-Object { [string]$_.id -eq 'feishu' }) | Select-Object -First 1
if ($null -eq $feishu) { throw 'MCP 清单没有 Feishu 服务记录。' }
$serviceSpecs.Add([pscustomobject]@{
        id = 'feishu'
        kind = 'mcp'
        title = '飞书 / Lark MCP'
        purpose = [string]$feishu.purpose
        fieldIds = Get-UniqueFieldIds @($feishu.connectionFields)
        relatedSkills = @($feishu.relatedSkills | ForEach-Object { [string]$_ })
    })
foreach ($service in @($apiManifest.services)) {
    $ids = Get-UniqueFieldIds (@($service.credentialFields) + @($service.configurationFields))
    $serviceSpecs.Add([pscustomobject]@{
            id = [string]$service.id
            kind = 'api'
            title = [string]$service.id
            purpose = if ([string]::IsNullOrWhiteSpace([string]$service.failureMessage)) { [string]$service.protocol } else { [string]$service.failureMessage }
            fieldIds = $ids
            relatedSkills = @($service.relatedSkills | ForEach-Object { [string]$_ })
        })
}

foreach ($service in $serviceSpecs.ToArray()) {
    foreach ($id in @($service.fieldIds)) {
        if (-not $fieldById.ContainsKey($id)) { throw "服务 $($service.id) 引用了不存在的连接字段: $id" }
    }
}

if ($ValidateOnly) {
    [ordered]@{
        schemaVersion = 1
        connectionFieldCount = @($connectionManifest.fields).Count
        services = @($serviceSpecs.ToArray() | ForEach-Object {
                [ordered]@{
                    id = $_.id
                    kind = $_.kind
                    fieldIds = @($_.fieldIds)
                    relatedSkills = @($_.relatedSkills)
                }
            })
        policy = 'All services are opt-in. Empty fields never enable a connection; secrets are never printed or written to the public pack.'
    } | ConvertTo-Json -Depth 8
    exit 0
}

try {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
} catch {
    throw '当前 Windows 环境无法加载 WinForms；请直接运行 install-all.ps1 进行无界面安装。'
}
[System.Windows.Forms.Application]::EnableVisualStyles()

$script:KitRoot = $kitRoot
$script:ServiceStates = New-Object System.Collections.Generic.List[object]
$script:FieldStates = New-Object System.Collections.Generic.List[object]
$script:FieldStatesById = @{}
$script:ClearBlank = $null
$script:StatusBox = $null
$script:Form = $null
$script:InstallButton = $null
$script:SkipButton = $null
$script:CancelButton = $null
$script:ConnectionOwnershipPath = $null
$script:AllowedConnectionEnvNames = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$script:OwnedConnectionEnvNames = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)

foreach ($field in @($connectionManifest.fields)) {
    $name = [string]$field.envVar
    if ($name -match '^[A-Z][A-Z0-9_]*$') { [void]$script:AllowedConnectionEnvNames.Add($name) }
}
$currentProfile = [Environment]::GetEnvironmentVariable('USERPROFILE')
if (-not [string]::IsNullOrWhiteSpace($currentProfile)) {
    $script:ConnectionOwnershipPath = Join-Path $currentProfile '.codex\codex-agent-kit\connection-ownership.json'
    if (Test-Path -LiteralPath $script:ConnectionOwnershipPath -PathType Leaf) {
        try {
            $ownership = Get-Content -LiteralPath $script:ConnectionOwnershipPath -Raw -Encoding UTF8 | ConvertFrom-Json
            if ([int]$ownership.schemaVersion -eq 1 -and [string]$ownership.packId -eq 'codex-starter') {
                foreach ($name in @($ownership.names)) {
                    if ($script:AllowedConnectionEnvNames.Contains([string]$name)) {
                        [void]$script:OwnedConnectionEnvNames.Add([string]$name)
                    }
                }
            }
        } catch {
            Write-Warning '无法读取能力包连接所有权记录；不会清理任何已有环境变量。'
        }
    }
}

function Get-ExistingFieldValue([object]$Field) {
    $envName = [string]$Field.envVar
    if ([string]::IsNullOrWhiteSpace($envName)) { return '' }
    $userValue = [Environment]::GetEnvironmentVariable($envName, 'User')
    if (-not [string]::IsNullOrWhiteSpace($userValue)) { return $userValue }
    $processValue = [Environment]::GetEnvironmentVariable($envName, 'Process')
    if (-not [string]::IsNullOrWhiteSpace($processValue)) { return $processValue }
    return ''
}

function Get-StateValue([object]$State) {
    $entered = ([string]$State.control.Text).Trim()
    if (-not [string]::IsNullOrWhiteSpace($entered)) { return $entered }
    if ($script:ClearBlank -and $script:ClearBlank.Checked) { return '' }
    if (-not [string]::IsNullOrWhiteSpace([string]$State.existingValue)) { return [string]$State.existingValue }
    return ([string]$State.defaultValue).Trim()
}

function Test-FieldValue([object]$Field, [string]$Value, [hashtable]$Values) {
    $conditional = $Field.conditionalRequired
    $required = [bool]$Field.required
    if ($null -ne $conditional) {
        $conditionValue = [string]$Values[[string]$conditional.field]
        if ($conditionValue -eq [string]$conditional.equals) { $required = $true }
    }
    if ($required -and [string]::IsNullOrWhiteSpace($Value)) {
        return "未填写必填项：$($Field.label)"
    }
    if ([string]::IsNullOrWhiteSpace($Value)) { return $null }
    $validation = $Field.validation
    if ($null -ne $validation.minLength -and $Value.Length -lt [int]$validation.minLength) {
        return "$($Field.label) 长度不足。"
    }
    if ($null -ne $validation.maxLength -and $Value.Length -gt [int]$validation.maxLength) {
        return "$($Field.label) 长度过长。"
    }
    if (-not [string]::IsNullOrWhiteSpace([string]$validation.regex) -and -not [regex]::IsMatch($Value, [string]$validation.regex)) {
        return "$($Field.label) 格式不正确。"
    }
    if (@($validation.choices).Count -gt 0 -and $Value -notin @($validation.choices | ForEach-Object { [string]$_ })) {
        return "$($Field.label) 不是允许的选项。"
    }
    if ([string]$Field.type -eq 'url') {
        $uri = $null
        if (-not [Uri]::TryCreate($Value, [UriKind]::Absolute, [ref]$uri)) { return "$($Field.label) 必须是完整 URL。" }
        if (-not [string]::IsNullOrWhiteSpace([string]$validation.scheme) -and $uri.Scheme -ne [string]$validation.scheme) {
            return "$($Field.label) 必须使用 $($validation.scheme)。"
        }
        if (@($validation.hostAllowlist).Count -gt 0 -and $uri.Host -notin @($validation.hostAllowlist | ForEach-Object { [string]$_ })) {
            return "$($Field.label) 的主机不在允许范围内。"
        }
        if (-not [string]::IsNullOrWhiteSpace([string]$validation.path) -and $uri.AbsolutePath.TrimEnd('/') -ne ([string]$validation.path).TrimEnd('/')) {
            return "$($Field.label) 的路径必须是 $($validation.path)。"
        }
    }
    return $null
}

function Set-UserEnvironmentValue([string]$Name, [string]$Value, [bool]$Enabled) {
    if ([string]::IsNullOrWhiteSpace($Name)) { return }
    if ($Enabled -and -not [string]::IsNullOrWhiteSpace($Value)) {
        [Environment]::SetEnvironmentVariable($Name, $Value, 'User')
        [Environment]::SetEnvironmentVariable($Name, $Value, 'Process')
        if ($script:AllowedConnectionEnvNames.Contains($Name)) { [void]$script:OwnedConnectionEnvNames.Add($Name) }
    } elseif (-not $Enabled -and $script:ClearBlank.Checked -and $script:OwnedConnectionEnvNames.Contains($Name)) {
        [Environment]::SetEnvironmentVariable($Name, $null, 'User')
        [Environment]::SetEnvironmentVariable($Name, $null, 'Process')
        [void]$script:OwnedConnectionEnvNames.Remove($Name)
    }
}

function Save-ConnectionOwnership {
    if ([string]::IsNullOrWhiteSpace($script:ConnectionOwnershipPath)) { return }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $script:ConnectionOwnershipPath) | Out-Null
    Write-JsonNoBom $script:ConnectionOwnershipPath ([ordered]@{
        schemaVersion = 1
        packId = 'codex-starter'
        names = @($script:OwnedConnectionEnvNames | Sort-Object)
        note = 'Only environment variable names are recorded; values and credentials are never stored.'
    }) 4
}

function Write-WizardStatus([string]$Text) {
    if ($null -eq $script:StatusBox) { return }
    $script:StatusBox.AppendText($Text + [Environment]::NewLine)
    $script:StatusBox.SelectionStart = $script:StatusBox.TextLength
    $script:StatusBox.ScrollToCaret()
    [System.Windows.Forms.Application]::DoEvents()
}

function Invoke-WizardInstall([switch]$SkipConnections) {
    $values = @{}
    foreach ($state in $script:FieldStates.ToArray()) {
        $values[[string]$state.field.id] = Get-StateValue $state
    }
    $selectedServices = if ($SkipConnections) { @() } else { @($script:ServiceStates.ToArray() | Where-Object { $_.check.Checked }) }
    foreach ($service in @($selectedServices)) {
        foreach ($state in @($service.fields)) {
            $errorText = Test-FieldValue $state.field ([string]$values[[string]$state.field.id]) $values
            if ($null -ne $errorText) {
                [System.Windows.Forms.MessageBox]::Show($script:Form, $errorText, '请补全配置', 'OK', 'Warning') | Out-Null
                $state.control.Focus()
                return
            }
        }
    }

    $enabledSkills = New-Object System.Collections.Generic.List[string]
    $enabledApiServices = New-Object System.Collections.Generic.List[string]
    $feishuEnabled = $false
    $feishuDomain = 'https://open.feishu.cn'
    $feishuAuthMode = 'tenant'
    foreach ($service in $script:ServiceStates.ToArray()) {
        $enabled = (-not $SkipConnections) -and $service.check.Checked
        foreach ($state in @($service.fields)) {
            $value = [string]$values[[string]$state.field.id]
            Set-UserEnvironmentValue ([string]$state.field.envVar) $value $enabled
        }
        if (-not $enabled) { continue }
        foreach ($skill in @($service.relatedSkills)) {
            if (-not $enabledSkills.Contains([string]$skill)) { $enabledSkills.Add([string]$skill) }
        }
        if ($service.kind -eq 'api') { $enabledApiServices.Add([string]$service.id) }
        if ($service.id -eq 'feishu') {
            $feishuEnabled = $true
            $feishuDomain = [string]$values['feishu.base-url']
            $feishuAuthMode = [string]$values['feishu.auth-mode']
        }
    }
    Save-ConnectionOwnership

    $script:InstallButton.Enabled = $false
    $script:SkipButton.Enabled = $false
    $script:CancelButton.Enabled = $false
    $script:Form.Cursor = [System.Windows.Forms.Cursors]::WaitCursor
    try {
        if ($SkipConnections) {
            Write-WizardStatus '已选择跳过所有外部连接；开始安装全部 Skills、Agents、运行时和 MCP 包。'
        } else {
            $names = @($selectedServices | ForEach-Object { [string]$_.title })
            if ($names.Count -eq 0) { Write-WizardStatus '未勾选任何外部连接；开始安装全部 Skills、Agents、运行时和 MCP 包。' }
            else { Write-WizardStatus ('将启用：' + ($names -join '、')) }
        }
        $installScript = Join-Path $script:KitRoot 'install-all.ps1'
        if (-not (Test-Path -LiteralPath $installScript -PathType Leaf)) { throw "安装脚本不存在: $installScript" }
        $installArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $installScript, '-IncludeNonDefault', '-IncludeUnsupported', '-Overwrite', '-FeishuAuthMode', $feishuAuthMode, '-FeishuDomain', $feishuDomain)
        if ($feishuEnabled) { $installArgs += '-EnableFeishu' } else { $installArgs += '-DisableFeishu' }
        if ($enabledSkills.Count -gt 0) { $installArgs += '-EnableSkills'; $installArgs += @($enabledSkills.ToArray()) }
        if ($enabledApiServices.Count -gt 0) { $installArgs += '-EnabledApiServices'; $installArgs += @($enabledApiServices.ToArray()) }
        $output = @(& powershell.exe @installArgs 2>&1)
        foreach ($line in $output) { Write-WizardStatus ([string]$line) }
        if ($LASTEXITCODE -ne 0) { throw "安装失败，退出码 $LASTEXITCODE。" }
        $readinessPath = Join-Path ([Environment]::GetEnvironmentVariable('USERPROFILE')) '.codex\codex-agent-kit\readiness.json'
        if (Test-Path -LiteralPath $readinessPath -PathType Leaf) {
            $ready = Get-Content -LiteralPath $readinessPath -Raw -Encoding UTF8 | ConvertFrom-Json
            Write-WizardStatus "安装完成：$($ready.installed.skills) 个 Skills，$($ready.installed.agents) 个 Agents，运行时状态 $($ready.runtime.status)。"
        } else {
            Write-WizardStatus '安装完成，但未找到 readiness 报告。'
        }
        [System.Windows.Forms.MessageBox]::Show($script:Form, '安装完成。请重启 Codex 或打开新任务，让新的 Skills 和 Agents 生效。', 'Codex Starter Kit', 'OK', 'Information') | Out-Null
    } catch {
        Write-WizardStatus ('错误：' + $_.Exception.Message)
        [System.Windows.Forms.MessageBox]::Show($script:Form, $_.Exception.Message, '安装失败', 'OK', 'Error') | Out-Null
    } finally {
        $script:InstallButton.Enabled = $true
        $script:SkipButton.Enabled = $true
        $script:CancelButton.Enabled = $true
        $script:Form.Cursor = [System.Windows.Forms.Cursors]::Default
    }
}

$form = New-Object System.Windows.Forms.Form
$form.Text = 'Codex Starter Kit - 一键安装向导'
$form.StartPosition = 'CenterScreen'
$form.ClientSize = New-Object -TypeName System.Drawing.Size -ArgumentList (1120, 820)
$form.MinimumSize = New-Object -TypeName System.Drawing.Size -ArgumentList (900, 650)
$form.MaximizeBox = $true
$form.AutoScaleMode = 'Dpi'
$script:Form = $form

$scroll = New-Object System.Windows.Forms.FlowLayoutPanel
$scroll.Dock = 'Fill'
$scroll.FlowDirection = 'TopDown'
$scroll.WrapContents = $false
$scroll.AutoScroll = $true
$scroll.Padding = New-Object -TypeName System.Windows.Forms.Padding -ArgumentList (14)
$scroll.BackColor = [System.Drawing.Color]::White

$intro = New-Object System.Windows.Forms.Label
$intro.Width = 1050
$intro.Height = 76
$intro.AutoSize = $false
$intro.Font = New-Object -TypeName System.Drawing.Font -ArgumentList ('Microsoft YaHei UI', 10)
$intro.Text = '本向导会安装全部 50 个 Skills、7 个 Agents、Python/Node/Feishu 运行时和本地 MCP 包。外部连接全部默认关闭；不勾选或不填写就跳过。敏感值只写入当前 Windows 用户环境变量，不写入安装包、Codex TOML、日志或 readiness。'
$scroll.Controls.Add($intro)

$inventory = New-Object System.Windows.Forms.Label
$inventory.Width = 1050
$inventory.Height = 30
$inventory.AutoSize = $false
$inventory.Text = "检测到 $(@($connectionManifest.fields).Count) 个连接字段、$($serviceSpecs.Count) 个可选连接。Skills/Agents 将全部安装。"
$inventory.ForeColor = [System.Drawing.Color]::DarkSlateGray
$scroll.Controls.Add($inventory)

foreach ($service in $serviceSpecs.ToArray()) {
    $fieldIds = @($service.fieldIds)
    $group = New-Object System.Windows.Forms.GroupBox
    $group.Width = 1050
    $group.Height = 92 + ($fieldIds.Count * 64)
    $group.Text = "$($service.title)  |  $($service.purpose)"
    $group.Font = New-Object -TypeName System.Drawing.Font -ArgumentList ('Microsoft YaHei UI', 9)

    $check = New-Object System.Windows.Forms.CheckBox
    $check.Location = New-Object -TypeName System.Drawing.Point -ArgumentList (18, 28)
    $check.AutoSize = $true
    $check.Text = '启用此连接'
    $check.Font = New-Object -TypeName System.Drawing.Font -ArgumentList ('Microsoft YaHei UI', 9, [System.Drawing.FontStyle]::Bold)
    $group.Controls.Add($check)

    $serviceStatus = New-Object System.Windows.Forms.Label
    $serviceStatus.Location = New-Object -TypeName System.Drawing.Point -ArgumentList (150, 30)
    $serviceStatus.Width = 800
    $serviceStatus.Height = 24
    $serviceStatus.Text = '默认关闭；不勾选即可跳过'
    $serviceStatus.ForeColor = [System.Drawing.Color]::DimGray
    $group.Controls.Add($serviceStatus)

    $fieldStates = New-Object System.Collections.Generic.List[object]
    $row = 62
    foreach ($fieldId in $fieldIds) {
        $field = $fieldById[[string]$fieldId]
        $label = New-Object System.Windows.Forms.Label
        $label.Location = New-Object -TypeName System.Drawing.Point -ArgumentList (22, ($row + 5))
        $label.Width = 250
        $label.Height = 26
        $label.Text = [string]$field.label
        $label.TextAlign = [System.Drawing.ContentAlignment]::MiddleLeft
        $group.Controls.Add($label)

        $control = if ([string]$field.type -eq 'select') { New-Object System.Windows.Forms.ComboBox } else { New-Object System.Windows.Forms.TextBox }
        $control.Location = New-Object -TypeName System.Drawing.Point -ArgumentList (278, $row)
        $control.Width = 430
        $control.Height = 28
        $existing = Get-ExistingFieldValue $field
        $defaultValue = [string]$field.defaultUrl
        if ([string]$field.type -eq 'select') {
            $control.DropDownStyle = [System.Windows.Forms.ComboBoxStyle]::DropDownList
            foreach ($choice in @($field.validation.choices)) { [void]$control.Items.Add([string]$choice) }
            $selected = if (-not [string]::IsNullOrWhiteSpace($existing)) { $existing } else { 'tenant' }
            if ($control.Items.Contains($selected)) { $control.SelectedItem = $selected } elseif ($control.Items.Count -gt 0) { $control.SelectedIndex = 0 }
        } else {
            if ([bool]$field.secret) {
                $control.UseSystemPasswordChar = $true
            } else {
                $control.Text = if (-not [string]::IsNullOrWhiteSpace($existing)) { $existing } else { $defaultValue }
            }
        }
        $group.Controls.Add($control)

        $help = New-Object System.Windows.Forms.Label
        $help.Location = New-Object -TypeName System.Drawing.Point -ArgumentList (724, ($row + 2))
        $help.Width = 300
        $help.Height = 54
        $help.AutoSize = $false
        $help.ForeColor = [System.Drawing.Color]::DimGray
        $help.Font = New-Object -TypeName System.Drawing.Font -ArgumentList ('Microsoft YaHei UI', 8)
        $help.Text = [string]$field.help
        if ([bool]$field.secret -and -not [string]::IsNullOrWhiteSpace($existing)) {
            $help.Text += '（检测到已有本机配置；留空将沿用，敏感值不显示。）'
        }
        $group.Controls.Add($help)

        $state = [pscustomobject]@{
            serviceId = [string]$service.id
            field = $field
            control = $control
            existingValue = $existing
            defaultValue = $defaultValue
        }
        $fieldStates.Add($state)
        $script:FieldStates.Add($state)
        $script:FieldStatesById[[string]$field.id] = $state
        $row += 64
    }
    $serviceState = [pscustomobject]@{
        id = [string]$service.id
        kind = [string]$service.kind
        title = [string]$service.title
        relatedSkills = @($service.relatedSkills)
        check = $check
        status = $serviceStatus
        fields = @($fieldStates.ToArray())
    }
    $script:ServiceStates.Add($serviceState)
    foreach ($state in $fieldStates.ToArray()) {
        $state.control.Enabled = $false
    }
    $check.Tag = $serviceState
    $check.Add_CheckedChanged({
            $currentCheck = [System.Windows.Forms.CheckBox]$this
            $currentService = $currentCheck.Tag
            if ($currentCheck.Checked) {
                $currentService.status.Text = '已选择；点击安装时会校验必填字段并启用相关 Skill。'
                $currentService.status.ForeColor = [System.Drawing.Color]::DarkGreen
            } else {
                $currentService.status.Text = '默认关闭；不勾选即可跳过'
                $currentService.status.ForeColor = [System.Drawing.Color]::DimGray
            }
        })
    $scroll.Controls.Add($group)
}

$script:ClearBlank = New-Object System.Windows.Forms.CheckBox
$script:ClearBlank.Width = 1040
$script:ClearBlank.Height = 30
$script:ClearBlank.AutoSize = $false
$script:ClearBlank.Text = '勾选后：仅清理本安装器此前记录拥有的未启用环境变量（谨慎；默认不清理）'
$script:ClearBlank.ForeColor = [System.Drawing.Color]::DarkRed
$scroll.Controls.Add($script:ClearBlank)

$note = New-Object System.Windows.Forms.Label
$note.Width = 1050
$note.Height = 58
$note.AutoSize = $false
$note.Text = '说明：当前清单没有网站用户名/密码字段；飞书的 App ID/App Secret 是应用账号与应用密码，user 模式另需 User Access Token。没有对应服务凭据时，相关 Skill 仍会安装，但保持关闭并在调用前安全失败。'
$note.ForeColor = [System.Drawing.Color]::DimGray
$scroll.Controls.Add($note)

$status = New-Object System.Windows.Forms.TextBox
$status.Multiline = $true
$status.ReadOnly = $true
$status.ScrollBars = 'Vertical'
$status.Dock = 'Bottom'
$status.Height = 110
$status.BackColor = [System.Drawing.Color]::WhiteSmoke
$script:StatusBox = $status

$buttons = New-Object System.Windows.Forms.FlowLayoutPanel
$buttons.Dock = 'Bottom'
$buttons.Height = 52
$buttons.FlowDirection = 'LeftToRight'
$buttons.Padding = New-Object -TypeName System.Windows.Forms.Padding -ArgumentList (12, 8, 0, 0)

$installButton = New-Object System.Windows.Forms.Button
$installButton.Width = 230
$installButton.Height = 34
$installButton.Text = '安装全部并应用已选连接'
$script:InstallButton = $installButton
$buttons.Controls.Add($installButton)

$skipButton = New-Object System.Windows.Forms.Button
$skipButton.Width = 210
$skipButton.Height = 34
$skipButton.Text = '跳过配置，直接安装'
$script:SkipButton = $skipButton
$buttons.Controls.Add($skipButton)

$cancelButton = New-Object System.Windows.Forms.Button
$cancelButton.Width = 100
$cancelButton.Height = 34
$cancelButton.Text = '取消'
$script:CancelButton = $cancelButton
$buttons.Controls.Add($cancelButton)

$installButton.Add_Click({ Invoke-WizardInstall })
$skipButton.Add_Click({ Invoke-WizardInstall -SkipConnections })
$cancelButton.Add_Click({ $script:Form.Close() })

$form.Controls.Add($scroll)
$form.Controls.Add($status)
$form.Controls.Add($buttons)
$form.Add_Shown({ $script:Form.Activate() })
[void]$form.ShowDialog()
