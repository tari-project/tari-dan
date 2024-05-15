# Set the directory path where TypeScript files are located
$directory = "./src"

# Get all TypeScript files in the directory
$typescriptFiles = Get-ChildItem -Path $directory -Filter *.ts -Recurse

# Loop through each TypeScript file
foreach ($file in $typescriptFiles) {
    # Read the content of the file
    $content = Get-Content -Path $file.FullName -Raw

    # Replace "\\" with "/"
    $newContent = $content -replace "\\\\", "/"

    # Write the updated content back to the file
    Set-Content -Path $file.FullName -Value $newContent
}

Write-Host "Conversion completed."
