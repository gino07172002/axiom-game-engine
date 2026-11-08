# Original procedural station illustration. No external art or model service required.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$advArtPath = Join-Path $PSScriptRoot '../content/adv/assets'
New-Item -ItemType Directory -Force -Path $advArtPath | Out-Null
$advBitmap = New-Object System.Drawing.Bitmap 1200,240
$advGraphics = [System.Drawing.Graphics]::FromImage($advBitmap)
$advGraphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$advGraphics.Clear([System.Drawing.ColorTranslator]::FromHtml('#0c1725'))
$advBrushes = @{}
function Brush([string]$Color) {
    if (-not $advBrushes.ContainsKey($Color)) { $advBrushes[$Color] = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($Color)) }
    return $advBrushes[$Color]
}
$advGraphics.FillEllipse((Brush '#25384a'), 825,18,125,125)
$advGraphics.FillRectangle((Brush '#162b38'), 0,134,1200,106)
$advGraphics.FillPolygon((Brush '#203b47'), [System.Drawing.Point[]]@(@(30,140),@(290,45),@(535,140) | ForEach-Object { New-Object System.Drawing.Point $_[0],$_[1] }))
$advGraphics.FillRectangle((Brush '#264650'), 100,134,365,66)
$advGraphics.FillRectangle((Brush '#345d62'), 145,155,62,45)
$advGraphics.FillRectangle((Brush '#345d62'), 235,155,62,45)
$advGraphics.FillRectangle((Brush '#e7b86a'), 325,151,55,49)
$advGraphics.FillRectangle((Brush '#7da9a7'), 525,72,6,144)
$advGraphics.FillRectangle((Brush '#e7c58d'), 495,68,66,8)
$advGraphics.FillRectangle((Brush '#2d505b'), 620,189,580,6)
$advGraphics.FillRectangle((Brush '#588080'), 0,214,1200,3)
$advGraphics.FillRectangle((Brush '#e7b86a'), 0,228,1200,2)
$advPen = New-Object System.Drawing.Pen ([System.Drawing.ColorTranslator]::FromHtml('#4e7d80')),1
for ($advX=0;$advX -lt 1200;$advX+=32) { $advGraphics.DrawLine($advPen,$advX,230,$advX+12,240) }
$advGraphics.FillEllipse((Brush '#d7eae1'), 700,137,13,13)
$advGraphics.FillRectangle((Brush '#65a9a1'), 700,150,13,33)
$advGraphics.FillEllipse((Brush '#d7eae1'), 743,137,13,13)
$advGraphics.FillRectangle((Brush '#d4a66a'), 743,150,13,33)
$advBitmap.Save((Join-Path $advArtPath 'station.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$advPen.Dispose()
foreach ($advBrush in $advBrushes.Values) { $advBrush.Dispose() }
$advGraphics.Dispose()
$advBitmap.Dispose()
