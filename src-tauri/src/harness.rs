use crate::kiwix::KiwixClient;
use crate::models::{
    Finding, ResearchDraft, ResearchProgress, ResearchReport, ResearchSection, ResearchSettings,
    RunResearchRequest, Source, Term, TimelineItem,
};
use crate::runtime::{authorized, ModelConnection};
use crate::store::{slugify, ResearchStore, StoreError};
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

struct CompletionOptions {
    response_format: Option<Value>,
    max_tokens: u32,
    thinking_budget: u32,
    parallel_tool_calls: bool,
    tool_choice: Option<Value>,
}
use uuid::Uuid;

#[cfg(test)]
const MODEL_LABEL: &str = "Ternary Bonsai 27B Q2_0";
#[cfg(test)]
const ARCHIVE_LABEL: &str = "English Wikipedia · 12 January 2024";
const HARNESS_VERSION: &str = "bonsai-wikipedia-v2";

#[derive(Debug, Deserialize)]
struct ResearchPlan {
    lanes: Vec<ResearchLane>,
}

#[derive(Debug, Deserialize)]
struct ResearchLane {
    name: String,
    query: String,
    purpose: String,
}

struct ReportContext<'a> {
    query: &'a str,
    now: String,
    edition: u32,
    parent_id: Option<String>,
    sources: Vec<Source>,
    expedition: bool,
    settings: &'a ResearchSettings,
    model_label: &'a str,
}

#[derive(Debug, Error)]
pub enum ResearchError {
    #[error("the research query must contain at least four characters")]
    InvalidQuery,
    #[error("research was stopped safely")]
    Cancelled,
    #[error("the selected local model is not ready: {0}")]
    Model(String),
    #[error("the selected local model returned an invalid research document: {0}")]
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
    http: Client,
}

