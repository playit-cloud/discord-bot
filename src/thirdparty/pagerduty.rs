use serde::{Deserialize, Serialize};


pub struct Pagerduty {
    reqwest: reqwest::Client,
    key: String,
}

impl Pagerduty {
    pub fn new(key: String) -> Self {
        Pagerduty {
            reqwest: reqwest::Client::new(),
            key,
        }
    }

    pub async fn trigger_incident(&self, event: &PDEvent) {
        let _ = self.reqwest.post("https://events.pagerduty.com/v2/enqueue")
            .json(event)
            .send()
            .await;
    }

    pub async fn get_incident_status(&self, incident_id: &str) {
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PDEvent {
    pub payload: PDPayload,
    pub routing_key: String,
    pub event_action: PDEventAction,
    pub dedup_key: Option<String>,
    pub client: Option<String>,
    pub client_url: Option<String>,
}


#[derive(Serialize, Deserialize, Debug)]
pub struct PDPayload {
    pub summary: String,
    pub severity: PDSeverity,
    pub source: String,
    pub component: Option<String>,
    pub group: Option<String>,
    pub class: Option<String>,
}


#[derive(Serialize, Deserialize, Debug)]
pub enum PDEventAction {
    #[serde(rename = "tirgger")]
    Trigger,
    #[serde(rename = "acknowledge")]
    Acknowledge,
    #[serde(rename = "resolve")]
    Resolve,
}


#[derive(Serialize, Deserialize, Debug)]
pub enum PDSeverity {
    #[serde(rename = "critical")]
    Critical,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "info")]
    Info,
}


