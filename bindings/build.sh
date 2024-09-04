#   Copyright 2023 The Tari Project
#   SPDX-License-Identifier: BSD-3-Clause

set -e

SOURCE_PATH="./src"
TYPES_DIR="types"
DIST_DIR="dist"
HELPERS_DIR="helpers"
MAIN_INDEX_FILE="index.ts"

if [ -f "$SOURCE_PATH/$TYPES_DIR" ]; then
  npx shx rm -rf $SOURCE_PATH/$TYPES_DIR
fi
if [ -f "$SOURCE_PATH/$MAIN_INDEX_FILE" ]; then
  npx shx rm -rf $SOURCE_PATH/$MAIN_INDEX_FILE
fi
if [ -f "$SOURCE_PATH/$DIST_DIR" ]; then
  npx shx rm -rf ./$DIST_DIR
fi

cargo test --workspace --exclude integration_tests export_bindings --features ts
npx shx mv ../dan_layer/bindings/src/types/* ./src/types/
npx shx rm -rf ../dan_layer/bindings/

# Add the license header
echo "//   Copyright 2023 The Tari Project" >> $SOURCE_PATH/$MAIN_INDEX_FILE
echo "//   SPDX-License-Identifier: BSD-3-Clause" >> $SOURCE_PATH/$MAIN_INDEX_FILE
echo "" >> $SOURCE_PATH/$MAIN_INDEX_FILE

cd ./src
# Generate the index file
for file in $(find $TYPES_DIR -name "*.ts" -maxdepth 1 | sort); do
  MODULE_NAME="${file%.*}"
  echo "export * from './$MODULE_NAME';" >> $MAIN_INDEX_FILE
done

for dir in $(find $TYPES_DIR -mindepth 1 -maxdepth 1 -type d | sort); do
  module_dir_name="$(basename $dir)"
  module_export_file="$module_dir_name.ts"
  if [ -f "$module_export_file" ]; then
    npx shx rm "$module_export_file"
  fi
  echo "//   Copyright 2023 The Tari Project" >> "$module_export_file"
  echo "//   SPDX-License-Identifier: BSD-3-Clause" >> "$module_export_file"
  echo "" >> "$module_export_file"
  for file in $(find $dir -name "*.ts" -maxdepth 1); do
    MODULE_NAME="${file%.*}"
    echo "export * from './$MODULE_NAME';" >> "$module_export_file"
  done
  echo "export * from './$module_dir_name';" >> $MAIN_INDEX_FILE
done

# Add helpers
for file in $(find $HELPERS_DIR -name "*.ts" | sort); do
  FILE_NAME=$(basename $file)
  if [ "$FILE_NAME" != "index.ts" ]; then
    MODULE_NAME="${FILE_NAME%.*}"
    echo "export * from './$HELPERS_DIR/$MODULE_NAME';" >> $MAIN_INDEX_FILE
  fi
done

# This is temporary solution to the problem of 'Commitment' not being exported, and we have to do manual types in the
# code for BTreeMap<Commitment, ConfidentialOutput>. Because of this the ConfidentialOutput type is not imported.
echo "import { ConfidentialOutput } from './ConfidentialOutput';" >> $TYPES_DIR/ResourceContainer.ts

npx prettier --write "./**/*.{ts,tsx,css,json}" --log-level=warn
