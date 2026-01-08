use anyhow::{Context, Result};
use google_tasks1::TasksHub;

pub use google_tasks1::api::Task;

use crate::asana;

#[derive(Debug)]
pub struct GTaskResult {
    pub incomplete: Vec<Task>,
    pub complete: Vec<Task>,
}

pub struct GoogleTaskMgr {
    hub: TasksHub<
        google_tasks1::hyper_rustls::HttpsConnector<
            google_tasks1::hyper_util::client::legacy::connect::HttpConnector,
        >,
    >,
    asana_task_list: String,
}

impl GoogleTaskMgr {
    pub async fn new() -> Result<Self> {
        #[cfg(not(feature = "docker"))]
        const SECRET_PATH: &str = "client_secret.json";

        #[cfg(feature = "docker")]
        const SECRET_PATH: &str = "/secret/client_secret.json";

        let secret = google_tasks1::yup_oauth2::read_application_secret(SECRET_PATH)
            .await
            .context("failed to read application secret")?;

        #[cfg(not(feature = "docker"))]
        const TOKEN_PATH: &str = "token_cache.json";

        #[cfg(feature = "docker")]
        const TOKEN_PATH: &str = "/data/token_cache.json";

        let auth = google_tasks1::yup_oauth2::InstalledFlowAuthenticator::builder(
            secret,
            google_tasks1::yup_oauth2::InstalledFlowReturnMethod::HTTPRedirect,
        )
        .persist_tokens_to_disk(TOKEN_PATH)
        .build()
        .await
        .context("failed to build auth")?;

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

    pub async fn new_task_from_asana(&self, task: &asana::Task) -> Result<()> {
        let new_g_task = Task {
            title: Some(task.name.clone()),
            due: Some(asana::asana_due_to_string(task)?),
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

    pub async fn get_tasks(&self) -> Result<GTaskResult> {
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

    pub async fn del_task(&self, id: &str) -> Result<()> {
        self.hub
            .tasks()
            .delete(&self.asana_task_list, id)
            .doit()
            .await?;
        Ok(())
    }
}

pub fn get_asana_task_gid(task: &Task) -> Option<String> {
    if let Some(note) = &task.notes {
        let mut lines = note.lines();

        while let Some(line) = lines.next() {
            if line == "---"
                && let Some(gid) = lines.next()
            {
                return Some(gid.to_string());
            }
        }
    }

    None
}
