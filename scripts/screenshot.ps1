# 按进程名截取主窗口图像（先置顶窗口避免遮挡）
param(
    [Parameter(Mandatory = $true)][string]$ProcessName,
    [Parameter(Mandatory = $true)][string]$Out
)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32Shot {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool repaint);
    public struct RECT { public int L, T, R, B; }
}
"@

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if ($null -eq $proc) { Write-Error "window not found: $ProcessName"; exit 1 }

$h = $proc.MainWindowHandle
$SW_RESTORE = 9
[Win32Shot]::ShowWindow($h, $SW_RESTORE) | Out-Null
Start-Sleep -Milliseconds 200
$TOPMOST = New-Object IntPtr (-1)
$NOTOPMOST = New-Object IntPtr (-2)
$SWP_NOMOVE = 0x2; $SWP_NOSIZE = 0x1; $SWP_SHOWWINDOW = 0x40
[Win32Shot]::SetWindowPos($h, $TOPMOST, 0, 0, 0, 0, ($SWP_NOMOVE -bor $SWP_NOSIZE -bor $SWP_SHOWWINDOW)) | Out-Null
[Win32Shot]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 900

$r = New-Object Win32Shot+RECT
[Win32Shot]::GetWindowRect($h, [ref]$r) | Out-Null
$wd = $r.R - $r.L; $ht = $r.B - $r.T
if ($wd -le 0 -or $ht -le 0) { Write-Error "bad window rect"; exit 1 }

$bmp = New-Object System.Drawing.Bitmap($wd, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size($wd, $ht)))
$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

[Win32Shot]::SetWindowPos($h, $NOTOPMOST, 0, 0, 0, 0, ($SWP_NOMOVE -bor $SWP_NOSIZE)) | Out-Null
Write-Host "OK -> $Out ($wd x $ht)"
