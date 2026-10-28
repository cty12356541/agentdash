# agentdash - antigravity-kit installer (PowerShell; use install.sh for Bash)
# Prerequisite: agentdash binary in PATH.
# Usage:
#   .\install.ps1 [-Global]                # Default (recommended): machine-level install into ~\.gemini\AGENTS.md + ~\.gemini\config\plugins\agentdash\
#   .\install.ps1 -Target <project-dir>    # Optional per-project install into <Target>\AGENTS.md + <Target>\.agents\
param(
    [switch]$Global,
    [string]$Target = ''
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

# --- 1. Check agentdash in PATH ---
$agentdashCmd = Get-Command -Name 'agentdash' -ErrorAction SilentlyContinue
if (-not $agentdashCmd) {
    Write-Error "[agentdash] agentdash binary not found in PATH.`n  1) cargo install --path <agentdash-repo>   2) Download from Releases and add to PATH"
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

function Copy-NormalizedUtf8 {
    param([string]$Src, [string]$Dst)
    $parent = Split-Path -Parent $Dst
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $raw = [System.IO.File]::ReadAllText($Src, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n"
    [System.IO.File]::WriteAllText($Dst, $raw, $utf8NoBom)
}

# ======================= Mode A: Machine-Level Global Install (Default / -Global) =======================
if ($Global -or [string]::IsNullOrWhiteSpace($Target)) {
    $userHome = if ($env:USERPROFILE) { $env:USERPROFILE } else { $HOME }
    $geminiHome = Join-Path $userHome '.gemini'
    $pluginDir = Join-Path $geminiHome 'config\plugins\agentdash'

    # 1. Merge into machine-level global ~\.gemini\AGENTS.md (Antigravity user_global globalScope)
    Merge-AgentdashInstructions `
        -AgentsMdPath (Join-Path $geminiHome 'AGENTS.md') `
        -BlockFilePath (Join-Path $here 'AGENTS.global.md')

    # 2. Sync global plugin bundle ~\.gemini\config\plugins\agentdash\ (without duplicate rules\AGENTS.md)
    $dupRulesDir = Join-Path $pluginDir 'rules'
    if (Test-Path -LiteralPath $dupRulesDir) {
        Remove-Item -LiteralPath $dupRulesDir -Recurse -Force
    }
    Copy-NormalizedUtf8 -Src (Join-Path $here 'plugin.json') -Dst (Join-Path $pluginDir 'plugin.json')
    Copy-NormalizedUtf8 -Src (Join-Path $here 'hooks.json') -Dst (Join-Path $pluginDir 'hooks.json')
    Copy-NormalizedUtf8 -Src (Join-Path $here 'assets\logo.svg') -Dst (Join-Path $pluginDir 'assets\logo.svg')
    Copy-NormalizedUtf8 -Src (Join-Path $here 'README.md') -Dst (Join-Path $pluginDir 'README.md')
    foreach ($skill in @('agentdash', 'agentdash-setup', 'agentdash-orchestrate')) {
        Copy-NormalizedUtf8 `
            -Src (Join-Path $here "skills\$skill\SKILL.md") `
            -Dst (Join-Path $pluginDir "skills\$skill\SKILL.md")
    }

    Write-Host "[agentdash] antigravity-kit installed globally (machine-level):"
    Write-Host "  - Global AGENTS.md : $(Join-Path $geminiHome 'AGENTS.md')"
    Write-Host "  - Global Plugin    : $pluginDir"
    Write-Host "[agentdash] Note: restart Antigravity once after initial global plugin setup so global hooks activate across all projects."
    exit 0
}

# ======================= Mode B: Explicit Per-Project Install (-Target <dir>) =======================
if (-not [System.IO.Path]::IsPathRooted($Target)) {
    $Target = Join-Path (Get-Location).Path $Target
}
$Target = [System.IO.Path]::GetFullPath($Target)
if (-not (Test-Path -LiteralPath $Target -PathType Container)) {
    Write-Error "[agentdash] Target directory does not exist: $Target"
}

Merge-AgentdashInstructions `
    -AgentsMdPath (Join-Path $Target 'AGENTS.md') `
    -BlockFilePath ([System.IO.Path]::GetFullPath((Join-Path $here '..\shared\AGENTDASH.md')))

$agentsDir = Join-Path $Target '.agents'
New-Item -ItemType Directory -Force -Path $agentsDir | Out-Null
$configFile = Join-Path $agentsDir 'hooks.json'
$tplFile = Join-Path $here 'hooks.template.json'
$tplRaw = [System.IO.File]::ReadAllText($tplFile, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n"

if (-not (Test-Path -LiteralPath $configFile -PathType Leaf)) {
    [System.IO.File]::WriteAllText($configFile, $tplRaw, $utf8NoBom)
    Write-Host "[agentdash] hooks registered (fresh): $configFile"
} else {
    $data = $null
    $isObject = $false
    try {
        $rawExisting = [System.IO.File]::ReadAllText($configFile, [System.Text.Encoding]::UTF8)
        if ($rawExisting.Trim().Length -gt 0) {
            $data = $rawExisting | ConvertFrom-Json
            if ($null -ne $data -and $data -is [System.Management.Automation.PSCustomObject]) {
                $isObject = $true
            }
        }
    } catch {
        $isObject = $false
    }

    if ($isObject) {
        $tplObj = $tplRaw | ConvertFrom-Json
        if ($data.PSObject.Properties['agentdash']) {
            $data.agentdash = $tplObj.agentdash
        } else {
            $data | Add-Member -NotePropertyName 'agentdash' -NotePropertyValue $tplObj.agentdash
        }
        $json = ($data | ConvertTo-Json -Depth 10) -replace "`r`n", "`n"
        [System.IO.File]::WriteAllText($configFile, $json.TrimEnd("`n") + "`n", $utf8NoBom)
        Write-Host "[agentdash] hooks registered (merged): $configFile"
    } else {
        Copy-Item -LiteralPath $configFile -Destination "$configFile.bak-agentdash" -Force
        [System.IO.File]::WriteAllText($configFile, $tplRaw, $utf8NoBom)
        Write-Warning "[agentdash] Corrupt $configFile backed up to $configFile.bak-agentdash and rebuilt"
    }
}

foreach ($skill in @('agentdash', 'agentdash-setup', 'agentdash-orchestrate')) {
    Copy-NormalizedUtf8 `
        -Src (Join-Path $here "skills\$skill\SKILL.md") `
        -Dst (Join-Path $agentsDir "skills\$skill\SKILL.md")
}
Write-Host "[agentdash] skills synced: $agentsDir\skills\{agentdash,agentdash-setup,agentdash-orchestrate}"

$gitignore = Join-Path $Target '.gitignore'
$hasEntry = $false
if (Test-Path -LiteralPath $gitignore -PathType Leaf) {
    $hasEntry = [bool](Select-String -LiteralPath $gitignore -Pattern '\.agentdash' -Quiet)
}
if (-not $hasEntry) {
    if (Test-Path -LiteralPath $gitignore -PathType Leaf) {
        $giText = [System.IO.File]::ReadAllText($gitignore, [System.Text.Encoding]::UTF8)
        if ($giText.Length -gt 0 -and $giText[-1] -ne "`n") {
            [System.IO.File]::AppendAllText($gitignore, "`n", $utf8NoBom)
        }
    }
    [System.IO.File]::AppendAllText($gitignore, ".agentdash/`n", $utf8NoBom)
}

Write-Host "[agentdash] antigravity-kit installed into $Target"
Write-Host "[agentdash] Note: start a new conversation or reload the window to activate newly added .agents/hooks.json and .agents/skills/"
