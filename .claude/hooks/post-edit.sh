#!/bin/bash
# PostToolUse hook: Write/Edit
# 実装ファイルを変更したとき、仕様同期の確認を促す

FILE=$(cat | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('file_path',''))" 2>/dev/null || echo "")

# crates/ 以下の .rs ファイル (テストファイル除く)
if [[ "$FILE" =~ /crates/.+\.rs$ ]] && [[ "$FILE" != */tests/* ]] && [[ "$FILE" != *_test.rs ]]; then
  echo "[spec-sync] 実装を変更しました。仕様との整合は /spec-review で確認してください。"
fi
