use anyhow::{Context, Result};
use domain::entity::{Sample, Solution};
use usecases::repository::solution_repository::SolutionRepository;

pub struct SolutionRepositoryImpl {
    root: std::path::PathBuf,
}

impl SolutionRepositoryImpl {
    pub fn new(root: std::path::PathBuf) -> Self {
        Self { root }
    }
}

impl SolutionRepository for SolutionRepositoryImpl {
    fn list(&self, _contest_id: &str, _problem_code: &str) -> Result<Vec<Solution>> {
        todo!()
    }

    fn exists(&self, contest_id: &str, problem_code: &str, name: &str) -> Result<bool> {
        let solution_dir = self
            .root
            .join("solutions")
            .join(contest_id)
            .join(problem_code)
            .join(name);
        Ok(solution_dir.is_dir())
    }

    fn create(
        &self,
        solution: &Solution,
        samples: &[Sample],
        input_format_raw: &str,
        constraints_raw: &str,
    ) -> Result<()> {
        let solution_dir = self
            .root
            .join("solutions")
            .join(&solution.contest_id)
            .join(&solution.problem_code)
            .join(&solution.name);

        // If the solution directory already exists, skip entirely to preserve user edits.
        if solution_dir.is_dir() {
            return Ok(());
        }
        // A non-directory entry at the same path would leave the repo in a broken state.
        if solution_dir.exists() {
            anyhow::bail!("solution path exists but is not a directory: {solution_dir:?}");
        }

        std::fs::create_dir_all(&solution_dir)
            .with_context(|| format!("failed to create solution dir: {solution_dir:?}"))?;

        // Expand templates; clean up the newly created dir if anything fails
        // to prevent future runs from silently skipping a broken solution dir.
        let result = expand_templates(
            &solution_dir,
            solution,
            samples,
            input_format_raw,
            constraints_raw,
            self,
        );
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&solution_dir);
        }
        result
    }

    fn get_source(&self, solution: &Solution, file_path: &str) -> Result<String> {
        // Reject absolute paths and any `..` components to prevent path traversal.
        let rel = std::path::Path::new(file_path);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            anyhow::bail!("invalid solution_file path: {file_path:?}");
        }
        let path = self
            .solution_dir(&solution.contest_id, &solution.problem_code, &solution.name)
            .join(file_path);
        std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read source file: {path:?}"))
    }

    fn solution_dir(
        &self,
        contest_id: &str,
        problem_code: &str,
        solution_name: &str,
    ) -> std::path::PathBuf {
        self.root
            .join("solutions")
            .join(contest_id)
            .join(problem_code)
            .join(solution_name)
    }
}

