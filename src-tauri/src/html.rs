use crate::models::ResearchReport;
use html_escape::encode_text;

pub fn render_report_html(report: &ResearchReport) -> String {
    let findings = report
        .findings
        .iter()
        .enumerate()
        .map(|(index, finding)| {
            format!(
                "<article class=\"finding\"><span>{:02}</span><h3>{}</h3><p>{}</p>{}</article>",
                index + 1,
                esc(&finding.title),
                esc(&finding.explanation),
                citations(&finding.citations)
            )
        })
        .collect::<String>();
    let sections = report.sections.iter().enumerate().map(|(index, section)| {
        let body = section.body.iter().map(|paragraph| format!("<p>{}</p>", esc(paragraph))).collect::<String>();
        format!(
            "<section id=\"{}\"><header><span>{:02}</span><div><small>Deep dive</small><h2>{}</h2></div></header><p class=\"lead\">{}</p>{}{}</section>",
            esc(&section.id), index + 2, esc(&section.heading), esc(&section.summary), body, citations(&section.citations)
        )
    }).collect::<String>();
    let timeline = if report.timeline.is_empty() {
        String::new()
    } else {
        let items = report
            .timeline
            .iter()
            .map(|item| {
                format!(
            "<div class=\"moment\"><time>{}</time><i></i><div><h3>{}</h3><p>{}</p>{}</div></div>",
            esc(&item.date), esc(&item.label), esc(&item.description), citations(&item.citations)
        )
            })
            .collect::<String>();
        format!("<section><header><span>↳</span><div><small>Sequence</small><h2>Timeline</h2></div></header><div class=\"timeline\">{items}</div></section>")
    };
    let terms = report
        .terms
        .iter()
        .map(|term| {
            format!(
                "<div><dt>{}</dt><dd>{}</dd></div>",
                esc(&term.term),
                esc(&term.meaning)
            )
        })
        .collect::<String>();
    let questions = report
        .open_questions
        .iter()
        .map(|question| format!("<li>{}</li>", esc(question)))
        .collect::<String>();
    let sources = report.sources.iter().map(|source| format!(
        "<article class=\"source\" id=\"source-{}\"><b>{}</b><div><h3>{}</h3><small>{} · snapshot {}</small><blockquote>{}</blockquote></div></article>",
        esc(&source.id), esc(&source.id), esc(&source.title), esc(source.section.as_deref().unwrap_or("Full article")), esc(source.snapshot.as_deref().unwrap_or(&report.archive_snapshot)), esc(&source.excerpt)
    )).collect::<String>();
    let source_links = report
        .sources
        .iter()
        .map(|source| {
            format!(
                "<a href=\"#source-{}\">{} · {}</a>",
                esc(&source.id),
                esc(&source.id),
                esc(&source.title)
            )
        })
        .collect::<String>();
    let improvement = if report.edition > 1 {
        format!(
            "<aside class=\"improvement\"><b>What changed in edition {}</b><p>{}</p></aside>",
            report.edition,
            esc(&report.improvement)
        )
    } else {
        format!(
            "<aside class=\"improvement\"><b>First edition</b><p>{}</p></aside>",
            esc(&report.improvement)
        )
    };
    let metadata = serde_json::to_string(report)
        .unwrap_or_else(|_| "{}".into())
        .replace("</", "<\\/");

    format!(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="generator" content="Kestrel Local 0.5"><title>{title} · Kestrel Research</title>
<style>
:root{{--ink:#282923;--muted:#6f6d64;--paper:#fffdf8;--canvas:#f4f0e8;--line:#ded9ce;--green:#2e5d4c;--wash:#e4ede6;--serif:Georgia,'Times New Roman',serif}}*{{box-sizing:border-box}}html{{scroll-behavior:smooth}}body{{margin:0;color:var(--ink);background:var(--canvas);font-family:Inter,Segoe UI,sans-serif}}.top{{height:55px;position:sticky;top:0;z-index:3;display:flex;align-items:center;justify-content:space-between;padding:0 24px;border-bottom:1px solid var(--line);background:rgba(255,253,248,.94);backdrop-filter:blur(12px)}}.brand{{display:flex;align-items:center;gap:9px;font-weight:750}}.mark{{width:30px;height:30px;display:grid;place-items:center;border-radius:9px;color:#fff;background:var(--green)}}.meta{{color:var(--muted);font-size:10px}}.layout{{display:grid;grid-template-columns:minmax(0,780px) 180px;gap:60px;justify-content:center;padding:64px 34px 100px}}.kicker,header small{{color:#477b67;font:800 10px sans-serif;letter-spacing:.1em;text-transform:uppercase}}h1{{max-width:720px;margin:14px 0;font:500 clamp(44px,7vw,68px)/1 var(--serif);letter-spacing:-.045em}}.dek{{max-width:680px;color:#615f57;font:20px/1.5 var(--serif)}}.byline{{display:flex;gap:18px;margin-top:24px;color:#79766d;font-size:10px}}.answer{{margin:32px 0 16px;padding:24px 28px;border-left:4px solid var(--green);border-radius:3px 12px 12px 3px;background:linear-gradient(135deg,#e5eee7,#eef2ea)}}.answer small{{font-weight:800;color:var(--green);text-transform:uppercase;letter-spacing:.1em}}.answer p{{margin:10px 0 0;font:500 19px/1.55 var(--serif)}}.improvement{{margin:16px 0 70px;padding:14px 17px;border:1px solid #dfd4c2;border-radius:9px;background:#f3e8d8;font-size:11px}}.improvement p{{margin:4px 0 0;color:#6f6559}}section{{scroll-margin-top:75px;margin-bottom:75px}}section>header{{display:flex;gap:16px;margin-bottom:24px}}section>header>span{{color:#aaa69c;font:italic 12px var(--serif)}}h2{{margin:3px 0 0;font:500 32px/1.15 var(--serif);letter-spacing:-.025em}}.findings{{display:grid;grid-template-columns:repeat(3,1fr);gap:11px}}.finding{{padding:17px;border:1px solid var(--line);border-radius:10px;background:rgba(255,253,248,.58)}}.finding>span{{color:var(--green);font:italic 11px var(--serif)}}h3{{margin:18px 0 7px;font:600 16px/1.25 var(--serif)}}.finding p{{color:#66645c;font-size:12px;line-height:1.55}}.lead{{padding-left:43px;color:var(--green);font:italic 18px/1.5 var(--serif)}}section>p:not(.lead){{max-width:710px;font:17px/1.78 var(--serif)}}.cites{{display:flex;gap:5px;margin-top:12px}}.cites a{{padding:4px 7px;border:1px solid #c8d5cc;border-radius:5px;color:var(--green);background:#edf2ed;font:800 9px sans-serif;text-decoration:none}}.moment{{display:grid;grid-template-columns:100px 12px 1fr;gap:14px;padding-bottom:24px}}.moment time{{color:var(--green);font:italic 13px var(--serif);text-align:right}}.moment i{{width:8px;height:8px;margin-top:4px;border:2px solid var(--green);border-radius:50%}}.moment h3{{margin:0}}.moment p{{margin:4px 0;color:var(--muted);font-size:12px}}.split{{display:grid;grid-template-columns:1fr 1fr;gap:45px;padding-block:30px;border-block:1px solid var(--line)}}dl>div{{margin-bottom:15px}}dt{{font:600 14px var(--serif)}}dd{{margin:3px 0;color:var(--muted);font-size:11px;line-height:1.5}}ol{{padding-left:20px}}li{{margin-bottom:10px;font:13px/1.45 var(--serif)}}.intro{{color:var(--muted);font:12px/1.6 sans-serif}}.source{{display:grid;grid-template-columns:36px 1fr;gap:11px;margin:9px 0;padding:15px;border:1px solid var(--line);border-radius:9px;background:rgba(255,253,248,.55)}}.source>b{{height:23px;display:grid;place-items:center;border-radius:5px;color:var(--green);background:var(--wash);font-size:9px}}.source h3{{margin:0}}.source small{{color:#89867d;font-size:9px}}blockquote{{margin:9px 0 0;padding-left:11px;border-left:2px solid #d8d3ca;color:#6e6b63;font:italic 11px/1.55 var(--serif)}}footer{{padding-top:20px;border-top:1px solid var(--line);color:#77746b;font-size:10px}}nav{{position:sticky;top:85px;display:flex;flex-direction:column;gap:8px;padding-left:15px;border-left:1px solid var(--line)}}nav b{{margin-bottom:4px;color:#477b67;font-size:9px;text-transform:uppercase;letter-spacing:.08em}}nav a{{color:#77746d;font-size:10px;text-decoration:none}}@media(max-width:930px){{.layout{{grid-template-columns:minmax(0,740px)}}nav{{display:none}}}}@media(max-width:620px){{.layout{{padding:40px 20px}}.findings,.split{{grid-template-columns:1fr}}h1{{font-size:42px}}.byline{{flex-wrap:wrap}}}}@media print{{.top,nav{{display:none}}.layout{{display:block;padding:0}}body{{background:#fff}}section{{break-inside:avoid}}}}
</style></head><body>
<div class="top"><div class="brand"><span class="mark">⌁</span>Kestrel Research</div><div class="meta">Offline only · {model} · {snapshot}</div></div>
<div class="layout"><main><div class="kicker">Research brief · Edition {edition} · {date}</div><h1>{title}</h1><p class="dek">{dek}</p><div class="byline"><span>{minutes} min read</span><span>{source_count} inspected sources</span><span>{words} words</span></div>
<div class="answer"><small>Short answer</small><p>{answer}</p></div>{improvement}
<section id="findings"><header><span>01</span><div><small>The evidence at a glance</small><h2>Key findings</h2></div></header><div class="findings">{findings}</div></section>
{sections}{timeline}<section class="split" id="terms"><div><header><small>Plain language</small><h2>Terms worth knowing</h2></header><dl>{terms}</dl></div><div><header><small>Research frontier</small><h2>What remains open</h2></header><ol>{questions}</ol></div></section>
<section id="sources"><header><span>↳</span><div><small>Evidence ledger</small><h2>Sources inspected</h2></div></header><p class="intro">Every source below was opened by the local model. Excerpts show the evidence it received; Wikipedia is a tertiary starting point, not a substitute for primary sources.</p>{sources}</section>
<footer>Produced entirely on this computer with {model} and {snapshot}. This edition is immutable; later work will be linked, not overwritten.</footer></main>
<nav><b>On this page</b><a href="#findings">Key findings</a>{source_links}<a href="#terms">Terms & questions</a><a href="#sources">Evidence ledger</a></nav></div>
<script type="application/json" id="kestrel-report">{metadata}</script></body></html>"##,
        title = esc(&report.title),
        dek = esc(&report.dek),
        model = esc(&report.model),
        snapshot = esc(&report.archive_snapshot),
        edition = report.edition,
        date = esc(&report.updated_at.chars().take(10).collect::<String>()),
        minutes = report.reading_minutes,
        source_count = report.sources.len(),
        words = report.word_count,
        answer = esc(&report.answer),
        improvement = improvement,
        findings = findings,
        sections = sections,
        timeline = timeline,
        terms = terms,
        questions = questions,
        sources = sources,
        source_links = source_links,
        metadata = metadata,
    )
}

fn citations(ids: &[String]) -> String {
    if ids.is_empty() {
        return String::new();
    }
    format!(
        "<div class=\"cites\">{}</div>",
        ids.iter()
            .map(|id| format!("<a href=\"#source-{}\">{}</a>", esc(id), esc(id)))
            .collect::<String>()
    )
}

fn esc(value: &str) -> String {
    encode_text(value).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    #[test]
    fn standalone_html_is_self_contained_and_escapes_content() {
        let report = ResearchReport {
            id: "one".into(),
            title: "A <B>".into(),
            dek: "Safe".into(),
            query: "Q".into(),
            answer: "x & y".into(),
            created_at: "2026-08-01T00:00:00Z".into(),
            updated_at: "2026-08-01T00:00:00Z".into(),
            edition: 1,
            parent_id: None,
            improvement: "Baseline".into(),
            model: "Bonsai".into(),
            archive_snapshot: "2024-01-12".into(),
            findings: vec![],
            sections: vec![],
            timeline: vec![],
            terms: vec![],
            open_questions: vec![],
            sources: vec![],
            html_path: String::new(),
            word_count: 2,
            reading_minutes: 1,
        };
        let html = render_report_html(&report);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("A &lt;B&gt;"));
        assert!(html.contains("x &amp; y"));
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
    }
}
