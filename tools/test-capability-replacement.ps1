$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$source = Join-Path $repo 'installer-pack\install-to-codex.ps1'
$tokens = $null; $parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Installer parse failed' }
foreach ($function in $ast.FindAll({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst]}, $false)) {
    . ([scriptblock]::Create($function.Extent.Text))
}
$fixture = Join-Path (Join-Path $repo 'tmp') ('cap-replace-' + [guid]::NewGuid().ToString('N'))
$PackRoot = Join-Path $fixture 'pack'
$ownershipRoot = Join-Path $fixture 'home\codex-agent-kit'
$skillTarget = Join-Path $fixture 'skills'
$previousFiles = @()
$installed = [System.Collections.Generic.List[object]]::new()
$Overwrite = $true
$ReplaceExisting = $true
New-Item -ItemType Directory -Force -Path $PackRoot,$ownershipRoot,$skillTarget | Out-Null
$newFile = Join-Path $PackRoot 'server.py'
$oldFile = Join-Path $ownershipRoot 'server.py'
[IO.File]::WriteAllText($newFile, 'current server')
[IO.File]::WriteAllText($oldFile, 'previous local server')
if (-not (Copy-OwnedFile $newFile $oldFile 'mcp-server')) { throw 'Replacement was skipped' }
if ([IO.File]::ReadAllText($oldFile) -ne 'current server') { throw 'Old file was not replaced' }
$backups = @(Get-ChildItem -LiteralPath (Join-Path $ownershipRoot 'backups') -Recurse -File)
if ($backups.Count -ne 1 -or [IO.File]::ReadAllText($backups[0].FullName) -ne 'previous local server') { throw 'Original file backup missing' }
$previousFiles = $installed.ToArray()
[IO.File]::WriteAllText($oldFile, 'locally modified server')
if (-not (Copy-OwnedFile $newFile $oldFile 'mcp-server')) { throw 'Modified owned file was skipped' }
if ($installed.Count -ne 1) { throw 'Replacing an owned file duplicated its ownership record' }
$backups = @(Get-ChildItem -LiteralPath (Join-Path $ownershipRoot 'backups') -Recurse -File)
if (@($backups | Where-Object { [IO.File]::ReadAllText($_.FullName) -eq 'locally modified server' }).Count -ne 1) { throw 'Modified owned file backup missing' }
$skillNames = @('fixture-skill')
$skillSource = Join-Path $PackRoot 'skills\fixture-skill'
$oldSkill = Join-Path $skillTarget 'fixture-skill'
New-Item -ItemType Directory -Force -Path $skillSource,$oldSkill | Out-Null
[IO.File]::WriteAllText((Join-Path $skillSource 'SKILL.md'), 'current skill')
[IO.File]::WriteAllText((Join-Path $oldSkill 'SKILL.md'), 'previous skill')
[IO.File]::WriteAllText((Join-Path $oldSkill 'user-note.txt'), 'keep user note')
$loop = $ast.Find({param($n) $n -is [System.Management.Automation.Language.ForEachStatementAst] -and $n.Variable.VariablePath.UserPath -eq 'skillName'}, $false)
. ([scriptblock]::Create($loop.Extent.Text))
if ([IO.File]::ReadAllText((Join-Path $oldSkill 'SKILL.md')) -ne 'current skill') { throw 'Skill was not replaced' }
if ([IO.File]::ReadAllText((Join-Path $oldSkill 'user-note.txt')) -ne 'keep user note') { throw 'User-added file changed' }
$oldSkillBackup = @(Get-ChildItem -LiteralPath (Join-Path $ownershipRoot 'backups\replaced-skills') -Recurse -Filter SKILL.md)
if ($oldSkillBackup.Count -ne 1 -or [IO.File]::ReadAllText($oldSkillBackup[0].FullName) -ne 'previous skill') { throw 'Skill backup missing' }
Write-Output 'Package file/skill replacement, backups and unrelated-file preservation passed.'
# Only delete this test's newly created, resolved temporary directory.
$resolved = (Get-Item -LiteralPath $fixture).FullName
$tempRoot = [IO.Path]::GetFullPath((Join-Path $repo 'tmp')).TrimEnd('\') + '\'
if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolved -Leaf) -notlike 'cap-replace-*') { throw 'Unexpected test cleanup target' }
Remove-Item -LiteralPath $resolved -Recurse -Force
