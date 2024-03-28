#   Copyright 2023 The Tari Project
#   SPDX-License-Identifier: BSD-3-Clause

set -e

DIRECTORY_PATH="./src/types" # replace with your directory path
HELPERS_PATH="./src/helpers" # replace with your directory path
INDEX_FILE="./index.ts"
npx shx rm -rf $DIRECTORY_PATH/*

cargo test --workspace --exclude integration_tests export_bindings --features ts
npx shx mv ../dan_layer/bindings/src/types/* ./src/types/
npx shx rm -rf ../dan_layer/bindings/

# Remove the index file if it exists
if [ -f "$INDEX_FILE" ]; then
  npx shx rm "$INDEX_FILE"
fi

# Add the license header
echo "//   Copyright 2023 The Tari Project" >> $INDEX_FILE
echo "//   SPDX-License-Identifier: BSD-3-Clause" >> $INDEX_FILE
echo "" >> $INDEX_FILE

# Generate the index file
for file in $(find $DIRECTORY_PATH -name "*.ts" -maxdepth 1 | sort); do
  MODULE_NAME="${file%.*}"
  echo "export * from '$MODULE_NAME';" >> $INDEX_FILE
done

for dir in $(find $DIRECTORY_PATH -mindepth 1 -maxdepth 1 -type d | sort); do
  index_file="$(basename $dir).ts"
  if [ -f "$index_file" ]; then
    npx shx rm "$index_file"
  fi
  echo "//   Copyright 2023 The Tari Project" >> $index_file
  echo "//   SPDX-License-Identifier: BSD-3-Clause" >> $index_file
  echo "" >> $index_file
  for file in $(find $dir -name "*.ts" -maxdepth 1); do
    # FILE_NAME=$(basename $file)
    MODULE_NAME="${file%.*}"
    echo "export * from '$MODULE_NAME';" >> $index_file
  done
done

# Add helpers
for file in $(find $HELPERS_PATH -name "*.ts" | sort); do
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