impl ResearchHarness {
    pub fn new(store: ResearchStore) -> Self {
        Self {
            store,
            http: Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3_600))
                .build()
                .expect("local HTTP client"),
        }
    }

    pub async fn run(
        &self,
        app: Option<&AppHandle>,
        request: RunResearchRequest,
        settings: ResearchSettings,
        connection: &ModelConnection,
        job_id: &str,
        cancel: CancellationToken,
    ) -> Result<ResearchReport, ResearchError> {
        let query = request.query.trim();
        if query.chars().count() < 4 {
            return Err(ResearchError::InvalidQuery);
        }
        let expedition = request.depth == "expedition" && settings.advanced_mode;
        let thorough = request.depth != "focused";
        let required_wikipedia = if expedition {
            settings.source_target as usize
        } else if thorough {
            4usize
        } else {
            2usize
        };
        let max_turns = if expedition {
            settings.tool_turns
        } else if thorough {
            14
        } else {
            9
        };
        let max_output = if expedition {
            settings.max_output_tokens
        } else if thorough {
            11_000
        } else {
            7_000
        };
        let thinking_budget = if expedition {
            settings.thinking_budget
        } else if thorough {
            2_048
        } else {
            1_024
        };
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

        let status = crate::services::status(&settings).await;
        if status.wikipedia != "ready" {
            return Err(ResearchError::Model(
                "offline Wikipedia is not ready; use Setup to install or locate it, then choose Prepare services".into(),
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
        let lane_context = if expedition {
            emit(
                app,
                job_id,
                started,
                "searching",
                "Mapping the research",
                &format!(
                    "The selected local model is dividing the question into {} coordinated evidence lanes",
                    settings.research_lanes
                ),
                2,
            )?;
            let planning_messages = vec![
                json!({"role":"system","content":format!("You are the planning pass for Kestrel's single-context offline researcher. Design complementary Wikipedia research lanes for breadth without duplicating work. Each lane needs a short name, a focused Wikipedia search query, and a one-sentence purpose. Cover mechanisms, chronology, competing interpretations, limitations, and useful context when relevant. Return strict JSON only. The archive snapshot is {}.", settings.wikipedia_snapshot)}),
                json!({"role":"user","content":format!("Question: {query}\nCreate exactly {} distinct research lanes.", settings.research_lanes)}),
            ];
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
                result = self.complete(connection, &planning_messages, None, CompletionOptions { response_format: Some(plan_schema(settings.research_lanes)), max_tokens: max_output, thinking_budget, parallel_tool_calls: false, tool_choice: None }) => result?,
            };
            let content = response_message(&response)?
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut plan: ResearchPlan = serde_json::from_str(&content)
                .unwrap_or_else(|_| fallback_plan(query, settings.research_lanes));
            plan.lanes.truncate(settings.research_lanes as usize);
            emit(
                app,
                job_id,
                started,
                "searching",
                "Scouting the archive",
                &format!(
                    "Searching {} Wikipedia lanes concurrently while keeping one shared GPU context",
                    plan.lanes.len()
                ),
                2,
            )?;
            self.presearch_lanes(&plan.lanes, settings.results_per_lane, &settings, &cancel)
                .await?
        } else {
            "No preplanned lanes; use the tools adaptively.".into()
        };
        let system = system_prompt(thorough, expedition, &settings, &connection.model_label);
        let user = format!(
            "Research question: {query}\n\nExisting-library matches:\n{related_context}\n\nShared lane memory (candidate references, not evidence until opened):\n{lane_context}\n\nUse search_archive and read_source. Inspect at least {required_wikipedia} distinct relevant Wikipedia articles. If prior research exists, inspect it and make a concrete improvement. Distinguish sourced statements from inference, explain specialist terms, preserve uncertainty, and remember the archive cutoff. Do not claim the result is final or the best possible."
        );
        let mut messages = vec![
            json!({"role":"system","content":system}),
            json!({"role":"user","content":user}),
        ];
        let tools = tool_schema(expedition, settings.max_source_chars);

        emit(
            app,
            job_id,
            started,
            "searching",
            "Searching the archive",
            "The selected local model is choosing useful article paths—not merely matching titles",
            2,
        )?;
        let mut required_tool = "search_archive";
        for turn in 0..max_turns {
            self.ensure_not_cancelled(&cancel)?;
            let wikipedia_before = evidence
                .iter()
                .filter(|source| source.kind == "wikipedia")
                .count();
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
                result = self.complete(connection, &messages, Some(&tools), CompletionOptions {
                    response_format: None,
                    max_tokens: if expedition { max_output } else { 1_800 },
                    thinking_budget,
                    parallel_tool_calls: expedition,
                    tool_choice: Some(named_tool_choice(required_tool)),
                }) => result?,
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
                if wikipedia_count >= required_wikipedia || turn + 1 == max_turns {
                    break;
                }
                required_tool = "search_archive";
                messages.push(json!({"role":"user","content":format!("The required archive function was not called. You have inspected {wikipedia_count} Wikipedia articles. Search the local archive now, then inspect at least {required_wikipedia} distinct relevant articles before synthesis.")}));
                continue;
            }
            let mut searched = false;
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
                        searched = true;
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
                        let result = self.search_all(search_query, limit, &settings).await.unwrap_or_else(|error| {
                            json!({"error": error.to_string(), "recovery": "Try a shorter or differently phrased query."})
                        });
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
                        let requested_chars = arguments
                            .get("max_chars")
                            .and_then(Value::as_u64)
                            .unwrap_or(if expedition {
                                settings.max_source_chars as u64
                            } else if thorough {
                                14_000
                            } else {
                                9_000
                            });
                        let max_chars = if expedition {
                            requested_chars.min(settings.max_source_chars as u64)
                        } else {
                            requested_chars.min(40_000)
                        } as usize;
                        let (text, title) = self
                            .read_source_for_tool(
                                reference,
                                section,
                                max_chars,
                                &settings,
                                &mut evidence,
                            )
                            .await;
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
            let wikipedia_after = evidence
                .iter()
                .filter(|source| source.kind == "wikipedia")
                .count();
            required_tool = if searched && wikipedia_after == wikipedia_before {
                "read_source"
            } else {
                "search_archive"
            };
            if wikipedia_after == wikipedia_before && !searched {
                messages.push(json!({"role":"user","content":"That tool call did not add a new inspected Wikipedia article. Search with a different focused query and choose a new sourceRef."}));
            }
        }

        self.ensure_not_cancelled(&cancel)?;
        let wikipedia_count = evidence
            .iter()
            .filter(|source| source.kind == "wikipedia")
            .count();
        if wikipedia_count == 0 {
            return Err(ResearchError::InvalidModelOutput(
                "the selected model did not complete the required local archive search/read cycle; try another tool-capable local model or review its chat template".into(),
            ));
        }
        if wikipedia_count < required_wikipedia {
            return Err(ResearchError::InvalidModelOutput(format!(
                "the source target was not met: inspected {wikipedia_count} of {required_wikipedia} requested Wikipedia articles; add tool turns or lower the source target"
            )));
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
                "Now publish the research document as strict JSON. You inspected these valid citation IDs: {}. Every finding, section, and timeline entry must cite only these IDs. The improvement field must name a concrete improvement over {}. Use plain language without flattening uncertainty. Keep the short answer concise, but make the sections genuinely explanatory. {}",
                evidence.iter().map(|source| format!("{} ({})", source.id, source.title)).collect::<Vec<_>>().join(", "),
                parent.as_ref().map(|item| format!("edition {} of {}", item.edition, item.title)).unwrap_or_else(|| "a blank first-edition baseline".into()),
                if expedition { "This is a solo expedition: integrate the distinct lanes, compare conflicts, state coverage gaps, and use the larger output budget for depth rather than repetition." } else { "" }
            )
        }));
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
            result = self.complete(connection, &messages, None, CompletionOptions { response_format: Some(report_schema(expedition)), max_tokens: max_output, thinking_budget, parallel_tool_calls: false, tool_choice: None }) => result?,
        };
        let mut content = response_message(&response)?
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let draft: ResearchDraft = match parse_research_draft(&content) {
            Ok(draft) => draft,
            Err(first_error) => {
                messages.push(json!({"role":"assistant","content":content}));
                messages.push(json!({"role":"user","content":if expedition { "The previous JSON was incomplete. Retry once with concise but complete prose that fits the schema. Return the strict JSON object only; preserve the evidence IDs, lane coverage, uncertainty, and concrete improvement." } else { "The previous JSON was incomplete. Retry once with compact prose: at most 4 findings, 4 sections, 2 paragraphs per section, 8 timeline items, and 8 terms. Return the complete strict JSON object only. Preserve the same evidence IDs and concrete improvement." }}));
                let retry = tokio::select! {
                    _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
                    result = self.complete(connection, &messages, None, CompletionOptions { response_format: Some(report_schema(expedition)), max_tokens: if expedition { max_output } else { 9_000 }, thinking_budget: 0, parallel_tool_calls: false, tool_choice: None }) => result?,
                };
                content = response_message(&retry)?
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                parse_research_draft(&content).map_err(|retry_error| {
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
            ReportContext {
                query,
                now,
                edition,
                parent_id: parent.as_ref().map(|item| item.id.clone()),
                sources: evidence,
                expedition,
                settings: &settings,
                model_label: &connection.model_label,
            },
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

    async fn presearch_lanes(
        &self,
        lanes: &[ResearchLane],
        results_per_lane: u32,
        settings: &ResearchSettings,
        cancel: &CancellationToken,
    ) -> Result<String, ResearchError> {
        let mut tasks = tokio::task::JoinSet::new();
        for lane in lanes {
            let harness = self.clone();
            let name = lane.name.clone();
            let purpose = lane.purpose.clone();
            let query = lane.query.clone();
            let settings = settings.clone();
            tasks.spawn(async move {
                let results = harness
                    .search_all(&query, results_per_lane as usize, &settings)
                    .await?;
                Ok::<_, ResearchError>((name, purpose, query, results))
            });
        }
        let mut memory = Vec::new();
        while let Some(result) = tokio::select! {
            _ = cancel.cancelled() => return Err(ResearchError::Cancelled),
            result = tasks.join_next() => result,
        } {
            let (name, purpose, query, results) = result
                .map_err(|error| ResearchError::Model(format!("archive lane failed: {error}")))??;
            memory.push(json!({
                "lane": name,
                "purpose": purpose,
                "query": query,
                "candidates": compact_candidates(&results),
            }));
        }
        Ok(serde_json::to_string(&memory).unwrap_or_default())
    }

    async fn search_all(
        &self,
        query: &str,
        limit: usize,
        settings: &ResearchSettings,
    ) -> Result<Value, ResearchError> {
        let research = self.store.search(query, 4)?.into_iter().map(|item| json!({
            "kind":"research", "sourceRef":format!("research:{}", item.id), "title":item.title, "snippet":item.dek, "edition":item.edition
        })).collect::<Vec<_>>();
        let wikipedia = KiwixClient::new(settings.wikipedia_book.clone()).search(query, limit).await?.into_iter().map(|item| json!({
            "kind":"wikipedia", "sourceRef":item.reference, "title":item.title, "snippet":item.snippet, "snapshot":settings.wikipedia_snapshot
        })).collect::<Vec<_>>();
        Ok(
            json!({"query":query,"research":research,"wikipedia":wikipedia,"archiveCutoff":settings.wikipedia_snapshot}),
        )
    }

    async fn read_source(
        &self,
        reference: &str,
        section: Option<&str>,
        max_chars: usize,
        settings: &ResearchSettings,
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
        let article = KiwixClient::new(settings.wikipedia_book.clone())
            .read(reference, section, max_chars)
            .await?;
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
                snapshot: Some(settings.wikipedia_snapshot.clone()),
                reference: article.reference.clone(),
                excerpt: truncate(&article.text, 900),
            });
        }
        Ok((format!("Evidence ID: {evidence_id}\nTitle: {}\nSnapshot: {}\nSection: {}\nSource ref: {}\n\n{}", article.title, settings.wikipedia_snapshot, article.section.as_deref().unwrap_or("Full article"), article.reference, article.text), article.title))
    }

    async fn read_source_for_tool(
        &self,
        reference: &str,
        section: Option<&str>,
        max_chars: usize,
        settings: &ResearchSettings,
        evidence: &mut Vec<Source>,
    ) -> (String, String) {
        self.read_source(reference, section, max_chars, settings, evidence)
            .await
            .unwrap_or_else(|error| {
                (
                    format!(
                        "Source could not be read: {error}. Search again and choose another exact sourceRef."
                    ),
                    "Unreadable archive result".into(),
                )
            })
    }

    async fn complete(
        &self,
        connection: &ModelConnection,
        messages: &[Value],
        tools: Option<&Value>,
        options: CompletionOptions,
    ) -> Result<Value, ResearchError> {
        let request = completion_request(&connection.model_id, messages, tools, options);
        let response = authorized(
            self.http
                .post(format!("{}/chat/completions", connection.endpoint)),
            connection,
        )
        .json(&request)
        .send()
        .await?;
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

fn completion_request(
    model_id: &str,
    messages: &[Value],
    tools: Option<&Value>,
    options: CompletionOptions,
) -> Value {
    let mut request = json!({
        "model": model_id, "messages": messages, "stream": false, "temperature": 0.2, "top_p": 0.9,
        "max_tokens": options.max_tokens, "thinking_budget_tokens": options.thinking_budget
    });
    if let Some(tools) = tools {
        request["tools"] = tools.clone();
        request["tool_choice"] = options.tool_choice.unwrap_or_else(|| json!("auto"));
        request["parallel_tool_calls"] = json!(options.parallel_tool_calls);
    }
    if let Some(format) = options.response_format {
        request["response_format"] = format;
    }
    request
}

fn named_tool_choice(name: &str) -> Value {
    json!({"type":"function","function":{"name":name}})
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

fn system_prompt(
    thorough: bool,
    expedition: bool,
    settings: &ResearchSettings,
    model_label: &str,
) -> String {
    format!(
        "You are Kestrel's offline research model, running as {model_label}. You have two tools only: search_archive finds both immutable Kestrel reports and the configured English Wikipedia archive; read_source opens one result. Work entirely from these tools. Never imply internet access or knowledge newer than the archive. Read before citing. Wikipedia is tertiary: attribute it and preserve disputes, uncertainty, dates, and the snapshot cutoff. Search with multiple phrasings when useful. Related prior research is context to improve, never authority. Always make at least one concrete improvement and identify open questions; never call a report final or best possible. Use simple explanations first and define specialist language. Research depth: {}. {} Harness: {HARNESS_VERSION}.",
        if expedition { "solo expedition" } else if thorough { "thorough" } else { "focused" },
        if expedition { format!("Act as one lead researcher coordinating {} complementary lanes inside one shared GPU context. Treat the supplied lane map as candidate memory, inspect before citing, keep a coverage checklist, resolve duplication and disagreement, and aim for at least {} distinct Wikipedia sources.", settings.research_lanes, settings.source_target) } else { String::new() }
    )
}

fn tool_schema(expedition: bool, max_source_chars: u32) -> Value {
    let max_chars = if expedition { max_source_chars } else { 40_000 };
    json!([
      {"type":"function","function":{"name":"search_archive","description":"Search local Wikipedia and existing Kestrel research. Use several focused searches rather than one broad query.","parameters":{"type":"object","properties":{"query":{"type":"string","description":"Short article title or focused keywords"},"limit":{"type":"integer","minimum":1,"maximum":12}},"required":["query"]}}},
      {"type":"function","function":{"name":"read_source","description":"Open a sourceRef returned by search_archive. Reading is required before citation.","parameters":{"type":"object","properties":{"source_ref":{"type":"string","description":"Exact sourceRef from search_archive"},"section":{"type":"string","description":"Optional heading for a focused excerpt"},"max_chars":{"type":"integer","minimum":2000,"maximum":max_chars}},"required":["source_ref"]}}}
    ])
}

fn plan_schema(lanes: u32) -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_research_map","strict":true,"schema":{
        "type":"object","additionalProperties":false,"required":["lanes"],"properties":{
            "lanes":{"type":"array","minItems":lanes,"maxItems":lanes,"items":{
                "type":"object","additionalProperties":false,"required":["name","query","purpose"],"properties":{
                    "name":{"type":"string","maxLength":80},
                    "query":{"type":"string","maxLength":160},
                    "purpose":{"type":"string","maxLength":280}
                }
            }}
        }
    }}})
}

