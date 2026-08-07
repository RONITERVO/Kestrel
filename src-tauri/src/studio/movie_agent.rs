use super::{
    producer_intent_issues, prompt_quality_issues, MoviePlan, MovieQualityReview, MovieReference,
    MovieSettings, PlannedClip, StudioError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

const MAX_WORKSPACE_FILE_BYTES: usize = 96 * 1024;
const MAX_READ_MANY_BYTES: usize = 256 * 1024;
const MAX_TOOL_FILES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MovieMetadata {
    title: String,
    logline: String,
    audience: String,
    creative_direction: String,
    #[serde(default, deserialize_with = "deserialize_workspace_text_list")]
    continuity_bible: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_workspace_text_list")]
    source_credits: Vec<String>,
}

fn deserialize_workspace_text_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| match value {
            Value::String(text) => Ok(text),
            Value::Object(fields) => Ok(fields
                .into_iter()
                .map(|(key, value)| {
                    let value = match value {
                        Value::String(text) => text,
                        other => other.to_string(),
                    };
                    format!("{key}: {value}")
                })
                .collect::<Vec<_>>()
                .join("; ")),
            other => Err(serde::de::Error::custom(format!(
                "expected a descriptive string or object, got {other}"
            ))),
        })
        .collect()
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceState {
    revision: u64,
    last_checked_revision: Option<u64>,
    clean_check_passes: u32,
    #[serde(default)]
    mutations_since_check: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkspaceToolRequest {
    action: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    files: Vec<WorkspaceFileWrite>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceFileWrite {
    path: String,
    content: String,
}

pub(super) struct WorkspaceToolResult {
    pub(super) message: String,
    pub(super) submitted: Option<MoviePlan>,
}

pub(super) struct MovieAgentWorkspace {
    root: PathBuf,
    prompt: String,
    references: Vec<MovieReference>,
    max_clips: usize,
    state: WorkspaceState,
}

impl MovieAgentWorkspace {
    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn open(
        root: PathBuf,
        prompt: &str,
        _manifest: &str,
        settings: &MovieSettings,
        references: &[MovieReference],
        seed: Option<&MoviePlan>,
        producer_feedback: Option<&str>,
    ) -> Result<Self, StudioError> {
        fs::create_dir_all(root.join("scenes"))?;
        write_text_atomic(&root.join("README.md"), &workspace_readme(settings))?;
        let brief = if let Some(feedback) = producer_feedback {
            format!("# Producer brief\n\n{prompt}\n\n# Current producer feedback\n\n{feedback}\n")
        } else {
            format!("# Producer brief\n\n{prompt}\n")
        };
        write_text_atomic(&root.join("BRIEF.md"), &brief)?;
        write_text_atomic(
            &root.join("REFERENCES.md"),
            &workspace_reference_manifest(references),
        )?;

        let state_path = root.join("state.json");
        let state = fs::read(&state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut workspace = Self {
            root,
            prompt: prompt.to_owned(),
            references: references.to_vec(),
            max_clips: settings.max_clips as usize,
            state,
        };
        if let Some(plan) = seed {
            workspace.replace_with_plan(plan)?;
        }
        workspace.persist_state()?;
        Ok(workspace)
    }

    pub(super) fn tools() -> Value {
        json!([{
            "type": "function",
            "function": {
                "name": "movie_workspace",
                "description": "Work on the durable, project-local movie codebase. It cannot access the OS, shell, network, renderer, or files outside this movie workspace. Read the contract, edit only movie.json and scenes/NNN.json, run check immediately after a complete draft, repair only reported issues, then submit.",
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "action": {"type":"string","enum":["list","read","read_many","write","write_batch","delete","check","submit"]},
                        "path": {"type":"string","description":"Workspace-relative path for read, write, or delete."},
                        "content": {"type":"string","description":"Complete JSON file content for write."},
                        "files": {"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,"properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]},"description":"Up to eight paths for read_many (content ignored), or up to eight complete files for write_batch. Use multiple small batches for a long movie."}
                    },
                    "required": ["action"]
                }
            }
        }])
    }

    pub(super) fn execute(&mut self, request: WorkspaceToolRequest) -> WorkspaceToolResult {
        match self.try_execute(request) {
            Ok(result) => result,
            Err(error) => WorkspaceToolResult {
                message: format!("ERROR: {error}"),
                submitted: None,
            },
        }
    }

    fn try_execute(
        &mut self,
        request: WorkspaceToolRequest,
    ) -> Result<WorkspaceToolResult, StudioError> {
        match request.action.as_str() {
            "list" => Ok(self.result(self.list_files()?, None)),
            "read" => Ok(self.result(self.read_file(&request.path)?, None)),
            "read_many" => {
                if request.files.is_empty() || request.files.len() > MAX_TOOL_FILES {
                    return Err(StudioError::Invalid(
                        "read_many needs 1 to 8 workspace paths".into(),
                    ));
                }
                let mut output = String::new();
                for file in request.files {
                    let section = format!(
                        "\n===== {} =====\n{}\n",
                        file.path,
                        self.read_file(&file.path)?
                    );
                    if output
                        .len()
                        .checked_add(section.len())
                        .is_none_or(|size| size > MAX_READ_MANY_BYTES)
                    {
                        return Err(StudioError::Invalid(
                            "read_many response exceeds the 256 KiB aggregate limit".into(),
                        ));
                    }
                    output.push_str(&section);
                }
                Ok(self.result(output, None))
            }
            "write" => {
                self.write_file(&request.path, &request.content)?;
                Ok(self.result(format!("WROTE {}", request.path), None))
            }
            "write_batch" => {
                let batch_limit = (self.max_clips + 1).min(MAX_TOOL_FILES);
                if request.files.is_empty() || request.files.len() > batch_limit {
                    return Err(StudioError::Invalid(format!(
                        "write_batch needs 1 to {} movie files",
                        batch_limit
                    )));
                }
                // Validate the entire batch before changing durable state.
                for file in &request.files {
                    validate_write(&file.path, &file.content)?;
                }
                self.ensure_mutation_allowed()?;
                self.changed()?;
                let mut paths = Vec::with_capacity(request.files.len());
                for file in request.files {
                    self.write_validated_file(&file.path, &file.content)?;
                    paths.push(file.path);
                }
                Ok(self.result(format!("WROTE {}", paths.join(", ")), None))
            }
            "delete" => {
                let path = writable_path(&request.path)?;
                if path == "movie.json" {
                    return Err(StudioError::Invalid(
                        "movie.json cannot be deleted; replace it instead".into(),
                    ));
                }
                let target = self.root.join(&path);
                self.ensure_mutation_allowed()?;
                self.changed()?;
                if target.is_file() {
                    fs::remove_file(target)?;
                }
                Ok(self.result(format!("DELETED {path}"), None))
            }
            "check" => {
                let (plan, issues) = self.compile_and_check()?;
                self.state.mutations_since_check = 0;
                if issues.is_empty() {
                    self.state.last_checked_revision = Some(self.state.revision);
                    self.state.clean_check_passes = self.state.clean_check_passes.saturating_add(1);
                    self.persist_state()?;
                    let next = if self.state.clean_check_passes == 1 {
                        "Native H3 checks pass. Now perform a CodeRabbit-style review: inspect movie.json and every scene for story causality, continuity, reference placement, shot specificity, production feasibility, and fidelity to BRIEF.md. Patch any weakness, then run check again."
                    } else {
                        "Native checks and the review pass are clean. Call submit without changing files."
                    };
                    Ok(self.result(
                        format!(
                            "CHECK PASS: {} scenes, {:.1} seconds. {next}",
                            plan.clips.len(),
                            total_seconds(&plan)
                        ),
                        None,
                    ))
                } else {
                    self.state.last_checked_revision = Some(self.state.revision);
                    self.state.clean_check_passes = 0;
                    self.persist_state()?;
                    Ok(self.result(format_issues(&issues), None))
                }
            }
            "submit" => {
                let (mut plan, issues) = self.compile_and_check()?;
                if !issues.is_empty() {
                    self.state.clean_check_passes = 0;
                    self.persist_state()?;
                    return Ok(self.result(format_issues(&issues), None));
                }
                if self.state.last_checked_revision != Some(self.state.revision)
                    || self.state.clean_check_passes < 2
                {
                    return Ok(self.result(
                        "SUBMIT BLOCKED: run check on the current revision. After the first clean check, inspect the complete codebase as a film/code reviewer and run a second clean check before submit."
                            .into(),
                        None,
                    ));
                }
                plan.quality_review = MovieQualityReview {
                    attempts: self.state.clean_check_passes,
                    score: 100,
                    verdict: "Bonsai completed an incremental workspace build, native H3 lint, and full code-review pass.".into(),
                };
                Ok(self.result(
                    format!(
                        "SUBMITTED: {} scenes, {:.1} seconds. The app will now preserve the canonical plan.",
                        plan.clips.len(),
                        total_seconds(&plan)
                    ),
                    Some(plan),
                ))
            }
            _ => Err(StudioError::Invalid(format!(
                "unknown movie workspace action: {}",
                request.action
            ))),
        }
    }

    fn result(&self, message: String, submitted: Option<MoviePlan>) -> WorkspaceToolResult {
        WorkspaceToolResult { message, submitted }
    }

    fn list_files(&self) -> Result<String, StudioError> {
        let mut files = vec![
            "README.md".to_string(),
            "BRIEF.md".into(),
            "REFERENCES.md".into(),
        ];
        if self.root.join("movie.json").is_file() {
            files.push("movie.json".into());
        }
        if let Ok(entries) = fs::read_dir(self.root.join("scenes")) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        files.push(format!("scenes/{name}"));
                    }
                }
            }
        }
        files.sort();
        Ok(format!(
            "revision={} cleanChecks={}\n{}",
            self.state.revision,
            self.state.clean_check_passes,
            files.join("\n")
        ))
    }

    fn read_file(&self, path: &str) -> Result<String, StudioError> {
        let normalized = readable_path(path)?;
        let target = self.root.join(normalized);
        let bytes = fs::read(&target).map_err(|error| {
            StudioError::Invalid(format!("cannot read {}: {error}", target.display()))
        })?;
        if bytes.len() > MAX_WORKSPACE_FILE_BYTES {
            return Err(StudioError::Invalid(
                "workspace file is too large to read".into(),
            ));
        }
        String::from_utf8(bytes)
            .map_err(|_| StudioError::Invalid("workspace files must be UTF-8".into()))
    }

    fn write_file(&mut self, path: &str, content: &str) -> Result<(), StudioError> {
        validate_write(path, content)?;
        self.ensure_mutation_allowed()?;
        self.changed()?;
        self.write_validated_file(path, content)
    }

    fn ensure_mutation_allowed(&self) -> Result<(), StudioError> {
        let limit = self.max_clips.saturating_add(1) as u64;
        if self.state.mutations_since_check >= limit {
            return Err(StudioError::Invalid(format!(
                "CHECK REQUIRED: {} unchecked workspace mutations reached the full-draft limit. Run check now; then repair only reported files before checking again.",
                self.state.mutations_since_check
            )));
        }
        Ok(())
    }

    fn write_validated_file(&self, path: &str, content: &str) -> Result<(), StudioError> {
        let normalized = writable_path(path)?;
        let canonical = canonical_workspace_content(&normalized, content)?;
        write_text_atomic(&self.root.join(normalized), &canonical)
    }

    fn changed(&mut self) -> Result<(), StudioError> {
        self.state.revision = self.state.revision.saturating_add(1);
        self.state.mutations_since_check = self.state.mutations_since_check.saturating_add(1);
        self.state.last_checked_revision = None;
        self.state.clean_check_passes = 0;
        self.persist_state()
    }

    fn compile_and_check(&self) -> Result<(MoviePlan, Vec<String>), StudioError> {
        let metadata_path = self.root.join("movie.json");
        if !metadata_path.is_file() {
            return Ok((empty_plan(), vec!["movie.json is missing.".into()]));
        }
        let metadata: MovieMetadata = serde_json::from_slice(&fs::read(metadata_path)?)?;
        let mut scene_paths = fs::read_dir(self.root.join("scenes"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        scene_paths.sort();
        let mut parse_issues = Vec::new();
        if let Some((index, path)) = scene_paths.iter().enumerate().find(|(index, path)| {
            let expected = format!("{:03}.json", index + 1);
            path.file_name().and_then(|name| name.to_str()) != Some(expected.as_str())
        }) {
            let expected = format!("scenes/{:03}.json", index + 1);
            let found = format!(
                "scenes/{}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("?")
            );
            parse_issues.push(format!(
                "Scene filenames must be one contiguous sequence so Clip diagnostics map to exact files: expected {expected} at position {}, found {found}. Write the missing/renumbered file and delete stale sparse files before checking again.",
                index + 1
            ));
        }
        let mut clips = Vec::with_capacity(scene_paths.len());
        for path in scene_paths {
            let relative = format!(
                "scenes/{}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("?")
            );
            if writable_path(&relative).is_err() {
                parse_issues.push(format!("Remove invalid scene file {relative}."));
                continue;
            }
            match fs::read(&path)
                .map_err(StudioError::from)
                .and_then(|bytes| {
                    serde_json::from_slice::<PlannedClip>(&bytes).map_err(StudioError::from)
                }) {
                Ok(mut clip) => {
                    clip.id = format!("clip-{:03}", clips.len() + 1);
                    self.resolve_reference_ids(&mut clip);
                    clips.push(clip);
                }
                Err(error) => parse_issues.push(format!("{relative} is invalid: {error}")),
            }
        }
        let plan = MoviePlan {
            title: metadata.title,
            logline: metadata.logline,
            audience: metadata.audience,
            creative_direction: metadata.creative_direction,
            continuity_bible: metadata.continuity_bible,
            source_credits: metadata.source_credits,
            quality_review: MovieQualityReview::default(),
            clips,
        };
        let mut issues = parse_issues;
        if plan.title.trim().is_empty() {
            issues.push("movie.json title is empty.".into());
        }
        if plan.clips.is_empty() {
            issues.push("No scene files exist.".into());
        }
        if plan.clips.len() > self.max_clips {
            issues.push(format!(
                "The workspace has {} scenes; the producer setting permits at most {}.",
                plan.clips.len(),
                self.max_clips
            ));
        }
        issues.extend(prompt_quality_issues(&plan, &self.references));
        issues.extend(producer_intent_issues(
            &self.prompt,
            &plan,
            &self.references,
        ));
        Ok((plan, issues))
    }

    fn resolve_reference_ids(&self, clip: &mut PlannedClip) {
        let workspace_ids = workspace_reference_ids(&self.references);
        for id in &mut clip.reference_ids {
            if let Some(index) = workspace_ids
                .iter()
                .position(|workspace_id| workspace_id == id)
            {
                *id = self.references[index].asset_id.clone();
            }
        }
    }

    fn replace_with_plan(&mut self, plan: &MoviePlan) -> Result<(), StudioError> {
        let metadata = MovieMetadata {
            title: plan.title.clone(),
            logline: plan.logline.clone(),
            audience: plan.audience.clone(),
            creative_direction: plan.creative_direction.clone(),
            continuity_bible: plan.continuity_bible.clone(),
            source_credits: plan.source_credits.clone(),
        };
        self.changed()?;
        write_text_atomic(
            &self.root.join("movie.json"),
            &serde_json::to_string_pretty(&metadata)?,
        )?;
        let scenes = self.root.join("scenes");
        for entry in fs::read_dir(&scenes)?.flatten() {
            if entry.path().is_file() {
                fs::remove_file(entry.path())?;
            }
        }
        for (index, clip) in plan.clips.iter().enumerate() {
            write_text_atomic(
                &scenes.join(format!("{:03}.json", index + 1)),
                &serde_json::to_string_pretty(clip)?,
            )?;
        }
        Ok(())
    }

    fn persist_state(&self) -> Result<(), StudioError> {
        let value = serde_json::to_string_pretty(&self.state)?;
        write_text_atomic(&self.root.join("state.json"), &value)
    }
}

fn workspace_readme(settings: &MovieSettings) -> String {
    let mut readme = format!(
        "# Kestrel movie workspace\n\nThis is a durable, sandboxed movie codebase. Read BRIEF.md and REFERENCES.md. Build one canonical movie.json plus ordered scenes/NNN.json files. You may batch-write the first draft, then run `check` immediately; the workspace requires a check after one full draft's worth of unchecked mutations. Patch only reported weaknesses, perform the requested full code-review pass, run `check` again, and `submit`. Do not ask the producer questions; infer tasteful choices. Never stop at prose.\n\n## movie.json\n\nA JSON object with exactly: title, logline, audience, creativeDirection (strings), continuityBible, and sourceCredits (arrays of descriptive strings or structured objects; objects are normalized to readable plan facts). Resolve unspecified story-critical subjects, creatures, objects, locations, wardrobe, and visual motifs into concrete repeatable production facts in the continuityBible; do not leave a recurring subject generic. There are no research tools: never invent defining anatomy, cultural details, or other real-world facts. When uncertain, choose a familiar concrete alternative that still serves the brief or avoid unsupported specificity. Leave sourceCredits empty unless BRIEF.md or REFERENCES.md explicitly supplies an attribution; never invent publications, organizations, research, licenses, or provenance.\n\n## scenes/NNN.json\n\nUse exactly three digits in every scene filename: scenes/001.json, scenes/002.json, and so on. Each ordered JSON object has: title, purpose, durationSeconds (5-15), prompt, continuityIn, continuityOut, transition, usePreviousFrame, sourceRefs (textual source-credit IDs only), referenceIds (short exact IDs from REFERENCES.md such as picture-1 or audio-1). Kestrel resolves those stable workspace IDs to immutable asset hashes; never copy hashes or H3 tags into scene files. The app assigns clip IDs. Maximum scenes: {}. Native output is 24fps at {}x{}.\n\nEvery prompt must be 120-450 words and be a final H3 renderer instruction, not a synopsis: medium/genre/environment, lighting/palette/lens/texture, scene overview, complete timecoded picture/action/camera/sound through the exact native endpoint, dialogue when relevant, transitions, and relevant exclusions. Whenever anyone speaks or narrates, write the exact short words in quotation marks and make them fit the scene duration; otherwise explicitly direct no dialogue. Preserve causality, identity, geography, screen direction, visual language, and sound. Long-form narrative work needs a mixed edit grammar: motivated independent cuts, useful subject-free scenes/inserts, flashbacks when story calls for them, and at least one exact previous-frame continuation across a genuinely continuous boundary. Do not pad runtime with scenes that merely repeat the same action, emotion, framing, and sound under a new title.\n\nReference conditioning and previous-frame continuation are mutually exclusive per scene. A referenced ref2va scene may attach referenceIds but cannot receive the prior last frame. A continuation fl2va scene may usePreviousFrame=true only with empty referenceIds. Establish a referenced subject first, end on the handoff pose, then continue reference-free; place required reference audio in a reference-locked scene. Age changes do not waive identity: an independently cut younger or older version of a referenced character still needs that identity reference, while a truly adjacent age-consistent shot may carry it through usePreviousFrame. Never solve conflicts by silently dropping requested media. References are H3 conditioning, not editorial tracks; never trim, pad, loop, replace, or add silence. When a supplied audio reference represents speech or a voice, include the literal role sentence `Use the supplied voice reference as the speaker's voice identity and vocal timbre.` and write the exact short dialogue in quotation marks so H3 is not forced to invent words; keep the dialogue feasible within the native scene duration.\n",
        settings.max_clips, settings.width, settings.height
    );
    readme.push_str(
        "\nExact-frame handoff invariant: continuityOut and the next continuityIn must repeat at least two concrete visible anchors (subject/object, pose/action, geography, or time state). Keep numbered scenes in truthful editorial order across present-day and flashback boundaries; sharing only a character name is not continuity. Only the actual last scene may declare the end, and it cannot promise a next scene. Do not ask a generic identity picture to guarantee an invented child/younger transformation; without a matching age reference, keep the character at the supplied age or make that memory POV, off-screen, or subject-free.\n",
    );
    readme
}

fn workspace_reference_manifest(references: &[MovieReference]) -> String {
    if references.is_empty() {
        return "# Producer references\n\nNo producer media is attached.\n".into();
    }
    let mut output = String::from(
        "# Producer references\n\nUse only the short Reference ID in scene referenceIds. Kestrel resolves it to the immutable asset internally. Descriptions guide placement and are never renderer prose.\n",
    );
    let workspace_ids = workspace_reference_ids(references);
    for (reference, workspace_id) in references.iter().zip(workspace_ids) {
        let reference_type = if reference.kind == "audio" {
            "native clip audio"
        } else {
            reference.kind.as_str()
        };
        output.push_str(&format!(
            "\nReference ID: {}\nType: {}\nProducer description: {}\n",
            workspace_id, reference_type, reference.description
        ));
        if reference.use_embedded_audio {
            output.push_str(&format!(
                "Existing embedded clip audio placement: {}\n",
                reference.embedded_audio_description
            ));
        }
    }
    output
}

fn normalized_workspace_reference_id(reference: &MovieReference, index: usize) -> String {
    let mut id = String::new();
    let mut separator = false;
    for character in reference.tag.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if separator && !id.is_empty() {
                id.push('-');
            }
            id.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    if id.is_empty() {
        format!("reference-{}", index + 1)
    } else {
        id
    }
}

fn workspace_reference_ids(references: &[MovieReference]) -> Vec<String> {
    let mut used = HashSet::new();
    references
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            let base = normalized_workspace_reference_id(reference, index);
            let mut candidate = base.clone();
            let mut suffix = 2_u32;
            while !used.insert(candidate.clone()) {
                candidate = format!("{base}-{suffix}");
                suffix = suffix.saturating_add(1);
            }
            candidate
        })
        .collect()
}

