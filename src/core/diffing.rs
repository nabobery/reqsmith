use similar::TextDiff;

use super::models::{DiffArtifact, DiffLine, HeaderDiff, ResponseArtifact};
use super::redaction;

/// Compute a diff between two response artifacts.
pub fn diff_responses(
    baseline: &ResponseArtifact,
    candidate: &ResponseArtifact,
    baseline_label: &str,
    candidate_label: &str,
) -> DiffArtifact {
    let status_diff = if baseline.status_code != candidate.status_code {
        Some((baseline.status_code, candidate.status_code))
    } else {
        None
    };

    let header_diffs = diff_headers(&baseline.headers, &candidate.headers);
    let body_diff = diff_bodies(
        baseline.body_text.as_deref(),
        candidate.body_text.as_deref(),
    );

    DiffArtifact {
        baseline_label: baseline_label.into(),
        candidate_label: candidate_label.into(),
        status_diff,
        header_diffs,
        body_diff,
    }
}

/// Diff two header sets. Change detection (`Changed`/`Added`/`Removed` vs. no
/// diff) is computed against the REAL values, so a rotated secret is still
/// reported as changed. The values actually stored in the returned
/// `HeaderDiff`s are redacted for sensitive header names via
/// [`redaction::redact_headers`] — so a secret rotation surfaces as
/// `<redacted> -> <redacted>` rather than either leaking the old/new value or
/// silently disappearing from the diff.
fn diff_headers(baseline: &[(String, String)], candidate: &[(String, String)]) -> Vec<HeaderDiff> {
    // Resolve the redaction policy once for the whole header set rather than
    // re-reading `REQSMITH_REDACT_HEADERS` for every header comparison.
    let policy = redaction::RedactionPolicy::from_env();
    let mut diffs = Vec::new();

    for (key, old_val) in baseline {
        match candidate.iter().find(|(k, _)| k == key) {
            Some((_, new_val)) if new_val != old_val => {
                diffs.push(HeaderDiff::Changed {
                    key: key.clone(),
                    old: policy.redact_value_for(key, old_val),
                    new: policy.redact_value_for(key, new_val),
                });
            }
            None => {
                diffs.push(HeaderDiff::Removed(
                    key.clone(),
                    policy.redact_value_for(key, old_val),
                ));
            }
            _ => {}
        }
    }

    for (key, val) in candidate {
        if !baseline.iter().any(|(k, _)| k == key) {
            diffs.push(HeaderDiff::Added(
                key.clone(),
                policy.redact_value_for(key, val),
            ));
        }
    }

    diffs
}

fn diff_bodies(baseline: Option<&str>, candidate: Option<&str>) -> Vec<DiffLine> {
    let base = normalize_for_diff(baseline.unwrap_or(""));
    let cand = normalize_for_diff(candidate.unwrap_or(""));

    let text_diff = TextDiff::from_lines(&base, &cand);
    text_diff
        .iter_all_changes()
        .map(|change| {
            let value = change.value().to_string();
            match change.tag() {
                similar::ChangeTag::Equal => DiffLine::Equal(value),
                similar::ChangeTag::Insert => DiffLine::Insert(value),
                similar::ChangeTag::Delete => DiffLine::Delete(value),
            }
        })
        .collect()
}

/// If the text is valid JSON, normalize by sorting keys and pretty-printing.
fn normalize_for_diff(text: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => {
            let sorted = sort_json_keys(&value);
            serde_json::to_string_pretty(&sorted).unwrap_or_else(|_| text.to_string())
        }
        Err(_) => text.to_string(),
    }
}

