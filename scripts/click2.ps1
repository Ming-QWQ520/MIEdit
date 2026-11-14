param([string]$ProcessName, [int]$X, [int]$Y)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WC2 {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    public struct RECT { public int L, T, R, B; }
}
"@
$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Sort-Object StartTime -Descending | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if ($null -eq $proc) { Write-Error "window not found"; exit 1 }
$h = $proc.MainWindowHandle
[WC2]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 300
$r = New-Object WC2+RECT
[WC2]::GetWindowRect($h, [ref]$r) | Out-Null
[WC2]::SetCursorPos($r.L + $X, $r.T + $Y) | Out-Null
Start-Sleep -Milliseconds 120
[WC2]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 60
[WC2]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Write-Host "clicked ($($r.L + $X), $($r.T + $Y))"
