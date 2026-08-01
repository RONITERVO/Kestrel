use crate::html::render_report_html;
use crate::models::{ReportSummary, ResearchReport};
use chrono::{Datelike, Utc};
use directories::UserDirs;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("research library error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("research file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("research JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("research report not found: {0}")]
    NotFound(String),
}

#[derive(Clone)]
pub struct ResearchStore {
    root: PathBuf,
}

impl ResearchStore {
    pub fn open_default() -> Result<Self, StoreError> {
        let root = UserDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Kestrel Research");
        Self::open(root)
    }

    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        fs::create_dir_all(root.join("reports"))?;
        let store = Self { root };
        store.initialize()?;
        store.ensure_guide()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn connect(&self) -> Result<Connection, StoreError> {
        let connection = Connection::open(self.root.join("catalog.sqlite3"))?;
        connection.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }

    fn initialize(&self) -> Result<(), StoreError> {
        self.connect()?.execute_batch(
            "CREATE TABLE IF NOT EXISTS reports (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                query TEXT NOT NULL,
                dek TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                edition INTEGER NOT NULL,
                parent_id TEXT,
                source_count INTEGER NOT NULL,
                reading_minutes INTEGER NOT NULL,
                json_path TEXT NOT NULL,
                html_path TEXT NOT NULL,
                search_text TEXT NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS reports_fts USING fts5(
                id UNINDEXED, title, query, dek, search_text, tokenize='unicode61 remove_diacritics 2'
            );
            CREATE TRIGGER IF NOT EXISTS reports_ai AFTER INSERT ON reports BEGIN
              INSERT INTO reports_fts(id,title,query,dek,search_text)
              VALUES (new.id,new.title,new.query,new.dek,new.search_text);
            END;
            CREATE TRIGGER IF NOT EXISTS reports_ad AFTER DELETE ON reports BEGIN
              DELETE FROM reports_fts WHERE id=old.id;
            END;"
        )?;
        Ok(())
    }

    fn ensure_guide(&self) -> Result<(), StoreError> {
        let guide = self.root.join("README.txt");
        if guide.exists() {
            return Ok(());
        }
        fs::write(
            guide,
            "KESTREL RESEARCH LIBRARY\n\nEvery report is immutable and lives under reports/YYYY/MM/.\nEach folder contains:\n  index.html      polished standalone page\n  report.json     structured research for local models\n  sources.json    inspected evidence ledger\n  provenance.json model, archive, query, and edition lineage\n\ncatalog.jsonl is a rebuildable, one-report-per-line index for local tools.\ncatalog.sqlite3 is Kestrel's fast full-text index. HTML and JSON remain usable without it.\n\nNew research never silently overwrites old work. Related work links through parentId and edition.\n",
        )?;
        Ok(())
    }

    pub fn list(&self, limit: usize) -> Result<Vec<ReportSummary>, StoreError> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id,title,query,dek,updated_at,edition,source_count,reading_minutes
             FROM reports ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], summary_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<ReportSummary>, StoreError> {
        let terms = search_terms(query);
        if terms.is_empty() {
            return self.list(limit);
        }
        let fts_query = terms
            .iter()
            .map(|term| format!("\"{}\"*", term.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT r.id,r.title,r.query,r.dek,r.updated_at,r.edition,r.source_count,r.reading_minutes
             FROM reports_fts f JOIN reports r ON r.id=f.id
             WHERE reports_fts MATCH ?1 ORDER BY bm25(reports_fts), r.updated_at DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![fts_query, limit as i64], summary_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn best_parent(&self, query: &str) -> Result<Option<ResearchReport>, StoreError> {
        let candidate = self
            .search(query, 5)?
            .into_iter()
            .find(|item| keyword_overlap(query, &item.query) >= 0.34);
        candidate.map(|item| self.get(&item.id)).transpose()
    }

    pub fn get(&self, id: &str) -> Result<ResearchReport, StoreError> {
        let connection = self.connect()?;
        let relative: Option<String> = connection
            .query_row("SELECT json_path FROM reports WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        let relative = relative.ok_or_else(|| StoreError::NotFound(id.to_owned()))?;
        let bytes = fs::read(self.root.join(relative))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn html_path(&self, id: &str) -> Result<PathBuf, StoreError> {
        let connection = self.connect()?;
        let relative: Option<String> = connection
            .query_row("SELECT html_path FROM reports WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        relative
            .map(|path| self.root.join(path))
            .ok_or_else(|| StoreError::NotFound(id.to_owned()))
    }

    pub fn save(&self, report: &mut ResearchReport) -> Result<(), StoreError> {
        let now = Utc::now();
        let short_id = report.id.chars().take(8).collect::<String>();
        let folder = self
            .root
            .join("reports")
            .join(now.year().to_string())
            .join(format!("{:02}", now.month()))
            .join(format!("{}--{}", slugify(&report.title), short_id));
        fs::create_dir_all(&folder)?;
        let json_path = folder.join("report.json");
        let source_path = folder.join("sources.json");
        let provenance_path = folder.join("provenance.json");
        let html_path = folder.join("index.html");
        report.html_path = relative_slashes(&self.root, &html_path);

        write_json_atomic(&json_path, report)?;
        write_json_atomic(&source_path, &report.sources)?;
        write_json_atomic(
            &provenance_path,
            &serde_json::json!({
                "schemaVersion": 1,
                "id": report.id,
                "query": report.query,
                "createdAt": report.created_at,
                "edition": report.edition,
                "parentId": report.parent_id,
                "improvement": report.improvement,
                "model": report.model,
                "archiveSnapshot": report.archive_snapshot,
                "researchProfile": report.research_profile,
                "contextWindow": report.context_window,
                "outputBudget": report.output_budget,
                "researchLanes": report.research_lanes,
                "offlineOnly": true,
            }),
        )?;
        write_atomic(&html_path, render_report_html(report).as_bytes())?;

        let search_text = format!(
            "{} {} {} {} {}",
            report.title,
            report.query,
            report.dek,
            report.answer,
            report
                .sections
                .iter()
                .map(|section| format!(
                    "{} {} {}",
                    section.heading,
                    section.summary,
                    section.body.join(" ")
                ))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO reports(id,title,query,dek,updated_at,edition,parent_id,source_count,reading_minutes,json_path,html_path,search_text)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                report.id,
                report.title,
                report.query,
                report.dek,
                report.updated_at,
                report.edition,
                report.parent_id,
                report.sources.len() as i64,
                report.reading_minutes,
                relative_slashes(&self.root, &json_path),
                report.html_path,
                search_text,
            ],
        )?;
        self.export_catalog()?;
        Ok(())
    }

    fn export_catalog(&self) -> Result<(), StoreError> {
        let summaries = self.list(100_000)?;
        let mut output = String::new();
        for summary in summaries {
            output.push_str(&serde_json::to_string(&summary)?);
            output.push('\n');
        }
        write_atomic(&self.root.join("catalog.jsonl"), output.as_bytes())
    }
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReportSummary> {
    Ok(ReportSummary {
        id: row.get(0)?,
        title: row.get(1)?,
        query: row.get(2)?,
        dek: row.get(3)?,
        updated_at: row.get(4)?,
        edition: row.get(5)?,
        source_count: row.get(6)?,
        reading_minutes: row.get(7)?,
    })
}

fn search_terms(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| {
            term.len() > 2
                && !matches!(
                    term.as_str(),
                    "the"
                        | "and"
                        | "for"
                        | "with"
                        | "what"
                        | "how"
                        | "why"
                        | "does"
                        | "was"
                        | "were"
                )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn keyword_overlap(left: &str, right: &str) -> f32 {
    let left = search_terms(left).into_iter().collect::<BTreeSet<_>>();
    let right = search_terms(right).into_iter().collect::<BTreeSet<_>>();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f32;
    intersection / left.len().min(right.len()) as f32
}

pub fn slugify(value: &str) -> String {
    let mut output = String::new();
    let mut previous_dash = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            output.push(character);
            previous_dash = false;
        } else if !previous_dash && !output.is_empty() {
            output.push('-');
            previous_dash = true;
        }
    }
    while output.ends_with('-') {
        output.pop();
    }
    output.truncate(72);
    if output.is_empty() {
        "research".into()
    } else {
        output
    }
}

fn relative_slashes(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), StoreError> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("file")
    ));
    fs::write(&temporary, bytes)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Finding, Source};

    fn report(id: &str, query: &str, title: &str) -> ResearchReport {
        ResearchReport {
            id: id.into(),
            title: title.into(),
            dek: "A durable test report.".into(),
            query: query.into(),
            answer: "Answer".into(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            edition: 1,
            parent_id: None,
            improvement: "Established a traceable first edition.".into(),
            model: "Bonsai".into(),
            archive_snapshot: "2024-01-12".into(),
            findings: vec![Finding {
                title: "Finding".into(),
                explanation: "Evidence".into(),
                citations: vec!["S1".into()],
            }],
            sections: vec![],
            timeline: vec![],
            terms: vec![],
            open_questions: vec![],
            sources: vec![Source {
                id: "S1".into(),
                kind: "wikipedia".into(),
                title: "Test".into(),
                section: None,
                snapshot: Some("2024-01-12".into()),
                reference: "/test".into(),
                excerpt: "Evidence".into(),
            }],
            html_path: String::new(),
            word_count: 10,
            reading_minutes: 1,
            research_profile: "standard".into(),
            context_window: 0,
            output_budget: 0,
            research_lanes: 1,
        }
    }

    #[test]
    fn creates_searchable_immutable_report_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let store = ResearchStore::open(directory.path().to_path_buf()).unwrap();
        let mut saved = report(
            "report-one",
            "ancient astronomical calculators",
            "Antikythera mechanism",
        );
        store.save(&mut saved).unwrap();
        assert!(directory.path().join(&saved.html_path).exists());
        assert!(store.get("report-one").is_ok());
        assert_eq!(
            store.search("astronomical calculator", 5).unwrap()[0].id,
            "report-one"
        );
        assert!(directory.path().join("catalog.jsonl").exists());
    }

    #[test]
    fn parent_matching_requires_meaningful_overlap() {
        assert!(
            keyword_overlap(
                "How did the Antikythera mechanism work?",
                "Antikythera mechanism gearing"
            ) > 0.5
        );
        assert!(keyword_overlap("Antikythera mechanism", "Forest ecology") < 0.1);
    }

    #[test]
    fn creates_safe_readable_slugs() {
        assert_eq!(slugify("  A Study: Stars & Time!  "), "a-study-stars-time");
        assert_eq!(slugify("???"), "research");
    }
}
