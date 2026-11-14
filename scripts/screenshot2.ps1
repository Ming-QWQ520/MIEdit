param([string]$ProcessName, [string]$Out)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WS2 {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    public struct RECT { public int L, T, R, B; }
}
"@
$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Sort-Object StartTime -Descending | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if ($null -eq $proc) { Write-Error "window not found"; exit 1 }
$h = $proc.MainWindowHandle
[WS2]::ShowWindow($h, 9) | Out-Null
Start-Sleep -Milliseconds 200
$TOPMOST = New-Object IntPtr (-1); $NOTOPMOST = New-Object IntPtr (-2)
[WS2]::SetWindowPos($h, $TOPMOST, 0, 0, 0, 0, 0x43) | Out-Null
[WS2]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800
$r = New-Object WS2+RECT
[WS2]::GetWindowRect($h, [ref]$r) | Out-Null
$wd = $r.R - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap($wd, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size($wd, $ht)))
$g.Dispose(); $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
[WS2]::SetWindowPos($h, $NOTOPMOST, 0, 0, 0, 0, 0x3) | Out-Null
Write-Host "OK -> $Out ($wd x $ht)"
