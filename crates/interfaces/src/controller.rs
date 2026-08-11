use anyhow::Result;
use domain::analysis::DiscoveryManifest;
use domain::library::LibraryProjectConfig;
use domain::library::{LibraryId, SolutionId};
use site_schema::BuildMode as SchemaBuildMode;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use usecases::check::{CheckSelection, CheckSummary};
use usecases::git_history::GitHistory;
use usecases::library_analyzer::LibraryAnalyzer;
use usecases::repository::site_data_repository::SiteDataRepository;
use usecases::repository::verification_repository::VerificationRepository;
use usecases::service::Service;
use usecases::site_data::ProjectedRelation;
use usecases::site_data_generator::{
    GenerateSiteData, default_output_dir, generate_site_data, write_site_data,
};

pub mod input;
use input::{
    InitInput, LoginInput, LogoutInput, NewInput, SiteDataBuildMode, SiteDataGenerateInput,
    SubmitInput, TestInput, WhoamiInput,
};

pub struct Controller {
    service: Service,
}

impl Controller {
    pub fn new(service: Service) -> Self {
        Self { service }
    }

    pub fn login(&self, args: &dyn LoginInput) -> Result<()> {
        self.service.login(args.oj(), args.credentials())
    }

    pub fn whoami(&self, args: &dyn WhoamiInput) -> Result<String> {
        self.service.whoami(&args.oj())
    }

    pub fn logout(&self, args: &dyn LogoutInput) -> Result<bool> {
        self.service.logout(&args.oj())
    }

    pub fn init(
        &self,
        args: &dyn InitInput,
        on_progress: &dyn Fn(&str),
    ) -> Result<usecases::service::init::InitResult> {
        self.service
            .init(&args.contest_id(), args.oj(), &args.language(), on_progress)
    }

    pub fn new_solution(&self, args: &dyn NewInput) -> Result<()> {
        use domain::entity::Solution;
        let problem_code = args.problem_code();
        let solution = Solution {
            contest_id: args.contest_id(),
            // problem_title is not available at this call site; fall back to the
            // problem code so that template variables like {{problem.title}} are
            // non-empty rather than blank.
            problem_title: problem_code.clone(),
            problem_code,
            name: args.solution_name(),
            language: args.language(),
        };
        self.service.new_solution(solution)
    }

    pub fn test(&self, args: &dyn TestInput) -> Result<i32> {
        self.service.test(
            &args.contest_id(),
            &args.problem_code(),
            &args.solution_name(),
        )
    }

    pub fn submit(&self, args: &dyn SubmitInput) -> Result<usecases::submission::SubmissionStart> {
        self.service.submit(
            &args.contest_id(),
            &args.problem_code(),
            &args.solution_name(),
        )
    }

    /// Prepares the submission source (incl. preprocess) without contacting the OJ.
    /// Returns the exact source `submit` would send. Backs `ce submit --dry-run`.
    pub fn submit_dry_run(&self, args: &dyn SubmitInput) -> Result<String> {
        self.service.submit_dry_run(
            &args.contest_id(),
            &args.problem_code(),
            &args.solution_name(),
        )
    }

    /// Runs `ce check` against the given project config. The shell layer is
    /// responsible for parsing `--language` into `selection` and locating the
    /// repository root, so this pass-through stays free of clap types.
    pub fn check(
        &self,
        config: &LibraryProjectConfig,
        selection: &CheckSelection,
        repository_root: &Path,
    ) -> Result<CheckSummary> {
        self.service.check(config, selection, repository_root)
    }

    /// Runs `ce site-data generate`. Ports are supplied by the shell layer so
    /// tests can inject fakes. The controller assembles the input struct and
    /// dispatches to the use-case orchestrator, then writes atomically.
    #[allow(clippy::too_many_arguments)]
    pub fn site_data_generate(
        &self,
        args: &dyn SiteDataGenerateInput,
        repository_root: &Path,
        config: &LibraryProjectConfig,
        manifest: &DiscoveryManifest,
        analyzer: &dyn LibraryAnalyzer,
        verifications: &dyn VerificationRepository,
        git_history: &dyn GitHistory,
        site_data_repository: &dyn SiteDataRepository,
        oj_by_contest: &BTreeMap<String, String>,
        relations: &BTreeMap<LibraryId, Vec<ProjectedRelation>>,
        manual_dependency_edges: &BTreeMap<LibraryId, BTreeSet<LibraryId>>,
        solution_has_preprocess: &BTreeMap<SolutionId, bool>,
        library_descriptions: &BTreeMap<LibraryId, String>,
    ) -> Result<std::path::PathBuf> {
        let mode = match args.mode() {
            SiteDataBuildMode::Production => SchemaBuildMode::Production,
            SiteDataBuildMode::Preview => SchemaBuildMode::Preview,
        };
        let output = args
            .output()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| default_output_dir(repository_root));
        let spec = GenerateSiteData {
            repository_root,
            config,
            manifest,
            analyzer,
            verifications,
            git_history,
            oj_by_contest,
            relations,
            manual_dependency_edges,
            solution_has_preprocess,
            library_descriptions,
            mode,
        };
        let data = generate_site_data(&spec)?;
        write_site_data(site_data_repository, &output, &data)?;
        Ok(output)
    }
}
