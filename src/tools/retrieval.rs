//! Structured, bounded evidence selection for chain results.
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Search the final chain result before truncation. Empty matches return `[]`.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RetrievalOptions {
    /// Independent lexical queries (up to 32); scores are summed across queries.
    pub queries: Vec<String>,
    /// Maximum ranked matching chunks across all queries (default 8, capped at 100).
    pub top_k: usize,
    /// Adjacent chunks on each side, added after hits (default 0, capped at 5).
    pub neighbors: usize,
    /// Total serialized JSON byte budget, including paths and scores (default 16000).
    /// Must be at least 2 to represent an empty result array.
    pub max_bytes: usize,
}

impl Default for RetrievalOptions {
    fn default() -> Self {
        Self {
            queries: Vec::new(),
            top_k: 8,
            neighbors: 0,
            max_bytes: 16_000,
        }
    }
}

#[derive(Debug)]
struct Chunk {
    source: String,
    text: String,
}

/// Split long text on paragraphs, lines, then whitespace, with a UTF-8 safe fallback.
fn push_text(chunks: &mut Vec<Chunk>, source: &str, text: &str) {
    for paragraph in text.split("\n\n").filter(|p| !p.trim().is_empty()) {
        let mut rest = paragraph.trim();
        while !rest.is_empty() {
            let mut end = rest.floor_char_boundary(rest.len().min(1024));
            if end < rest.len() {
                let prefix = &rest[..end];
                if let Some(boundary) = prefix
                    .rfind('\n')
                    .or_else(|| prefix.rfind(char::is_whitespace))
                    && boundary > 0
                {
                    end = boundary;
                }
            }
            chunks.push(Chunk {
                source: source.to_owned(),
                text: rest[..end].to_owned(),
            });
            rest = rest[end..].trim_start();
        }
    }
}

fn walk_json(value: &Value, path: &str, chunks: &mut Vec<Chunk>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, value) in map {
                walk_json(
                    value,
                    &format!("{path}[{}]", serde_json::to_string(key).unwrap()),
                    chunks,
                );
            }
        }
        Value::Array(values) if !values.is_empty() => {
            for (i, value) in values.iter().enumerate() {
                walk_json(value, &format!("{path}[{i}]"), chunks);
            }
        }
        Value::String(text) => push_text(chunks, path, text),
        _ => push_text(chunks, path, &value.to_string()),
    }
}

fn chunk_output(raw: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        walk_json(&value, "$", &mut chunks);
        return chunks;
    }
    let mut headings: Vec<(usize, String)> = Vec::new();
    let mut body = String::new();
    let mut source = "text".to_owned();
    let mut fence = None;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        let marker = if trimmed.starts_with("```") {
            Some('`')
        } else if trimmed.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        if let Some(marker) = marker {
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
        }
        let level = line.bytes().take_while(|c| *c == b'#').count();
        if fence.is_none() && (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ')
        {
            push_text(&mut chunks, &source, &body);
            body.clear();
            while headings
                .last()
                .is_some_and(|(previous, _)| *previous >= level)
            {
                headings.pop();
            }
            headings.push((level, line[level..].trim().to_owned()));
            source = headings
                .iter()
                .map(|(_, h)| h.as_str())
                .collect::<Vec<_>>()
                .join(" > ");
        }
        body.push_str(line);
        body.push('\n');
    }
    push_text(&mut chunks, &source, &body);
    chunks
}

#[derive(Serialize)]
struct Evidence<'a> {
    chunk: usize,
    source: &'a str,
    score: f64,
    context: bool,
    text: &'a str,
}

