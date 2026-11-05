# agentdash · claude-code 集成包安装器(PowerShell 版;类 Unix shell 用 install.sh)
# 前提:agentdash 二进制在 PATH——hook 直调二进制子命令,零 Python 前置(spec §6 修订)。
# 用法:.\install.ps1 [-Target <目标项目目录>](默认当前目录)
# 动作:skill 复制;settings.json 幂等注册五钩子(命令为常量,无路径 baked);
#       .gitignore 幂等追加 .agentdash/(M-2);清理老版本 Python 垫片残留。
# 幂等:重复执行只刷新自家注册与文件,不动 settings.json 其他内容。
# 兼容 Windows PowerShell 5.1+(::new 为 5.0+ 语法)。
param(
    [string]$Target = (Get-Location).Path
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

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

function Merge-AgentdashInstructions {
    param(
        [string]$AgentsMdPath,
        [string]$BlockFilePath
    )
    $parent = Split-Path -Parent $AgentsMdPath
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $blockText = ([System.IO.File]::ReadAllText($BlockFilePath, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n").TrimEnd("`n") + "`n"
    if (-not (Test-Path -LiteralPath $AgentsMdPath -PathType Leaf)) {
        [System.IO.File]::WriteAllText($AgentsMdPath, $blockText, $utf8NoBom)
        Write-Host "[agentdash] instructions created: $AgentsMdPath"
        return
    }
    $existing = [System.IO.File]::ReadAllText($AgentsMdPath, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n"
    if ($existing -match 'agentdash:begin') {
        $lines = $existing -split "`n"
        $outLines = New-Object System.Collections.Generic.List[string]
        $skip = $false
        $hasTrailingNewline = $existing.EndsWith("`n")
        $limit = if ($hasTrailingNewline -and $lines.Count -gt 0 -and $lines[$lines.Count - 1] -eq '') { $lines.Count - 1 } else { $lines.Count }
        for ($i = 0; $i -lt $limit; $i++) {
            $line = $lines[$i]
            if ($line -match '<!-- agentdash:begin') {
                $skip = $true
                foreach ($bLine in ($blockText.TrimEnd("`n") -split "`n")) {
                    $outLines.Add($bLine)
                }
                continue
            }
            if ($line -match '<!-- agentdash:end') {
                $skip = $false
                continue
            }
            if (-not $skip) {
                $outLines.Add($line)
            }
        }
        $mergedText = ($outLines -join "`n") + "`n"
        [System.IO.File]::WriteAllText($AgentsMdPath, $mergedText, $utf8NoBom)
        Write-Host "[agentdash] instructions refreshed: $AgentsMdPath"
    } else {
        $prefix = $existing
        if ($prefix.Length -gt 0 -and -not $prefix.EndsWith("`n")) {
            $prefix += "`n"
        }
        [System.IO.File]::WriteAllText($AgentsMdPath, $prefix + $blockText, $utf8NoBom)
        Write-Host "[agentdash] instructions appended: $AgentsMdPath"
    }
}

# --- skill 复制与 CLAUDE.md 标记段合并 ---
foreach ($skill in @('agentdash', 'agentdash-setup', 'agentdash-orchestrate')) {
    $skillDir = Join-Path $Target ".claude\skills\$skill"
    New-Item -ItemType Directory -Force -Path $skillDir | Out-Null
    Copy-Item (Join-Path $here "skills\$skill\SKILL.md") (Join-Path $skillDir 'SKILL.md') -Force
}
Merge-AgentdashInstructions `
    -AgentsMdPath (Join-Path $Target 'CLAUDE.md') `
    -BlockFilePath ([System.IO.Path]::GetFullPath((Join-Path $here '..\shared\AGENTDASH.md')))

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

# --- M-2 / W15-03:.gitignore 幂等维护 .agentdash 白名单(兼容旧版单行 .agentdash/、CRLF 与半白名单原地升级)---
$gitignore = Join-Path $Target '.gitignore'
$canonicalBlock = @(
    '# agentdash: track ledger, config, and wave archives; ignore live event streams',
    '/.agentdash/*',
    '!/.agentdash/ledger.json',
    '!/.agentdash/config.json',
    '!/.agentdash/waves/',
    '/.agentdash/waves/.tmp-*',
    '/.agentdash/waves/.old-*',
    '/.agentdash/waves/*/events.jsonl'
)
$managedPattern = '^[\t ]*(?:# agentdash: track ledger, config, and wave archives; ignore live event streams|/?\.agentdash/?|/?\.agentdash/\*|!/?\.agentdash/ledger\.json|!/?\.agentdash/config\.json|!/?\.agentdash/waves/|/?\.agentdash/waves/\.tmp-\*|/?\.agentdash/waves/\.old-\*|/?\.agentdash/waves/\*/events\.jsonl)[\t ]*$'
if (-not (Test-Path -LiteralPath $gitignore -PathType Leaf)) {
    [System.IO.File]::WriteAllText($gitignore, ($canonicalBlock -join "`n") + "`n", $utf8NoBom)
} else {
    $rawGi = ([System.IO.File]::ReadAllText($gitignore, [System.Text.Encoding]::UTF8) -replace "`r+`n", "`n") -replace "`r", "`n"
    $lines = New-Object System.Collections.Generic.List[string]
    $emitted = $false
    $trimmedGi = $rawGi.TrimEnd("`n")
    $srcLines = if ($trimmedGi.Length -eq 0) { @() } else { @($trimmedGi -split "`n") }
    foreach ($line in $srcLines) {
        if ($line -match '^[\t ]*# agentdash: track ledger, config, and wave archives; ignore live event streams[\t ]*$') {
            continue
        }
        if ($line -match $managedPattern) {
            if (-not $emitted) {
                foreach ($b in $canonicalBlock) { $lines.Add($b) }
                $emitted = $true
            }
            continue
        }
        $lines.Add($line)
    }
    if (-not $emitted) {
        foreach ($b in $canonicalBlock) { $lines.Add($b) }
    }
    [System.IO.File]::WriteAllText($gitignore, ($lines -join "`n") + "`n", $utf8NoBom)
}
Write-Host "[agentdash] .gitignore ensured: /.agentdash/* whitelist ($gitignore)"

# --- settings.json 幂等注册四钩子 ---
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
    PostToolUse         = 'agentdash hook --host claude posttooluse || true'
    PostToolUseFailure  = 'agentdash hook --host claude posttoolusefailure || true'
    PreToolUse          = 'agentdash hook --host claude pretooluse || true'
    Stop                = 'agentdash hook --host claude stop || true'
    SubagentStop        = 'agentdash hook --host claude subagentstop || true'
}
# matcher 限定(与 hooks.json 清单一致):PreToolUse 只拦 Task/Agent 派发;
# 未列出的(PostToolUse/Stop/SubagentStop)不设 matcher = 全事件。
$matchers = @{
    PreToolUse = 'Task|Agent'
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
    if ($matchers.ContainsKey($event)) {
        $newBlock = [pscustomobject]@{ matcher = $matchers[$event]; hooks = @($newEntry) }
    } else {
        # PostToolUse 不设 matcher = 全工具(其他工具也要落 tool 事件)
        $newBlock = [pscustomobject]@{ hooks = @($newEntry) }
    }
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
Write-Host "[agentdash] hooks registered in $settingsPath : PostToolUse/PostToolUseFailure/PreToolUse/Stop/SubagentStop"
Write-Host "[agentdash] installed into $Target\.claude (skill: /agentdash)"
