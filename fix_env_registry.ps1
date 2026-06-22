# Fix Registry Environment Variable Types
# Converts REG_SZ to REG_EXPAND_SZ for values containing %
# This fixes: %SystemRoot%, %USERPROFILE%, etc. not being expanded
# Run as Administrator for HKLM fixes

$ErrorActionPreference = "Continue"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Registry Env Type Fix Tool" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

function Repair-EnvKey {
    param($RegPath, $Label)
    
    Write-Host "--- $Label ---" -ForegroundColor Yellow
    Write-Host "  Path: $RegPath" -ForegroundColor DarkGray
    
    try {
        $key = Get-Item -Path $RegPath -ErrorAction Stop
    } catch {
        Write-Host "  [SKIP] Cannot access (need Admin?)" -ForegroundColor DarkYellow
        return
    }
    
    $fixed = 0
    $ok = 0
    
    foreach ($prop in $key.Property) {
        $kind = $key.GetValueKind($prop)
        $value = $key.GetValue($prop, $null)
        
        if ($null -eq $value) { continue }
        if ($value -isnot [string]) { continue }
        
        $hasPercent = $value.Contains('%')
        $correctKind = if ($hasPercent) { "ExpandString" } else { "String" }
        $needsFix = ($kind -ne $correctKind)
        
        if ($needsFix) {
            Set-ItemProperty -Path $RegPath -Name $prop -Value $value -Type $correctKind -Force
            Write-Host "  [FIX] $($prop): $kind -> $correctKind" -ForegroundColor Green
            $fixed++
        } else {
            if ($prop -eq "SystemRoot" -or $prop -eq "Path" -or $prop -eq "windir" -or $prop -eq "TEMP" -or $prop -eq "TMP" -or $prop -eq "USERPROFILE" -or $prop -eq "ComSpec") {
                Write-Host "  [OK]  $($prop) = $correctKind" -ForegroundColor Gray
            }
            $ok++
        }
    }
    
    Write-Host "  Result: fixed=$fixed, ok=$ok" -ForegroundColor White
    Write-Host ""
}

# 1. HKCU
Repair-EnvKey -RegPath "HKCU:\Environment" -Label "HKEY_CURRENT_USER\Environment"

# 2. HKLM (needs admin) - THIS IS THE CRITICAL ONE for SystemRoot
Repair-EnvKey -RegPath "HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" -Label "HKLM\SYSTEM\...\Environment"

# 3. Broadcast change
Write-Host "Broadcasting env change..." -ForegroundColor Cyan
try {
    $sig = @'
[DllImport("user32.dll", CharSet = CharSet.Auto)]
public static extern IntPtr SendMessageTimeout(
    IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam,
    uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
'@
    $type = Add-Type -MemberDefinition $sig -Name "NativeMethods" -Namespace "Win32" -PassThru
    $result = [UIntPtr]::Zero
    $type::SendMessageTimeout(0xffff, 0x001a, [UIntPtr]::Zero, "Environment", 2, 5000, [ref]$result) | Out-Null
    Write-Host "Broadcast done." -ForegroundColor Green
} catch {
    Write-Host "Broadcast warning (non-critical): $_" -ForegroundColor DarkYellow
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Done!" -ForegroundColor Green
Write-Host ""
Write-Host "  Please RESTART your computer for the" -ForegroundColor Yellow
Write-Host "  change to take full effect, then try" -ForegroundColor Yellow
Write-Host "  Control Panel -> Advanced System Settings" -ForegroundColor Yellow
Write-Host "========================================" -ForegroundColor Cyan

Read-Host "Press Enter to exit"
