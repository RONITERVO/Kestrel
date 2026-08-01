use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use serde::Serialize;
use thiserror::Error;
use url::Url;

pub const BOOK: &str = "wikipedia_en_all_maxi_2024-01";
pub const SNAPSHOT: &str = "2024-01-12";

#[derive(Debug, Error)]
pub enum KiwixError {
    #[error("offline Wikipedia request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("offline Wikipedia returned an unsafe path")]
    UnsafePath,
    #[error("offline Wikipedia article has no readable text")]
    EmptyArticle,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub title: String,
    pub reference: String,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct Article {
    pub title: String,
    pub reference: String,
    pub section: Option<String>,
    pub text: String,
}

#[derive(Clone)]
pub struct KiwixClient {
    client: Client,
    base: Url,
}

impl KiwixClient {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(35))
                .build()
                .expect("HTTP client"),
            base: Url::parse("http://127.0.0.1:8085/").expect("fixed loopback URL"),
        }
    }

    pub async fn health(&self) -> bool {
        self.client
            .get(self.base.clone())
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, KiwixError> {
        let mut url = self
            .base
            .join("search")
            .map_err(|_| KiwixError::UnsafePath)?;
        url.query_pairs_mut()
            .append_pair("content", BOOK)
            .append_pair("pattern", query)
            .append_pair("pageLength", &limit.max(1).to_string());
        let html = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(parse_search_html(&html, limit))
    }

    pub async fn read(
        &self,
        reference: &str,
        section: Option<&str>,
        max_chars: usize,
    ) -> Result<Article, KiwixError> {
        let reference = normalize_reference(reference)?;
        let url = self
            .base
            .join(reference.trim_start_matches('/'))
            .map_err(|_| KiwixError::UnsafePath)?;
        if !matches!(url.host_str(), Some("127.0.0.1" | "localhost")) {
            return Err(KiwixError::UnsafePath);
        }
        let html = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let (title, mut text, actual_section) = parse_article_html(&html, section);
        if text.trim().is_empty() {
            return Err(KiwixError::EmptyArticle);
        }
        let limit = max_chars.clamp(2_000, 40_000);
        if text.len() > limit {
            text.truncate(floor_char_boundary(&text, limit));
            text.push_str(
                "\n\n[Excerpt truncated. Read a named section for more focused evidence.]",
            );
        }
        Ok(Article {
            title,
            reference,
            section: actual_section,
            text,
        })
    }
}

fn normalize_reference(input: &str) -> Result<String, KiwixError> {
    let trimmed = input.trim();
    let path = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let parsed = Url::parse(trimmed).map_err(|_| KiwixError::UnsafePath)?;
        if parsed.scheme() != "http"
            || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
            || parsed.port_or_known_default() != Some(8085)
        {
            return Err(KiwixError::UnsafePath);
        }
        parsed.path().to_owned()
    } else {
        trimmed.to_owned()
    };
    if path.contains("..") || path.contains(['\r', '\n', '\\', '?', '#']) || path.len() > 2_048 {
        return Err(KiwixError::UnsafePath);
    }
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() < 5
        || !segments[0].is_empty()
        || segments[1] != "content"
        || segments[3].is_empty()
        || segments[4].is_empty()
    {
        return Err(KiwixError::UnsafePath);
    }
    // Kiwix only serves the configured local archive. Canonicalizing its book segment
    // tolerates a small-model copy typo without broadening the loopback/path boundary.
    Ok(format!("/content/{BOOK}/{}", segments[3..].join("/")))
}