fn compact_candidates(results: &Value) -> Value {
    let mut candidates = Vec::new();
    for kind in ["research", "wikipedia"] {
        if let Some(items) = results.get(kind).and_then(Value::as_array) {
            for item in items {
                candidates.push(json!({
                    "kind": kind,
                    "sourceRef": item.get("sourceRef").and_then(Value::as_str).unwrap_or_default(),
                    "title": item.get("title").and_then(Value::as_str).unwrap_or_default(),
                    "snippet": truncate(item.get("snippet").and_then(Value::as_str).unwrap_or_default(), 240),
                }));
            }
        }
    }
    Value::Array(candidates)
}

fn fallback_plan(query: &str, lane_count: u32) -> ResearchPlan {
    let focuses = [
        (
            "Core account",
            "overview definition",
            "Establish the central account and vocabulary.",
        ),
        (
            "Mechanism",
            "mechanism operation",
            "Explain how the relevant process or system works.",
        ),
        (
            "Chronology",
            "history chronology",
            "Trace dates, development, and turning points.",
        ),
        (
            "Evidence",
            "evidence discovery",
            "Find the observations or records behind the account.",
        ),
        (
            "Debate",
            "controversy interpretation",
            "Surface competing interpretations and disputes.",
        ),
        (
            "Limits",
            "limitations uncertainty",
            "Identify missing evidence, limitations, and uncertainty.",
        ),
        (
            "Context",
            "historical context",
            "Connect the topic to its wider setting.",
        ),
        (
            "Consequences",
            "impact significance",
            "Examine consequences and why the topic matters.",
        ),
    ];
    ResearchPlan {
        lanes: (0..lane_count)
            .map(|index| {
                let focus = focuses[index as usize % focuses.len()];
                ResearchLane {
                    name: if index < focuses.len() as u32 {
                        focus.0.into()
                    } else {
                        format!("{} {}", focus.0, index + 1)
                    },
                    query: format!("{query} {}", focus.1),
                    purpose: focus.2.into(),
                }
            })
            .collect(),
    }
}

