use anyhow::{bail, Context};
use std::process::Command;
use log::error;
use super::plans_by_sessions;

use crate::internal_config::Plan;
use camino::Utf8Path;
use robotmk::session::Session;
use std::collections::HashMap;

pub fn run_icacls_command<'a>(arguments: impl IntoIterator<Item = &'a str>) -> anyhow::Result<()> {
    let mut command = Command::new("icacls.exe");
    command.args(arguments);
    let output = command
        .output()
        .context(format!("Calling icacls.exe failed. Command:\n{command:?}"))?;
    if !output.status.success() {
        bail!(
            "icacls.exe exited non-successfully.\n\nCommand:\n{command:?}\n\nStdout:\n{}\n\nStderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }
    Ok(())
}

pub fn grant_permissions_to_all_plan_users(
    path: &Utf8Path,
    plans: Vec<Plan>,
    permissions: &str,
    additional_icacls_args: &[&str],
) -> (Vec<Plan>, HashMap<String, String>) {
    let mut surviving_plans = vec![];
    let mut failures_by_plan_id = HashMap::new();

    for (session, plans_in_session) in plans_by_sessions(plans) {
        if let Session::User(user_session) = session {
            let icacls_permission_arg = format!("{}:{}", user_session.user_name, permissions);
            let mut icacls_args = vec![path.as_str(), "/grant", &icacls_permission_arg];
            icacls_args.extend(additional_icacls_args);

            match run_icacls_command(icacls_args).context(format!(
                "Adjusting permissions of {path} for user `{}` failed",
                user_session.user_name
            )) {
                Ok(_) => surviving_plans.extend(plans_in_session),
                Err(error) => {
                    error!("{error:?}");
                    for plan in plans_in_session {
                        failures_by_plan_id.insert(plan.id, format!("{error:?}"));
                    }
                }
            }
        } else {
            surviving_plans.extend(plans_in_session);
        }
    }

    (surviving_plans, failures_by_plan_id)
}

pub fn grant_full_access(user: &str, target_path: &Utf8Path) -> anyhow::Result<()> {
    let arguments = [
        target_path.as_ref(),
        "/grant",
        &format!("{user}:(OI)(CI)F"),
        "/T",
    ];
    run_icacls_command(arguments).map_err(|e| {
        let message = format!("Adjusting permissions of {target_path} for user `{user}` failed");
        e.context(message)
    })
}
