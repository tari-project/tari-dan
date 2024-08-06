#   Copyright 2023 The Tari Project
#   SPDX-License-Identifier: BSD-3-Clause

set -e

SOURCE_PATH="./src"
TYPES_DIR="types"
HELPERS_DIR="helpers"
INDEX_FILE="index.ts"
npx shx rm -rf $SOURCE_PATH/$TYPES_DIR
npx shx rm -rf $SOURCE_PATH/$INDEX_FILE

cargo test --workspace --exclude integration_tests export_bindings --features ts
npx shx mv ../dan_layer/bindings/src/types/* ./src/types/
npx shx rm -rf ../dan_layer/bindings/

# Add the license header
echo "//   Copyright 2023 The Tari Project" >> $SOURCE_PATH/$INDEX_FILE
echo "//   SPDX-License-Identifier: BSD-3-Clause" >> $SOURCE_PATH/$INDEX_FILE
echo "" >> $SOURCE_PATH/$INDEX_FILE

cd ./src
# Generate the index file
for file in $(find $TYPES_DIR -name "*.ts" -maxdepth 1 | sort); do
  MODULE_NAME="${file%.*}"
  echo "export * from './$MODULE_NAME';" >> $INDEX_FILE
done

for dir in $(find $TYPES_DIR -mindepth 1 -maxdepth 1 -type d | sort); do
  index_file="$(basename $dir).ts"
  if [ -f "$index_file" ]; then
    npx shx rm "$index_file"
  fi
  echo "//   Copyright 2023 The Tari Project" >> "$index_file"
  echo "//   SPDX-License-Identifier: BSD-3-Clause" >> "$index_file"
  echo "" >> "$index_file"
  for file in $(find $dir -name "*.ts" -maxdepth 1); do
    MODULE_NAME="${file%.*}"
    echo "export * from './$MODULE_NAME';" >> "$index_file"
  done
  # echo "export * from './$(basename $dir)';" >> $INDEX_FILE // TODO: solve namespace conflict between validator-node-client and tari-indexer-client
done

# Add helpers
for file in $(find $HELPERS_DIR -name "*.ts" | sort); do
  FILE_NAME=$(basename $file)
  if [ "$FILE_NAME" != "index.ts" ]; then
    MODULE_NAME="${FILE_NAME%.*}"
    echo "export * from './$HELPERS_DIR/$MODULE_NAME';" >> $INDEX_FILE
  fi
done

# This is temporary solution to the problem of 'Commitment' not being exported, and we have to do manual types in the
# code for BTreeMap<Commitment, ConfidentialOutput>. Because of this the ConfidentialOutput type is not imported.
echo "import { ConfidentialOutput } from './ConfidentialOutput';" >> $TYPES_DIR/ResourceContainer.ts

npx prettier --write "./**/*.{ts,tsx,css,json}" --log-level=warn
