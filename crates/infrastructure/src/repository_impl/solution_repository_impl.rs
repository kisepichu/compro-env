use anyhow::{Context, Result};
use domain::entity::{Language, Solution};
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

    fn exists(
        &self,
        contest_id: &str,
        problem_code: &str,
        name: &str,
        _lang: &Language,
    ) -> Result<bool> {
        let solution_dir = self
            .root
            .join("solutions")
            .join(contest_id)
            .join(problem_code)
            .join(name);
        Ok(solution_dir.is_dir())
    }

    fn create(&self, solution: &Solution) -> Result<()> {
        let solution_dir = self
            .root
            .join("solutions")
            .join(&solution.contest_id)
            .join(&solution.problem_code)
            .join(&solution.name);

        // If the solution directory already exists, skip entirely to preserve user edits.
        if solution_dir.exists() {
            return Ok(());
        }

        std::fs::create_dir_all(&solution_dir)
            .with_context(|| format!("failed to create solution dir: {solution_dir:?}"))?;

        // Expand templates; clean up the newly created dir if anything fails
        // to prevent future runs from silently skipping a broken solution dir.
        let result = expand_templates(&solution_dir, solution, self);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&solution_dir);
        }
        result
    }

    fn get_source(&self, _solution: &Solution) -> Result<String> {
        todo!()
    }
}

fn expand_templates(
    solution_dir: &std::path::Path,
    solution: &Solution,
    repo: &SolutionRepositoryImpl,
) -> Result<()> {
    let lang_dir = solution.language.dir_name();
    // Guard against path traversal via language strings like "../.." or "foo/bar".
    let mut components = std::path::Path::new(lang_dir).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => {}
        _ => anyhow::bail!(
            "invalid language template directory name: {lang_dir:?}"
        ),
    }
    let template_dir = repo.root.join("templates").join(lang_dir);

    // Build Tera context
    let mut ctx = tera::Context::new();
    ctx.insert("contest", &serde_json::json!({"id": solution.contest_id}));
    ctx.insert(
        "problem",
        &serde_json::json!({"code": solution.problem_code, "title": solution.problem_title}),
    );
    ctx.insert("solution", &serde_json::json!({"name": solution.name}));

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
    fn exists_returns_false_when_solution_dir_not_present() {
        let dir = setup_temp_root();
        let repo = SolutionRepositoryImpl::new(dir.path().to_path_buf());

        let result = repo
            .exists("abc001", "a", "main", &Language::new("rust"))
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn exists_returns_true_when_solution_dir_present() {
        let dir = setup_temp_root();
        let root = dir.path();

        let solution_dir = root.join("solutions/abc001/a/main");
        fs::create_dir_all(&solution_dir).unwrap();

        let repo = SolutionRepositoryImpl::new(root.to_path_buf());
        let result = repo
            .exists("abc001", "a", "main", &Language::new("rust"))
            .unwrap();
        assert!(result);
    }

    #[test]
    fn create_expands_tera_template() {
        let dir = setup_temp_root();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        repo.create(&solution).unwrap();

        let cargo_toml_path = root.join("solutions/abc001/a/main/Cargo.toml");
        assert!(
            cargo_toml_path.exists(),
            "Cargo.toml should exist after create"
        );

        let contents = fs::read_to_string(&cargo_toml_path).unwrap();
        assert_eq!(contents.trim(), r#"name = "a-main""#);
    }

    #[test]
    fn create_copies_static_files() {
        let dir = setup_temp_root();
        let root = dir.path();
        let repo = SolutionRepositoryImpl::new(root.to_path_buf());

        let solution = make_solution("abc001", "a", "main", Language::new("rust"));
        repo.create(&solution).unwrap();

        let main_rs_path = root.join("solutions/abc001/a/main/src/main.rs");
        assert!(
            main_rs_path.exists(),
            "src/main.rs should be copied after create"
        );

        let contents = fs::read_to_string(&main_rs_path).unwrap();
        assert_eq!(contents, "fn main() {}");
    }

    #[test]
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
        repo.create(&solution).unwrap();

        // User's file must be untouched
        let contents = fs::read_to_string(&main_rs).unwrap();
        assert_eq!(
            contents, "fn main() { /* user edited */ }",
            "create() must not overwrite existing solution files"
        );
    }
}