fn validate_write(path: &str, content: &str) -> Result<(), StudioError> {
    let normalized = writable_path(path)?;
    if content.len() > MAX_WORKSPACE_FILE_BYTES {
        return Err(StudioError::Invalid(format!(
            "{normalized} exceeds the 96 KiB workspace-file limit"
        )));
    }
    canonical_workspace_content(&normalized, content)?;
    Ok(())
}

fn canonical_workspace_content(path: &str, content: &str) -> Result<String, StudioError> {
    if path == "movie.json" {
        serde_json::from_str::<MovieMetadata>(content)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .map_err(|error| {
                StudioError::Invalid(format!(
                    "movie.json schema error: {error}. Expected title, logline, audience, and creativeDirection as strings, plus continuityBible and sourceCredits as arrays. Array entries may be descriptive strings or structured objects."
                ))
            })
    } else {
        serde_json::from_str::<PlannedClip>(content)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .map_err(|error| {
                StudioError::Invalid(format!(
                    "{path} schema error: {error}. Expected title, purpose, prompt, continuityIn, continuityOut, and transition as strings; durationSeconds as a number from 5 to 15; usePreviousFrame as a boolean; and sourceRefs and referenceIds as string arrays."
                ))
            })
    }
}

fn readable_path(path: &str) -> Result<String, StudioError> {
    let path = path.trim().replace('\\', "/");
    if matches!(
        path.as_str(),
        "README.md" | "BRIEF.md" | "REFERENCES.md" | "movie.json"
    ) || scene_path_is_valid(&path)
    {
        Ok(path)
    } else {
        Err(StudioError::Invalid(
            "path must be README.md, BRIEF.md, REFERENCES.md, movie.json, or scenes/NNN.json"
                .into(),
        ))
    }
}