/// Rank structural chunks using summed per-query token coverage. Stable source order
/// breaks ties. Hits precede deduplicated neighbors; whole chunks only, never raw fallback.
/// The caller validates budgets >= 2; smaller internal budgets return no bytes.
pub fn retrieve(raw: &str, options: &RetrievalOptions, max_output: usize) -> String {
    let budget = options.max_bytes.min(max_output);
    if budget < 2 {
        return String::new();
    }
    let chunks = chunk_output(raw);
    let queries: Vec<HashSet<String>> = options
        .queries
        .iter()
        .take(32)
        .map(|q| crate::registry::tokenize(q).into_iter().collect())
        .filter(|q: &HashSet<String>| !q.is_empty())
        .collect();
    let mut scores = vec![0.0; chunks.len()];
    for (i, chunk) in chunks.iter().enumerate() {
        let terms: HashSet<_> =
            crate::registry::tokenize(&format!("{} {}", chunk.source, chunk.text))
                .into_iter()
                .collect();
        scores[i] = queries
            .iter()
            .map(|q| q.intersection(&terms).count() as f64 / q.len() as f64)
            .sum();
    }
    let mut ranked: Vec<usize> = (0..chunks.len()).filter(|i| scores[*i] > 0.0).collect();
    ranked.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]).then(a.cmp(b)));
    ranked.truncate(options.top_k.min(100));
    let mut selected: HashSet<_> = ranked.iter().copied().collect();
    let mut candidates: Vec<_> = ranked.iter().map(|i| (*i, false)).collect();
    let radius = options.neighbors.min(5);
    for i in ranked {
        for j in
            i.saturating_sub(radius)..=i.saturating_add(radius).min(chunks.len().saturating_sub(1))
        {
            if selected.insert(j) {
                candidates.push((j, true));
            }
        }
    }
    let mut results = Vec::new();
    let mut output = "[]".to_owned();
    for (i, context) in candidates {
        results.push(Evidence {
            chunk: i,
            source: &chunks[i].source,
            score: scores[i],
            context,
            text: &chunks[i].text,
        });
        let candidate = serde_json::to_string(&results).expect("evidence is serializable");
        if candidate.len() <= budget {
            output = candidate;
        } else {
            results.pop();
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_chunks_preserve_json_paths_and_markdown_headings() {
        let json = chunk_output(r#"{"users":[{"name":"Ada"},{"name":"Grace"}]}"#);
        assert_eq!(json.len(), 2);
        assert_eq!(json[1].source, "$[\"users\"][1][\"name\"]");
        assert!(json[1].text.contains("Grace"));
        let markdown = chunk_output("# Intro\nhello\n## Errors\nretry failed\n# End\ndone");
        assert_eq!(markdown.len(), 3);
        assert_eq!(markdown[1].source, "Intro > Errors");
    }

    #[test]
    fn text_boundaries_and_unicode_are_bounded() {
        let chunks = chunk_output(&format!("first paragraph\n\n{}", "🦀 word ".repeat(1000)));
        assert_eq!(chunks[0].text, "first paragraph");
        assert!(chunks.len() > 2);
        assert!(chunks.iter().all(|c| c.text.len() <= 1024));
    }

    #[test]
    fn multi_query_ranking_neighbors_and_total_budget() {
        let raw = "alpha alpha\n\nneighbor\n\nbeta\n\nunrelated";
        let options = RetrievalOptions {
            queries: vec!["alpha".into(), "beta".into()],
            top_k: 2,
            neighbors: 0,
            max_bytes: 1000,
        };
        let result: serde_json::Value =
            serde_json::from_str(&retrieve(raw, &options, 2000)).unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
        let options = RetrievalOptions {
            queries: vec!["alpha".into()],
            top_k: 1,
            neighbors: 1,
            ..options
        };
        assert!(retrieve(raw, &options, 2000).contains("neighbor"));
        for budget in 2..250 {
            let result = retrieve(raw, &options, budget);
            assert!(result.len() <= budget);
            assert!(
                serde_json::from_str::<serde_json::Value>(&result)
                    .unwrap()
                    .is_array()
            );
        }
        assert_eq!(
            retrieve(
                raw,
                &RetrievalOptions {
                    queries: vec!["absent".into()],
                    ..options.clone()
                },
                1000
            ),
            "[]"
        );
        assert_eq!(
            retrieve(
                raw,
                &RetrievalOptions {
                    queries: vec![],
                    ..options
                },
                1000
            ),
            "[]"
        );
    }
}
