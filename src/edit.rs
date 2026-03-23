/// A single search-and-replace edit parsed from an LLM response.
#[derive(Debug, PartialEq)]
pub struct Edit {
    pub search: String,
    pub replace: String,
}

/// Parse edit blocks from an LLM response.
///
/// Returns the edits and any non-edit text (summary, explanation).
/// If no edit blocks are found, returns an empty list and the full response.
/// If any edit block is malformed, returns no edits and the entire response
/// as non-edit text (degrades to plain text behavior).
pub fn parse_edits(response: &str) -> (Vec<Edit>, String) {
    let blocks = match find_edit_blocks(response) {
        Some(blocks) if !blocks.is_empty() => blocks,
        _ => return (vec![], response.to_string()),
    };

    let edits = blocks
        .iter()
        .map(|b| Edit {
            search: b.search.clone(),
            replace: b.replace.clone(),
        })
        .collect();

    let remaining = collect_non_edit_text(response, &blocks);
    (edits, remaining)
}

/// Format edits as a response body with `status="proposed"` blocks.
///
/// Combines the summary text (if any) with serialized edit blocks.
/// The output is intended to be written inside a `<magent-response>` block.
pub fn format_proposed_edits(edits: &[Edit], summary: &str) -> String {
    let mut result = String::new();
    if !summary.is_empty() {
        result.push_str(summary);
        result.push('\n');
    }
    for edit in edits {
        result.push_str("<magent-edit status=\"proposed\">\n");
        result.push_str("<magent-search>");
        result.push_str(&edit.search);
        result.push_str("</magent-search>\n");
        result.push_str("<magent-replace>");
        result.push_str(&edit.replace);
        result.push_str("</magent-replace>\n");
        result.push_str("</magent-edit>\n");
    }
    result
}

/// Result of applying a single edit.
#[derive(Debug, PartialEq)]
pub enum EditResult {
    Applied,
    Failed,
}

/// Apply a sequence of edits to document content.
///
/// Returns the modified content and a result per edit (in the same order).
/// Each edit replaces the first occurrence of `search` with `replace`.
/// Failed edits (search text not found) do not prevent other edits from applying.
pub fn apply_edits(content: &str, edits: &[Edit]) -> (String, Vec<EditResult>) {
    let mut result = content.to_string();
    let mut results = Vec::with_capacity(edits.len());

    for edit in edits {
        if !edit.search.is_empty() && result.contains(&edit.search) {
            result = result.replacen(&edit.search, &edit.replace, 1);
            results.push(EditResult::Applied);
        } else {
            results.push(EditResult::Failed);
        }
    }

    (result, results)
}

/// Status of an edit block in the document.
#[derive(Debug, PartialEq)]
pub enum EditStatus {
    Proposed,
    Accepted,
    Applied,
    Failed,
}

/// An edit block with status, as found in a `<magent-response>` block.
#[derive(Debug, PartialEq)]
pub struct PendingEdit {
    pub search: String,
    pub replace: String,
    pub status: EditStatus,
}

/// Parse edit blocks with status from a `<magent-response>` block.
///
/// Skips blocks that have no status attribute or an unrecognized status.
/// Returns an empty list if the content contains no parseable edit blocks.
pub fn parse_edit_blocks(response_content: &str) -> Vec<PendingEdit> {
    let blocks = match find_edit_blocks(response_content) {
        Some(blocks) => blocks,
        None => return vec![],
    };

    blocks
        .into_iter()
        .filter_map(|b| {
            let status = parse_status(b.status.as_deref()?)?;
            Some(PendingEdit {
                search: b.search,
                replace: b.replace,
                status,
            })
        })
        .collect()
}

struct RawEditBlock {
    search: String,
    replace: String,
    status: Option<String>,
    start: usize,
    end: usize,
}

