use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

/// Searches markdown files across the knowledge base.
pub struct SearchTool {
    root: PathBuf,
}

impl SearchTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Execute a search query. Returns formatted results or an error message.
    ///
    /// Never fails — errors are returned as the result string so the LLM
    /// can see what went wrong and adjust.
    pub fn execute(&self, input: &str) -> String {
        let options = parse_input(input);

        if options.query.is_empty() {
            return "Error: empty search query".to_string();
        }

        let regex = match Regex::new(&options.query) {
            Ok(r) => r,
            Err(e) => return format!("Error: invalid regex pattern: {e}"),
        };

        let search_dir = match self.resolve_search_dir(&options.path) {
            Ok(d) => d,
            Err(msg) => return msg,
        };

        let mut files = collect_files(&search_dir, &options.glob);
        files.sort();

        let results = search_files(&files, &self.root, &regex);

        if results.is_empty() {
            return format!("No matches found for: \"{}\"", options.query);
        }

        format_results(&results, options.max)
    }

    fn resolve_search_dir(&self, path_filter: &Option<String>) -> Result<PathBuf, String> {
        let Some(p) = path_filter else {
            return Ok(self.root.clone());
        };

        let dir = self.root.join(p);
        if !dir.is_dir() {
            return Err(format!("Error: path '{p}' not found"));
        }

        let canonical = dir
            .canonicalize()
            .map_err(|_| format!("Error: cannot resolve path '{p}'"))?;
        let root_canonical = self
            .root
            .canonicalize()
            .map_err(|_| "Error: cannot resolve knowledge base root".to_string())?;

        if !canonical.starts_with(&root_canonical) {
            return Err(format!("Error: path '{p}' is outside the knowledge base"));
        }

        Ok(dir)
    }
}

// --- Input parsing ---

struct SearchOptions {
    path: Option<String>,
    glob: String,
    max: usize,
    query: String,
}

/// Greedy prefix parse: consume recognized `key:value` tokens from the front,
/// stop at the first unrecognized token. Everything remaining is the query.
fn parse_input(input: &str) -> SearchOptions {
    let mut path = None;
    let mut glob = "*.md".to_string();
    let mut max = 20;
    let mut remaining = input.trim();

    loop {
        remaining = remaining.trim_start();
        if let Some(rest) = remaining.strip_prefix("path:") {
            let (value, rest) = next_token(rest);
            path = Some(value.to_string());
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("glob:") {
            let (value, rest) = next_token(rest);
            glob = value.to_string();
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("max:") {
            let (value, rest) = next_token(rest);
            if let Ok(n) = value.parse() {
                max = n;
            }
            remaining = rest;
        } else {
            break;
        }
    }

    SearchOptions {
        path,
        glob,
        max,
        query: remaining.trim().to_string(),
    }
}

/// Split at the first whitespace: returns (token, rest).
fn next_token(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

// --- File collection ---

fn collect_files(dir: &Path, glob: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_dir(dir, glob, &mut files);
    files
}

fn walk_dir(dir: &Path, glob: &str, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, glob, files);
        } else if matches_glob(&path, glob) {
            files.push(path);
        }
    }
}

fn matches_glob(path: &Path, glob: &str) -> bool {
    if glob == "*" {
        return true;
    }
    if let Some(ext) = glob.strip_prefix("*.") {
        return path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e == ext);
    }
    // Exact filename match
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == glob)
}

// --- Search ---

struct FileResult {
    relative_path: String,
    /// Lines to display: (1-based line number, content).
    /// Includes matching lines and ±1 context lines, with overlapping ranges merged.
    display_lines: Vec<(usize, String)>,
    match_count: usize,
}

fn search_files(files: &[PathBuf], root: &Path, regex: &Regex) -> Vec<FileResult> {
    let mut results = Vec::new();

    for path in files {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let match_indices: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| regex.is_match(line))
            .map(|(i, _)| i)
            .collect();

        if match_indices.is_empty() {
            continue;
        }

        let ranges = expand_context(&match_indices, lines.len(), 1);
        let display_lines = ranges
            .into_iter()
            .flat_map(|(start, end)| (start..=end).map(|i| (i + 1, lines[i].to_string())))
            .collect();

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        results.push(FileResult {
            relative_path: relative,
            display_lines,
            match_count: match_indices.len(),
        });
    }

    results
}

