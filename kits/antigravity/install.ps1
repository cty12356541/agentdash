# agentdash - antigravity-kit installer (PowerShell; use install.sh for Bash)
# Prerequisite: agentdash binary in PATH.
# Usage: .\install.ps1 [-Target <project-dir>] (defaults to current directory)
# Steps (idempotent, UTF-8 without BOM):
#   1. Validate target directory exists and agentdash is in PATH
#   2. Merge kits/shared/AGENTDASH.md marker block into <Target>\AGENTS.md
#   3. Register top-level "agentdash" namespace in <Target>\.agents\hooks.json
#      (preserves sibling top-level keys/plugins; backs up corrupt JSON to .bak-agentdash)
#   4. Copy skills/{agentdash,agentdash-setup,agentdash-orchestrate} into <Target>\.agents\skills\
#   5. Ensure .agentdash/ is listed in <Target>\.gitignore
param(
    [string]$Target = (Get-Location).Path
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not [System.IO.Path]::IsPathRooted($Target)) {
    $Target = Join-Path (Get-Location).Path $Target
}
$Target = [System.IO.Path]::GetFullPath($Target)
if (-not (Test-Path -LiteralPath $Target -PathType Container)) {
    Write-Error "[agentdash] Target directory does not exist: $Target"
}

# --- 1. Check agentdash in PATH ---
$agentdashCmd = Get-Command -Name 'agentdash' -ErrorAction SilentlyContinue
if (-not $agentdashCmd) {
    Write-Error "[agentdash] agentdash binary not found in PATH.`n  1) cargo install --path <agentdash-repo>   2) Download from Releases and add to PATH"
}

# --- 2. Idempotently merge AGENTDASH.md marker block into AGENTS.md ---
$blockFile = [System.IO.Path]::GetFullPath((Join-Path $here '..\shared\AGENTDASH.md'))
$blockText = ([System.IO.File]::ReadAllText($blockFile, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n").TrimEnd("`n") + "`n"
$agentsMd = Join-Path $Target 'AGENTS.md'

if (-not (Test-Path -LiteralPath $agentsMd -PathType Leaf)) {
    [System.IO.File]::WriteAllText($agentsMd, $blockText, $utf8NoBom)
    Write-Host "[agentdash] instructions created: $agentsMd"
} else {
    $existing = [System.IO.File]::ReadAllText($agentsMd, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n"
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
        [System.IO.File]::WriteAllText($agentsMd, $mergedText, $utf8NoBom)
        Write-Host "[agentdash] instructions refreshed: $agentsMd"
    } else {
        $prefix = $existing
        if ($prefix.Length -gt 0 -and -not $prefix.EndsWith("`n")) {
            $prefix += "`n"
        }
        [System.IO.File]::WriteAllText($agentsMd, $prefix + $blockText, $utf8NoBom)
        Write-Host "[agentdash] instructions appended: $agentsMd"
    }
}

# --- 3. Idempotently register "agentdash" namespace in .agents\hooks.json ---
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

# --- 4. Copy skills into .agents\skills\ ---
foreach ($skill in @('agentdash', 'agentdash-setup', 'agentdash-orchestrate')) {
    $dstSkillDir = Join-Path $agentsDir (Join-Path 'skills' $skill)
    New-Item -ItemType Directory -Force -Path $dstSkillDir | Out-Null
    $srcSkill = Join-Path $here (Join-Path 'skills' (Join-Path $skill 'SKILL.md'))
    $dstSkill = Join-Path $dstSkillDir 'SKILL.md'
    $skillContent = [System.IO.File]::ReadAllText($srcSkill, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n"
    [System.IO.File]::WriteAllText($dstSkill, $skillContent, $utf8NoBom)
}
Write-Host "[agentdash] skills synced: $agentsDir\skills\{agentdash,agentdash-setup,agentdash-orchestrate}"

# --- 5. Idempotently append .agentdash/ to .gitignore ---
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
