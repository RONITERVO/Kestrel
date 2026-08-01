use crate::kiwix::{KiwixClient, SNAPSHOT};
use crate::models::{
    Finding, ResearchDraft, ResearchProgress, ResearchReport, ResearchSection, RunResearchRequest,
    Source,
};
use crate::store::{slugify, ResearchStore, StoreError};
use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MODEL_ENDPOINT: &str = "http://127.0.0.1:8080/v1/chat/completions";
const MODEL_ID: &str = "bonsai-27b";
const MODEL_LABEL: &str = "Ternary Bonsai 27B Q2_0";
const ARCHIVE_LABEL: &str = "English Wikipedia · 12 January 2024";
const HARNESS_VERSION: &str = "bonsai-wikipedia-v1";

#[derive(Debug, Error)]
pub enum ResearchError {
    #[error("the research query must contain at least four characters")]
    InvalidQuery,
    #[error("research was stopped safely")]
    Cancelled,
    #[error("Bonsai is not ready: {0}")]
    Model(String),
    #[error("Bonsai returned an invalid research document: {0}")]
    InvalidModelOutput(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("offline Wikipedia error: {0}")]
    Wikipedia(#[from] crate::kiwix::KiwixError),
    #[error("local request failed: {0}")]
    Request(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct ResearchHarness {
    store: ResearchStore,
    kiwix: KiwixClient,
    http: Client,
}

impl ResearchHarness {
    pub fn new(store: ResearchStore) -> Self {
        Self {
            store,
            kiwix: KiwixClient::new(),
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(360))
                .build()
                .expect("local HTTP client"),
        }
    }

    pub async fn run(
        &self,
        app: Option<&AppHandle>,
        request: RunResearchRequest,
        job_id: &str,
        cancel: CancellationToken,
    ) -> Result<ResearchReport, ResearchError> {
        let query = request.query.trim();
        if query.chars().count() < 4 {
            return Err(ResearchError::InvalidQuery);
        }
        let thorough = request.depth != "focused";
        let started = Instant::now();
        emit(
            app,
            job_id,
            started,
            "preparing",
            "Preparing private research",
            "Verifying the local model and Wikipedia archive",
            0,
        )?;
        self.ensure_not_cancelled(&cancel)?;

        let status = crate::services::status().await;
        if status.bonsai != "ready" || status.wikipedia != "ready" {
            return Err(ResearchError::Model(
                "use “Prepare services” so Bonsai and offline Wikipedia are both ready".into(),
            ));
        }

        emit(
            app,
            job_id,
            started,
            "library",
            "Checking earlier work",
            "Searching prior reports for an answer worth expanding",
            1,
        )?;
        let parent = self.store.best_parent(query)?;
        let related = self.store.search(query, 5)?;
        let mut evidence = Vec::<Source>::new();
        if let Some(previous) = &parent {
            evidence.push(Source {
                id: "S1".into(),
                kind: "research".into(),
                title: previous.title.clone(),
                section: Some(format!("Edition {}", previous.edition)),
                snapshot: Some(previous.updated_at.chars().take(10).collect()),
                reference: format!("research:{}", previous.id),
                excerpt: truncate(&previous.answer, 1_200),
            });
            emit(
                app,
                job_id,
                started,
                "library",
                "Earlier research found",
                &format!(
                    "Edition {} of “{}” will be inspected and improved",
                    previous.edition, previous.title
                ),
                1,
            )?;
        } else {
            emit(
                app,
                job_id,
                started,
                "library",
                "No close prior edition",
                "Building a traceable first edition and recording future questions",
                1,
            )?;
        }

        let related_context = if related.is_empty() {
            "No related Kestrel reports were found.".to_string()
        } else {
            related
                .iter()
                .map(|item| {
                    format!(
                        "- research:{} — {} (edition {}): {}",
                        item.id, item.title, item.edition, item.dek
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let system = system_prompt(thorough);
        let user = format!(
            "Research question: {query}\n\nExisting-library matches:\n{related_context}\n\nUse search_archive and read_source. Inspect at least {} distinct relevant Wikipedia articles. If prior research exists, inspect it and make a concrete improvement. Distinguish sourced statements from inference, explain specialist terms, preserve uncertainty, and remember the archive cutoff. Do not claim the result is final or the best possible.",
            if thorough { 4 } else { 2 }
        );
        let mut messages = vec![
            json!({"role":"system","content":system}),
            json!({"role":"user","content":user}),
        ];
        let tools = tool_schema();
        let required_wikipedia = if thorough { 4 } else { 2 };
        let max_turns = if thorough { 14 } else { 9 };
        let mut forced_retry = false;

        emit(
            app,
            job_id,
            started,
            "searching",
            "Searching the archive",
            "Bonsai is choosing useful article paths—not merely matching titles",
            2,
        )?;
        for turn in 0..max_turns {
            self.ensure_not_cancelled(&cancel)?;
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
                result = self.complete(&messages, Some(&tools), None, 1800, if thorough { 2048 } else { 1024 }) => result?,
            };
            let assistant = response_message(&response)?;
            let calls = assistant
                .get("tool_calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            messages.push(assistant.clone());
            if calls.is_empty() {
                let wikipedia_count = evidence
                    .iter()
                    .filter(|source| source.kind == "wikipedia")
                    .count();
                if wikipedia_count >= required_wikipedia || forced_retry || turn + 1 == max_turns {
                    break;
                }
                forced_retry = true;
                messages.push(json!({"role":"user","content":format!("You have inspected {wikipedia_count} Wikipedia articles. Inspect at least {required_wikipedia} distinct relevant articles before synthesis; broaden or refine the search if needed.")}));
                continue;
            }
            forced_retry = false;
            for call in calls {
                self.ensure_not_cancelled(&cancel)?;
                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool-call");
                let function = call.get("function").cloned().unwrap_or(Value::Null);
                let name = function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments: Value = serde_json::from_str(
                    function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}"),
                )
                .unwrap_or_else(|_| json!({}));
                let result = match name {
                    "search_archive" => {
                        let search_query = arguments
                            .get("query")
                            .and_then(Value::as_str)
                            .unwrap_or(query)
                            .trim();
                        let limit = arguments
                            .get("limit")
                            .and_then(Value::as_u64)
                            .unwrap_or(8)
                            .clamp(1, 12) as usize;
                        let result = self.search_all(search_query, limit).await?;
                        emit(
                            app,
                            job_id,
                            started,
                            "searching",
                            "Searching the archive",
                            &format!(
                                "Looked for “{search_query}” across Wikipedia and prior research"
                            ),
                            2,
                        )?;
                        serde_json::to_string(&result).unwrap_or_default()
                    }
                    "read_source" => {
                        let reference = arguments
                            .get("source_ref")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let section = arguments
                            .get("section")
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty());
                        let max_chars = arguments
                            .get("max_chars")
                            .and_then(Value::as_u64)
                            .unwrap_or(if thorough { 14_000 } else { 9_000 })
                            as usize;
                        let (text, title) = self
                            .read_source(reference, section, max_chars, &mut evidence)
                            .await?;
                        emit(
                            app,
                            job_id,
                            started,
                            "reading",
                            "Reading source evidence",
                            &format!(
                                "Inspected “{title}”{}",
                                section
                                    .map(|value| format!(" · {value}"))
                                    .unwrap_or_default()
                            ),
                            3,
                        )?;
                        text
                    }
                    _ => format!("Unknown research tool: {name}"),
                };
                messages.push(json!({"role":"tool","tool_call_id":id,"content":result}));
            }
        }

        self.ensure_not_cancelled(&cancel)?;
        if !evidence.iter().any(|source| source.kind == "wikipedia") {
            return Err(ResearchError::InvalidModelOutput(
                "no Wikipedia article was inspected; try a more specific question".into(),
            ));
        }
        emit(
            app,
            job_id,
            started,
            "synthesizing",
            "Building the explanation",
            &format!(
                "Comparing {} inspected sources, resolving overlaps, and preserving uncertainty",
                evidence.len()
            ),
            4,
        )?;
        messages.push(json!({
            "role":"user",
            "content":format!(
                "Now publish the research document as strict JSON. You inspected these valid citation IDs: {}. Every finding, section, and timeline entry must cite only these IDs. The improvement field must name a concrete improvement over {}. Use plain language without flattening uncertainty. Keep the answer concise, but make the sections genuinely explanatory.",
                evidence.iter().map(|source| format!("{} ({})", source.id, source.title)).collect::<Vec<_>>().join(", "),
                parent.as_ref().map(|item| format!("edition {} of {}", item.edition, item.title)).unwrap_or_else(|| "a blank first-edition baseline".into())
            )
        }));
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
            result = self.complete(&messages, None, Some(report_schema()), if thorough { 11_000 } else { 7_000 }, if thorough { 2048 } else { 1024 }) => result?,
        };
        let mut content = response_message(&response)?
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let draft: ResearchDraft = match serde_json::from_str(&content) {
            Ok(draft) => draft,
            Err(first_error) => {
                messages.push(json!({"role":"assistant","content":content}));
                messages.push(json!({"role":"user","content":"The previous JSON was incomplete. Retry once with compact prose: at most 4 findings, 4 sections, 2 paragraphs per section, 8 timeline items, and 8 terms. Return the complete strict JSON object only. Preserve the same evidence IDs and concrete improvement."}));
                let retry = tokio::select! {
                    _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
                    result = self.complete(&messages, None, Some(report_schema()), 9_000, 0) => result?,
                };
                content = response_message(&retry)?
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                serde_json::from_str(&content).map_err(|retry_error| {
                    ResearchError::InvalidModelOutput(format!(
                        "initial {first_error}; retry {retry_error}; retry began: {}",
                        truncate(&content, 220)
                    ))
                })?
            }
        };
        let now = Utc::now().to_rfc3339();
        let edition = parent
            .as_ref()
            .map(|report| report.edition + 1)
            .unwrap_or(1);
        let mut report = normalize_report(
            draft,
            query,
            now,
            edition,
            parent.as_ref().map(|item| item.id.clone()),
            evidence,
        );

        emit(
            app,
            job_id,
            started,
            "publishing",
            "Publishing the local edition",
            "Writing immutable JSON, evidence, provenance, searchable index, and standalone HTML",
            5,
        )?;
        self.store.save(&mut report)?;
        emit(
            app,
            job_id,
            started,
            "complete",
            "Research ready",
            &format!(
                "Edition {} saved with {} inspected sources",
                report.edition,
                report.sources.len()
            ),
            6,
        )?;
        Ok(report)
    }

    async fn search_all(&self, query: &str, limit: usize) -> Result<Value, ResearchError> {
        let research = self.store.search(query, 4)?.into_iter().map(|item| json!({
            "kind":"research", "sourceRef":format!("research:{}", item.id), "title":item.title, "snippet":item.dek, "edition":item.edition
        })).collect::<Vec<_>>();
        let wikipedia = self.kiwix.search(query, limit).await?.into_iter().map(|item| json!({
            "kind":"wikipedia", "sourceRef":item.reference, "title":item.title, "snippet":item.snippet, "snapshot":SNAPSHOT
        })).collect::<Vec<_>>();
        Ok(
            json!({"query":query,"research":research,"wikipedia":wikipedia,"archiveCutoff":SNAPSHOT}),
        )
    }

    async fn read_source(
        &self,
        reference: &str,
        section: Option<&str>,
        max_chars: usize,
        evidence: &mut Vec<Source>,
    ) -> Result<(String, String), ResearchError> {
        if let Some(id) = reference.strip_prefix("research:") {
            let report = self.store.get(id)?;
            let existing = evidence
                .iter()
                .find(|source| source.reference == reference)
                .map(|source| source.id.clone());
            let evidence_id = existing.unwrap_or_else(|| next_source_id(evidence));
            if !evidence.iter().any(|source| source.reference == reference) {
                evidence.push(Source {
                    id: evidence_id.clone(),
                    kind: "research".into(),
                    title: report.title.clone(),
                    section: Some(format!("Edition {}", report.edition)),
                    snapshot: Some(report.updated_at.chars().take(10).collect()),
                    reference: reference.into(),
                    excerpt: truncate(&report.answer, 1_600),
                });
            }
            let digest = json!({"evidenceId":evidence_id,"title":report.title,"edition":report.edition,"answer":report.answer,"findings":report.findings,"openQuestions":report.open_questions,"improvement":report.improvement});
            return Ok((
                serde_json::to_string_pretty(&digest).unwrap_or_default(),
                report.title,
            ));
        }
        let article = self.kiwix.read(reference, section, max_chars).await?;
        let existing = evidence
            .iter()
            .find(|source| {
                source.reference == article.reference && source.section == article.section
            })
            .map(|source| source.id.clone());
        let evidence_id = existing.unwrap_or_else(|| next_source_id(evidence));
        if !evidence.iter().any(|source| {
            source.reference == article.reference && source.section == article.section
        }) {
            evidence.push(Source {
                id: evidence_id.clone(),
                kind: "wikipedia".into(),
                title: article.title.clone(),
                section: article.section.clone(),
                snapshot: Some(SNAPSHOT.into()),
                reference: article.reference.clone(),
                excerpt: truncate(&article.text, 900),
            });
        }
        Ok((format!("Evidence ID: {evidence_id}\nTitle: {}\nSnapshot: {SNAPSHOT}\nSection: {}\nSource ref: {}\n\n{}", article.title, article.section.as_deref().unwrap_or("Full article"), article.reference, article.text), article.title))
    }

    async fn complete(
        &self,
        messages: &[Value],
        tools: Option<&Value>,
        response_format: Option<Value>,
        max_tokens: u32,
        thinking_budget: i32,
    ) -> Result<Value, ResearchError> {
        let mut request = json!({
            "model": MODEL_ID, "messages": messages, "stream": false, "temperature": 0.2, "top_p": 0.9,
            "max_tokens": max_tokens, "thinking_budget_tokens": thinking_budget
        });
        if let Some(tools) = tools {
            request["tools"] = tools.clone();
            request["tool_choice"] = json!("auto");
            request["parallel_tool_calls"] = json!(false);
        }
        if let Some(format) = response_format {
            request["response_format"] = format;
        }
        let response = self.http.post(MODEL_ENDPOINT).json(&request).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(ResearchError::Model(format!(
                "HTTP {status}: {}",
                truncate(&body, 500)
            )));
        }
        serde_json::from_str(&body)
            .map_err(|error| ResearchError::Model(format!("invalid API JSON: {error}")))
    }

    fn ensure_not_cancelled(&self, cancel: &CancellationToken) -> Result<(), ResearchError> {
        if cancel.is_cancelled() {
            Err(ResearchError::Cancelled)
        } else {
            Ok(())
        }
    }
}

fn emit(
    app: Option<&AppHandle>,
    job_id: &str,
    started: Instant,
    stage: &str,
    title: &str,
    detail: &str,
    current: u32,
) -> Result<(), ResearchError> {
    if let Some(app) = app {
        app.emit(
            "research-progress",
            ResearchProgress {
                job_id: job_id.into(),
                stage: stage.into(),
                title: title.into(),
                detail: detail.into(),
                current,
                total: 6,
                elapsed_seconds: started.elapsed().as_secs(),
            },
        )
        .map_err(|error| ResearchError::Model(format!("could not update progress: {error}")))?;
    }
    Ok(())
}

fn response_message(response: &Value) -> Result<Value, ResearchError> {
    response
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| {
            ResearchError::Model(format!(
                "missing choices[0].message: {}",
                truncate(&response.to_string(), 500)
            ))
        })
}

fn system_prompt(thorough: bool) -> String {
    format!(
        "You are Kestrel's offline research model, running as {MODEL_LABEL}. You have two tools only: search_archive finds both immutable Kestrel reports and the January 2024 English Wikipedia archive; read_source opens one result. Work entirely from these tools. Never imply internet access or knowledge newer than the archive. Read before citing. Wikipedia is tertiary: attribute it and preserve disputes, uncertainty, dates, and the snapshot cutoff. Search with multiple phrasings when useful. Related prior research is context to improve, never authority. Always make at least one concrete improvement and identify open questions; never call a report final or best possible. Use simple explanations first and define specialist language. Research depth: {}. Harness: {HARNESS_VERSION}.",
        if thorough { "thorough" } else { "focused" }
    )
}

fn tool_schema() -> Value {
    json!([
      {"type":"function","function":{"name":"search_archive","description":"Search local Wikipedia and existing Kestrel research. Use several focused searches rather than one broad query.","parameters":{"type":"object","properties":{"query":{"type":"string","description":"Short article title or focused keywords"},"limit":{"type":"integer","minimum":1,"maximum":12}},"required":["query"]}}},
      {"type":"function","function":{"name":"read_source","description":"Open a sourceRef returned by search_archive. Reading is required before citation.","parameters":{"type":"object","properties":{"source_ref":{"type":"string","description":"Exact sourceRef from search_archive"},"section":{"type":"string","description":"Optional heading for a focused excerpt"},"max_chars":{"type":"integer","minimum":2000,"maximum":40000}},"required":["source_ref"]}}}
    ])
}

fn report_schema() -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_research","strict":true,"schema":{
      "type":"object","additionalProperties":false,
      "required":["title","dek","answer","improvement","findings","sections","timeline","terms","openQuestions"],
      "properties":{
        "title":{"type":"string","maxLength":120},"dek":{"type":"string","maxLength":260},"answer":{"type":"string","maxLength":1000},"improvement":{"type":"string","maxLength":600},
        "findings":{"type":"array","minItems":3,"maxItems":6,"items":{"type":"object","additionalProperties":false,"required":["title","explanation","citations"],"properties":{"title":{"type":"string","maxLength":120},"explanation":{"type":"string","maxLength":650},"citations":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":12}}}}},
        "sections":{"type":"array","minItems":2,"maxItems":7,"items":{"type":"object","additionalProperties":false,"required":["id","heading","summary","body","citations"],"properties":{"id":{"type":"string","maxLength":100},"heading":{"type":"string","maxLength":140},"summary":{"type":"string","maxLength":500},"body":{"type":"array","minItems":1,"maxItems":5,"items":{"type":"string","maxLength":1200}},"citations":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":12}}}}},
        "timeline":{"type":"array","maxItems":12,"items":{"type":"object","additionalProperties":false,"required":["label","date","description","citations"],"properties":{"label":{"type":"string","maxLength":100},"date":{"type":"string","maxLength":80},"description":{"type":"string","maxLength":500},"citations":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":12}}}}},
        "terms":{"type":"array","maxItems":12,"items":{"type":"object","additionalProperties":false,"required":["term","meaning"],"properties":{"term":{"type":"string","maxLength":100},"meaning":{"type":"string","maxLength":500}}}},
        "openQuestions":{"type":"array","minItems":2,"maxItems":8,"items":{"type":"string","maxLength":500}}
      }
    }}})
}

fn normalize_report(
    draft: ResearchDraft,
    query: &str,
    now: String,
    edition: u32,
    parent_id: Option<String>,
    sources: Vec<Source>,
) -> ResearchReport {
    let valid = sources
        .iter()
        .map(|source| source.id.clone())
        .collect::<HashSet<_>>();
    let fallback = sources.first().map(|source| source.id.clone());
    let normalize_citations = |citations: Vec<String>| {
        let mut result = citations
            .into_iter()
            .filter(|id| valid.contains(id))
            .collect::<Vec<_>>();
        result.sort();
        result.dedup();
        if result.is_empty() {
            if let Some(id) = &fallback {
                result.push(id.clone());
            }
        }
        result
    };
    let findings = draft
        .findings
        .into_iter()
        .take(6)
        .map(|finding| Finding {
            title: finding.title,
            explanation: finding.explanation,
            citations: normalize_citations(finding.citations),
        })
        .collect::<Vec<_>>();
    let sections = draft
        .sections
        .into_iter()
        .take(7)
        .enumerate()
        .map(|(index, section)| ResearchSection {
            id: {
                let value = slugify(&section.heading);
                if value == "research" {
                    format!("section-{}", index + 1)
                } else {
                    value
                }
            },
            heading: section.heading,
            summary: section.summary,
            body: section.body.into_iter().take(5).collect(),
            citations: normalize_citations(section.citations),
        })
        .collect::<Vec<_>>();
    let timeline = draft
        .timeline
        .into_iter()
        .take(12)
        .map(|item| crate::models::TimelineItem {
            citations: normalize_citations(item.citations),
            ..item
        })
        .collect::<Vec<_>>();
    let improvement = if draft.improvement.trim().is_empty()
        || draft.improvement.to_lowercase().contains("best possible")
    {
        if edition > 1 {
            "Expanded the prior edition with newly inspected evidence, clearer explanations, and explicit open questions.".into()
        } else {
            "Established a traceable first edition with inspected evidence, plain-language terms, and questions for a later expansion.".into()
        }
    } else {
        draft.improvement.trim().to_owned()
    };
    let word_count = count_words(&draft.answer, &findings, &sections);
    ResearchReport {
        id: Uuid::new_v4().to_string(),
        title: draft.title.trim().to_owned(),
        dek: draft.dek.trim().to_owned(),
        query: query.into(),
        answer: draft.answer.trim().to_owned(),
        created_at: now.clone(),
        updated_at: now,
        edition,
        parent_id,
        improvement,
        model: MODEL_LABEL.into(),
        archive_snapshot: ARCHIVE_LABEL.into(),
        findings,
        sections,
        timeline,
        terms: draft.terms.into_iter().take(12).collect(),
        open_questions: draft.open_questions.into_iter().take(8).collect(),
        sources,
        html_path: String::new(),
        word_count,
        reading_minutes: word_count.div_ceil(220).max(1),
    }
}

fn count_words(answer: &str, findings: &[Finding], sections: &[ResearchSection]) -> u32 {
    let mut count = answer.split_whitespace().count();
    count += findings
        .iter()
        .map(|item| {
            item.title.split_whitespace().count() + item.explanation.split_whitespace().count()
        })
        .sum::<usize>();
    count += sections
        .iter()
        .map(|item| {
            item.heading.split_whitespace().count()
                + item.summary.split_whitespace().count()
                + item
                    .body
                    .iter()
                    .map(|value| value.split_whitespace().count())
                    .sum::<usize>()
        })
        .sum::<usize>();
    count as u32
}

fn next_source_id(sources: &[Source]) -> String {
    format!("S{}", sources.len() + 1)
}
fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_owned()
    } else {
        format!("{}…", value.chars().take(max_chars).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ResearchSection, Term};

    #[test]
    fn normalization_rejects_unknown_citations_and_never_claims_finality() {
        let draft = ResearchDraft {
            title: "Test".into(),
            dek: "Dek".into(),
            answer: "A short answer".into(),
            improvement: "This is the best possible report".into(),
            findings: vec![Finding {
                title: "One".into(),
                explanation: "Finding".into(),
                citations: vec!["FAKE".into()],
            }],
            sections: vec![ResearchSection {
                id: "bad id".into(),
                heading: "Useful section".into(),
                summary: "Summary".into(),
                body: vec!["Body".into()],
                citations: vec!["S1".into(), "FAKE".into()],
            }],
            timeline: vec![],
            terms: vec![Term {
                term: "Term".into(),
                meaning: "Meaning".into(),
            }],
            open_questions: vec!["Next?".into()],
        };
        let sources = vec![Source {
            id: "S1".into(),
            kind: "wikipedia".into(),
            title: "Source".into(),
            section: None,
            snapshot: None,
            reference: "/x".into(),
            excerpt: "x".into(),
        }];
        let report = normalize_report(draft, "query", Utc::now().to_rfc3339(), 1, None, sources);
        assert_eq!(report.findings[0].citations, vec!["S1"]);
        assert_eq!(report.sections[0].citations, vec!["S1"]);
        assert!(!report.improvement.to_lowercase().contains("best possible"));
        assert_eq!(report.sections[0].id, "useful-section");
    }

    #[tokio::test]
    #[ignore = "requires live Bonsai on 127.0.0.1:8080 and Kiwix on 127.0.0.1:8085"]
    async fn live_bonsai_research_creates_a_complete_offline_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let store = ResearchStore::open(directory.path().to_path_buf()).unwrap();
        let now = Utc::now().to_rfc3339();
        let mut prior = ResearchReport {
            id: "prior-antikythera".into(), title: "Antikythera eclipse prediction".into(), dek: "An earlier short note about the Saros dial.".into(),
            query: "How did the Antikythera mechanism predict eclipses, and what remains uncertain?".into(), answer: "The rear dials represented repeating eclipse cycles, but this note does not yet explain the inscriptions or uncertainty.".into(),
            created_at: now.clone(), updated_at: now, edition: 1, parent_id: None, improvement: "Established a short first note.".into(), model: "Bonsai".into(), archive_snapshot: ARCHIVE_LABEL.into(),
            findings: vec![Finding { title: "Saros dial".into(), explanation: "A repeating eclipse cycle was represented mechanically.".into(), citations: vec!["S1".into()] }],
            sections: vec![], timeline: vec![], terms: vec![], open_questions: vec!["How were eclipse characteristics encoded?".into()],
            sources: vec![Source { id: "S1".into(), kind: "wikipedia".into(), title: "Antikythera mechanism".into(), section: None, snapshot: Some(SNAPSHOT.into()), reference: format!("/content/{}/A/Antikythera_mechanism", crate::kiwix::BOOK), excerpt: "Earlier evidence.".into() }],
            html_path: String::new(), word_count: 50, reading_minutes: 1,
        };
        store.save(&mut prior).unwrap();
        let harness = ResearchHarness::new(store.clone());
        let report = harness.run(
            None,
            RunResearchRequest { query: "How did the Antikythera mechanism predict eclipses, and what remains uncertain?".into(), depth: "focused".into() },
            "live-acceptance",
            CancellationToken::new(),
        ).await.unwrap();
        assert!(
            report
                .sources
                .iter()
                .filter(|source| source.kind == "wikipedia")
                .count()
                >= 2
        );
        assert_eq!(report.edition, 2);
        assert_eq!(report.parent_id.as_deref(), Some("prior-antikythera"));
        assert!(report
            .sources
            .iter()
            .any(|source| source.kind == "research"));
        assert!(report.findings.len() >= 3);
        assert!(report.sections.len() >= 2);
        assert!(directory.path().join(&report.html_path).is_file());
        assert!(store
            .search("Antikythera eclipses", 3)
            .unwrap()
            .iter()
            .any(|item| item.id == report.id));
        println!(
            "LIVE_REPORT={} HTML={}",
            report.title,
            directory.path().join(&report.html_path).display()
        );
    }
}
