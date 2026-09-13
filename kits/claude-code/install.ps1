# agentdash · claude-code 集成包安装器(PowerShell 版;类 Unix shell 用 install.sh)
# 用法:.\install.ps1 [-Target <目标项目目录>](默认当前目录)
# 动作:hooks/skill 复制进 <目标>\.claude\,并在 settings.json 幂等注册三钩子。
# 幂等:重复执行只刷新文件与自家注册,不动 settings.json 其他内容。
# 兼容 Windows PowerShell 5.1+(-AsHashtable 不可用;::new 为 5.0+ 语法)。
param(
    [string]$Target = (Get-Location).Path
)

$ErrorActionPreference = 'Stop'

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not [System.IO.Path]::IsPathRooted($Target)) {
    $Target = Join-Path (Get-Location).Path $Target
}
if (-not (Test-Path -LiteralPath $Target -PathType Container)) {
    Write-Error "[agentdash] 目标目录不存在: $Target"
}

# --- 复制 hooks / skill ---
$hookDir  = Join-Path $Target '.claude\agentdash\hooks'
$skillDir = Join-Path $Target '.claude\skills\agentdash'
New-Item -ItemType Directory -Force -Path $hookDir  | Out-Null
New-Item -ItemType Directory -Force -Path $skillDir | Out-Null
Copy-Item (Join-Path $here 'hooks\record_event.py') (Join-Path $hookDir 'record_event.py') -Force
Copy-Item (Join-Path $here 'skills\agentdash\SKILL.md') (Join-Path $skillDir 'SKILL.md') -Force

# --- 探测 hook 运行时解释器(py -3 → python → python3;Windows 优先官方启动器)---
$py = $null
foreach ($c in @(
    @{ Exe = 'py';      Args = @('-3') },
    @{ Exe = 'python';  Args = @() },
    @{ Exe = 'python3'; Args = @() }
)) {
    try {
        & $c.Exe @($c.Args + @('-c', 'import sys')) | Out-Null
        if ($LASTEXITCODE -eq 0) {
            $py = (@($c.Exe) + @($c.Args)) -join ' '
            break
        }
    } catch { }
}
if (-not $py) {
    Write-Error '[agentdash] 未找到 py/python —— hook 需要它;文件已复制,请装好 Python 后重跑'
}

# --- settings.json 幂等注册 ---
$settingsPath = Join-Path $Target '.claude\settings.json'
$data = $null
if (Test-Path -LiteralPath $settingsPath) {
    try {
        $data = Get-Content -LiteralPath $settingsPath -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch { $data = $null }
    if (-not $data) {
        # 损坏的 settings.json:备份后重建,不吞用户文件
        Copy-Item -LiteralPath $settingsPath -Destination "$settingsPath.bak-agentdash" -Force
    }
}
if (-not $data) { $data = [pscustomobject]@{} }

if (-not ($data.PSObject.Properties['hooks'])) {
    $data | Add-Member -NotePropertyName hooks -NotePropertyValue ([pscustomobject]@{})
}
$hooks = $data.hooks

$cmd = '{0} "{1}"' -f $py, (Join-Path $hookDir 'record_event.py')

function Remove-AgentdashBlocks {
    param($Existing)
    $out = @()
    if ($Existing) {
        foreach ($block in @($Existing)) {
            if ($null -eq $block) { continue }
            if ($block.PSObject.Properties['hooks'] -and $null -ne $block.hooks) {
                $kept = @()
                foreach ($h in @($block.hooks)) {
                    $c = ''
                    if ($h -and $h.PSObject.Properties['command']) { $c = [string]$h.command }
                    # 自家旧注册判定:命令同时含 agentdash 与 record_event.py
                    if (-not ($c.Contains('agentdash') -and $c.Contains('record_event.py'))) { $kept += $h }
                }
                if ($kept.Count -gt 0) {
                    $block.hooks = $kept
                    $out += $block
                }
            } else {
                $out += $block  # 无 hooks 字段的块:非我方形态,原样保留
            }
        }
    }
    return $out  # 不加逗号包装:让调用侧 @(...) 收集为扁平块数组
}

foreach ($event in @('PostToolUse', 'Stop', 'SubagentStop')) {
    $existing = $null
    if ($hooks.PSObject.Properties[$event]) { $existing = $hooks.$event }
    $newEntry = [pscustomobject]@{ type = 'command'; command = $cmd }
    $newBlock = [pscustomobject]@{ hooks = @($newEntry) }
    # PostToolUse 不设 matcher = 全工具(其他工具也要落 tool 事件)
    $merged = @(Remove-AgentdashBlocks $existing) + @($newBlock)
    if ($hooks.PSObject.Properties[$event]) {
        $hooks.$event = $merged
    } else {
        $hooks | Add-Member -NotePropertyName $event -NotePropertyValue $merged
    }
}

$json = $data | ConvertTo-Json -Depth 10
# 无 BOM UTF-8(JSON 带 BOM 会毒化部分解析器)
[System.IO.File]::WriteAllText($settingsPath, $json + "`n", [System.Text.UTF8Encoding]::new($false))
Write-Host "[agentdash] hooks registered in $settingsPath : PostToolUse/Stop/SubagentStop"
Write-Host "[agentdash] installed into $Target\.claude (skill: /agentdash)"
