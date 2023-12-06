use crate::environment::ResultCode;
use crate::internal_config::Suite;
use crate::rf::{rebot::Rebot, robot::Attempt};
use crate::sessions::session::{RunOutcome, RunSpec};
use robotmk::results::{AttemptOutcome, AttemptsConfig, SuiteExecutionReport};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use log::{debug, error};
use robotmk::section::WritePiggybackSection;
use std::fs::create_dir_all;

pub fn run_suite(suite: &Suite) -> Result<()> {
    debug!("Running suite {}", &suite.id);
    produce_suite_results(suite)?
        .write(
            &suite.results_file,
            suite.host.clone(),
            &suite.results_directory_locker,
        )
        .context("Reporting suite results failed")?;
    debug!("Suite {} finished", &suite.id);

    Ok(())
}

fn produce_suite_results(suite: &Suite) -> Result<SuiteExecutionReport> {
    let output_directory = suite
        .working_directory
        .join(Utc::now().format("%Y-%m-%dT%H.%M.%S%.f%z").to_string());

    create_dir_all(&output_directory).context(format!(
        "Failed to create directory for suite run: {}",
        output_directory
    ))?;

    let (attempt_outcomes, output_paths) = run_attempts_until_succesful(suite, &output_directory)?;

    Ok(SuiteExecutionReport {
        suite_id: suite.id.clone(),
        attempts: attempt_outcomes,
        rebot: if output_paths.is_empty() {
            None
        } else {
            Some(
                Rebot {
                    environment: &suite.environment,
                    input_paths: &output_paths,
                    path_xml: &output_directory.join("rebot.xml"),
                    path_html: &output_directory.join("rebot.html"),
                }
                .rebot(),
            )
        },
        config: AttemptsConfig {
            interval: suite.execution_interval_seconds,
            timeout: suite.timeout,
            n_attempts_max: suite.robot.n_attempts_max,
        },
    })
}

fn run_attempts_until_succesful(
    suite: &Suite,
    output_directory: &Utf8Path,
) -> Result<(Vec<AttemptOutcome>, Vec<Utf8PathBuf>)> {
    let mut outcomes = vec![];
    let mut output_paths: Vec<Utf8PathBuf> = vec![];

    for attempt in suite.robot.attempts(output_directory) {
        let (outcome, output_path) = run_attempt(suite, attempt, output_directory)?;
        let success = matches!(&outcome, &AttemptOutcome::AllTestsPassed);
        outcomes.push(outcome);
        if let Some(output_path) = output_path {
            output_paths.push(output_path);
        }
        if success {
            break;
        }
    }

    Ok((outcomes, output_paths))
}

fn run_attempt(
    suite: &Suite,
    attempt: Attempt,
    output_directory: &Utf8Path,
) -> Result<(AttemptOutcome, Option<Utf8PathBuf>)> {
    let log_message_start = format!("Suite {}, attempt {}", suite.id, attempt.index);

    let run_outcome = match suite.session.run(&RunSpec {
        id: &format!("robotmk_suite_{}_attempt_{}", suite.id, attempt.index),
        command_spec: &suite.environment.wrap(attempt.command_spec),
        base_path: &output_directory.join(attempt.index.to_string()),
        timeout: suite.timeout,
        cancellation_token: &suite.cancellation_token,
    }) {
        Ok(run_outcome) => run_outcome,
        Err(error_) => {
            error!("{log_message_start}: {error_:?}");
            return Ok((AttemptOutcome::OtherError(format!("{error_:?}")), None));
        }
    };
    let exit_code = match run_outcome {
        RunOutcome::Exited(exit_code) => exit_code,
        RunOutcome::TimedOut => {
            error!("{log_message_start}: timed out");
            return Ok((AttemptOutcome::TimedOut, None));
        }
        RunOutcome::Terminated => bail!("Terminated"),
    };
    let exit_code = match exit_code {
        Some(exit_code) => exit_code,
        None => {
            error!("{log_message_start}: failed to query exit code");
            return Ok((
                AttemptOutcome::OtherError(
                    "Failed to query exit code of Robot Framework call".into(),
                ),
                None,
            ));
        }
    };
    match suite.environment.create_result_code(exit_code) {
        ResultCode::AllTestsPassed => {
            debug!("{log_message_start}: all tests passed");
            Ok((
                AttemptOutcome::AllTestsPassed,
                Some(attempt.output_xml_file),
            ))
        }
        ResultCode::EnvironmentFailed => {
            error!("{log_message_start}: environment failure");
            Ok((AttemptOutcome::EnvironmentFailure, None))
        }
        ResultCode::RobotCommandFailed => {
            if attempt.output_xml_file.exists() {
                debug!("{log_message_start}: some tests failed");
                Ok((AttemptOutcome::TestFailures, Some(attempt.output_xml_file)))
            } else {
                error!("{log_message_start}: Robot Framework failure (no output)");
                Ok((AttemptOutcome::RobotFrameworkFailure, None))
            }
        }
    }
}
