$ErrorActionPreference = 'Stop'
$taskInvitePath = Join-Path ([Environment]::GetFolderPath('UserProfile')) 'Downloads\Canna-First-Admin-Invite.txt.dpapi'
$secret = ConvertTo-SecureString ([IO.File]::ReadAllText($taskInvitePath))
$ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret)
try {
    $text = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr)
    $code = [regex]::Match($text, '(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])').Value
    if (!$code) { throw 'Invitation code not found in protected file' }
    Set-Clipboard -Value $code
    Write-Host 'Administrator invitation copied. Paste it into the Canna account form.'
    Write-Host 'Clear your clipboard after creating your account.'
} finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr)
    $text = $null; $code = $null
}
