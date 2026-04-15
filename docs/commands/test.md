# ce test

## 概要

解法ディレクトリの `ce.toml` に定義されたテストコマンドを実行する。
テストケースの照合方法・出力形式はテンプレートでユーザーが自由に定義する。

## シグネチャ

```
ce test <contest_id> <problem_code> [solution_name]
```

- `contest_id`: コンテスト ID
- `problem_code`: 問題コード
- `solution_name`: 解法名 (省略時: `main`)

## 挙動

1. `solutions/{contest_id}/{problem_code}/{solution_name}/ce.toml` を読む
2. `test_command` を `/bin/sh -c` 経由で実行する
   - 作業ディレクトリ: 解法ディレクトリ (`solutions/{contest_id}/{problem_code}/{solution_name}/`)
   - 環境変数 `CE_TESTCASES_DIR` に `solutions/{contest_id}/testcases/{problem_code}/` の絶対パスをセット
3. 標準出力・標準エラーはそのまま端末に流す
4. `test_command` の終了コードをそのまま `ce test` の終了コードとして返す

将来的には `ce sub` がこの終了コードを参照し、0 以外なら提出をスキップする想定（詳細: `docs/commands/submit.md`）。

## テンプレートでの定義

`templates/{lang}/ce.toml.tera` を `ce init` 時に Tera でレンダリングして解法ディレクトリに `ce.toml` を生成する。

**利用可能な Tera 変数**:

| 変数 | 内容 |
| --- | --- |
| `contest.id` | コンテスト ID (例: `abc334`) |
| `problem.code` | 問題コード (例: `a`) |
| `problem.title` | 問題タイトル |
| `solution.name` | 解法名 (例: `main`) |
| `samples` | サンプルテストケースの配列。各要素は `input: String`, `output: String` を持つ |

`samples` は他の `.tera` ファイルでも利用可能（例: `main.rs.tera` でテストコードを生成）。

生成された `ce.toml` はユーザーが自由に編集できる。解法ディレクトリが既に存在する場合は再生成しない。

## エラーケース

- 解法ディレクトリが存在しない: `ce init` を実行するよう促して exit 1
- 解法ディレクトリはあるが `ce.toml` がない: テンプレートに `ce.toml.tera` を追加するよう促して exit 1
- `test_command` キーが未定義: エラーメッセージを表示して exit 1
- コマンド起動失敗 (`sh` が見つからない等): エラーメッセージを表示して exit 1

---

## 例

### Rust: `cargo test` でサンプルテストを実行

`ce init` 時に `main.rs.tera` でサンプルを `#[test]` として埋め込み、`cargo test` で実行する設計例。

`templates/rust/ce.toml.tera`:
```toml
test_command = "cargo test"
```

`templates/rust/src/main.rs.tera`:
```rust
use std::io::Read;

fn solve(input: &str) -> String {
    todo!()
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    print!("{}", solve(&input));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str) -> String {
        solve(input)
    }

    {% for sample in samples -%}
    #[test]
    fn test_sample_{{ loop.index }}() {
        let input = r#"{{ sample.input }}"#;
        let expected = r#"{{ sample.output }}"#;
        assert_eq!(run(input).trim(), expected.trim());
    }
    {% endfor -%}
}
```

### C++: コンパイル + テストケースごとに実行

`templates/cpp/ce.toml.tera`:
```toml
test_command = """
g++ -O2 -o a.out main.cpp || exit 1
result=0
for f in "$CE_TESTCASES_DIR"/*.in; do
    expected=$(cat "${f%.in}.out")
    actual=$(./a.out < "$f")
    if [ "$actual" = "$expected" ]; then
        echo "AC: $(basename "$f")"
    else
        echo "WA: $(basename "$f")"
        echo "  expected: $expected"
        echo "  actual:   $actual"
        result=1
    fi
done
exit $result
"""
```

## 将来拡張

- TLE 判定 (time_limit をテストケースメタデータから取得)
