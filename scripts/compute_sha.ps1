$ErrorActionPreference = 'Stop'
$commit = '614241f622f53c4eeff9890bdc4f31cfecc418b3'
$baseUrl = "https://huggingface.co/intfloat/multilingual-e5-small/raw/$commit"
$tempDir = Join-Path $env:TEMP 'e5-small-sha-pin'
if (Test-Path $tempDir) { Remove-Item $tempDir -Recurse -Force }
New-Item -ItemType Directory -Path $tempDir | Out-Null

$files = @('config.json', 'special_tokens_map.json', 'tokenizer_config.json')
foreach ($f in $files) {
    $dest = Join-Path $tempDir $f
    $url = "$baseUrl/$f"
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    $size = (Get-Item $dest).Length
    $hash = (Get-FileHash $dest -Algorithm SHA256).Hash.ToLower()
    Write-Output ("{0}|size={1}|sha256={2}" -f $f, $size, $hash)
}
