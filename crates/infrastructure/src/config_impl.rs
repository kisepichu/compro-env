use domain::entity::{Language, OJKind};
use usecases::config::Config;

pub struct ConfigImpl;

impl Config for ConfigImpl {
    fn default_language(&self) -> Language {
        todo!()
    }

    fn default_online_judge(&self) -> OJKind {
        todo!()
    }

    fn test_command(&self, _lang: &Language) -> String {
        todo!()
    }

    fn run_command(&self, _lang: &Language) -> String {
        todo!()
    }

    fn submit_file(&self, _lang: &Language) -> String {
        todo!()
    }

    fn submit_preprocess(&self, _lang: &Language) -> String {
        todo!()
    }

    fn lang_id(&self, _lang: &Language, _oj: &OJKind) -> Option<String> {
        todo!()
    }
}
