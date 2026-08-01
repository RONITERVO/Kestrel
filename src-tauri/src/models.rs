use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub bonsai: String,
    pub wikipedia: String,
    pub model: String,
    pub archive: String,
    pub offline_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub status: ServiceStatus,
    pub reports: Vec<ReportSummary>,
    pub library_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    pub reference: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub title: String,
    pub explanation: String,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSection {
    pub id: String,
    pub heading: String,
    pub summary: String,
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    pub label: String,
    pub date: String,
    pub description: String,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub term: String,
    pub meaning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub id: String,
    pub title: String,
    pub dek: String,
    pub query: String,
    pub answer: String,
    pub created_at: String,
    pub updated_at: String,
    pub edition: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub improvement: String,
    pub model: String,
    pub archive_snapshot: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub sections: Vec<ResearchSection>,
    #[serde(default)]
    pub timeline: Vec<TimelineItem>,
    #[serde(default)]
    pub terms: Vec<Term>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub sources: Vec<Source>,
    pub html_path: String,
    pub word_count: u32,
    pub reading_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    pub id: String,
    pub title: String,
    pub query: String,
    pub dek: String,
    pub updated_at: String,
    pub edition: u32,
    pub source_count: u32,
    pub reading_minutes: u32,
}

impl From<&ResearchReport> for ReportSummary {
    fn from(report: &ResearchReport) -> Self {
        Self {
            id: report.id.clone(),
            title: report.title.clone(),
            query: report.query.clone(),
            dek: report.dek.clone(),
            updated_at: report.updated_at.clone(),
            edition: report.edition,
            source_count: report.sources.len() as u32,
            reading_minutes: report.reading_minutes,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResearchRequest {
    pub query: String,
    pub depth: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchProgress {
    pub job_id: String,
    pub stage: String,
    pub title: String,
    pub detail: String,
    pub current: u32,
    pub total: u32,
    pub elapsed_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchDraft {
    pub title: String,
    pub dek: String,
    pub answer: String,
    pub improvement: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub sections: Vec<ResearchSection>,
    #[serde(default)]
    pub timeline: Vec<TimelineItem>,
    #[serde(default)]
    pub terms: Vec<Term>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}
