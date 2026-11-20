//! `endmark-core` behind one function, for JavaScript.
//!
//! The module exports no JavaScript glue and needs no tool to bind it. A
//! caller writes a JSON request into memory and gets a JSON response back:
//!
//! 1. `endmark_alloc(len)` returns a pointer; write the UTF-8 request there.
//! 2. `endmark_call(ptr, len)` returns a pointer to the response: a 4-byte
//!    little-endian length, then that many bytes of UTF-8 JSON. The request
//!    buffer is freed by the call.
//! 3. `endmark_free(ptr, 4 + length)` frees the response.
//!
//! `packages/render/src/core.ts` wraps this. The requests are listed at
//! [`call`], which is also usable directly from Rust.

use endmark_core::write::{self, Stamp, WriteError, Written};
use endmark_core::{json, parse, Edit, Quote, Verb};
use serde_json::{json, Value};

/// Answers one request.
///
/// Every request has `op` and `source`, the text of the file.
///
/// - `status`: returns `document`, as `endmark status --format json` prints it.
/// - `preview`, with `id`: returns `body`, the document as it would read with
///   that suggestion accepted. Nothing is written.
/// - `comment`, `reply`, `suggest`, `edit`, `accept`, `reject`, `retract`,
///   `resolve`, `reopen`, `anchor`, `revert`: a write. Takes `by` and `at`,
///   and where they apply `for`, `note`, `id` (the item acted on), `new_id`
///   (the id for a new item), `on` and `in` (a quote), and `edits` (a list
///   of `{in, replace, with}`). Returns the new `source`, its `document`,
///   and what the write did: `id`, `target`, `reanchored`, `detached`, `stale`.
///
/// A response is `{"ok": true, ...}` or `{"ok": false, "code", "message", "hint"}`.
pub fn call(request: &Value) -> Value {
    match answer(request) {
        Ok(value) => value,
        Err((code, message, hint)) => json!({"ok": false, "code": code, "message": message, "hint": hint}),
    }
}

type Failure = (String, String, String);

fn bad_request(message: impl Into<String>) -> Failure {
    ("bad-request".into(), message.into(), String::new())
}

fn refused(e: WriteError) -> Failure {
    (e.code().to_string(), e.to_string(), e.hint())
}

fn answer(request: &Value) -> Result<Value, Failure> {
    let text = |key: &str| request[key].as_str();
    let need = |key: &str| text(key).ok_or_else(|| bad_request(format!("the request has no \"{key}\"")));
    let op = need("op")?;
    let source = need("source")?;

    if op == "status" {
        return Ok(json!({"ok": true, "document": json::document("", &parse(source))}));
    }

    let stamp = Stamp {
        by: text("by").unwrap_or("preview"),
        at: text("at").unwrap_or("1970-01-01T00:00:00Z"),
        on_behalf_of: text("for"),
        note: text("note").unwrap_or(""),
    };
    if op == "preview" {
        let applied = write::accept(source, need("id")?, &stamp).map_err(refused)?;
        return Ok(json!({"ok": true, "body": parse(&applied.source).body}));
    }
    need("by")?;
    need("at")?;

    let quote = text("on").map(|on| Quote {
        scope: text("in").map(str::to_string),
        on: on.to_string(),
    });
    let edits = || -> Result<Vec<Edit>, Failure> {
        let list = request["edits"]
            .as_array()
            .ok_or_else(|| bad_request("the request has no \"edits\""))?;
        list.iter()
            .map(|e| {
                Ok(Edit {
                    scope: e["in"].as_str().map(str::to_string),
                    replace: e["replace"]
                        .as_str()
                        .ok_or_else(|| bad_request("an edit has no \"replace\""))?
                        .to_string(),
                    with: e["with"]
                        .as_str()
                        .ok_or_else(|| bad_request("an edit has no \"with\""))?
                        .to_string(),
                })
            })
            .collect()
    };

    let written: Result<Written, WriteError> = match op {
        "comment" => write::comment(source, need("new_id")?, &stamp, quote.as_ref(), None),
        "reply" => write::comment(source, need("new_id")?, &stamp, quote.as_ref(), Some(need("id")?)),
        "suggest" => write::suggest(source, need("new_id")?, &stamp, &edits()?),
        "edit" => write::edit(source, need("new_id")?, &stamp, &edits()?),
        "accept" => write::accept(source, need("id")?, &stamp),
        "anchor" => {
            let quote = quote.ok_or_else(|| bad_request("the request has no \"on\""))?;
            write::anchor(source, need("id")?, &stamp, &quote)
        }
        "revert" => write::revert(source, need("id")?, need("new_id")?, &stamp),
        "reject" | "retract" | "resolve" | "reopen" => {
            let verb = Verb::from_name(op).expect("a verb");
            write::act(source, verb, need("id")?, &stamp)
        }
        other => return Err(bad_request(format!("unknown op \"{other}\""))),
    };
    let written = written.map_err(refused)?;
    Ok(json!({
        "ok": true,
        "document": json::document("", &parse(&written.source)),
        "source": written.source,
        "id": written.id,
        "target": written.target,
        "reanchored": written.reanchored,
        "detached": written.detached,
        "stale": written.stale,
    }))
}