/// Expand match indices by ±context lines, merge overlapping ranges.
/// Returns sorted, non-overlapping (start, end) pairs (0-based, inclusive).
fn expand_context(indices: &[usize], total_lines: usize, context: usize) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();

    for &idx in indices {
        let start = idx.saturating_sub(context);
        let end = (idx + context).min(total_lines.saturating_sub(1));

        if let Some(last) = ranges.last_mut()
            && start <= last.1 + 1
        {
            last.1 = last.1.max(end);
            continue;
        }
        ranges.push((start, end));
    }

    ranges
}

// --- Formatting ---

fn format_results(results: &[FileResult], max: usize) -> String {
    let total_matches: usize = results.iter().map(|r| r.match_count).sum();

    let mut matches_shown = 0;
    let mut files_shown = 0;
    let mut body = String::new();

    for result in results {
        if matches_shown >= max {
            break;
        }
        files_shown += 1;
        matches_shown += result.match_count;
        body.push('\n');
        for (num, line) in &result.display_lines {
            body.push_str(&format!("{}:{}: {}\n", result.relative_path, num, line));
        }
    }

    let overflow = total_matches - matches_shown;
    let mut out = format!(
        "{matches_shown} {} across {files_shown} {}:\n",
        if matches_shown == 1 {
            "match"
        } else {
            "matches"
        },
        if files_shown == 1 { "file" } else { "files" },
    );
    out.push_str(&body);

    if overflow > 0 {
        out.push_str(&format!("\n({overflow} more matches not shown)"));
    }

    out
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // --- parse_input ---

    #[test]
    fn parse_input__should_extract_query_only() {
        let opts = parse_input("error handling");
        assert_eq!(opts.query, "error handling");
        assert_eq!(opts.path, None);
        assert_eq!(opts.glob, "*.md");
        assert_eq!(opts.max, 20);
    }

    #[test]
    fn parse_input__should_extract_path_option() {
        let opts = parse_input("path:notes/ error handling");
        assert_eq!(opts.query, "error handling");
        assert_eq!(opts.path, Some("notes/".to_string()));
    }

    #[test]
    fn parse_input__should_extract_glob_option() {
        let opts = parse_input("glob:*.txt search term");
        assert_eq!(opts.query, "search term");
        assert_eq!(opts.glob, "*.txt");
    }

    #[test]
    fn parse_input__should_extract_max_option() {
        let opts = parse_input("max:5 search term");
        assert_eq!(opts.query, "search term");
        assert_eq!(opts.max, 5);
    }

    #[test]
    fn parse_input__should_extract_multiple_options() {
        let opts = parse_input("path:notes/ max:10 Result<T, E>");
        assert_eq!(opts.query, "Result<T, E>");
        assert_eq!(opts.path, Some("notes/".to_string()));
        assert_eq!(opts.max, 10);
    }

    #[test]
    fn parse_input__should_stop_at_non_option_token() {
        // "something" is not a known key:value, so parsing stops there
        let opts = parse_input("path:notes/ something max:10");
        assert_eq!(opts.query, "something max:10");
        assert_eq!(opts.path, Some("notes/".to_string()));
        assert_eq!(opts.max, 20); // default — not parsed
    }

    #[test]
    fn parse_input__should_handle_empty_input() {
        let opts = parse_input("");
        assert_eq!(opts.query, "");
    }

    #[test]
    fn parse_input__should_handle_options_only() {
        let opts = parse_input("path:notes/ max:5");
        assert_eq!(opts.query, "");
        assert_eq!(opts.path, Some("notes/".to_string()));
        assert_eq!(opts.max, 5);
    }

    // --- matches_glob ---

    #[test]
    fn matches_glob__should_match_md_extension() {
        assert!(matches_glob(Path::new("test.md"), "*.md"));
        assert!(!matches_glob(Path::new("test.txt"), "*.md"));
    }

    #[test]
    fn matches_glob__should_match_all_with_star() {
        assert!(matches_glob(Path::new("test.md"), "*"));
        assert!(matches_glob(Path::new("test.txt"), "*"));
        assert!(matches_glob(Path::new("no_ext"), "*"));
    }

    #[test]
    fn matches_glob__should_match_exact_filename() {
        assert!(matches_glob(Path::new("README.md"), "README.md"));
        assert!(!matches_glob(Path::new("other.md"), "README.md"));
    }

    #[test]
    fn matches_glob__should_not_match_file_without_extension() {
        assert!(!matches_glob(Path::new("Makefile"), "*.md"));
    }

    // --- expand_context ---

    #[test]
    fn expand_context__should_add_surrounding_lines() {
        // Given: match at index 5 in a 10-line file
        let ranges = expand_context(&[5], 10, 1);
        // Then: [4, 6]
        assert_eq!(ranges, vec![(4, 6)]);
    }

    #[test]
    fn expand_context__should_clamp_at_start_of_file() {
        let ranges = expand_context(&[0], 5, 1);
        assert_eq!(ranges, vec![(0, 1)]);
    }

    #[test]
    fn expand_context__should_clamp_at_end_of_file() {
        let ranges = expand_context(&[4], 5, 1);
        assert_eq!(ranges, vec![(3, 4)]);
    }

    #[test]
    fn expand_context__should_merge_overlapping_ranges() {
        // Matches at 2 and 4: expanded to [1,3] and [3,5] → merged to [1,5]
        let ranges = expand_context(&[2, 4], 10, 1);
        assert_eq!(ranges, vec![(1, 5)]);
    }

    #[test]
    fn expand_context__should_merge_adjacent_ranges() {
        // Matches at 2 and 5: expanded to [1,3] and [4,6] → adjacent, merged to [1,6]
        let ranges = expand_context(&[2, 5], 10, 1);
        assert_eq!(ranges, vec![(1, 6)]);
    }

    #[test]
    fn expand_context__should_keep_separate_ranges() {
        // Matches at 2 and 8: expanded to [1,3] and [7,9] → gap, kept separate
        let ranges = expand_context(&[2, 8], 10, 1);
        assert_eq!(ranges, vec![(1, 3), (7, 9)]);
    }

    #[test]
    fn expand_context__should_handle_single_line_file() {
        let ranges = expand_context(&[0], 1, 1);
        assert_eq!(ranges, vec![(0, 0)]);
    }

    // --- execute (integration) ---

    #[test]
    fn execute__should_find_matches_across_files() {
        // Given
        let dir = tempdir().unwrap();
        create_file(
            dir.path(),
            "rust.md",
            "Rust uses Result for error handling.\nIt also has panic.",
        );
        create_file(
            dir.path(),
            "go.md",
            "Go uses error handling with multiple returns.",
        );
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("error handling");

        // Then
        assert!(result.contains("2 matches across 2 files:"));
        assert!(result.contains("go.md:1: Go uses error handling"));
        assert!(result.contains("rust.md:1: Rust uses Result for error handling"));
    }

    #[test]
    fn execute__should_return_no_matches_message() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "nothing relevant here");
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("nonexistent phrase");

        // Then
        assert_eq!(result, "No matches found for: \"nonexistent phrase\"");
    }

    #[test]
    fn execute__should_handle_invalid_regex() {
        // Given
        let dir = tempdir().unwrap();
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("[invalid");

        // Then
        assert!(result.starts_with("Error: invalid regex pattern:"));
    }

    #[test]
    fn execute__should_handle_empty_query() {
        // Given
        let dir = tempdir().unwrap();
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When / Then
        assert_eq!(tool.execute(""), "Error: empty search query");
        assert_eq!(tool.execute("   "), "Error: empty search query");
    }

    #[test]
    fn execute__should_handle_options_only_as_empty_query() {
        // Given
        let dir = tempdir().unwrap();
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("path:notes/ max:5");

        // Then
        assert_eq!(result, "Error: empty search query");
    }

    #[test]
    fn execute__should_respect_path_filter() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "notes/rust.md", "error handling in Rust");
        create_file(dir.path(), "other/go.md", "error handling in Go");
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("path:notes/ error handling");

        // Then
        assert!(result.contains("notes/rust.md"));
        assert!(!result.contains("go.md"));
    }

    #[test]
    fn execute__should_reject_path_outside_root() {
        // Given
        let dir = tempdir().unwrap();
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("path:../../etc test");

        // Then
        assert!(result.starts_with("Error: path"));
    }

    #[test]
    fn execute__should_respect_max_limit() {
        // Given: 4 total matches across 2 files, max:2
        let dir = tempdir().unwrap();
        create_file(dir.path(), "a.md", "match one\nmatch two");
        create_file(dir.path(), "b.md", "match three\nmatch four");
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("max:2 match");

        // Then: first file shown (2 matches), second file's matches reported as overflow
        assert!(result.contains("2 matches across 1 file:"));
        assert!(result.contains("(2 more matches not shown)"));
    }

    #[test]
    fn execute__should_include_context_lines() {
        // Given
        let dir = tempdir().unwrap();
        create_file(
            dir.path(),
            "test.md",
            "line 1\nline 2\nerror here\nline 4\nline 5",
        );
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("error here");

        // Then: match at line 3, context includes lines 2 and 4
        assert!(result.contains("test.md:2: line 2"));
        assert!(result.contains("test.md:3: error here"));
        assert!(result.contains("test.md:4: line 4"));
        // Line 1 and 5 should NOT appear
        assert!(!result.contains("test.md:1:"));
        assert!(!result.contains("test.md:5:"));
    }

    #[test]
    fn execute__should_only_search_md_files_by_default() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "notes.md", "findable content");
        create_file(dir.path(), "notes.txt", "findable content");
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("findable");

        // Then
        assert!(result.contains("notes.md"));
        assert!(!result.contains("notes.txt"));
    }

    #[test]
    fn execute__should_respect_glob_filter() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "notes.md", "findable content");
        create_file(dir.path(), "notes.txt", "findable content");
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("glob:*.txt findable");

        // Then
        assert!(!result.contains("notes.md"));
        assert!(result.contains("notes.txt"));
    }

    #[test]
    fn execute__should_search_subdirectories() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "top.md", "match here");
        create_file(dir.path(), "sub/nested.md", "match here too");
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("match here");

        // Then
        assert!(result.contains("2 matches across 2 files:"));
    }

    #[test]
    fn execute__should_show_line_numbers() {
        // Given
        let dir = tempdir().unwrap();
        create_file(dir.path(), "test.md", "line 1\nline 2\nfind me\nline 4");
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("find me");

        // Then
        assert!(result.contains("test.md:3: find me"));
    }

    #[test]
    fn execute__should_support_regex_patterns() {
        // Given: matches separated by enough lines so Vec<String> isn't context
        let dir = tempdir().unwrap();
        create_file(
            dir.path(),
            "test.md",
            "Result<T, E>\nfiller\nfiller\nfiller\nOption<T>\nfiller\nfiller\nfiller\nVec<String>",
        );
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("Result|Option");

        // Then
        assert!(result.contains("2 matches"));
        assert!(result.contains("Result<T, E>"));
        assert!(result.contains("Option<T>"));
        assert!(!result.contains("Vec<String>"));
    }

    #[test]
    fn execute__should_skip_non_utf8_files() {
        // Given: a valid md file and one with invalid UTF-8
        let dir = tempdir().unwrap();
        create_file(dir.path(), "good.md", "findable text");
        let bad_path = dir.path().join("bad.md");
        fs::write(&bad_path, [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        let tool = SearchTool::new(dir.path().to_path_buf());

        // When
        let result = tool.execute("findable");

        // Then: finds the good file, silently skips the bad one
        assert!(result.contains("good.md"));
        assert!(result.contains("1 match across 1 file:"));
    }
}
