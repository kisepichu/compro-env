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
use domain::verification::VerifyFingerprint;
use input::{
    InitInput, InternalVerifyPollInput, InternalVerifyPrepareInput, InternalVerifyStartInput,
    LoginInput, LogoutInput, NewInput, SiteDataBuildMode, SiteDataGenerateInput, SubmitInput,
    TestInput, VerifyInput, WhoamiInput,
};
use usecases::clock::Clock;
use usecases::id_generator::AttemptIdGenerator;
use usecases::service::verify::{
    VerifyInputs, VerifyOutcome, VerifyPorts, VerifySelection, compute_solution_fingerprint,
    poll_current, prepare_solution, run_verify, start_prepared_plan,
};
use usecases::submission_lifecycle::{
    PollEvent, PollingPolicy, RetryAfterHint, Sleeper, StartEvent,
};
use usecases::verification::plan::SubmissionPlan;

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

    /// Runs `ce verify [solution-id]`. Callers provide the discovery manifest
    /// and the pre-normalized [`domain::analysis::AnalysisSnapshot`] so this
    /// layer stays free of infrastructure concerns.
    #[allow(clippy::too_many_arguments)]
    pub fn verify(
        &self,
        args: &dyn VerifyInput,
        repository_root: &Path,
        library_config: &LibraryProjectConfig,
        manifest: &DiscoveryManifest,
        snapshot: &domain::analysis::AnalysisSnapshot,
        clock: &dyn Clock,
        ids: &dyn AttemptIdGenerator,
        sleeper: &dyn Sleeper,
        retry_hint: &dyn RetryAfterHint,
        policy: PollingPolicy,
    ) -> Result<VerifyOutcome> {
        let selection = parse_verify_selection(args)?;
        let ports = self.verify_ports(clock, ids, sleeper, retry_hint, policy)?;
        let inputs = VerifyInputs {
            repository_root,
            library_config,
            manifest,
            snapshot,
            selection,
            submit_preprocess: self.service.config().submit_preprocess(),
        };
        run_verify(inputs, ports)
    }

    /// Hidden `internal verify-prepare`: freeze a submission plan and write the
    /// canonical plan JSON to `--plan-out`. The `Starting` record is persisted
    /// by `verify-start` before OJ contact, not here.
    #[allow(clippy::too_many_arguments)]
    pub fn internal_verify_prepare(
        &self,
        args: &dyn InternalVerifyPrepareInput,
        repository_root: &Path,
        library_config: &LibraryProjectConfig,
        manifest: &DiscoveryManifest,
        snapshot: &domain::analysis::AnalysisSnapshot,
        clock: &dyn Clock,
        ids: &dyn AttemptIdGenerator,
        sleeper: &dyn Sleeper,
        retry_hint: &dyn RetryAfterHint,
        policy: PollingPolicy,
    ) -> Result<std::path::PathBuf> {
        let solution_id = SolutionId::parse(&args.solution())
            .map_err(|e| anyhow::anyhow!("invalid --solution: {e}"))?;
        let ports = self.verify_ports(clock, ids, sleeper, retry_hint, policy)?;
        let inputs = VerifyInputs {
            repository_root,
            library_config,
            manifest,
            snapshot,
            selection: VerifySelection::Single(solution_id.clone()),
            submit_preprocess: self.service.config().submit_preprocess(),
        };
        let plan = prepare_solution(&inputs, &ports, &solution_id)?;
        let out_path = std::path::PathBuf::from(args.plan_out());
        std::fs::write(&out_path, plan.to_canonical_json_bytes())
            .map_err(|e| anyhow::anyhow!("failed to write plan to {}: {e}", out_path.display()))?;
        if let Some(starting_out) = args.starting_out() {
            let starting = plan.as_starting_record();
            let starting_path = std::path::PathBuf::from(&starting_out);
            let bytes = serde_json::to_vec_pretty(&starting)
                .map_err(|e| anyhow::anyhow!("failed to serialize starting record: {e}"))?;
            std::fs::write(&starting_path, bytes).map_err(|e| {
                anyhow::anyhow!(
                    "failed to write starting record to {}: {e}",
                    starting_path.display()
                )
            })?;
        }
        Ok(out_path)
    }

    /// Hidden `internal verify-start`: read a prepared plan JSON and dispatch
    /// it through the starter.
    #[allow(clippy::too_many_arguments)]
    pub fn internal_verify_start(
        &self,
        args: &dyn InternalVerifyStartInput,
        repository_root: &Path,
        library_config: &LibraryProjectConfig,
        manifest: &DiscoveryManifest,
        snapshot: &domain::analysis::AnalysisSnapshot,
        clock: &dyn Clock,
        ids: &dyn AttemptIdGenerator,
        sleeper: &dyn Sleeper,
        retry_hint: &dyn RetryAfterHint,
        policy: PollingPolicy,
    ) -> Result<StartEvent> {
        let bytes = std::fs::read(args.plan_in())?;
        let plan = SubmissionPlan::from_canonical_json_bytes(&bytes)?;
        let ports = self.verify_ports(clock, ids, sleeper, retry_hint, policy)?;
        let inputs = VerifyInputs {
            repository_root,
            library_config,
            manifest,
            snapshot,
            selection: VerifySelection::Single(plan.body.solution_id.clone()),
            submit_preprocess: self.service.config().submit_preprocess(),
        };
        start_prepared_plan(&plan, &inputs, &ports)
    }

    /// Hidden `internal verify-poll`: drive the stored record forward.
    #[allow(clippy::too_many_arguments)]
    pub fn internal_verify_poll(
        &self,
        args: &dyn InternalVerifyPollInput,
        repository_root: &Path,
        library_config: &LibraryProjectConfig,
        manifest: &DiscoveryManifest,
        snapshot: &domain::analysis::AnalysisSnapshot,
        clock: &dyn Clock,
        ids: &dyn AttemptIdGenerator,
        sleeper: &dyn Sleeper,
        retry_hint: &dyn RetryAfterHint,
        policy: PollingPolicy,
    ) -> Result<PollEvent> {
        let solution_id = SolutionId::parse(&args.solution())
            .map_err(|e| anyhow::anyhow!("invalid --solution: {e}"))?;
        let ports = self.verify_ports(clock, ids, sleeper, retry_hint, policy)?;
        let inputs = VerifyInputs {
            repository_root,
            library_config,
            manifest,
            snapshot,
            selection: VerifySelection::Single(solution_id.clone()),
            submit_preprocess: self.service.config().submit_preprocess(),
        };
        poll_current(&solution_id, &inputs, &ports)
    }

    /// Recompute the current [`VerifyFingerprint`] for one published solution
    /// (plan 063 pick-candidate).
    ///
    /// Reuses the same fingerprint pipeline exercised by `verify-prepare` so
    /// the picker can detect input drift on `VerificationState::Completed`
    /// overlay records byte-identically to what the worker would produce for
    /// the same tree.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_solution_fingerprint(
        &self,
        repository_root: &Path,
        library_config: &LibraryProjectConfig,
        manifest: &DiscoveryManifest,
        snapshot: &domain::analysis::AnalysisSnapshot,
        solution_id: &SolutionId,
        clock: &dyn Clock,
        ids: &dyn AttemptIdGenerator,
        sleeper: &dyn Sleeper,
        retry_hint: &dyn RetryAfterHint,
        policy: PollingPolicy,
    ) -> Result<VerifyFingerprint> {
        let ports = self.verify_ports(clock, ids, sleeper, retry_hint, policy)?;
        let inputs = VerifyInputs {
            repository_root,
            library_config,
            manifest,
            snapshot,
            selection: VerifySelection::Single(solution_id.clone()),
            submit_preprocess: self.service.config().submit_preprocess(),
        };
        compute_solution_fingerprint(&inputs, &ports, solution_id)
    }

    /// Assemble a [`VerifyPorts`] view over the service's owned registries.
    /// Fails when the service was constructed without the verification bundle
    /// (i.e. it holds `verification: None`).
    fn verify_ports<'a>(
        &'a self,
        clock: &'a dyn Clock,
        ids: &'a dyn AttemptIdGenerator,
        sleeper: &'a dyn Sleeper,
        retry_hint: &'a dyn RetryAfterHint,
        policy: PollingPolicy,
    ) -> Result<VerifyPorts<'a>> {
        let verification = self.service.verification_services().ok_or_else(|| {
            anyhow::anyhow!("verify pipeline is not wired on this Service instance")
        })?;
        Ok(VerifyPorts {
            verifications: verification.verifications.as_ref(),
            runner: self.service.command_runner(),
            starters: self.service.starter_registry(),
            pollers: &verification.pollers,
            recovery: &verification.recovery,
            sessions: self.service.session_repository(),
            clock,
            ids,
            sleeper,
            retry_hint,
            policy,
        })
    }
}

fn parse_verify_selection(args: &dyn VerifyInput) -> Result<VerifySelection> {
    match args.solution() {
        Some(s) => {
            let id = SolutionId::parse(&s)
                .map_err(|e| anyhow::anyhow!("invalid solution id `{s}`: {e}"))?;
            Ok(VerifySelection::Single(id))
        }
        None => Ok(VerifySelection::All),
    }
}