/// Allocates `len` bytes for a request.
#[no_mangle]
pub extern "C" fn endmark_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Frees memory returned by [`endmark_alloc`] or [`endmark_call`].
///
/// # Safety
/// `ptr` and `len` must be exactly what one of those functions handed out,
/// and must not be freed twice.
#[no_mangle]
pub unsafe extern "C" fn endmark_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// Answers the JSON request at `ptr`. See the module documentation.
///
/// # Safety
/// `ptr` must come from `endmark_alloc(len)` and hold `len` initialised bytes.
/// The call takes ownership of that buffer.
#[no_mangle]
pub unsafe extern "C" fn endmark_call(ptr: *mut u8, len: usize) -> *mut u8 {
    let request = Vec::from_raw_parts(ptr, len, len.max(1));
    let response = match serde_json::from_slice::<Value>(&request) {
        Ok(value) => call(&value),
        Err(e) => json!({"ok": false, "code": "bad-request", "message": e.to_string(), "hint": ""}),
    };
    let body = response.to_string().into_bytes();
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    // Capacity equals length, so `endmark_free(ptr, 4 + length)` is exact.
    let mut out = out.into_boxed_slice();
    let ptr = out.as_mut_ptr();
    std::mem::forget(out);
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_then_a_read() {
        let source = "Relay listens on port 8080.\n";
        let done = call(&json!({
            "op": "suggest", "source": source, "by": "you", "at": "2026-11-20T10:00:00Z", "new_id": "s-1",
            "edits": [{"replace": "port 8080", "with": "port 8443"}],
        }));
        assert_eq!(done["ok"], true, "{done}");
        assert_eq!(done["document"]["items"][0]["edits"][0]["range"], json!([17, 26]));

        let preview = call(&json!({"op": "preview", "source": done["source"], "id": "s-1"}));
        assert_eq!(
            preview["body"].as_str().unwrap().trim_end(),
            "Relay listens on port 8443."
        );

        let refused = call(&json!({
            "op": "comment", "source": source, "by": "you", "at": "2026-11-20T10:00:00Z", "new_id": "c-1", "on": "nope",
        }));
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["code"], "quote-not-found");
    }

    #[test]
    fn the_memory_interface_round_trips() {
        let request = json!({"op": "status", "source": "Hello.\n"}).to_string();
        unsafe {
            let ptr = endmark_alloc(request.len());
            std::ptr::copy_nonoverlapping(request.as_ptr(), ptr, request.len());
            let out = endmark_call(ptr, request.len());
            let len = u32::from_le_bytes(*(out as *const [u8; 4])) as usize;
            let response: Value = serde_json::from_slice(std::slice::from_raw_parts(out.add(4), len)).unwrap();
            endmark_free(out, 4 + len);
            assert_eq!(response["document"]["body"], "Hello.\n");
        }
    }
}
