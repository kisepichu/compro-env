# hooks/expand-libraries.sh 設計

`hooks/expand-libraries.sh` は repo が同梱する言語非依存の submit preprocess フック。project-local
`[submit].preprocess = "hooks/expand-libraries.sh"` に配線されている。契約は既存
`hooks/submit-preprocess.sh` と同一 (`docs/commands/submit.md` §提出前 preprocess フック 参照)。

## 言語別分岐

```
case "$CE_LANGUAGE" in
  rust)      exec python3 "$(dirname "$0")/rust_expand.py" ;;
  cpp|lean)  exec cat ;;   # TODO: 別 issue で bundler を追加する
  *)         exec cat ;;
esac
```

## Rust bundler (`hooks/rust_expand.py`)

### 入出力

- stdin: 解法の `src/main.rs` (Rust source)。
- stdout: `mod` 宣言を再帰的に inline した展開後 source。末尾改行 1 個で normalize。
- exit 0: 成功。展開結果を採用。
- exit != 0: 展開失敗。stderr に理由を出し、`ce` は提出を中止する。
  - 1: file not found (`#[path]` 先のファイルが読めない)
  - 2: cycle detected
  - 3: non-UTF-8 file
  - その他: internal error

### 展開ルール

1. `#[path = "REL"] mod NAME;` — `REL` は entry file の親ディレクトリ相対で解決。
2. path 属性のない `mod NAME;` — Rust 標準の暗黙解決:
   - `<entry_dir>/NAME.rs`
   - `<entry_dir>/NAME/mod.rs`
   - どちらも無ければ **passthrough** (stderr に 1 行 warn、`mod NAME;` は元のまま残す)。
3. 各 `mod NAME;` を `mod NAME { <expanded body> }` に inline 置換。
4. body 中の mod 宣言も同様に再帰展開 (DFS)。
5. 展開済みファイルは `visited: set[abs_path]` に記録。再訪 → cycle として exit 2。

> **限界 (nesting 未追跡)**: bare `mod NAME;` の解決は常に「処理中のファイルの親ディレクトリ」を基準に行い、
> `mod outer { mod inner; }` のような inline module 内でも Rust 本来の `<outer>/inner.rs` を参照しない。
> inline module 内でファイルを include したい場合は必ず `#[path]` 属性を書く。sample 解法では bare `mod` を
> top-level に限る運用で実害なし。

### コメント/文字列の扱い

- **採用**: 素の regex で走査し、コメント / 文字列 literal 内の誤検出は許容する (sample 解法で発生しない前提)。
- **将来拡張**: `//` 行末コメント、`/* … */` ブロックコメント、`"…"` / `r"…"` / `r#"…"#` string literal を
  「同長スペース列」に置換したスキャン用バッファを別途作り、マッチ位置検出だけそちらで行う。実装コスト高のため
  必要になった時点で切り替える。

### entry file の決定

- 引数 (`python3 rust_expand.py <entry_file>`) を優先。
- 引数なしなら `$CE_SOURCE_FILE` env の親ディレクトリを entry_dir とする (`ce` から呼ぶ場合は必ず
  この env が渡る)。

## 別言語 bundler の追加方針

新しい言語で bundler を書く場合:

1. `hooks/<lang>_expand.py` (or `.sh`) を新規追加。stdin/stdout 契約は `rust_expand.py` と同じ。
2. `hooks/expand-libraries.sh` の case 分岐に 1 行加える。
3. `hooks/tests/fixtures/<lang>/` に fixture in/expected を置き、`hooks/tests/run.sh` に diff 比較を
   追加。
4. `docs/operations/library-expand.md` (このファイル) に該当節を追記。

アプリ (Rust クレート群) 側の変更は一切不要。