fn writable_path(path: &str) -> Result<String, StudioError> {
    let path = path.trim().replace('\\', "/");
    if path == "movie.json" || scene_path_is_valid(&path) {
        Ok(path)
    } else {
        Err(StudioError::Invalid(
            "only movie.json and scenes/NNN.json are writable".into(),
        ))
    }
}

fn scene_path_is_valid(path: &str) -> bool {
    let Some(name) = path.strip_prefix("scenes/") else {
        return false;
    };
    let Some(number) = name.strip_suffix(".json") else {
        return false;
    };
    number.len() == 3 && number.bytes().all(|byte| byte.is_ascii_digit()) && number != "000"
}

fn write_text_atomic(path: &Path, content: &str) -> Result<(), StudioError> {
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("file")
    ));
    fs::write(&temporary, content.as_bytes())?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn format_issues(issues: &[String]) -> String {
    let mut output = format!(
        "CHECK FAIL: {} issue(s). Patch only affected files.\n",
        issues.len()
    );
    for (index, issue) in issues.iter().enumerate() {
        output.push_str(&format!("{}. {}\n", index + 1, issue));
    }
    output
}

fn total_seconds(plan: &MoviePlan) -> f32 {
    plan.clips.iter().map(|clip| clip.duration_seconds).sum()
}

