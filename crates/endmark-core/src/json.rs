//! The JSON form of a parsed document: what `endmark status --format json`
//! prints, and what the conformance fixtures are compared with
//! (`conformance/README.md`).

use serde_json::{json, Map, Value};

use crate::{Anchor, Applicability, Detail, Diagnostic, Document, Edit, Item, Landing};

/// One finding.
pub fn finding(d: &Diagnostic) -> Value {
    json!({
        "line": d.line,
        "severity": d.severity().as_str(),
        "code": d.code.as_str(),
        "id": d.id,
        "message": d.message,
        "hint": d.hint(),
    })
}

/// The whole document.
pub fn document(file: &str, doc: &Document) -> Value {
    json!({
        "file": file,
        "body": doc.body,
        "items": doc.items.iter().map(item).collect::<Vec<_>>(),
        "diagnostics": doc.diagnostics.iter().map(finding).collect::<Vec<_>>(),
        "errors": doc.errors(),
        "warnings": doc.warnings(),
    })
}

fn edits(edits: &[Edit], ranges: &[Option<(usize, usize)>]) -> Value {
    edits
        .iter()
        .enumerate()
        .map(|(n, e)| {
            let range = ranges.get(n).copied().flatten().map(|(start, end)| [start, end]);
            json!({"in": e.scope, "replace": e.replace, "with": e.with, "range": range})
        })
        .collect()
}

/// One item. Keys that do not apply to an item are left out, not set to null:
/// `thread` appears on thread roots only, `root` on replies only, and `status`,
/// `reason` and `reverted_by` only when there is one.
pub fn item(item: &Item) -> Value {
    let mut m = Map::new();
    let mut put = |key: &str, value: Value| {
        m.insert(key.to_string(), value);
    };
    put("id", json!(item.id));
    put("kind", json!(item.kind()));
    put("line", json!(item.line));
    put("by", json!(item.by));
    put("at", json!(item.at));
    put("for", json!(item.on_behalf_of));
    put("re", json!(item.re));
    put("note", json!(item.note));
    match &item.detail {
        Detail::Comment(c) => {
            put("state", json!(if c.retracted { "retracted" } else { "live" }));
            match c.thread {
                Some(thread) => put("thread", json!(thread.as_str())),
                None => put("root", json!(c.root)),
            }
            put("anchor", json!(c.anchor.as_str()));
            if let Anchor::Attached { start, end } = c.anchor {
                put("range", json!([start, end]));
            }
            put(
                "quote",
                match &c.quote {
                    Some(q) => json!({"in": q.scope, "on": q.on}),
                    None => Value::Null,
                },
            );
        }
        Detail::Suggestion(s) => {
            put("state", json!(s.state.as_str()));
            match (s.applicability, s.landing) {
                (Some(Applicability::Applicable), _) => put("status", json!("applicable")),
                (Some(Applicability::Stale(reason)), _) => {
                    put("status", json!("stale"));
                    put("reason", json!(reason.as_str()));
                }
                (None, Some(landing)) => put("status", json!(landing.as_str())),
                (None, None) if !s.in_effect => put("status", json!("reverted")),
                (None, None) => {}
            }
            if let Some(by) = &s.reverted_by {
                put("reverted_by", json!(by));
            }
            put("edits", edits(&s.edits, &s.ranges));
        }
        Detail::Edit(e) | Detail::Revert(e) => {
            put("state", json!("applied"));
            put("status", json!(e.landing.map_or("reverted", Landing::as_str)));
            if let Some(by) = &e.reverted_by {
                put("reverted_by", json!(by));
            }
            put("edits", edits(&e.edits, &e.ranges));
        }
    }
    put(
        "events",
        item.events
            .iter()
            .map(|e| {
                json!({
                    "verb": e.verb.as_str(),
                    "line": e.line,
                    "by": e.by,
                    "at": e.at,
                    "for": e.on_behalf_of,
                    "note": e.note,
                })
            })
            .collect(),
    );
    Value::Object(m)
}