fn report_schema(expedition: bool) -> Value {
    let findings = if expedition { 12 } else { 6 };
    let sections = if expedition { 16 } else { 7 };
    let body_items = if expedition { 10 } else { 5 };
    let paragraph_chars = 1_200;
    let timeline = if expedition { 24 } else { 12 };
    let terms = if expedition { 24 } else { 12 };
    let questions = if expedition { 16 } else { 8 };
    json!({"type":"json_schema","json_schema":{"name":"kestrel_research","strict":true,"schema":{
      "type":"object","additionalProperties":false,
      "required":["title","dek","answer","improvement","findings","sections","timeline","terms","openQuestions"],
      "properties":{
        "title":{"type":"string","maxLength":120},"dek":{"type":"string","maxLength":260},"answer":{"type":"string","maxLength":1000},"improvement":{"type":"string","maxLength":600},
        "findings":{"type":"array","minItems":3,"maxItems":findings,"items":{"type":"object","additionalProperties":false,"required":["title","explanation","citations"],"properties":{"title":{"type":"string","maxLength":120},"explanation":{"type":"string","maxLength":650},"citations":{"type":"array","maxItems":12,"items":{"type":"string","maxLength":12}}}}},
        "sections":{"type":"array","minItems":2,"maxItems":sections,"items":{"type":"object","additionalProperties":false,"required":["id","heading","summary","body","citations"],"properties":{"id":{"type":"string","maxLength":100},"heading":{"type":"string","maxLength":140},"summary":{"type":"string","maxLength":500},"body":{"type":"array","minItems":1,"maxItems":body_items,"items":{"type":"string","maxLength":paragraph_chars}},"citations":{"type":"array","maxItems":12,"items":{"type":"string","maxLength":12}}}}},
        "timeline":{"type":"array","maxItems":timeline,"items":{"type":"object","additionalProperties":false,"required":["label","date","description","citations"],"properties":{"label":{"type":"string","maxLength":100},"date":{"type":"string","maxLength":80},"description":{"type":"string","maxLength":500},"citations":{"type":"array","maxItems":12,"items":{"type":"string","maxLength":12}}}}},
        "terms":{"type":"array","maxItems":terms,"items":{"type":"object","additionalProperties":false,"required":["term","meaning"],"properties":{"term":{"type":"string","maxLength":100},"meaning":{"type":"string","maxLength":500}}}},
        "openQuestions":{"type":"array","minItems":2,"maxItems":questions,"items":{"type":"string","maxLength":500}}
      }
    }}})
}