/// Find and parse all `<magent-edit>` blocks in text.
///
/// Returns `None` if any block is structurally malformed (missing tags,
/// unclosed elements). Returns `Some(vec![])` if no blocks are found.
fn find_edit_blocks(text: &str) -> Option<Vec<RawEditBlock>> {
    let mut blocks = Vec::new();
    let mut pos = 0;

    while let Some(offset) = text[pos..].find("<magent-edit") {
        let edit_start = pos + offset;

        // Verify the tag isn't part of a longer element name (e.g. <magent-editor>)
        let after_name = edit_start + "<magent-edit".len();
        match text.as_bytes().get(after_name) {
            Some(b'>' | b' ' | b'\n' | b'\r') => {}
            _ => {
                pos = after_name;
                continue;
            }
        }

        // Find end of opening tag
        let tag_close = text[edit_start..].find('>')?;
        let tag_end = edit_start + tag_close;

        // Extract status from opening tag if present
        let opening_tag = &text[edit_start..=tag_end];
        let status = extract_status_attr(opening_tag);

        // Find closing tag
        let close_tag = "</magent-edit>";
        let close_offset = text[tag_end + 1..].find(close_tag)?;
        let block_end = tag_end + 1 + close_offset + close_tag.len();

        let inner = &text[tag_end + 1..tag_end + 1 + close_offset];

        let search = extract_tag_content(inner, "magent-search")?;
        let replace = extract_tag_content(inner, "magent-replace")?;

        blocks.push(RawEditBlock {
            search,
            replace,
            status,
            start: edit_start,
            end: block_end,
        });

        pos = block_end;
    }

    Some(blocks)
}

/// Extract text content between `<tag>` and `</tag>`.
fn extract_tag_content(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);

    let open_pos = text.find(&open)?;
    let start = open_pos + open.len();
    let end = text[start..].find(&close)? + start;

    Some(text[start..end].to_string())
}

/// Extract the `status` attribute value from an opening tag string.
fn extract_status_attr(opening_tag: &str) -> Option<String> {
    let attr = "status=\"";
    let start = opening_tag.find(attr)? + attr.len();
    let end = opening_tag[start..].find('"')? + start;
    Some(opening_tag[start..end].to_string())
}

/// Collect all text outside of edit blocks, trimmed.
fn collect_non_edit_text(text: &str, blocks: &[RawEditBlock]) -> String {
    let mut parts = Vec::new();
    let mut pos = 0;

    for block in blocks {
        if block.start > pos {
            parts.push(&text[pos..block.start]);
        }
        pos = block.end;
    }

    if pos < text.len() {
        parts.push(&text[pos..]);
    }

    parts.join("").trim().to_string()
}

