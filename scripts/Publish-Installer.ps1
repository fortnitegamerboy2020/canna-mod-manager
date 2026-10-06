param([string]$Server = 'canna-admin@165.227.83.76', [string]$Identity = "$env:USERPROFILE/.ssh/canna_server_ed25519")
$ErrorActionPreference = 'Stop'
$cannaRoot = Split-Path $PSScriptRoot -Parent
$cannaManifestPath = Join-Path $cannaRoot 'dist/installer-latest.json'
$cannaManifest = Get-Content -LiteralPath $cannaManifestPath -Raw | ConvertFrom-Json
if ($cannaManifest.version -notmatch '^v\d+\.\d+\.\d+$') { throw 'Invalid installer version.' }
if ($cannaManifest.sha256 -notmatch '^[a-f0-9]{64}$') { throw 'Invalid installer digest.' }
$cannaVersion = $cannaManifest.version.Substring(1)
$cannaSetup = Join-Path $cannaRoot "dist/Canna-Setup-$cannaVersion.exe"
if ((Get-FileHash -LiteralPath $cannaSetup -Algorithm SHA256).Hash.ToLowerInvariant() -ne $cannaManifest.sha256 -or (Get-Item -LiteralPath $cannaSetup).Length -ne $cannaManifest.size) { throw 'Installer does not match its manifest.' }
$cannaLive = Invoke-RestMethod 'https://cannamods.vip/updates/latest'
if ($cannaLive.tag_name -ne $cannaManifest.version) { throw 'Installer version does not match the public desktop release.' }
& scp -i $Identity -o BatchMode=yes $cannaSetup "${Server}:/home/canna-admin/canna-setup-upload.exe"
if ($LASTEXITCODE -ne 0) { throw 'Installer upload failed.' }
& scp -i $Identity -o BatchMode=yes $cannaManifestPath "${Server}:/home/canna-admin/canna-installer-upload.json"
if ($LASTEXITCODE -ne 0) { throw 'Manifest upload failed.' }
$cannaRemote = "set -e; test `"`$(sha256sum /home/canna-admin/canna-setup-upload.exe | cut -d ' ' -f 1)`" = '$($cannaManifest.sha256)'; sudo install -m 644 /home/canna-admin/canna-setup-upload.exe /opt/canna/releases/v$cannaVersion-setup.exe; sudo install -m 644 /home/canna-admin/canna-installer-upload.json /opt/canna/releases/installer-latest.json.tmp; sudo mv /opt/canna/releases/installer-latest.json.tmp /opt/canna/releases/installer-latest.json; rm /home/canna-admin/canna-setup-upload.exe /home/canna-admin/canna-installer-upload.json"
& ssh -i $Identity -o BatchMode=yes $Server $cannaRemote
if ($LASTEXITCODE -ne 0) { throw 'Installer publication failed.' }
Write-Output "Published installer $($cannaManifest.version)."
