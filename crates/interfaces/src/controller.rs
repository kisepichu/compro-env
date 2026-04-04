use anyhow::Result;
use usecases::service::Service;

pub mod input;
use input::{InitInput, LoginInput, NewInput, SubmitInput, TestInput, WhoamiInput};

pub struct Controller {
    service: Service,
}

impl Controller {
    pub fn new(service: Service) -> Self {
        Self { service }
    }

    pub fn login(&self, args: &dyn LoginInput) -> Result<()> {
        self.service.login(args.oj(), args.cookie())
    }

    pub fn whoami(&self, args: &dyn WhoamiInput) -> Result<String> {
        self.service.whoami(&args.oj())
    }

    pub fn init(&self, args: &dyn InitInput) -> Result<usecases::service::init::InitResult> {
        self.service
            .init(&args.contest_id(), args.oj(), &args.language())
    }

    pub fn new_solution(&self, args: &dyn NewInput) -> Result<()> {
        use domain::entity::Solution;
        let solution = Solution {
            contest_id: args.contest_id(),
            problem_code: args.problem_code(),
            name: args.solution_name(),
            language: args.language(),
        };
        self.service.new_solution(solution)
    }

    pub fn test(&self, args: &dyn TestInput) -> Result<usecases::service::test::TestResult> {
        use domain::entity::Solution;
        let solution = Solution {
            contest_id: args.contest_id(),
            problem_code: args.problem_code(),
            name: args.solution_name(),
            language: args.language(),
        };
        self.service.test(&solution)
    }

    pub fn submit(&self, args: &dyn SubmitInput) -> Result<domain::entity::SubmitResult> {
        use domain::entity::Solution;
        let solution = Solution {
            contest_id: args.contest_id(),
            problem_code: args.problem_code(),
            name: args.solution_name(),
            language: args.language(),
        };
        self.service.submit(&solution)
    }
}
