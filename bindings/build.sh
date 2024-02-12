#   Copyright 2023 The Tari Project
#   SPDX-License-Identifier: BSD-3-Clause

set -e

cargo test --workspace --exclude integration_tests export_bindings --features ts
npx shx mv ../dan_layer/bindings/src/types/* ./src/types/
npx shx rm -rf ../dan_layer/bindings/
DIRECTORY_PATH="./src/types" # replace with your directory path
HELPERS_PATH="./src/helpers" # replace with your directory path
INDEX_FILE="./index.ts"

# Remove the index file if it exists
if [ -f "$INDEX_FILE" ]; then
  npx shx rm "$INDEX_FILE"
fi

# Add the license header
echo "//   Copyright 2023 The Tari Project" >> $INDEX_FILE
echo "//   SPDX-License-Identifier: BSD-3-Clause" >> $INDEX_FILE
echo "" >> $INDEX_FILE

# Generate the index file
for file in $(find $DIRECTORY_PATH -name "*.ts"); do
  FILE_NAME=$(basename $file)
  if [ "$FILE_NAME" != "index.ts" ]; then
    MODULE_NAME="${FILE_NAME%.*}"
    echo "export * from '$DIRECTORY_PATH/$MODULE_NAME';" >> $INDEX_FILE
  fi
done

# Add helpers
for file in $(find $HELPERS_PATH -name "*.ts"); do
  FILE_NAME=$(basename $file)
  if [ "$FILE_NAME" != "index.ts" ]; then
    MODULE_NAME="${FILE_NAME%.*}"
    echo "export * from '$HELPERS_PATH/$MODULE_NAME';" >> $INDEX_FILE
  fi
done

# This is temporary solution to the problem of 'Commitment' not being exported, and we have to do manual types in the
# code for BTreeMap<Commitment, ConfidentialOutput>. Because of this the ConfidentialOutput type is not imported.
echo "import { ConfidentialOutput } from './ConfidentialOutput';" >> $DIRECTORY_PATH/ResourceContainer.ts

npx prettier --write "./**/*.{ts,tsx,css,json}" --log-level=warn
