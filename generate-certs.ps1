# PowerShell script to generate root CA, CSR, and signed localhost certificate in PEM format
# Usage: .\generate-certs.ps1 [-OutputPath <path>]
# If -OutputPath is not specified, certificates will be generated in the script's directory

param(
    [Parameter(Mandatory=$false)]
    [ValidateNotNullOrEmpty()]
    [string]$OutputPath = (Split-Path -Parent $MyInvocation.MyCommand.Path)
)

# Ensure output directory exists
if (-not (Test-Path $OutputPath)) {
    New-Item -ItemType Directory -Path $OutputPath | Out-Null
}

Write-Host "Generating certificates in: $OutputPath"

# Step 1: Create Root CA Certificate
Write-Host "`n[1/5] Creating Root CA certificate..."
$rootCA = New-SelfSignedCertificate -Type Custom `
    -KeyExportPolicy Exportable `
    -Subject "CN=Root CA,O=Test Organization,C=US" `
    -KeyUsage CertSign, CRLSign `
    -KeyLength 2048 `
    -NotAfter (Get-Date).AddYears(10) `
    -CertStoreLocation "Cert:\CurrentUser\My"

Write-Host "Root CA Thumbprint: $($rootCA.Thumbprint)"

# Step 2: Create a server certificate request
Write-Host "`n[2/5] Creating server certificate signing request..."
$serverCert = New-SelfSignedCertificate -Type Custom `
    -KeyExportPolicy Exportable `
    -Subject "CN=localhost,O=Test Organization,C=US" `
    -DnsName localhost, 127.0.0.1, "::1" `
    -KeyUsage DigitalSignature, KeyEncipherment `
    -KeyLength 2048 `
    -NotAfter (Get-Date).AddYears(1) `
    -CertStoreLocation "Cert:\CurrentUser\My"

Write-Host "Server Cert Thumbprint: $($serverCert.Thumbprint)"

# Step 3: Sign the server certificate with root CA
Write-Host "`n[3/5] Signing server certificate with Root CA..."
$signedCert = New-SelfSignedCertificate -Type Custom `
    -KeyExportPolicy Exportable `
    -Subject "CN=localhost,O=Test Organization,C=US" `
    -DnsName localhost, 127.0.0.1, "::1" `
    -KeyUsage DigitalSignature, KeyEncipherment `
    -KeyLength 2048 `
    -Signer $rootCA `
    -NotAfter (Get-Date).AddYears(1) `
    -CertStoreLocation "Cert:\CurrentUser\My"

Write-Host "Signed Cert Thumbprint: $($signedCert.Thumbprint)"

# Step 4: Export Root CA to PEM format
Write-Host "`n[4/5] Exporting Root CA to PEM format..."
$rootCAPath = Join-Path $OutputPath "rootCA.pem"
$rootCABytes = $rootCA.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Cert)
$rootCABase64 = [Convert]::ToBase64String($rootCABytes, 'InsertLineBreaks')
$rootCAContent = "-----BEGIN CERTIFICATE-----`r`n$rootCABase64`r`n-----END CERTIFICATE-----"
Set-Content -Path $rootCAPath -Value $rootCAContent -Encoding UTF8

Write-Host "Root CA saved to: $rootCAPath"

# Step 5: Export signed server certificate and private key to PEM format
Write-Host "`n[5/5] Exporting server certificate and private key to PEM format..."

# Export certificate
$certPath = Join-Path $OutputPath "localhost.crt"
$certBytes = $signedCert.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Cert)
$certBase64 = [Convert]::ToBase64String($certBytes, 'InsertLineBreaks')
$certContent = "-----BEGIN CERTIFICATE-----`r`n$certBase64`r`n-----END CERTIFICATE-----"
Set-Content -Path $certPath -Value $certContent -Encoding UTF8

Write-Host "Certificate saved to: $certPath"

# Export private key
$keyPath = Join-Path $OutputPath "localhost.key"

# Retrieve the certificate with private key from the store
$certStore = New-Object System.Security.Cryptography.X509Certificates.X509Store([System.Security.Cryptography.X509Certificates.StoreName]::My, [System.Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser)
$certStore.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadOnly)
$certs = $certStore.Certificates.Find([System.Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint, $signedCert.Thumbprint, $false)
$certWithKey = $certs[0]
$certStore.Close()

# Get the RSA private key using the extension method
$rsaKey = [System.Security.Cryptography.X509Certificates.RSACertificateExtensions]::GetRSAPrivateKey($certWithKey)

# Export as PKCS8 format (required by Rust's PrivateKeyDer::from_pem_file())
[System.Security.Cryptography.CngKey]$cngKey = $null
$keyObject = $rsaKey
if ($keyObject -is [System.Security.Cryptography.RSACng]) {
    # RSACng has a Key property that gives us the CngKey
    $cngKey = $keyObject.Key
    # Export PKCS8 from CNG key
    $keyBytes = $cngKey.Export([System.Security.Cryptography.CngKeyBlobFormat]::Pkcs8PrivateBlob)
} else {
    # Fallback for other RSA implementations
    $keyBytes = $rsaKey.Key.Export([System.Security.Cryptography.CngKeyBlobFormat]::Pkcs8PrivateBlob)
}

$keyBase64 = [Convert]::ToBase64String($keyBytes, 'InsertLineBreaks')
$keyContent = "-----BEGIN PRIVATE KEY-----`r`n$keyBase64`r`n-----END PRIVATE KEY-----"
Set-Content -Path $keyPath -Value $keyContent -Encoding UTF8

Write-Host "Private key saved to: $keyPath"

# Verify files were created
Write-Host ""
Write-Host "[OK] Certificate generation complete!"
Write-Host "Generated files:"
Get-Item -Path @($rootCAPath, $certPath, $keyPath) -ErrorAction SilentlyContinue | ForEach-Object {
    $size = $_.Length
    Write-Host "  - $($_.Name) ($size bytes)"
}

Write-Host ""
Write-Host "Trust relationship: localhost.crt is signed by rootCA.pem"
Write-Host "A client with rootCA.pem can verify the localhost.crt certificate."