/// Parse a status string into an `EditStatus`.
fn parse_status(s: &str) -> Option<EditStatus> {
    match s {
        "proposed" => Some(EditStatus::Proposed),
        "accepted" => Some(EditStatus::Accepted),
        "applied" => Some(EditStatus::Applied),
        "failed" => Some(EditStatus::Failed),
        _ => None,
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    // --- parse_edits ---

    #[test]
    fn parse_edits__should_extract_single_edit() {
        // Given
        let response = "\
<magent-edit>
<magent-search>old text</magent-search>
<magent-replace>new text</magent-replace>
</magent-edit>";

        // When
        let (edits, _remaining) = parse_edits(response);

        // Then
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].search, "old text");
        assert_eq!(edits[0].replace, "new text");
    }

    #[test]
    fn parse_edits__should_extract_multiple_edits() {
        // Given
        let response = "\
<magent-edit>
<magent-search>first old</magent-search>
<magent-replace>first new</magent-replace>
</magent-edit>
<magent-edit>
<magent-search>second old</magent-search>
<magent-replace>second new</magent-replace>
</magent-edit>";

        // When
        let (edits, _remaining) = parse_edits(response);

        // Then
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].search, "first old");
        assert_eq!(edits[0].replace, "first new");
        assert_eq!(edits[1].search, "second old");
        assert_eq!(edits[1].replace, "second new");
    }

    #[test]
    fn parse_edits__should_preserve_summary_text() {
        // Given
        let response = "\
Fixed 2 broken URLs:
<magent-edit>
<magent-search>htps://rust-lang.org</magent-search>
<magent-replace>https://rust-lang.org</magent-replace>
</magent-edit>
<magent-edit>
<magent-search>htps://tokio.rs</magent-search>
<magent-replace>https://tokio.rs</magent-replace>
</magent-edit>";

        // When
        let (edits, remaining) = parse_edits(response);

        // Then
        assert_eq!(edits.len(), 2);
        assert_eq!(remaining, "Fixed 2 broken URLs:");
    }

    #[test]
    fn parse_edits__should_preserve_text_between_and_after_blocks() {
        // Given
        let response = "\
Summary:
<magent-edit>
<magent-search>old</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>
Also changed:
<magent-edit>
<magent-search>old2</magent-search>
<magent-replace>new2</magent-replace>
</magent-edit>
Done.";

        // When
        let (_edits, remaining) = parse_edits(response);

        // Then
        assert_eq!(remaining, "Summary:\n\nAlso changed:\n\nDone.");
    }

    #[test]
    fn parse_edits__should_return_no_edits_for_plain_text() {
        // Given
        let response = "The sky is blue because of Rayleigh scattering.";

        // When
        let (edits, remaining) = parse_edits(response);

        // Then
        assert!(edits.is_empty());
        assert_eq!(remaining, response);
    }

    #[test]
    fn parse_edits__should_degrade_when_search_tag_missing() {
        // Given
        let response = "\
<magent-edit>
<magent-replace>new text</magent-replace>
</magent-edit>";

        // When
        let (edits, remaining) = parse_edits(response);

        // Then
        assert!(edits.is_empty());
        assert_eq!(remaining, response);
    }

    #[test]
    fn parse_edits__should_degrade_when_replace_tag_missing() {
        // Given
        let response = "\
<magent-edit>
<magent-search>old text</magent-search>
</magent-edit>";

        // When
        let (edits, remaining) = parse_edits(response);

        // Then
        assert!(edits.is_empty());
        assert_eq!(remaining, response);
    }

    #[test]
    fn parse_edits__should_degrade_when_edit_tag_unclosed() {
        // Given
        let response = "\
<magent-edit>
<magent-search>old text</magent-search>
<magent-replace>new text</magent-replace>";

        // When
        let (edits, remaining) = parse_edits(response);

        // Then
        assert!(edits.is_empty());
        assert_eq!(remaining, response);
    }

    #[test]
    fn parse_edits__should_handle_multiline_content() {
        // Given
        let response = "\
<magent-edit>
<magent-search>line 1
line 2
line 3</magent-search>
<magent-replace>new line 1
new line 2</magent-replace>
</magent-edit>";

        // When
        let (edits, _remaining) = parse_edits(response);

        // Then
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].search, "line 1\nline 2\nline 3");
        assert_eq!(edits[0].replace, "new line 1\nnew line 2");
    }

    #[test]
    fn parse_edits__should_handle_empty_replace() {
        // Given
        let response = "\
<magent-edit>
<magent-search>delete this</magent-search>
<magent-replace></magent-replace>
</magent-edit>";

        // When
        let (edits, _remaining) = parse_edits(response);

        // Then
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].search, "delete this");
        assert_eq!(edits[0].replace, "");
    }

    #[test]
    fn parse_edits__should_ignore_status_attribute() {
        // Given — edit blocks with status (as found in file) should still parse
        let response = "\
<magent-edit status=\"proposed\">
<magent-search>old</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>";

        // When
        let (edits, _remaining) = parse_edits(response);

        // Then
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].search, "old");
        assert_eq!(edits[0].replace, "new");
    }

    #[test]
    fn parse_edits__should_not_match_longer_tag_names() {
        // Given — <magent-editor> is not <magent-edit>
        let response = "See <magent-editor>config</magent-editor> for details.";

        // When
        let (edits, remaining) = parse_edits(response);

        // Then
        assert!(edits.is_empty());
        assert_eq!(remaining, response);
    }

    // --- format_proposed_edits ---

    #[test]
    fn format_proposed_edits__should_format_single_edit_with_summary() {
        // Given
        let edits = vec![Edit {
            search: "old text".to_string(),
            replace: "new text".to_string(),
        }];

        // When
        let result = format_proposed_edits(&edits, "Fixed one thing:");

        // Then
        assert_eq!(
            result,
            "Fixed one thing:\n\
             <magent-edit status=\"proposed\">\n\
             <magent-search>old text</magent-search>\n\
             <magent-replace>new text</magent-replace>\n\
             </magent-edit>\n"
        );
    }

    #[test]
    fn format_proposed_edits__should_format_multiple_edits() {
        // Given
        let edits = vec![
            Edit {
                search: "first".to_string(),
                replace: "1st".to_string(),
            },
            Edit {
                search: "second".to_string(),
                replace: "2nd".to_string(),
            },
        ];

        // When
        let result = format_proposed_edits(&edits, "");

        // Then
        assert!(result.contains("status=\"proposed\""));
        // Both edits present
        assert!(result.contains("<magent-search>first</magent-search>"));
        assert!(result.contains("<magent-search>second</magent-search>"));
    }

    #[test]
    fn format_proposed_edits__should_omit_summary_line_when_empty() {
        // Given
        let edits = vec![Edit {
            search: "old".to_string(),
            replace: "new".to_string(),
        }];

        // When
        let result = format_proposed_edits(&edits, "");

        // Then — should start directly with the edit block
        assert!(result.starts_with("<magent-edit"));
    }

    #[test]
    fn format_proposed_edits__should_roundtrip_with_parse_edit_blocks() {
        // Given
        let edits = vec![
            Edit {
                search: "old text".to_string(),
                replace: "new text".to_string(),
            },
            Edit {
                search: "line 1\nline 2".to_string(),
                replace: "replaced".to_string(),
            },
        ];

        // When
        let formatted = format_proposed_edits(&edits, "Summary:");
        let parsed = parse_edit_blocks(&formatted);

        // Then
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].status, EditStatus::Proposed);
        assert_eq!(parsed[0].search, "old text");
        assert_eq!(parsed[0].replace, "new text");
        assert_eq!(parsed[1].search, "line 1\nline 2");
        assert_eq!(parsed[1].replace, "replaced");
    }

    // --- parse_edit_blocks ---

    #[test]
    fn parse_edit_blocks__should_parse_proposed_status() {
        // Given
        let content = "\
<magent-edit status=\"proposed\">
<magent-search>old</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].status, EditStatus::Proposed);
        assert_eq!(blocks[0].search, "old");
        assert_eq!(blocks[0].replace, "new");
    }

    #[test]
    fn parse_edit_blocks__should_parse_accepted_status() {
        // Given
        let content = "\
<magent-edit status=\"accepted\">
<magent-search>old</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].status, EditStatus::Accepted);
    }

    #[test]
    fn parse_edit_blocks__should_parse_applied_status() {
        // Given
        let content = "\
<magent-edit status=\"applied\">
<magent-search>old</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].status, EditStatus::Applied);
    }

    #[test]
    fn parse_edit_blocks__should_parse_failed_status() {
        // Given
        let content = "\
<magent-edit status=\"failed\">
<magent-search>old</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].status, EditStatus::Failed);
    }

    #[test]
    fn parse_edit_blocks__should_parse_multiple_statuses() {
        // Given
        let content = "\
<magent-edit status=\"applied\">
<magent-search>first</magent-search>
<magent-replace>first new</magent-replace>
</magent-edit>
<magent-edit status=\"failed\">
<magent-search>second</magent-search>
<magent-replace>second new</magent-replace>
</magent-edit>";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].status, EditStatus::Applied);
        assert_eq!(blocks[0].search, "first");
        assert_eq!(blocks[1].status, EditStatus::Failed);
        assert_eq!(blocks[1].search, "second");
    }

    #[test]
    fn parse_edit_blocks__should_skip_blocks_without_status() {
        // Given
        let content = "\
<magent-edit>
<magent-search>no status</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>
<magent-edit status=\"proposed\">
<magent-search>has status</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "has status");
    }

    #[test]
    fn parse_edit_blocks__should_skip_unrecognized_status() {
        // Given
        let content = "\
<magent-edit status=\"pending\">
<magent-search>old</magent-search>
<magent-replace>new</magent-replace>
</magent-edit>";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert!(blocks.is_empty());
    }

    #[test]
    fn parse_edit_blocks__should_return_empty_for_no_blocks() {
        // Given
        let content = "Just some plain text summary.";

        // When
        let blocks = parse_edit_blocks(content);

        // Then
        assert!(blocks.is_empty());
    }

    #[test]
    fn parse_edit_blocks__should_return_empty_for_empty_input() {
        // Given / When
        let blocks = parse_edit_blocks("");

        // Then
        assert!(blocks.is_empty());
    }

    // --- apply_edits ---

    #[test]
    fn apply_edits__should_apply_single_edit() {
        // Given
        let content = "Hello world!";
        let edits = vec![Edit {
            search: "world".to_string(),
            replace: "Rust".to_string(),
        }];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(result, "Hello Rust!");
        assert_eq!(results, vec![EditResult::Applied]);
    }

    #[test]
    fn apply_edits__should_apply_multiple_edits() {
        // Given
        let content = "- [Rust](htps://rust-lang.org)\n- [Tokio](htps://tokio.rs)\n";
        let edits = vec![
            Edit {
                search: "htps://rust-lang.org".to_string(),
                replace: "https://rust-lang.org".to_string(),
            },
            Edit {
                search: "htps://tokio.rs".to_string(),
                replace: "https://tokio.rs".to_string(),
            },
        ];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(
            result,
            "- [Rust](https://rust-lang.org)\n- [Tokio](https://tokio.rs)\n"
        );
        assert_eq!(results, vec![EditResult::Applied, EditResult::Applied]);
    }

    #[test]
    fn apply_edits__should_report_failure_when_search_not_found() {
        // Given
        let content = "Hello world!";
        let edits = vec![Edit {
            search: "goodbye".to_string(),
            replace: "hi".to_string(),
        }];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(result, "Hello world!");
        assert_eq!(results, vec![EditResult::Failed]);
    }

    #[test]
    fn apply_edits__should_continue_after_failed_edit() {
        // Given
        let content = "Hello world!";
        let edits = vec![
            Edit {
                search: "nonexistent".to_string(),
                replace: "gone".to_string(),
            },
            Edit {
                search: "world".to_string(),
                replace: "Rust".to_string(),
            },
        ];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(result, "Hello Rust!");
        assert_eq!(results, vec![EditResult::Failed, EditResult::Applied]);
    }

    #[test]
    fn apply_edits__should_replace_first_occurrence_only() {
        // Given
        let content = "foo bar foo bar foo";
        let edits = vec![Edit {
            search: "foo".to_string(),
            replace: "baz".to_string(),
        }];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(result, "baz bar foo bar foo");
        assert_eq!(results, vec![EditResult::Applied]);
    }

    #[test]
    fn apply_edits__should_handle_empty_edit_list() {
        // Given
        let content = "Hello world!";

        // When
        let (result, results) = apply_edits(content, &[]);

        // Then
        assert_eq!(result, "Hello world!");
        assert!(results.is_empty());
    }

    #[test]
    fn apply_edits__should_handle_empty_replace_as_deletion() {
        // Given
        let content = "Hello cruel world!";
        let edits = vec![Edit {
            search: "cruel ".to_string(),
            replace: "".to_string(),
        }];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(result, "Hello world!");
        assert_eq!(results, vec![EditResult::Applied]);
    }

    #[test]
    fn apply_edits__should_fail_on_empty_search() {
        // Given
        let content = "Hello world!";
        let edits = vec![Edit {
            search: "".to_string(),
            replace: "inserted".to_string(),
        }];

        // When
        let (_result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(results, vec![EditResult::Failed]);
    }

    #[test]
    fn apply_edits__should_apply_in_order_so_earlier_edits_affect_later_ones() {
        // Given — edit 1 changes "A" to "B", edit 2 changes "B" to "C"
        // Since edit 1 applies first, the "B" it creates is found by edit 2
        let content = "A";
        let edits = vec![
            Edit {
                search: "A".to_string(),
                replace: "B".to_string(),
            },
            Edit {
                search: "B".to_string(),
                replace: "C".to_string(),
            },
        ];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(result, "C");
        assert_eq!(results, vec![EditResult::Applied, EditResult::Applied]);
    }

    #[test]
    fn apply_edits__should_report_all_failed_when_none_match() {
        // Given
        let content = "Hello world!";
        let edits = vec![
            Edit {
                search: "foo".to_string(),
                replace: "bar".to_string(),
            },
            Edit {
                search: "baz".to_string(),
                replace: "qux".to_string(),
            },
        ];

        // When
        let (result, results) = apply_edits(content, &edits);

        // Then
        assert_eq!(result, "Hello world!");
        assert_eq!(results, vec![EditResult::Failed, EditResult::Failed]);
    }
}