fn expand_templates(
    solution_dir: &std::path::Path,
    solution: &Solution,
    samples: &[Sample],
    input_format_raw: &str,
    constraints_raw: &str,
    repo: &SolutionRepositoryImpl,
) -> Result<()> {
    let lang_dir = solution.language.dir_name();
    // Guard against path traversal via language strings like "../.." or "foo/bar".
    let mut components = std::path::Path::new(lang_dir).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => {}
        _ => anyhow::bail!("invalid language template directory name: {lang_dir:?}"),
    }
    let template_dir = repo.root.join("templates").join(lang_dir);

    // Build Tera context
    let mut ctx = tera::Context::new();
    ctx.insert("contest", &serde_json::json!({"id": &solution.contest_id}));
    ctx.insert(
        "problem",
        &serde_json::json!({"code": &solution.problem_code, "title": &solution.problem_title}),
    );
    ctx.insert("solution", &serde_json::json!({"name": &solution.name}));
    ctx.insert(
        "samples",
        &serde_json::json!(
            samples
                .iter()
                .map(|s| serde_json::json!({"input": &s.input, "output": &s.output}))
                .collect::<Vec<_>>()
        ),
    );
    let input_format_spec = usecases::input_format::parse(input_format_raw, constraints_raw);
    let input_format_json =
        serde_json::to_value(&input_format_spec).unwrap_or(serde_json::Value::Null);
    ctx.insert("input_format", &input_format_json);

    // Walk the template directory recursively
    for entry in walkdir::WalkDir::new(&template_dir) {
        let entry = entry.with_context(|| "error walking template dir")?;
        let src_path = entry.path();

        if src_path.is_dir() {
            continue;
        }

        // Relative path from the template root
        let rel_path = src_path
            .strip_prefix(&template_dir)
            .with_context(|| "failed to strip template prefix")?;

        let file_name = src_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if file_name.ends_with(".tera") {
            // Render with Tera and write with .tera stripped
            let content = std::fs::read_to_string(src_path)
                .with_context(|| format!("failed to read template: {src_path:?}"))?;
            let rendered = tera::Tera::one_off(&content, &ctx, false)
                .with_context(|| format!("failed to render template: {src_path:?}"))?;

            // Strip ".tera" from the destination filename
            let dest_rel = rel_path.with_extension(""); // removes last extension (.tera)
            let dest_path = solution_dir.join(&dest_rel);
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest_path, rendered)
                .with_context(|| format!("failed to write rendered file: {dest_path:?}"))?;
        } else {
            // Copy as-is
            let dest_path = solution_dir.join(rel_path);
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(src_path, &dest_path)
                .with_context(|| format!("failed to copy file: {src_path:?}"))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::entity::Language;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    fn make_solution(contest_id: &str, problem_code: &str, name: &str, lang: Language) -> Solution {
        Solution {
            contest_id: contest_id.to_string(),
            problem_code: problem_code.to_string(),
            problem_title: "Test Problem".to_string(),
            name: name.to_string(),
            language: lang,
        }
    }

    /// Set up a temp root with:
    ///   templates/rust/Cargo.toml.tera  → `name = "{{problem.code}}-{{solution.name}}"`
    ///   templates/rust/src/main.rs      → `fn main() {}`
    fn setup_temp_root() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let tmpl_rust = root.join("templates/rust");
        fs::create_dir_all(&tmpl_rust).unwrap();

        fs::write(
            tmpl_rust.join("Cargo.toml.tera"),
            r#"name = "{{problem.code}}-{{solution.name}}""#,
        )
        .unwrap();

        let tmpl_src = tmpl_rust.join("src");
        fs::create_dir_all(&tmpl_src).unwrap();
        fs::write(tmpl_src.join("main.rs"), "fn main() {}").unwrap();

        dir
    }

    #[test]
    #[serial]
    fn exists_returns_false_when_solution_dir_not_present() {
        let dir = setup_temp_root();
        let repo = SolutionRepositoryImpl::new(dir.path().to_path_buf());

        let result = repo.exists("abc001", "a", "main").unwrap();
        assert!(!result);
    }

    #[test]
    #[serial]
    fn exists_returns_true_when_solution_dir_present() {
        let dir = setup_temp_root();
        let root = dir.path();

        let solution_dir = root.join("solutions/abc001/a/main");
        fs::create_dir_all(&solution_dir).unwrap();

        let repo = SolutionRepositoryImpl::new(root.to_path_buf());
        let result = repo.exists("abc001", "a", "main").unwrap();
        assert!(result);
    }

    #[test]
    #[serial]
    fn create_expands_tera_template() {
        let dir = setup_temp_root();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        repo.create(&solution, &[], "", "").unwrap();

        let cargo_toml_path = root.join("solutions/abc001/a/main/Cargo.toml");
        assert!(
            cargo_toml_path.exists(),
            "Cargo.toml should exist after create"
        );

        let contents = fs::read_to_string(&cargo_toml_path).unwrap();
        assert_eq!(contents.trim(), r#"name = "a-main""#);
    }

    #[test]
    #[serial]
    fn create_copies_static_files() {
        let dir = setup_temp_root();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        repo.create(&solution, &[], "", "").unwrap();

        let main_rs_path = root.join("solutions/abc001/a/main/src/main.rs");
        assert!(
            main_rs_path.exists(),
            "src/main.rs should be copied after create"
        );

        let contents = fs::read_to_string(&main_rs_path).unwrap();
        assert_eq!(contents, "fn main() {}");
    }

    #[test]
    #[serial]
    fn get_source_returns_file_contents_at_given_path() {
        let dir = setup_temp_root();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        // Create solution directory with a source file
        let solution_dir = root.join("solutions/abc001/a/main");
        let src_dir = solution_dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("main.rs"),
            "fn main() { println!(\"hello\"); }",
        )
        .unwrap();

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        let content = repo.get_source(&solution, "src/main.rs").unwrap();
        assert_eq!(content, "fn main() { println!(\"hello\"); }");
    }

    /// Set up a temp root with a Tera template that references input_format.ok
    fn setup_temp_root_with_input_format_template() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let tmpl_rust = root.join("templates/rust");
        fs::create_dir_all(&tmpl_rust).unwrap();

        // Template that renders input_format.ok (true/false)
        fs::write(tmpl_rust.join("result.txt.tera"), "{{input_format.ok}}").unwrap();

        dir
    }

    #[test]
    #[serial]
    fn create_injects_input_format_into_tera_context() {
        let dir = setup_temp_root_with_input_format_template();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        // "N M\n" is a valid input format — parse should succeed → ok = true
        repo.create(&solution, &[], "N M\n", "").unwrap();

        let result_path = root.join("solutions/abc001/a/main/result.txt");
        assert!(result_path.exists(), "result.txt should be generated");
        let contents = fs::read_to_string(&result_path).unwrap();
        assert_eq!(
            contents.trim(),
            "true",
            "input_format.ok should be true for valid input, got: {:?}",
            contents
        );
    }

    #[test]
    #[serial]
    fn create_injects_input_format_ok_false_when_empty() {
        let dir = setup_temp_root_with_input_format_template();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "b", "main", Language::new("rust"));
        // Empty string → parse fails → ok = false
        repo.create(&solution, &[], "", "").unwrap();

        let result_path = root.join("solutions/abc001/b/main/result.txt");
        assert!(result_path.exists(), "result.txt should be generated");
        let contents = fs::read_to_string(&result_path).unwrap();
        assert_eq!(
            contents.trim(),
            "false",
            "input_format.ok should be false for empty input, got: {:?}",
            contents
        );
    }

    /// Set up a temp root whose `templates/rust/src/main.rs.tera` is the real template
    /// from the workspace root, so we test the actual template rendering.
    fn setup_temp_root_with_real_main_template() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Real template content embedded at compile time so the path is always relative
        // to the infrastructure crate, then we climb up to the workspace root.
        let template_content = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../templates/rust/src/main.rs.tera"
        ));

        let tmpl_src = root.join("templates/rust/src");
        fs::create_dir_all(&tmpl_src).unwrap();
        fs::write(tmpl_src.join("main.rs.tera"), template_content).unwrap();

        dir
    }

    #[test]
    #[serial]
    fn create_generates_solve_with_args_when_ok_true() {
        let dir = setup_temp_root_with_real_main_template();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        // "N\nA_1 A_2 \ldots A_N\n" → ok=true, n scalar + a array
        repo.create(&solution, &[], "N\nA_1 A_2 \\ldots A_N\n", "")
            .unwrap();

        let main_rs = root.join("solutions/abc001/a/main/src/main.rs");
        assert!(main_rs.exists(), "src/main.rs should be generated");
        let contents = fs::read_to_string(&main_rs).unwrap();

        assert!(
            contents.contains("fn solve("),
            "expected plain 'fn solve(' (no generic), got:\n{contents}"
        );
        assert!(
            !contents.contains("fn solve<R"),
            "expected NO 'fn solve<R' (no BufRead generic) in ok=true path, got:\n{contents}"
        );
        assert!(
            contents.contains("n: usize"),
            "expected 'n: usize' in solve signature, got:\n{contents}"
        );
        assert!(
            contents.contains("Vec<i64>"),
            "expected 'Vec<i64>' in solve signature, got:\n{contents}"
        );
        assert!(
            contents.contains("solve(n, a)"),
            "expected 'solve(n, a)' call in main, got:\n{contents}"
        );
    }

    #[test]
    #[serial]
    fn create_generates_fallback_when_ok_false() {
        let dir = setup_temp_root_with_real_main_template();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "b", "main", Language::new("rust"));
        // Empty input_format_raw → ok=false → fallback template path
        repo.create(&solution, &[], "", "").unwrap();

        let main_rs = root.join("solutions/abc001/b/main/src/main.rs");
        assert!(main_rs.exists(), "src/main.rs should be generated");
        let contents = fs::read_to_string(&main_rs).unwrap();

        assert!(
            contents.contains("fn solve<R"),
            "expected 'fn solve<R' (BufRead generic fallback) for ok=false, got:\n{contents}"
        );
    }

    #[test]
    #[serial]
    fn create_is_noop_when_solution_dir_already_exists() {
        let dir = setup_temp_root();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        // Pre-create the solution dir with a sentinel file the user "edited"
        let solution_dir = root.join("solutions/abc001/a/main");
        fs::create_dir_all(&solution_dir).unwrap();
        let main_rs = solution_dir.join("src/main.rs");
        fs::create_dir_all(main_rs.parent().unwrap()).unwrap();
        fs::write(&main_rs, "fn main() { /* user edited */ }").unwrap();

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        repo.create(&solution, &[], "", "").unwrap();

        // User's file must be untouched
        let contents = fs::read_to_string(&main_rs).unwrap();
        assert_eq!(
            contents, "fn main() { /* user edited */ }",
            "create() must not overwrite existing solution files"
        );
    }

    /// Phase 2: multi-var vdots loop should generate real loop code (Vec::new, for _ in 0.., push)
    #[test]
    #[serial]
    fn create_generates_loop_code_for_multi_var_loop() {
        let dir = setup_temp_root_with_real_main_template();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "c", "main", Language::new("rust"));
        // 2-variable loop: Q followed by t_i k_i for i in 0..Q
        repo.create(&solution, &[], "Q\nt_1 k_1\n\\vdots\nt_Q k_Q\n", "")
            .unwrap();

        let main_rs = root.join("solutions/abc001/c/main/src/main.rs");
        let contents = fs::read_to_string(&main_rs).unwrap();

        // solve signature should have Vec<i64> params
        assert!(contents.contains("fn solve("), "expected plain fn solve(");
        assert!(
            contents.contains("Vec<i64>"),
            "expected Vec<i64> in solve signature: {contents}"
        );
        // main should have loop code
        assert!(
            contents.contains("Vec::new()"),
            "expected Vec::new() for loop vars: {contents}"
        );
        assert!(
            contents.contains("for _ in 0.."),
            "expected for loop: {contents}"
        );
        assert!(contents.contains(".push("), "expected push: {contents}");
    }
}
