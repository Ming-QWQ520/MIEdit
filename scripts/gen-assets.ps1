# Generate demo assets: full-window background image (gradient + glow)
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
$dir = Join-Path $root "examples\assets"
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$out = Join-Path $dir "bg.png"

$w = 1960; $h = 1300
$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'

# diagonal dark-blue gradient
$rect = New-Object System.Drawing.Rectangle(0, 0, $w, $h)
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    $rect,
    [System.Drawing.Color]::FromArgb(255, 10, 15, 30),
    [System.Drawing.Color]::FromArgb(255, 24, 48, 106),
    55.0)
$g.FillRectangle($brush, $rect)

# top-right accent glow (concentric ellipses, fading)
for ($i = 0; $i -lt 7; $i++) {
    $alpha = [int](34 - $i * 4)
    if ($alpha -le 0) { break }
    $b = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb($alpha, 79, 140, 255))
    $size = 900 - $i * 120
    $cx = $w - 260; $cy = 170
    $g.FillEllipse($b, $cx - $size / 2, $cy - $size / 2, $size, $size)
    $b.Dispose()
}

# bottom-left secondary glow (cyan, faint)
for ($i = 0; $i -lt 5; $i++) {
    $alpha = [int](20 - $i * 3)
    if ($alpha -le 0) { break }
    $b = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb($alpha, 56, 189, 248))
    $size = 700 - $i * 130
    $cx = 240; $cy = $h - 180
    $g.FillEllipse($b, $cx - $size / 2, $cy - $size / 2, $size, $size)
    $b.Dispose()
}

# subtle dot grid
$dot = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(18, 255, 255, 255))
for ($x = 40; $x -lt $w; $x += 80) {
    for ($y = 40; $y -lt $h; $y += 80) {
        $g.FillEllipse($dot, $x, $y, 2, 2)
    }
}
$dot.Dispose()

$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Host "OK -> $out"
