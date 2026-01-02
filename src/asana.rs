use anyhow::{Result, bail};
use jiff::{Timestamp, ToSpan, civil};
use reqwest::{
    Response,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};

pub struct AsanaClient {
    client: reqwest::Client,
    headers: HeaderMap,
    project_me: String,
}

impl AsanaClient {
    pub fn new(personal_token: &str, project_me_gid: &str) -> Result<Self> {
        // Create headers for authentication
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", personal_token))?,
        );

        Ok(Self {
            client: reqwest::Client::new(),
            headers,
            project_me: project_me_gid.into(),
        })
    }

    async fn request_get(&self, url: &str) -> Result<Response> {
        let resp = self
            .client
            .get(url)
            .headers(self.headers.clone())
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(resp);
        }

        bail!("Failed to fetch. Status: {}", resp.status())
    }

    async fn request_put<T: Serialize>(&self, url: &str, body: T) -> Result<Response> {
        let resp = self
            .client
            .put(url)
            .headers(self.headers.clone())
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(resp);
        }

        bail!("Failed to put. Status: {}", resp.status())
    }

    pub async fn get_tasks(&self) -> Result<TaskResult> {
        let past_day_ts = jiff::Timestamp::now() - 24.hours();

        let tasks_url = format!(
            "https://app.asana.com/api/1.0/user_task_lists/{}/tasks?opt_fields=name,notes,due_on,due_at,completed_at&completed_since={past_day_ts}&limit=100",
            self.project_me
        );

        let tasks_response = self.request_get(&tasks_url).await?;
        let tasks_response: TasksResponse = tasks_response.json().await?;

        if tasks_response.next_page.is_some() {
            todo!();
        }

        let tasks: Vec<Task> = tasks_response
            .data
            .into_iter()
            .filter(|t| t.due_at.is_some() || t.due_on.is_some())
            .collect();

        let incomplete = tasks
            .iter()
            .filter(|t| t.completed_at.is_none())
            .cloned()
            .collect();
        let complete = tasks
            .into_iter()
            .filter(|t| t.completed_at.is_some())
            .collect();

        Ok(TaskResult {
            incomplete,
            complete,
        })
    }

    pub async fn complete_task(&self, task_gid: &str) -> Result<()> {
        let update_url = format!("https://app.asana.com/api/1.0/tasks/{task_gid}");
        let update_body = UpdateTaskRequest {
            data: UpdateTaskData { completed: true },
        };

        self.request_put(&update_url, update_body).await?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub gid: String,
    // ... other fields
    // assignee: Option<Assignee>,
    pub name: String,
    pub notes: String,
    pub due_on: Option<civil::Date>,
    pub due_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
}

#[derive(Debug, Deserialize)]
struct TasksResponse {
    data: Vec<Task>,
    next_page: Option<String>,
}

pub struct TaskResult {
    pub incomplete: Vec<Task>,
    pub complete: Vec<Task>,
}

#[derive(Debug, Serialize)]
struct UpdateTaskRequest {
    data: UpdateTaskData,
}

#[derive(Debug, Serialize)]
struct UpdateTaskData {
    completed: bool,
}

pub fn asana_due_to_string(atask: &Task) -> Result<String> {
    match (atask.due_on, atask.due_at) {
        (None, None) => bail!("Somehow got to gtask with no due date"),
        (None, Some(due_at)) => Ok(timestamp_to_local_date(due_at)),
        (Some(due_on), None) => Ok(format!("{}T00:00:00Z", due_on)),
        (Some(_due_on), Some(due_at)) => Ok(timestamp_to_local_date(due_at)),
    }
}

fn timestamp_to_local_date(ts: jiff::Timestamp) -> String {
    format!(
        "{}T00:00:00Z",
        ts.to_zoned(jiff::tz::TimeZone::UTC)
            .in_tz("America/Chicago")
            .unwrap()
            .date()
    )
}