fn empty_plan() -> MoviePlan {
    MoviePlan {
        title: String::new(),
        logline: String::new(),
        audience: String::new(),
        creative_direction: String::new(),
        continuity_bible: Vec::new(),
        source_credits: Vec::new(),
        quality_review: MovieQualityReview::default(),
        clips: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace() -> PathBuf {
        std::env::temp_dir().join(format!("kestrel-movie-agent-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn workspace_rejects_paths_outside_the_movie_codebase() {
        assert!(writable_path("../project.json").is_err());
        assert!(writable_path("C:\\Windows\\win.ini").is_err());
        assert!(writable_path("README.md").is_err());
        assert_eq!(writable_path("scenes/001.json").unwrap(), "scenes/001.json");
    }

    #[test]
    fn workspace_persists_seed_as_incremental_files() {
        let root = temp_workspace();
        let plan = MoviePlan {
            title: "Test".into(),
            logline: "A sufficiently descriptive test logline for the movie.".into(),
            audience: "Film producers".into(),
            creative_direction: "A detailed practical live-action direction for a durable test."
                .into(),
            continuity_bible: vec!["The hero always wears a weathered red field jacket.".into()],
            source_credits: Vec::new(),
            quality_review: MovieQualityReview::default(),
            clips: vec![PlannedClip {
                id: "old-id".into(),
                title: "Opening".into(),
                purpose: "Establish the hero".into(),
                duration_seconds: 5.0,
                prompt: "placeholder".into(),
                continuity_in: "Independent opening frame".into(),
                continuity_out: "Hero holds on the doorway".into(),
                transition: "Hard cut".into(),
                use_previous_frame: false,
                source_refs: Vec::new(),
                reference_ids: Vec::new(),
            }],
        };
        let workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a test film",
            "",
            &MovieSettings::default(),
            &[],
            Some(&plan),
            Some("Keep the opening"),
        )
        .unwrap();
        assert!(root.join("movie.json").is_file());
        assert!(root.join("scenes/001.json").is_file());
        assert!(workspace
            .read_file("BRIEF.md")
            .unwrap()
            .contains("Keep the opening"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_normalizes_structured_metadata_facts() {
        let root = temp_workspace();
        let mut workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a test film",
            "",
            &MovieSettings::default(),
            &[],
            None,
            None,
        )
        .unwrap();
        workspace
            .write_file(
                "movie.json",
                &json!({
                    "title":"Jungle Awakening",
                    "logline":"A vlogger encounters a rare deer in a misty forest.",
                    "audience":"Film buyers",
                    "creativeDirection":"Naturalistic live-action photography.",
                    "continuityBible":[{
                        "subject":"Golden deer",
                        "appearance":"golden-brown coat and white facial markings"
                    }],
                    "sourceCredits":[{
                        "referenceId":"reference-001",
                        "use":"vlogger identity"
                    }]
                })
                .to_string(),
            )
            .unwrap();

        let metadata: MovieMetadata =
            serde_json::from_str(&workspace.read_file("movie.json").unwrap()).unwrap();
        assert_eq!(metadata.continuity_bible.len(), 1);
        assert!(metadata.continuity_bible[0].contains("subject: Golden deer"));
        assert!(metadata.continuity_bible[0].contains("appearance: golden-brown coat"));
        assert!(metadata.source_credits[0].contains("referenceId: reference-001"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_reports_file_specific_schema_errors() {
        let error = canonical_workspace_content(
            "movie.json",
            r#"{"title":"Test","logline":"Test","audience":"Test","creativeDirection":{},"continuityBible":[],"sourceCredits":[]}"#,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("movie.json schema error"));
        assert!(message.contains("creativeDirection as strings"));
    }

    #[test]
    fn workspace_resolves_short_reference_ids_to_asset_hashes() {
        let root = temp_workspace();
        let asset_id = "ad240595fc48dd73529176a4f6df1a68a0896ac7e3ce358e94699a0538f4c3d3";
        let reference = MovieReference {
            asset_id: asset_id.into(),
            tag: "<Picture 1>".into(),
            audio_tag: String::new(),
            name: "vlogger.png".into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: 1,
            duration_seconds: 0.0,
            width: 1024,
            height: 1024,
            has_audio: false,
            path: "vlogger.png".into(),
            description: "This is the vlogger's identity reference.".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
        };
        let mut workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a test film",
            "unused canonical manifest",
            &MovieSettings::default(),
            std::slice::from_ref(&reference),
            None,
            None,
        )
        .unwrap();
        let references = workspace.read_file("REFERENCES.md").unwrap();
        assert!(references.contains("Reference ID: picture-1"));
        assert!(!references.contains(asset_id));
        workspace
            .write_file(
                "movie.json",
                &json!({
                    "title":"Test",
                    "logline":"A vlogger enters a forest.",
                    "audience":"Film buyers",
                    "creativeDirection":"Live-action photography",
                    "continuityBible":["The vlogger keeps the same face."],
                    "sourceCredits":[]
                })
                .to_string(),
            )
            .unwrap();
        workspace
            .write_file(
                "scenes/001.json",
                &json!({
                    "title":"Vlogger enters",
                    "purpose":"Show the vlogger entering",
                    "durationSeconds":10,
                    "prompt":"A deliberately incomplete test prompt.",
                    "continuityIn":"Independent opening",
                    "continuityOut":"The vlogger stops",
                    "transition":"Hard cut",
                    "usePreviousFrame":false,
                    "sourceRefs":[],
                    "referenceIds":["picture-1"]
                })
                .to_string(),
            )
            .unwrap();
        let (plan, _) = workspace.compile_and_check().unwrap();
        assert_eq!(plan.clips[0].reference_ids, vec![asset_id]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_reference_ids_are_normalized_and_collision_safe() {
        let reference = |tag: &str, asset_id: &str| MovieReference {
            asset_id: asset_id.into(),
            tag: tag.into(),
            audio_tag: String::new(),
            name: "reference.png".into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: 1,
            duration_seconds: 0.0,
            width: 1,
            height: 1,
            has_audio: false,
            path: "reference.png".into(),
            description: "Identity reference".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
        };
        let references = vec![
            reference("Picture 10", "first"),
            reference("picture-10", "second"),
            reference("picture 10-2", "third"),
            reference("!!!", "fourth"),
        ];
        assert_eq!(
            workspace_reference_ids(&references),
            vec![
                "picture-10",
                "picture-10-2",
                "picture-10-2-2",
                "reference-4"
            ]
        );
        let root = temp_workspace();
        let workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a reference test",
            "",
            &MovieSettings::default(),
            &references,
            None,
            None,
        )
        .unwrap();
        let manifest = workspace.read_file("REFERENCES.md").unwrap();
        assert!(manifest.contains("Reference ID: picture-10-2"));
        let mut clip = PlannedClip {
            id: String::new(),
            title: "Reference test".into(),
            purpose: "Verify collision-safe binding".into(),
            duration_seconds: 5.0,
            prompt: "Placeholder".into(),
            continuity_in: String::new(),
            continuity_out: String::new(),
            transition: "cut".into(),
            use_previous_frame: false,
            source_refs: vec![],
            reference_ids: vec!["picture-10-2".into()],
        };
        workspace.resolve_reference_ids(&mut clip);
        assert_eq!(clip.reference_ids, vec!["second"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_many_rejects_oversized_aggregate_responses() {
        let root = temp_workspace();
        let mut workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a test film",
            "",
            &MovieSettings::default(),
            &[],
            None,
            None,
        )
        .unwrap();
        let content = "x".repeat(90 * 1024);
        for index in 1..=3 {
            fs::write(
                root.join("scenes").join(format!("{index:03}.json")),
                &content,
            )
            .unwrap();
        }
        let result = workspace.execute(WorkspaceToolRequest {
            action: "read_many".into(),
            path: String::new(),
            content: String::new(),
            files: (1..=3)
                .map(|index| WorkspaceFileWrite {
                    path: format!("scenes/{index:03}.json"),
                    content: String::new(),
                })
                .collect(),
        });
        assert!(result.message.contains("aggregate limit"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_rejects_sparse_scene_filenames() {
        let root = temp_workspace();
        let mut workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a test film",
            "",
            &MovieSettings::default(),
            &[],
            None,
            None,
        )
        .unwrap();
        workspace
            .write_file(
                "movie.json",
                &json!({
                    "title":"Test",
                    "logline":"A subject enters a forest.",
                    "audience":"Film buyers",
                    "creativeDirection":"Live-action photography",
                    "continuityBible":["The subject keeps the same wardrobe throughout."],
                    "sourceCredits":[]
                })
                .to_string(),
            )
            .unwrap();
        let scene = json!({
            "title":"Forest shot",
            "purpose":"Show the forest",
            "durationSeconds":10,
            "prompt":"A deliberately incomplete test prompt.",
            "continuityIn":"Independent cut",
            "continuityOut":"The subject stops",
            "transition":"Hard cut",
            "usePreviousFrame":false,
            "sourceRefs":[],
            "referenceIds":[]
        })
        .to_string();
        workspace.write_file("scenes/001.json", &scene).unwrap();
        workspace.write_file("scenes/003.json", &scene).unwrap();

        let (_, issues) = workspace.compile_and_check().unwrap();
        assert!(issues
            .join(" ")
            .contains("expected scenes/002.json at position 2, found scenes/003.json"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_requires_check_after_a_full_draft_mutation_budget() {
        let root = temp_workspace();
        let settings = MovieSettings {
            max_clips: 1,
            ..MovieSettings::default()
        };
        let mut workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a test film",
            "",
            &settings,
            &[],
            None,
            None,
        )
        .unwrap();
        let metadata = json!({
            "title":"Test",
            "logline":"A subject enters a forest and finds a signal.",
            "audience":"Film buyers",
            "creativeDirection":"Live-action photography with tactile natural detail.",
            "continuityBible":["The subject keeps the same wardrobe throughout."],
            "sourceCredits":[]
        })
        .to_string();
        let scene = json!({
            "title":"Forest shot",
            "purpose":"Show the forest",
            "durationSeconds":10,
            "prompt":"A deliberately incomplete test prompt.",
            "continuityIn":"Independent cut",
            "continuityOut":"The subject stops",
            "transition":"Hard cut",
            "usePreviousFrame":false,
            "sourceRefs":[],
            "referenceIds":[]
        })
        .to_string();
        workspace.write_file("movie.json", &metadata).unwrap();
        workspace.write_file("scenes/001.json", &scene).unwrap();
        let blocked = workspace.execute(WorkspaceToolRequest {
            action: "write".into(),
            path: "scenes/001.json".into(),
            content: scene.clone(),
            files: Vec::new(),
        });
        assert!(blocked.message.contains("CHECK REQUIRED"));
        assert_eq!(workspace.state.revision, 2);

        let checked = workspace
            .try_execute(WorkspaceToolRequest {
                action: "check".into(),
                path: String::new(),
                content: String::new(),
                files: Vec::new(),
            })
            .unwrap();
        assert!(checked.message.contains("CHECK FAIL"));
        assert_eq!(workspace.state.mutations_since_check, 0);
        workspace.write_file("scenes/001.json", &scene).unwrap();
        assert_eq!(workspace.state.revision, 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_lints_a_bad_scene_then_accepts_an_incremental_repair() {
        let root = temp_workspace();
        let mut workspace = MovieAgentWorkspace::open(
            root.clone(),
            "Make a compact atmospheric film",
            "",
            &MovieSettings::default(),
            &[],
            None,
            None,
        )
        .unwrap();
        let metadata = json!({
            "title":"Signal Fire",
            "logline":"A lone keeper discovers a signal crossing the winter valley.",
            "audience":"Festival film buyers",
            "creativeDirection":"Practical live-action photography with restrained suspense and tactile winter detail.",
            "continuityBible":["The keeper wears the same charcoal coat and carries one brass lantern throughout."],
            "sourceCredits":[]
        })
        .to_string();
        let bad_scene = json!({
            "title":"The signal",
            "purpose":"Establish the discovery",
            "durationSeconds":5,
            "prompt":"Too short",
            "continuityIn":"Independent winter valley opening",
            "continuityOut":"The keeper holds the lantern at chest height",
            "transition":"Hard cut",
            "usePreviousFrame":false,
            "sourceRefs":[],
            "referenceIds":[]
        })
        .to_string();
        workspace
            .try_execute(WorkspaceToolRequest {
                action: "write_batch".into(),
                path: String::new(),
                content: String::new(),
                files: vec![
                    WorkspaceFileWrite {
                        path: "movie.json".into(),
                        content: metadata,
                    },
                    WorkspaceFileWrite {
                        path: "scenes/001.json".into(),
                        content: bad_scene,
                    },
                ],
            })
            .unwrap();
        let failed = workspace
            .try_execute(WorkspaceToolRequest {
                action: "check".into(),
                path: String::new(),
                content: String::new(),
                files: Vec::new(),
            })
            .unwrap();
        assert!(failed.message.contains("prompt has 2 words"));

        let mut prompt = String::from(
            "Live-action cinematic winter mystery with practical film photography, pale blue dawn light, warm lantern contrast, restrained grading, fine film grain, and a 40mm anamorphic lens. Scene overview: a lone keeper discovers a distant signal across a snowbound valley and decides to answer it. [0s-2s] Wide locked framing shows the keeper entering from screen left while wind drives powder across dark pines; cloth and footsteps move with natural weight. [2s-4s] The camera makes a slow deliberate push toward the keeper as the brass lantern rises and a tiny answering light appears on the opposite ridge. [4s-5s] Hold through the exact endpoint on the keeper's silent reaction, lantern steady at chest height, with the distant light still visible. Camera remains physically plausible with shallow depth of field and subtle handheld breathing. Audio: winter wind, boots compressing snow, coat fabric, a low restrained score, and one distant metallic pulse; sound settles but continues through 5s. No text, subtitles, logos, watermarks, animation, cartoon rendering, glossy CGI, identity drift, implausible motion, or abrupt audio cut.",
        );
        while prompt.split_whitespace().count() < 120 {
            prompt.push_str(
                " The composition preserves valley geography and left-to-right screen direction.",
            );
        }
        let repaired_scene = json!({
            "title":"The signal",
            "purpose":"Establish the discovery",
            "durationSeconds":5,
            "prompt":prompt,
            "continuityIn":"Independent winter valley opening",
            "continuityOut":"The keeper holds the lantern at chest height",
            "transition":"Hard cut",
            "usePreviousFrame":false,
            "sourceRefs":[],
            "referenceIds":[]
        })
        .to_string();
        workspace
            .try_execute(WorkspaceToolRequest {
                action: "write".into(),
                path: "scenes/001.json".into(),
                content: repaired_scene,
                files: Vec::new(),
            })
            .unwrap();
        for expected_pass in 1..=2 {
            let checked = workspace
                .try_execute(WorkspaceToolRequest {
                    action: "check".into(),
                    path: String::new(),
                    content: String::new(),
                    files: Vec::new(),
                })
                .unwrap();
            assert!(checked.message.contains("CHECK PASS"));
            assert_eq!(workspace.state.clean_check_passes, expected_pass);
        }
        let submitted = workspace
            .try_execute(WorkspaceToolRequest {
                action: "submit".into(),
                path: String::new(),
                content: String::new(),
                files: Vec::new(),
            })
            .unwrap();
        assert!(submitted.submitted.is_some());
        assert_eq!(submitted.submitted.unwrap().quality_review.score, 100);
        fs::remove_dir_all(root).unwrap();
    }
}
