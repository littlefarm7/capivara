# Gera packaging/windows/capivara.ico a partir de ui/icon.svg (Edge headless + System.Drawing).
# Só é preciso rodar quando o ícone mudar — o .ico gerado fica versionado no repositório.
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\..\.."
$edge = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Google\Chrome\Application\chrome.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw 'Edge/Chrome não encontrado' }
$tmp = Join-Path $env:TEMP 'capivara-icon'
New-Item -ItemType Directory -Force $tmp | Out-Null
$svg = Get-Content "$root\ui\icon.svg" -Raw
Set-Content "$tmp\icon.html" "<html><body style='margin:0;background:transparent'><div style='width:512px;height:512px'>$svg</div><style>svg{width:512px;height:512px;display:block}</style></body></html>" -Encoding utf8
$png512 = "$tmp\icon512.png"
& $edge --headless=new --disable-gpu --hide-scrollbars --default-background-color=00000000 --window-size=512,512 "--screenshot=$png512" "file:///$($tmp -replace '\\','/')/icon.html" | Out-Null
Start-Sleep -Milliseconds 500
if (-not (Test-Path $png512)) { throw 'falha ao renderizar o SVG' }

Add-Type -AssemblyName System.Drawing
$src = [System.Drawing.Image]::FromFile($png512)
$sizes = 16, 24, 32, 48, 64, 128, 256
$pngs = foreach ($s in $sizes) {
  $bmp = New-Object System.Drawing.Bitmap $s, $s
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $g.DrawImage($src, 0, 0, $s, $s)
  $ms = New-Object System.IO.MemoryStream
  $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  , $ms.ToArray()
}
$src.Dispose()

# ICO com imagens PNG embutidas (suportado desde o Windows Vista)
$out = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $out
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
  $s = $sizes[$i]; $len = $pngs[$i].Length
  $w.Write([byte]($s % 256)); $w.Write([byte]($s % 256)); $w.Write([byte]0); $w.Write([byte]0)
  $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$len); $w.Write([uint32]$offset)
  $offset += $len
}
foreach ($p in $pngs) { $w.Write($p) }
[System.IO.File]::WriteAllBytes("$root\packaging\windows\capivara.ico", $out.ToArray())
Copy-Item $png512 "$root\packaging\windows\capivara-512.png" -Force
Write-Host "OK -> packaging\windows\capivara.ico"
