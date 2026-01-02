use anyhow::{Context, Result};
use log::{debug, info};

use crate::{asana::AsanaClient, google::GoogleTaskMgr};

mod asana;
mod google;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();

    if std::env::var("SLEEP_TO_CONFIG").is_ok() {
        println!(
            "SLEEP_TO_CONFIG env var set, sleeping. Please connect to console and manually run binary to configure OAuth"
        );
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    }

    rustls::crypto::ring::default_provider()
        .install_default()
        .unwrap();

    let asana_token = std::env::var("ASANA_PAT").context("ASANA_PAT env var missing")?;
    let project_me_gid =
        std::env::var("PROJECT_ME_GID").context("PROJECT_ME_GID env var missing")?;

    let asana_mgr = AsanaClient::new(&asana_token, &project_me_gid)?;
    let gtasks_mgr = GoogleTaskMgr::new().await?;

    loop {
        process_tasks(&asana_mgr, &gtasks_mgr).await?;
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

async fn process_tasks(asana_mgr: &AsanaClient, gtasks_mgr: &GoogleTaskMgr) -> Result<()> {
    let asana_tasks = asana_mgr.get_tasks().await?;
    let google_tasks = gtasks_mgr.get_tasks().await?;

    // One way sync of new asana task to google task
    for atask in &asana_tasks.incomplete {
        let mut matching_google_task = None;
        for gtask in google_tasks
            .incomplete
            .iter()
            .chain(google_tasks.complete.iter())
        {
            if let Some(asana_task_gid) = google::get_asana_task_gid(gtask)
                && atask.gid == asana_task_gid
            {
                matching_google_task = Some(gtask.clone());
                break;
            }
        }

        if let Some(google_task) = matching_google_task {
            // check if it needs updating, since asana might report different names or notes
            if !asana_google_same(atask, &google_task) {
                info!(
                    "Asana -> Google task mismatch, updating google task (Asana: \"{}\")",
                    atask.name
                );
                gtasks_mgr
                    .del_task(google_task.id.as_ref().unwrap())
                    .await?;
                gtasks_mgr.new_task_from_asana(atask).await?;
            }
        } else {
            // create task in google
            info!(
                "Asana -> Google new task \"{}\" created, creating in google",
                atask.name
            );
            gtasks_mgr.new_task_from_asana(atask).await?;
        }
    }

    // remove google completed tasks from asana
    for gtask in &google_tasks.complete {
        if let Some(asana_task_gid) = google::get_asana_task_gid(gtask) {
            info!(
                "Google -> Asana task \"{}\" complete, completing in asana",
                gtask.title.as_ref().unwrap()
            );
            asana_mgr.complete_task(&asana_task_gid).await?;
        }

        // remove this google task
        info!(
            "Deleting task {} from google",
            gtask.title.as_ref().unwrap()
        );
        gtasks_mgr.del_task(gtask.id.as_ref().unwrap()).await?;
    }

    // remove asana completed tasks from google
    for atask in &asana_tasks.complete {
        for gtask in &google_tasks.incomplete {
            if let Some(asana_task_gid) = google::get_asana_task_gid(gtask)
                && atask.gid == asana_task_gid
            {
                info!(
                    "Asana -> Google task \"{}\" complete, deleting in google",
                    gtask.title.as_ref().unwrap()
                );
                gtasks_mgr.del_task(gtask.id.as_ref().unwrap()).await?;
            }
        }
    }

    Ok(())
}

fn asana_google_same(atask: &asana::Task, gtask: &google::Task) -> bool {
    // Check title
    match &gtask.title {
        Some(gtask_title) => {
            if gtask_title != &atask.name {
                debug!(
                    "name mismatch. Asana: \"{}\", Gtasks: \"{gtask_title}\"",
                    atask.name
                );
                return false;
            }
        }
        None => {
            debug!("name mismatch. gtask has no name");
            return false;
        }
    }

    // Check Due Time
    match &gtask.due {
        Some(gtask_due) => {
            let gtask_due = gtask_due.replace(".000Z", "Z");
            let asana_due = asana::asana_due_to_string(atask).unwrap();
            if gtask_due != asana_due {
                debug!("due time mismatch. Asana: \"{asana_due}\", Gtasks: \"{gtask_due}\"");
                return false;
            }
        }
        None => {
            debug!("due time mismatch. gtask has no due date");
            return false;
        }
    }

    // Check Notes Body
    match &gtask.notes {
        Some(gtask_notes) => {
            let lines = gtask_notes.lines().take_while(|l| *l != "---");

            for (gtask_lines, atask_lines) in lines.zip(atask.notes.lines()) {
                if gtask_lines != atask_lines {
                    debug!("notes mismatch. Asana: \"{atask_lines}\", Gtasks: \"{gtask_lines}\"");
                    return false;
                }
            }
        }
        None => {
            debug!("notes mismatch. gtask has no notes");
            return false;
        }
    }

    true
}