fn sort_json_keys(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                sorted.insert(key.clone(), sort_json_keys(&map[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(sort_json_keys).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(status: u16, headers: Vec<(&str, &str)>, body: Option<&str>) -> ResponseArtifact {
        ResponseArtifact {
            status_code: status,
            http_version: "HTTP/1.1".into(),
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            content_type: None,
            content_length: None,
            duration_ms: 100,
            body_text: body.map(String::from),
            body_bytes: None,
            is_binary: false,
        }
    }

    #[test]
    fn identical_responses_produce_empty_diff() {
        let a = artifact(200, vec![("x", "1")], Some(r#"{"a":1}"#));
        let diff = diff_responses(&a, &a, "a", "b");
        assert!(diff.status_diff.is_none());
        assert!(diff.header_diffs.is_empty());
        assert!(
            diff.body_diff
                .iter()
                .all(|l| matches!(l, DiffLine::Equal(_)))
        );
    }

    #[test]
    fn different_status_detected() {
        let a = artifact(200, vec![], None);
        let b = artifact(404, vec![], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert_eq!(diff.status_diff, Some((200, 404)));
    }

    #[test]
    fn header_added() {
        let a = artifact(200, vec![], None);
        let b = artifact(200, vec![("x-new", "val")], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert!(matches!(&diff.header_diffs[0], HeaderDiff::Added(k, _) if k == "x-new"));
    }

    #[test]
    fn header_removed() {
        let a = artifact(200, vec![("x-old", "val")], None);
        let b = artifact(200, vec![], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert!(matches!(&diff.header_diffs[0], HeaderDiff::Removed(k, _) if k == "x-old"));
    }

    #[test]
    fn header_changed() {
        let a = artifact(200, vec![("ct", "text")], None);
        let b = artifact(200, vec![("ct", "json")], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert!(
            matches!(&diff.header_diffs[0], HeaderDiff::Changed { key, old, new } if key == "ct" && old == "text" && new == "json")
        );
    }

    #[test]
    fn json_body_diff_with_reordered_keys() {
        let a = artifact(200, vec![], Some(r#"{"b":2,"a":1}"#));
        let b = artifact(200, vec![], Some(r#"{"a":1,"b":2}"#));
        let diff = diff_responses(&a, &b, "a", "b");
        // After normalization (keys sorted), these should be identical
        assert!(
            diff.body_diff
                .iter()
                .all(|l| matches!(l, DiffLine::Equal(_)))
        );
    }

    #[test]
    fn json_body_diff_detects_changes() {
        let a = artifact(200, vec![], Some(r#"{"count":5}"#));
        let b = artifact(200, vec![], Some(r#"{"count":6}"#));
        let diff = diff_responses(&a, &b, "a", "b");
        let has_insert = diff
            .body_diff
            .iter()
            .any(|l| matches!(l, DiffLine::Insert(_)));
        let has_delete = diff
            .body_diff
            .iter()
            .any(|l| matches!(l, DiffLine::Delete(_)));
        assert!(has_insert && has_delete);
    }

    #[test]
    fn plain_text_diff() {
        let a = artifact(200, vec![], Some("line1\nline2\n"));
        let b = artifact(200, vec![], Some("line1\nline3\n"));
        let diff = diff_responses(&a, &b, "a", "b");
        let has_changes = diff
            .body_diff
            .iter()
            .any(|l| matches!(l, DiffLine::Insert(_) | DiffLine::Delete(_)));
        assert!(has_changes);
    }

    #[test]
    fn sort_json_keys_nested() {
        let input: serde_json::Value =
            serde_json::from_str(r#"{"z":{"b":2,"a":1},"a":1}"#).unwrap();
        let sorted = sort_json_keys(&input);
        let keys: Vec<&String> = sorted.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["a", "z"]);
        let nested_keys: Vec<&String> = sorted["z"].as_object().unwrap().keys().collect();
        assert_eq!(nested_keys, vec!["a", "b"]);
    }

    #[test]
    fn sort_json_keys_preserves_array_order() {
        let input: serde_json::Value = serde_json::from_str(r#"[3,1,2]"#).unwrap();
        let sorted = sort_json_keys(&input);
        assert_eq!(sorted, serde_json::json!([3, 1, 2]));
    }

    // --- B1: header diff redaction ---

    #[test]
    fn header_diff_redacts_changed_sensitive_value() {
        let a = artifact(200, vec![("authorization", "Bearer old-token")], None);
        let b = artifact(200, vec![("authorization", "Bearer new-token")], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert!(matches!(
            &diff.header_diffs[0],
            HeaderDiff::Changed { key, old, new }
                if key == "authorization" && old == "<redacted>" && new == "<redacted>"
        ));
    }

    #[test]
    fn header_diff_does_not_flag_unchanged_sensitive_header() {
        let a = artifact(200, vec![("authorization", "Bearer same-token")], None);
        let b = artifact(200, vec![("authorization", "Bearer same-token")], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert!(diff.header_diffs.is_empty());
    }

    #[test]
    fn header_diff_redacts_removed_and_added_sensitive_values() {
        let a = artifact(200, vec![("set-cookie", "session=abc")], None);
        let b = artifact(200, vec![("set-cookie", "session=xyz")], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert!(matches!(
            &diff.header_diffs[0],
            HeaderDiff::Changed { key, old, new }
                if key == "set-cookie" && old == "<redacted>" && new == "<redacted>"
        ));

        let c = artifact(200, vec![], None);
        let removed_diff = diff_responses(&a, &c, "a", "c");
        assert!(matches!(
            &removed_diff.header_diffs[0],
            HeaderDiff::Removed(k, v) if k == "set-cookie" && v == "<redacted>"
        ));

        let added_diff = diff_responses(&c, &a, "c", "a");
        assert!(matches!(
            &added_diff.header_diffs[0],
            HeaderDiff::Added(k, v) if k == "set-cookie" && v == "<redacted>"
        ));
    }

    #[test]
    fn header_diff_does_not_redact_non_sensitive_headers() {
        let a = artifact(200, vec![("content-type", "text/plain")], None);
        let b = artifact(200, vec![("content-type", "application/json")], None);
        let diff = diff_responses(&a, &b, "a", "b");
        assert!(matches!(
            &diff.header_diffs[0],
            HeaderDiff::Changed { key, old, new }
                if key == "content-type" && old == "text/plain" && new == "application/json"
        ));
    }

    #[test]
    fn one_body_null() {
        let a = artifact(200, vec![], Some("hello"));
        let b = artifact(200, vec![], None);
        let diff = diff_responses(&a, &b, "a", "b");
        let has_delete = diff
            .body_diff
            .iter()
            .any(|l| matches!(l, DiffLine::Delete(_)));
        assert!(has_delete);
    }
}
