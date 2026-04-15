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
2. `test_command` を `sh -c` 経由で実行する
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
use proconio::input;

fn solve<R: std::io::BufRead>(src: &mut impl proconio::source::Source<R>) -> String {
    let _ = src; // Replace with: input! { from src, n: usize, a: [i64; n] }
    todo!()
}

fn main() {
    use proconio::source::line::LineSource;
    use std::io::BufReader;
    let src = &mut LineSource::new(BufReader::new(std::io::stdin()));
    print!("{}", solve(src));
}

#[cfg(test)]
mod tests {
    use super::*;
    use proconio::source::once::OnceSource;

    // ↓ Add test cases here
    const CASES: &[(&str, &str)] = &[
        {% for sample in samples -%}
        ({{ sample.input | json_encode() }}, {{ sample.output | json_encode() }}),
        {% endfor -%}
    ];

    #[test]
    fn test_samples() {
        for (i, &(input, expected)) in CASES.iter().enumerate() {
            let src = &mut OnceSource::from(input);
            assert_eq!(solve(src).trim(), expected.trim(), "case {}", i + 1);
        }
    }
}
```

### C++: コンパイル + テストケースごとに実行

`templates/cpp/ce.toml.tera`:
```toml
test_command = """
g++ -O2 -o a.out main.cpp || exit 1
set -- "$CE_TESTCASES_DIR"/*.in
if [ ! -e "$1" ]; then echo "no testcases found in $CE_TESTCASES_DIR"; exit 1; fi
result=0
for f in "$@"; do
    expected_file="${f%.in}.out"
    actual_file=$(mktemp) || exit 1
    stderr_file=$(mktemp) || { rm -f "$actual_file"; exit 1; }
    if ./a.out < "$f" > "$actual_file" 2> "$stderr_file"; then
        if diff -u "$expected_file" "$actual_file" > /dev/null; then
            echo "AC: $(basename "$f")"
        else
            echo "WA: $(basename "$f")"
            diff -u "$expected_file" "$actual_file"
            result=1
        fi
    else
        exit_code=$?
        echo "RE: $(basename "$f") (exit code: $exit_code)"
        if [ -s "$actual_file" ]; then echo "--- stdout ---"; cat "$actual_file"; fi
        if [ -s "$stderr_file" ]; then echo "--- stderr ---" >&2; cat "$stderr_file" >&2; fi
        result=1
    fi
    rm -f "$actual_file" "$stderr_file"
done
exit $result
"""
```

## 将来拡張

- TLE 判定 (time_limit をテストケースメタデータから取得)