fn parse_search_html(input: &str, limit: usize) -> Vec<SearchResult> {
    let document = Html::parse_document(input);
    let anchor_selector = Selector::parse("a[href]").unwrap();
    let mut results = Vec::new();
    for anchor in document.select(&anchor_selector) {
        let reference = anchor.value().attr("href").unwrap_or_default();
        if !reference.starts_with(&format!("/content/{BOOK}/"))
            || results
                .iter()
                .any(|item: &SearchResult| item.reference == reference)
        {
            continue;
        }
        let title = clean_text(anchor.text().collect::<Vec<_>>().join(" "));
        if title.is_empty() {
            continue;
        }
        let snippet = anchor
            .parent()
            .and_then(ElementRef::wrap)
            .map(|parent| clean_text(parent.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        results.push(SearchResult {
            title,
            reference: reference.to_owned(),
            snippet: snippet.chars().take(420).collect(),
        });
        if results.len() >= limit {
            break;
        }
    }
    results
}

fn parse_article_html(
    input: &str,
    requested_section: Option<&str>,
) -> (String, String, Option<String>) {
    let document = Html::parse_document(input);
    let title_selector = Selector::parse("h1, title").unwrap();
    let title = document
        .select(&title_selector)
        .next()
        .map(|element| {
            clean_text(element.text().collect::<Vec<_>>().join(" ")).replace(" - Wikipedia", "")
        })
        .unwrap_or_else(|| "Wikipedia article".into());
    let content_selector = Selector::parse("article, main, .mw-parser-output, body").unwrap();
    let root = document
        .select(&content_selector)
        .next()
        .unwrap_or_else(|| document.root_element());
    let blocks = Selector::parse("h1,h2,h3,h4,p,li").unwrap();
    let requested = requested_section
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let mut collecting = requested.is_none();
    let mut selected_level = usize::MAX;
    let mut actual_section = None;
    let mut lines = Vec::new();
    for element in root.select(&blocks) {
        let tag = element.value().name();
        let text = clean_text(element.text().collect::<Vec<_>>().join(" "));
        if text.is_empty() {
            continue;
        }
        let heading_level = match tag {
            "h1" => Some(1),
            "h2" => Some(2),
            "h3" => Some(3),
            "h4" => Some(4),
            _ => None,
        };
        if let Some(level) = heading_level {
            if collecting && requested.is_some() && level <= selected_level {
                break;
            }
            if !collecting
                && requested
                    .as_ref()
                    .is_some_and(|needle| text.to_lowercase().contains(needle))
            {
                collecting = true;
                selected_level = level;
                actual_section = Some(text.clone());
            }
            if collecting {
                lines.push(format!("{} {text}", "#".repeat(level)));
            }
        } else if collecting {
            lines.push(if tag == "li" {
                format!("- {text}")
            } else {
                text
            });
        }
    }
    if requested.is_some() && !collecting {
        return (title, String::new(), None);
    }
    (
        title,
        lines.join("\n\n"),
        actual_section.or_else(|| requested_section.map(str::to_owned)),
    )
}

fn clean_text(value: String) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    while !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_local_archive_search_links() {
        let html = format!(
            r#"<ul><li><a href="/content/{BOOK}/A/Ada_Lovelace">Ada Lovelace</a><cite>English mathematician</cite></li><li><a href="https://example.com">Remote</a></li></ul>"#
        );
        let results = parse_search_html(&html, 8);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Ada Lovelace");
    }

    #[test]
    fn extracts_named_section_without_following_peer() {
        let html = "<html><body><h1>Example</h1><p>Lead</p><h2>History</h2><p>First</p><h3>Detail</h3><p>Second</p><h2>Elsewhere</h2><p>Excluded</p></body></html>";
        let (_, text, section) = parse_article_html(html, Some("History"));
        assert_eq!(section.as_deref(), Some("History"));
        assert!(text.contains("First") && text.contains("Second"));
        assert!(!text.contains("Excluded"));
    }

    #[test]
    fn canonicalizes_only_safe_local_kiwix_references() {
        assert_eq!(
            normalize_reference("/content/wikipedia_en_all_maxi_2024-001/A/Antikythera_mechanism")
                .unwrap(),
            format!("/content/{BOOK}/A/Antikythera_mechanism")
        );
        assert!(normalize_reference("https://example.com/content/book/A/Test").is_err());
        assert!(normalize_reference("/content/book/A/../secret").is_err());
        assert!(normalize_reference("/search?pattern=test").is_err());
    }

    #[tokio::test]
    #[ignore = "requires the user's local Kiwix archive on 127.0.0.1:8085"]
    async fn live_archive_search_and_read() {
        let client = KiwixClient::new();
        let results = client.search("Antikythera mechanism", 4).await.unwrap();
        let source = results
            .iter()
            .find(|item| item.title.to_lowercase().contains("antikythera mechanism"))
            .expect("Antikythera result");
        let article = client.read(&source.reference, None, 8_000).await.unwrap();
        assert!(article.title.to_lowercase().contains("antikythera"));
        assert!(article.text.len() > 2_000);
    }
}
