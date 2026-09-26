# bilibili-api-sync / sync.ps1
# Status + self-check for the Bilibili contracts used by the favorites module.
# NOTE: PowerShell 5.1 - this file must stay pure ASCII (no CJK), otherwise
# the parser decodes it as ANSI and fails.
#
# Usage:
#   powershell -NoProfile -File .agents/skills/bilibili-api-sync/scripts/sync.ps1
#   $env:BILI_COOKIE = "<cookie with SESSDATA>"
#   powershell -NoProfile -File .agents/skills/bilibili-api-sync/scripts/sync.ps1 -Live

param(
    [switch]$Live
)

$ErrorActionPreference = "Continue"
# scripts/ -> <skill> -> skills -> .agents -> repo root
$SkillDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path (Join-Path $SkillDir "..\..\..\..")
$Upstream = "E:\pro\other-sdk\tools\bilibili-API-collect"

function Write-Section($text) {
    Write-Output ""
    Write-Output ("=== " + $text + " ===")
}

Write-Section "sync-point"
$point = Join-Path (Split-Path -Parent $SkillDir) "sync-point.txt"
if (Test-Path $point) {
    # PS 5.1: Get-Content decodes UTF-8 without BOM as ANSI - read as UTF8 explicitly
    [System.IO.File]::ReadAllLines($point, [System.Text.Encoding]::UTF8) | Select-Object -First 14
} else {
    Write-Output "sync-point.txt not found"
}

Write-Section "upstream repo (expected: deprecated)"
if (Test-Path (Join-Path $Upstream ".git")) {
    $head = & git -C $Upstream rev-parse --short HEAD 2>$null
    $branch = & git -C $Upstream rev-parse --abbrev-ref HEAD 2>$null
    Write-Output ("path   : " + $Upstream)
    Write-Output ("branch : " + $branch)
    Write-Output ("head   : " + $head)
    & git -C $Upstream fetch --depth 60 origin 2>$null | Out-Null
    $after = & git -C $Upstream rev-parse --short HEAD 2>$null
    if ($head -ne $after) {
        Write-Output ("UPDATED: " + $head + " -> " + $after)
        & git -C $Upstream log --oneline "$head..$after" | Select-Object -First 20
    } else {
        Write-Output "no new commits upstream"
    }
    $rm = Join-Path $Upstream "README.md"
    if (Test-Path $rm) {
        $first = (Get-Content $rm -TotalCount 1 -ErrorAction SilentlyContinue)
        if ($first -match "Deprecated") {
            Write-Output "STATUS : upstream is deprecated - no docs to sync"
        }
    }
} else {
    Write-Output ("upstream clone missing: " + $Upstream)
    Write-Output "clone with: git clone --depth 60 https://github.com/SocialSisterYi/bilibili-API-collect.git"
}

Write-Section "self-check: wbi signature vectors (cargo test)"
$core = Join-Path $RepoRoot "src-tauri"
if (Test-Path $core) {
    Push-Location $core
    & cargo test --lib favorites::wbi 2>&1 | Select-String -Pattern "test result|^error|panicked" | Select-Object -First 10
    Pop-Location
} else {
    Write-Output ("src-tauri not found under " + $RepoRoot)
}

if ($Live) {
    Write-Section "live probe: /x/web-interface/nav (read-only)"
    $cookie = $env:BILI_COOKIE
    if (-not $cookie) {
        Write-Output "BILI_COOKIE not set - skipping live probe"
    } else {
        try {
            $resp = Invoke-RestMethod -Uri "https://api.bilibili.com/x/web-interface/nav" -Headers @{
                "Cookie"      = $cookie
                "User-Agent"  = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
                "Referer"     = "https://www.bilibili.com/"
            } -Method Get -TimeoutSec 20
            $code = $resp.code
            Write-Output ("code    : " + $code)
            if ($code -eq 0) {
                Write-Output ("mid     : " + $resp.data.mid)
                $img = $resp.data.wbi_img.img_url
                $sub = $resp.data.wbi_img.sub_url
                if ($img -and $sub) {
                    Write-Output ("wbi img : " + $img)
                    Write-Output ("wbi sub : " + $sub)
                    Write-Output "OK: login live and wbi keys still served"
                } else {
                    Write-Output "WARN: wbi_img missing - key acquisition changed"
                }
            } else {
                Write-Output ("WARN: bussiness code " + $code + " - cookie likely expired")
            }
        } catch {
            Write-Output ("probe failed: " + $_.Exception.Message)
        }
    }
}

Write-Section "next"
Write-Output "If wbi vectors fail or wbi_img is missing, read references/contracts.md"
Write-Output "and report a plan to the user before editing wbi.rs / bilibili.rs."
