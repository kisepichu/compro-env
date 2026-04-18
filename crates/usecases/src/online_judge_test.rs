#[cfg(test)]
mod tests {
    use anyhow::Result;
    use chrono::{DateTime, TimeZone, Utc};
    use domain::entity::{Problem, Session, SubmitResult};

    use crate::online_judge::{ContestMeta, OnlineJudge};

    /// Stub implementation of OnlineJudge used only in these tests.
    struct MockOJ {
        contest_meta: ContestMeta,
    }

    impl OnlineJudge for MockOJ {
        fn name(&self) -> &str {
            "mock"
        }

        fn whoami(&self, _session: &Session) -> Result<String> {
            Ok("mock_user".to_string())
        }

        fn get_contest_meta(&self, _contest_id: &str) -> Result<ContestMeta> {
            Ok(ContestMeta {
                start_time: self.contest_meta.start_time,
                problem_id_hints: self.contest_meta.problem_id_hints.clone(),
            })
        }

        fn get_problems_detail(
            &self,
            _contest_id: &str,
            _session: Option<&Session>,
            _problem_id_hints: &[(String, String)],
        ) -> Result<Vec<Problem>> {
            Ok(vec![])
        }

        fn submit(
            &self,
            _contest_id: &str,
            _problem_id: &str,
            _lang_id: &str,
            _source: &str,
            _session: &Session,
        ) -> Result<SubmitResult> {
            todo!()
        }
    }

    fn make_mock_oj_with_start_time(start_time: Option<DateTime<Utc>>) -> MockOJ {
        MockOJ {
            contest_meta: ContestMeta {
                start_time,
                problem_id_hints: vec![],
            },
        }
    }

    /// get_contest_meta returns ContestMeta with the correct start_time when known.
    #[test]
    fn get_contest_meta_returns_start_time_when_known() {
        let expected = Utc.with_ymd_and_hms(2024, 12, 21, 21, 0, 0).unwrap();
        let oj = make_mock_oj_with_start_time(Some(expected));
        let meta = oj.get_contest_meta("abc334").unwrap();
        assert_eq!(meta.start_time, Some(expected));
    }

    /// get_contest_meta returns ContestMeta with start_time = None when unknown.
    #[test]
    fn get_contest_meta_returns_none_start_time_when_unknown() {
        let oj = make_mock_oj_with_start_time(None);
        let meta = oj.get_contest_meta("abc334").unwrap();
        assert_eq!(meta.start_time, None);
    }

    /// get_contest_meta returns the problem_id_hints stored in the mock.
    #[test]
    fn get_contest_meta_returns_problem_id_hints() {
        let hints = vec![
            ("a".to_string(), "abc334_a".to_string()),
            ("b".to_string(), "abc334_b".to_string()),
        ];
        let oj = MockOJ {
            contest_meta: ContestMeta {
                start_time: None,
                problem_id_hints: hints.clone(),
            },
        };
        let meta = oj.get_contest_meta("abc334").unwrap();
        assert_eq!(meta.problem_id_hints, hints);
    }

    /// get_problems_detail accepts the new 3-arg signature (with problem_id_hints).
    #[test]
    fn get_problems_detail_accepts_problem_id_hints_arg() {
        let oj = make_mock_oj_with_start_time(None);
        let hints = vec![("a".to_string(), "abc334_a".to_string())];
        let result = oj.get_problems_detail("abc334", None, &hints).unwrap();
        assert!(result.is_empty());
    }

    /// get_problems_detail works with an empty hints slice.
    #[test]
    fn get_problems_detail_works_with_empty_hints() {
        let oj = make_mock_oj_with_start_time(None);
        let result = oj.get_problems_detail("abc334", None, &[]).unwrap();
        assert!(result.is_empty());
    }
}
