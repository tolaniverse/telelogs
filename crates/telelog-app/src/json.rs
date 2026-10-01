//! JSON rendering of log records, as in the design: `timestamp`, `level`, `source`,
//! `origin`, `stream`, `body` (inlined when it is itself JSON) and `labels`.

use chrono::{DateTime, SecondsFormat, Utc};
use gpui_kit::*;
use serde_json::{Map, Value};
use telelog_core::LogRecord;

use crate::theme::Tokens;

/// Indent per nesting level, in pixels.
pub const INDENT: f32 = 16.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Key,
    Punct,
    Str,
    Num,
    Level,
}

/// One rendered line: its nesting depth and colored spans.
#[derive(Debug, Default)]
pub struct Line {
    pub depth: usize,
    text: String,
    parts: Vec<(std::ops::Range<usize>, Part)>,
}

impl Line {
    fn push(&mut self, s: &str, part: Part) {
        let start = self.text.len();
        self.text.push_str(s);
        self.parts.push((start..self.text.len(), part));
    }

    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn styled(&self, t: Tokens, level_color: Hsla) -> StyledText {
        let highlights = self.parts.iter().map(|(range, part)| {
            let color = match part {
                Part::Key => t.fg2,
                Part::Punct => t.fg3,
                Part::Str => t.fg,
                Part::Num => t.fg2,
                Part::Level => level_color,
            };
            (range.clone(), HighlightStyle { color: Some(color), ..Default::default() })
        });
        StyledText::new(self.text.clone()).with_highlights(highlights)
    }
}

pub fn record_value(record: &LogRecord) -> Value {
    let time: DateTime<Utc> = record.timestamp.into();
    let mut obj = Map::new();
    obj.insert("timestamp".into(), time.to_rfc3339_opts(SecondsFormat::Millis, true).into());
    obj.insert("level".into(), record.level.as_str().into());
    obj.insert("source".into(), record.source.as_str().into());
    obj.insert("origin".into(), record.origin.clone().into());
    obj.insert("stream".into(), record.stream.as_str().into());
    let body = match serde_json::from_str::<Value>(record.body.trim()) {
        Ok(parsed @ Value::Object(_)) => parsed,
        _ => record.body.clone().into(),
    };
    obj.insert("body".into(), body);
    let labels = record.labels.iter().map(|(k, v)| (k.clone(), Value::from(v.clone())));
    obj.insert("labels".into(), Value::Object(labels.collect()));
    Value::Object(obj)
}

pub fn pretty(record: &LogRecord) -> String {
    serde_json::to_string_pretty(&record_value(record)).unwrap_or_default()
}

pub fn lines(record: &LogRecord) -> Vec<Line> {
    let mut out = Vec::new();
    write(&record_value(record), None, 0, true, &mut out);
    out
}

fn open_line(depth: usize, key: Option<&str>) -> Line {
    let mut line = Line { depth, ..Default::default() };
    if let Some(key) = key {
        line.push(&format!("{key:?}"), Part::Key);
        line.push(": ", Part::Punct);
    }
    line
}

fn write(value: &Value, key: Option<&str>, depth: usize, last: bool, out: &mut Vec<Line>) {
    let tail = if last { "" } else { "," };
    let mut line = open_line(depth, key);
    match value {
        Value::Object(map) if !map.is_empty() => {
            line.push("{", Part::Punct);
            out.push(line);
            for (i, (k, v)) in map.iter().enumerate() {
                write(v, Some(k), depth + 1, i + 1 == map.len(), out);
            }
            let mut close = open_line(depth, None);
            close.push("}", Part::Punct);
            close.push(tail, Part::Punct);
            out.push(close);
        }
        Value::Array(items) if !items.is_empty() => {
            line.push("[", Part::Punct);
            out.push(line);
            for (i, v) in items.iter().enumerate() {
                write(v, None, depth + 1, i + 1 == items.len(), out);
            }
            let mut close = open_line(depth, None);
            close.push("]", Part::Punct);
            close.push(tail, Part::Punct);
            out.push(close);
        }
        scalar => {
            let (text, part) = match scalar {
                Value::String(s) => (format!("{s:?}"), if key == Some("level") && depth == 1 { Part::Level } else { Part::Str }),
                Value::Object(_) => ("{}".to_string(), Part::Punct),
                Value::Array(_) => ("[]".to_string(), Part::Punct),
                other => (other.to_string(), Part::Num),
            };
            line.push(&text, part);
            line.push(tail, Part::Punct);
            out.push(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{lines, record_value};
    use serde_json::{Value, json};
    use std::time::{Duration, SystemTime};
    use telelog_core::{Level, LogRecord, SourceKind, Stream};

    fn record(body: &str) -> LogRecord {
        LogRecord {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_millis(1_500),
            source: SourceKind::Docker,
            origin: "api".into(),
            stream: Stream::Stdout,
            level: Level::Info,
            body: body.into(),
            labels: [("image".to_string(), "alpine".to_string())].into(),
        }
    }

    #[test]
    fn inlines_json_bodies() {
        let v = record_value(&record(r#"{"msg":"hi","n":2}"#));
        assert_eq!(v["body"], json!({"msg": "hi", "n": 2}));
        assert_eq!(v["timestamp"], "1970-01-01T00:00:01.500Z");
        assert_eq!(v["source"], "docker");
        assert_eq!(record_value(&record("plain"))["body"], "plain");
    }

    #[test]
    fn lines_reassemble_to_the_same_json() {
        let r = record(r#"{"msg":"hi","tags":["a","b"],"empty":{}}"#);
        let text: Vec<String> =
            lines(&r).iter().map(|l| format!("{}{}", "  ".repeat(l.depth), l.text())).collect();
        assert_eq!(serde_json::from_str::<Value>(&text.join("\n")).unwrap(), record_value(&r));
        assert_eq!(text[0], "{");
        assert_eq!(text.last().unwrap(), "}");
    }
}
