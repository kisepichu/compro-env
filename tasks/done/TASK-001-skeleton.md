# TASK-001: プロジェクト骨格 + ドメインモデル実装

## 参照仕様

- docs/spec.md
- docs/PLAN.md

## 実装チェックリスト

### Cargo ワークスペース

- [x] ルート Cargo.toml (workspace + ce バイナリ)
- [x] crates/domain/Cargo.toml
- [x] crates/usecases/Cargo.toml
- [x] crates/interfaces/Cargo.toml
- [x] crates/infrastructure/Cargo.toml

### domain/

- [x] entity.rs: OJKind, Language, Contest, Problem, Sample, Solution, Session, SubmitResult
- [x] error.rs: CeError (thiserror)

### usecases/

- [x] repository/contest_repository.rs: ContestRepository trait
- [x] repository/solution_repository.rs: SolutionRepository trait
- [x] repository/session_repository.rs: SessionRepository trait
- [x] online_judge.rs: OnlineJudge trait
- [x] config.rs: Config trait
- [x] service.rs: Service struct (フィールド定義)
- [x] service/login.rs: todo!()
- [x] service/whoami.rs: todo!()
- [x] service/init.rs: todo!()
- [x] service/new_solution.rs: todo!()
- [x] service/test.rs: todo!()
- [x] service/submit.rs: todo!()

### interfaces/

- [x] controller/input.rs: Input traits (LoginInput, WhoamiInput, InitInput, NewInput, TestInput, SubmitInput)
- [x] controller.rs: Controller struct + todo!() メソッド

### infrastructure/

- [x] shell/commands.rs: clap CLI 定義
- [x] shell/mod.rs: run() + todo!()
- [x] repository_impl/: todo!() stubs
- [x] online_judge_impl/atcoder.rs: todo!() stubs
- [x] config_impl.rs: todo!()

### バイナリ

- [x] src/main.rs

## 完了条件

- [x] `cargo check` が通る

## 作業ログ

- 2026-04-04: 作業開始
- 2026-04-04: spec-review 完了。差異 2 件を仕様側に反映 (OnlineJudge::submit の problem_code→problem_id、SolutionRepository::exists の lang: Language→&Language)。全コメント・doc コメントを英語に統一
