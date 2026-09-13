# agentdash · claude-code 集成包安装器(PowerShell 版;类 Unix shell 用 install.sh)
# 前提:agentdash 二进制在 PATH——hook 直调二进制子命令,零 Python 前置(spec §6 修订)。
# 用法:.\install.ps1 [-Target <目标项目目录>](默认当前目录)
# 动作:skill 复制;settings.json 幂等注册三钩子(命令为常量,无路径 baked);
#       .gitignore 幂等追加 .agentdash/(M-2);清理老版本 Python 垫片残留。
# 幂等:重复执行只刷新自家注册与文件,不动 settings.json 其他内容。
# 兼容 Windows PowerShell 5.1+(::new 为 5.0+ 语法)。
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

# --- 前提检测:agentdash 在 PATH ---
$agentdashCmd = Get-Command -Name 'agentdash' -ErrorAction SilentlyContinue
if (-not $agentdashCmd) {
    Write-Error "[agentdash] 未找到 agentdash —— hook 直调二进制,需要它先在 PATH。安装方式二选一:`n  1) 仓库内安装:  cargo install --path <agentdash 仓库根目录>`n  2) 发布件:      从项目 Releases 下载对应平台二进制,放入 PATH"
}

# --- skill 复制 ---
$skillDir = Join-Path $Target '.claude\skills\agentdash'
New-Item -ItemType Directory -Force -Path $skillDir | Out-Null
Copy-Item (Join-Path $here 'skills\agentdash\SKILL.md') (Join-Path $skillDir 'SKILL.md') -Force

# --- 清理被二进制方案替代的 Python 垫片残留(老版本安装产物)---
$staleHook = Join-Path $Target '.claude\agentdash\hooks\record_event.py'
if (Test-Path -LiteralPath $staleHook) {
    Remove-Item -LiteralPath $staleHook -Force
    foreach ($dir in @('agentdash\hooks', 'agentdash')) {
        $p = Join-Path $Target (Join-Path '.claude' $dir)
        if ((Test-Path -LiteralPath $p) -and -not (Get-ChildItem -LiteralPath $p)) {
            Remove-Item -LiteralPath $p -Force   # 仅删空目录,非空保留用户内容
        }
    }
}

# --- M-2:.gitignore 幂等追加 .agentdash/ ---
$gitignore = Join-Path $Target '.gitignore'
$hasEntry = $false
if (Test-Path -LiteralPath $gitignore) {
    $hasEntry = [bool](Select-String -LiteralPath $gitignore -Pattern '^\s*\.agentdash/?\s*$' -Quiet)
}
if (-not $hasEntry) {
    # 末行无换行时先补一个:否则 .agentdash/ 粘上最后一行失效,且幂等检查永远失配
    if (Test-Path -LiteralPath $gitignore) {
        $text = [System.IO.File]::ReadAllText($gitignore)
        if ($text.Length -gt 0 -and $text[-1] -ne "`n") {
            [System.IO.File]::AppendAllText($gitignore, [Environment]::NewLine)
        }
    }
    # .NET 追加:UTF-8 无 BOM(BOM 会使 .gitignore 首行模式失配)
    [System.IO.File]::AppendAllText($gitignore, '.agentdash/' + [Environment]::NewLine)
}
Write-Host "[agentdash] .gitignore ensured: .agentdash/ ($gitignore)"

# --- settings.json 幂等注册三钩子 ---
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

$commands = [ordered]@{
    PostToolUse  = 'agentdash hook posttooluse || true'
    Stop         = 'agentdash hook stop || true'
    SubagentStop = 'agentdash hook subagentstop || true'
}

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
                    # 自家注册判定:新形态 `agentdash hook`;旧形态含 record_event.py(一并替换)
                    if (-not (($c -like '*agentdash hook*') -or (($c -like '*agentdash*') -and ($c -like '*record_event.py*')))) {
                        $kept += $h
                    }
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

foreach ($event in $commands.Keys) {
    $existing = $null
    if ($hooks.PSObject.Properties[$event]) { $existing = $hooks.$event }
    $newEntry = [pscustomobject]@{ type = 'command'; command = $commands[$event] }
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
