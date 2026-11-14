# 按进程名在主窗口客户区内模拟鼠标点击（坐标为窗口内相对位置）
param(
    [Parameter(Mandatory = $true)][string]$ProcessName,
    [Parameter(Mandatory = $true)][int]$X,
    [Parameter(Mandatory = $true)][int]$Y
)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32Click {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    public struct RECT { public int L, T, R, B; }
}
"@

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if ($null -eq $proc) { Write-Error "window not found: $ProcessName"; exit 1 }

$h = $proc.MainWindowHandle
[Win32Click]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 300

$r = New-Object Win32Click+RECT
[Win32Click]::GetWindowRect($h, [ref]$r) | Out-Null
$ax = $r.L + $X; $ay = $r.T + $Y

[Win32Click]::SetCursorPos($ax, $ay) | Out-Null
Start-Sleep -Milliseconds 120
[Win32Click]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)  # LEFTDOWN
Start-Sleep -Milliseconds 60
[Win32Click]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)  # LEFTUP
Write-Host "clicked ($ax, $ay)"
