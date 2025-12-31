use anyhow::{Context, Result, bail};
use google_tasks1::{
    TasksHub,
    api::Task as GTask,
    yup_oauth2::{InstalledFlowAuthenticator, InstalledFlowReturnMethod, read_application_secret},
};
use jiff::Timestamp;
use log::info;

use crate::asana::AsanaClient;

mod asana;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();

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
            if let Some(note) = &gtask.notes
                && let Some(asana_task_gid) = get_asana_task_gid_from_note(note)
                && atask.gid == asana_task_gid
            {
                matching_google_task = Some(gtask.clone());
                break;
            }
        }

        if let Some(google_task) = matching_google_task {
            // check if it needs updating, since asana might report different names or notes
            let mut needs_updating = false;
            if !asana_google_notes_same(atask, &google_task) {
                // dbg!(&atask.notes, &google_task.notes);

                needs_updating = true;
            } else if google_task.title.unwrap() != atask.name {
                needs_updating = true;
            }

            if needs_updating {
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
        if let Some(note) = &gtask.notes
            && let Some(asana_task_gid) = get_asana_task_gid_from_note(note)
        {
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
            if let Some(note) = &gtask.notes
                && let Some(asana_task_gid) = get_asana_task_gid_from_note(note)
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

struct GoogleTaskMgr {
    hub: TasksHub<
        google_tasks1::hyper_rustls::HttpsConnector<
            google_tasks1::hyper_util::client::legacy::connect::HttpConnector,
        >,
    >,
    asana_task_list: String,
}

impl GoogleTaskMgr {
    async fn new() -> Result<Self> {
        let secret = google_tasks1::yup_oauth2::parse_application_secret(include_str!(
            "../client_secret.json"
        ))?;

        let auth =
            InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::Interactive)
                .persist_tokens_to_disk("token_cache.json")
                .build()
                .await?;

        let client = google_tasks1::hyper_util::client::legacy::Client::builder(
            google_tasks1::hyper_util::rt::TokioExecutor::new(),
        )
        .build(
            google_tasks1::hyper_rustls::HttpsConnectorBuilder::new()
                .with_native_roots()
                .unwrap()
                .https_or_http()
                .enable_http1()
                .build(),
        );
        let hub = TasksHub::new(client, auth);

        let lists = hub.tasklists().list().doit().await?.1;

        let asana_task_list = lists
            .items
            .unwrap()
            .iter()
            .find(|a| {
                if let Some(title) = &a.title
                    && title == "Asana"
                {
                    true
                } else {
                    false
                }
            })
            .unwrap()
            .id
            .clone()
            .unwrap();

        Ok(Self {
            hub,
            asana_task_list,
        })
    }

    async fn new_task_from_asana(&self, task: &asana::Task) -> Result<()> {
        let new_g_task = GTask {
            title: Some(task.name.clone()),
            due: Some(match (task.due_on, task.due_at) {
                (None, None) => bail!("Somehow got to gtask with no due date"),
                (None, Some(due_at)) => timestamp_to_local_date(due_at),
                (Some(due_on), None) => format!("{}T00:00:00Z", due_on),
                (Some(_due_on), Some(due_at)) => timestamp_to_local_date(due_at),
            }),
            notes: Some({
                let mut note = task.notes.clone();
                note.push_str("\n---\n");
                note.push_str(&task.gid);
                note
            }),
            ..Default::default()
        };

        self.hub
            .tasks()
            .insert(new_g_task, &self.asana_task_list)
            .doit()
            .await?;
        Ok(())
    }

    async fn get_tasks(&self) -> Result<GTaskResult> {
        let mut result = GTaskResult {
            incomplete: Vec::new(),
            complete: Vec::new(),
        };

        let mut next_page: Option<String> = None;
        loop {
            let tasks_result = self
                .hub
                .tasks()
                .list(&self.asana_task_list)
                .max_results(100)
                .show_completed(true)
                .show_hidden(true);

            let tasks_result = if let Some(page_token) = next_page {
                tasks_result.page_token(&page_token).doit().await?
            } else {
                tasks_result.doit().await?
            };

            next_page = tasks_result.1.next_page_token;

            for task in tasks_result.1.items.unwrap() {
                if task.completed.is_some() {
                    result.complete.push(task);
                } else {
                    result.incomplete.push(task);
                }
            }

            if next_page.is_none() {
                break;
            }
        }

        Ok(result)
    }

    async fn del_task(&self, id: &str) -> Result<()> {
        self.hub
            .tasks()
            .delete(&self.asana_task_list, id)
            .doit()
            .await?;
        Ok(())
    }
}

#[derive(Debug)]
struct GTaskResult {
    incomplete: Vec<GTask>,
    complete: Vec<GTask>,
}

fn timestamp_to_local_date(ts: Timestamp) -> String {
    format!(
        "{}T00:00:00Z",
        ts.to_zoned(jiff::tz::TimeZone::UTC)
            .in_tz("America/Chicago")
            .unwrap()
            .date()
    )
}

fn get_asana_task_gid_from_note(note: &str) -> Option<String> {
    let mut lines = note.lines();

    while let Some(line) = lines.next() {
        if line == "---"
            && let Some(gid) = lines.next()
        {
            return Some(gid.to_string());
        }
    }

    None
}

fn asana_google_notes_same(atask: &asana::Task, gtask: &GTask) -> bool {
    if let Some(gtask_note) = &gtask.notes {
        let lines = gtask_note.lines().take_while(|l| *l != "---");

        for (gtask_lines, atask_lines) in lines.zip(atask.notes.lines()) {
            if gtask_lines != atask_lines {
                return false;
            }
        }
        return true;
    }
    false
}