fn parse_research_draft(content: &str) -> Result<ResearchDraft, String> {
    let original = content.trim();
    let without_prefix = original.strip_prefix("```json").unwrap_or(original).trim();
    let trimmed = without_prefix
        .strip_suffix("```")
        .unwrap_or(without_prefix)
        .trim();
    let value: Value = serde_json::from_str(trimmed).map_err(|error| error.to_string())?;
    if let Ok(draft) = serde_json::from_value::<ResearchDraft>(value.clone()) {
        return Ok(draft);
    }

    let title = text_field(&value, &["title"]).unwrap_or_default();
    let answer = text_field(&value, &["answer", "shortAnswer", "short_answer"]).unwrap_or_default();
    if title.is_empty() || answer.is_empty() {
        return Err("missing a usable title or short answer".into());
    }
    let sections = value
        .get("sections")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let heading = text_field(item, &["heading", "title"])?;
                    let content = text_field(item, &["content", "text"]);
                    let body = item
                        .get("body")
                        .and_then(Value::as_array)
                        .map(|items| string_array(items))
                        .filter(|items| !items.is_empty())
                        .or_else(|| content.clone().map(|text| vec![text]))
                        .unwrap_or_default();
                    let summary = text_field(item, &["summary"])
                        .or_else(|| content.as_deref().map(first_sentence))
                        .unwrap_or_else(|| heading.clone());
                    Some(ResearchSection {
                        id: String::new(),
                        heading,
                        summary,
                        body,
                        citations: citation_array(item),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if sections.is_empty() {
        return Err("missing usable research sections".into());
    }
    let findings = value
        .get("findings")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<Finding>(item.clone()).ok())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            sections
                .iter()
                .take(6)
                .map(|section| Finding {
                    title: section.heading.clone(),
                    explanation: section.summary.clone(),
                    citations: section.citations.clone(),
                })
                .collect()
        });
    let timeline = value
        .get("timeline")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let date = text_field(item, &["date"])?;
                    let description = text_field(item, &["description", "event"])?;
                    Some(TimelineItem {
                        label: text_field(item, &["label"])
                            .unwrap_or_else(|| truncate(&description, 90)),
                        date,
                        description,
                        citations: citation_array(item),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let terms = value
        .get("terms")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<Term>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let mut open_questions = Vec::new();
    for key in [
        "openQuestions",
        "open_questions",
        "coverage_gaps",
        "conflicts_and_debates",
    ] {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            open_questions.extend(string_array(items));
        }
    }
    Ok(ResearchDraft {
        title,
        dek: text_field(&value, &["dek", "subtitle"]).unwrap_or_else(|| truncate(&answer, 240)),
        answer,
        improvement: text_field(&value, &["improvement"]).unwrap_or_default(),
        findings,
        sections,
        timeline,
        terms,
        open_questions,
    })
}

fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn string_array(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect()
}

fn citation_array(value: &Value) -> Vec<String> {
    value
        .get("citations")
        .and_then(Value::as_array)
        .map(|items| string_array(items))
        .unwrap_or_default()
}

fn first_sentence(value: &str) -> String {
    let sentence = value
        .split_inclusive(['.', '!', '?'])
        .next()
        .unwrap_or(value)
        .trim();
    truncate(sentence, 500)
}

fn normalize_report(draft: ResearchDraft, context: ReportContext<'_>) -> ResearchReport {
    let ReportContext {
        query,
        now,
        edition,
        parent_id,
        sources,
        expedition,
        settings,
        model_label,
    } = context;
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
        .take(if expedition { 12 } else { 6 })
        .map(|finding| Finding {
            title: finding.title,
            explanation: finding.explanation,
            citations: normalize_citations(finding.citations),
        })
        .collect::<Vec<_>>();
    let sections = draft
        .sections
        .into_iter()
        .take(if expedition { 16 } else { 7 })
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
            body: section
                .body
                .into_iter()
                .take(if expedition { 10 } else { 5 })
                .collect(),
            citations: normalize_citations(section.citations),
        })
        .collect::<Vec<_>>();
    let timeline = draft
        .timeline
        .into_iter()
        .take(if expedition { 24 } else { 12 })
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
        model: model_label.into(),
        archive_snapshot: format!("English Wikipedia · {}", settings.wikipedia_snapshot),
        findings,
        sections,
        timeline,
        terms: draft
            .terms
            .into_iter()
            .take(if expedition { 24 } else { 12 })
            .collect(),
        open_questions: draft
            .open_questions
            .into_iter()
            .take(if expedition { 16 } else { 8 })
            .collect(),
        sources,
        html_path: String::new(),
        word_count,
        reading_minutes: word_count.div_ceil(220).max(1),
        research_profile: if expedition {
            "solo-expedition".into()
        } else {
            "standard".into()
        },
        context_window: if expedition {
            settings.context_window
        } else {
            0
        },
        output_budget: if expedition {
            settings.max_output_tokens
        } else {
            0
        },
        research_lanes: if expedition {
            settings.research_lanes
        } else {
            1
        },
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
        let settings = ResearchSettings::default();
        let report = normalize_report(
            draft,
            ReportContext {
                query: "query",
                now: Utc::now().to_rfc3339(),
                edition: 1,
                parent_id: None,
                sources,
                expedition: false,
                settings: &settings,
                model_label: "Test model",
            },
        );
        assert_eq!(report.findings[0].citations, vec!["S1"]);
        assert_eq!(report.sections[0].citations, vec!["S1"]);
        assert!(!report.improvement.to_lowercase().contains("best possible"));
        assert_eq!(report.sections[0].id, "useful-section");
    }

    #[test]
    fn advanced_request_preserves_validated_high_capacity_values() {
        let request = completion_request(
            "bonsai-27b",
            &[json!({"role":"user","content":"test"})],
            Some(&tool_schema(true, 65_536)),
            CompletionOptions {
                response_format: None,
                max_tokens: 32_768,
                thinking_budget: 8_192,
                parallel_tool_calls: true,
                tool_choice: None,
            },
        );
        assert_eq!(request["max_tokens"], 32_768);
        assert_eq!(request["thinking_budget_tokens"], 8_192);
        assert_eq!(request["parallel_tool_calls"], true);
        assert_eq!(
            request.pointer("/tools/1/function/parameters/properties/max_chars/maximum"),
            Some(&json!(65_536))
        );
    }

    #[test]
    fn evidence_requests_force_the_required_archive_function() {
        let request = completion_request(
            "local-model",
            &[json!({"role":"user","content":"Where are jungles?"})],
            Some(&tool_schema(false, 40_000)),
            CompletionOptions {
                response_format: None,
                max_tokens: 1_800,
                thinking_budget: 1_024,
                parallel_tool_calls: false,
                tool_choice: Some(named_tool_choice("search_archive")),
            },
        );

        assert_eq!(request["tool_choice"]["type"], "function");
        assert_eq!(request["tool_choice"]["function"]["name"], "search_archive");
    }

    #[test]
    fn adapts_bonsai_expedition_shape_without_weakening_citations() {
        let content = json!({
            "title": "Adapted report",
            "short_answer": "A concise answer.",
            "sections": [{"title":"Mechanism","content":"A clear explanation. More detail.","citations":["S1"]}],
            "timeline": [{"date":"1901","event":"The object was recovered.","citations":["S1"]}],
            "coverage_gaps": ["Which workshop built it?"],
            "conflicts_and_debates": ["The calibration date remains disputed."],
            "improvement": "Added explicit uncertainty."
        })
        .to_string();
        let draft = parse_research_draft(&content).unwrap();
        assert_eq!(draft.answer, "A concise answer.");
        assert_eq!(draft.sections[0].heading, "Mechanism");
        assert_eq!(draft.sections[0].citations, vec!["S1"]);
        assert_eq!(draft.timeline[0].description, "The object was recovered.");
        assert_eq!(draft.open_questions.len(), 2);
        assert!(!draft.findings.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live Bonsai on 127.0.0.1:8080 and Kiwix on 127.0.0.1:8085"]
    async fn live_solo_expedition_uses_shared_lanes_and_high_output_budget() {
        let directory = tempfile::tempdir().unwrap();
        let store = ResearchStore::open(directory.path().to_path_buf()).unwrap();
        let harness = ResearchHarness::new(store.clone());
        let settings = ResearchSettings {
            advanced_mode: true,
            context_window: 98_304,
            max_output_tokens: 32_768,
            research_lanes: 3,
            results_per_lane: 4,
            source_target: 4,
            tool_turns: 16,
            thinking_budget: 2_048,
            max_source_chars: 16_000,
            ..ResearchSettings::default()
        };
        let report = harness
            .run(
                None,
                RunResearchRequest {
                    query: "How did the Antikythera mechanism model eclipse cycles, and which parts of its reconstruction remain uncertain?".into(),
                    depth: "expedition".into(),
                },
                settings,
                &live_connection(),
                "live-expedition",
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.research_profile, "solo-expedition");
        assert_eq!(report.context_window, 98_304);
        assert_eq!(report.output_budget, 32_768);
        assert_eq!(report.research_lanes, 3);
        assert!(
            report
                .sources
                .iter()
                .filter(|source| source.kind == "wikipedia")
                .count()
                >= 4
        );
        assert!(directory.path().join(&report.html_path).is_file());
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
            sources: vec![Source { id: "S1".into(), kind: "wikipedia".into(), title: "Antikythera mechanism".into(), section: None, snapshot: Some("2024-01-12".into()), reference: format!("/content/{}/A/Antikythera_mechanism", crate::kiwix::BOOK), excerpt: "Earlier evidence.".into() }],
            html_path: String::new(), word_count: 50, reading_minutes: 1,
            research_profile: "standard".into(), context_window: 0, output_budget: 0, research_lanes: 1,
        };
        store.save(&mut prior).unwrap();
        let harness = ResearchHarness::new(store.clone());
        let report = harness.run(
            None,
            RunResearchRequest { query: "How did the Antikythera mechanism predict eclipses, and what remains uncertain?".into(), depth: "focused".into() },
            ResearchSettings::default(),
            &live_connection(),
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

    #[tokio::test]
    #[ignore = "requires a tool-capable local model on 127.0.0.1:8080, Kiwix on 127.0.0.1:8085, and KESTREL_LIVE_WIKIPEDIA_BOOK"]
    async fn live_simple_question_forces_search_and_inspects_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let store = ResearchStore::open(directory.path().to_path_buf()).unwrap();
        let harness = ResearchHarness::new(store);
        let settings = ResearchSettings {
            wikipedia_book: std::env::var("KESTREL_LIVE_WIKIPEDIA_BOOK")
                .expect("KESTREL_LIVE_WIKIPEDIA_BOOK is required"),
            ..ResearchSettings::default()
        };
        let report = harness
            .run(
                None,
                RunResearchRequest {
                    query: "Where are jungles?".into(),
                    depth: "focused".into(),
                },
                settings,
                &live_connection(),
                "live-simple-research",
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert!(
            report
                .sources
                .iter()
                .filter(|source| source.kind == "wikipedia")
                .count()
                >= 2
        );
        assert!(!report.answer.trim().is_empty());
    }

    fn live_connection() -> ModelConnection {
        ModelConnection {
            endpoint: "http://127.0.0.1:8080/v1".into(),
            api_key: None,
            model_id: "bonsai-27b".into(),
            model_label: MODEL_LABEL.into(),
        }
    }
}
