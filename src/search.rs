//! The search behind the panel. Literal text goes through rhun (`search-text`: it also searches
//! unsaved editor text). A regular expression is run here: the Rust SDK takes a compiled RNRX
//! program for search-text and cannot build one, so the files find-files lists are read and
//! matched with the `regex` crate. Unsaved editor text is not part of a regex search.

use std::fs;

use regex::RegexBuilder;
use rhun_ext::files::{self, FileQuery, FoundFile, FoundPath, Pattern, SearchMatch};
use rhun_ext::resources::Folder;
use rhun_ext::Ext;

/// Matches shown at most (rhun stops its search there too).
pub const MAX_MATCHES: usize = 2000;
/// Files the regex search reads at most, and the largest of them.
const MAX_FILES: u32 = 100_000;
const MAX_FILE_BYTES: u64 = 2 << 20;
/// Text kept of a line for a match (the panel shows it).
const MAX_LINE_BYTES: usize = 512;

/// What the form asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub query: String,
    /// Glob text: comma-separated globs (see [`globs`]).
    pub include: String,
    pub exclude: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
}

/// The files with matches, and the reply's status: `complete` or `limit`.
pub struct Outcome {
    pub files: Vec<FoundFile>,
    pub status: String,
}

/// The workspace folders: id and root path, so a found path can be read and shown.
#[derive(Default)]
pub struct Roots(Vec<(u64, String)>);

impl Roots {
    pub fn new(folders: &[Folder]) -> Roots {
        Roots(folders.iter().map(|f| (f.id, f.path.clone())).collect())
    }

    /// The absolute path of a found path (rhun may give it relative to its folder).
    pub fn abs(&self, folder: u64, path: &str) -> String {
        if path.starts_with('/') {
            return path.to_string();
        }
        match self.0.iter().find(|(id, _)| *id == folder) {
            Some((_, root)) => format!("{}/{path}", root.trim_end_matches('/')),
            None => path.to_string(),
        }
    }

    /// The path shown: relative to its workspace folder when it is inside one.
    pub fn display(&self, abs: &str) -> String {
        for (_, root) in &self.0 {
            if let Some(rel) = strip_root(abs, root) {
                return rel.to_string();
            }
        }
        abs.to_string()
    }

    /// The include glob for a folder (what "Find in Folder" fills in): its files, relative to the
    /// workspace folder that holds it; empty for a workspace folder itself. None outside them.
    pub fn folder_glob(&self, abs: &str) -> Option<String> {
        for (_, root) in &self.0 {
            let root = root.trim_end_matches('/');
            if abs.trim_end_matches('/') == root {
                return Some(String::new());
            }
            if let Some(rel) = strip_root(abs, root) {
                return Some(format!("{}/**", escape_glob(rel)));
            }
        }
        None
    }
}

fn strip_root<'a>(abs: &'a str, root: &str) -> Option<&'a str> {
    abs.strip_prefix(root.trim_end_matches('/'))?.strip_prefix('/')
}

/// Splits glob text at commas, except inside braces (`{a,b}` stays one glob).
pub fn globs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0u32;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                push_glob(&mut out, &cur);
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    push_glob(&mut out, &cur);
    out
}

fn push_glob(out: &mut Vec<String>, text: &str) {
    let t = text.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
}

/// A path as a glob that matches exactly it: each glob character in brackets.
pub fn escape_glob(path: &str) -> String {
    path.chars()
        .map(|c| match c {
            '*' | '?' | '[' | ']' | '{' | '}' => format!("[{c}]"),
            _ => c.to_string(),
        })
        .collect()
}

/// Literal search through rhun.
pub fn literal(ext: &Ext, o: &Options) -> Result<Outcome, rhun_ext::Error> {
    let mut flags = 0;
    if !o.case {
        flags |= files::IGNORE_CASE;
    }
    if o.word {
        flags |= files::WHOLE_WORD;
    }
    let q = FileQuery {
        folder: 0,
        max: MAX_MATCHES as u32,
        flags,
        include: globs(&o.include),
        exclude: globs(&o.exclude),
    };
    let mut found = Vec::new();
    let done = ext.search_text(&Pattern::Literal(o.query.clone()), &q, |batch| {
        found.extend_from_slice(batch);
        true
    })?;
    Ok(Outcome {
        files: found,
        status: done.status,
    })
}

/// Regular expression search over the files rhun lists (git's ignore rules included).
pub fn regex(ext: &Ext, o: &Options, roots: &Roots) -> Result<Outcome, String> {
    let pattern = if o.word {
        format!(r"\b(?:{})\b", o.query)
    } else {
        o.query.clone()
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(!o.case)
        .build()
        .map_err(|e| format!("invalid regex: {e}"))?;
    let q = FileQuery {
        folder: 0,
        max: MAX_FILES,
        flags: 0,
        include: globs(&o.include),
        exclude: globs(&o.exclude),
    };
    let mut paths: Vec<FoundPath> = Vec::new();
    ext.find_files(&q, |batch| {
        paths.extend_from_slice(batch);
        true
    })
    .map_err(|e| e.to_string())?;

    let mut files = Vec::new();
    let mut total = 0;
    let mut status = "complete";
    for p in paths {
        let abs = roots.abs(p.folder, &p.path);
        let Some(text) = read_text(&abs) else {
            continue;
        };
        let mut matches = Vec::new();
        'lines: for (i, line) in text.lines().enumerate() {
            for m in re.find_iter(line) {
                if m.is_empty() {
                    continue;
                }
                matches.push(SearchMatch {
                    line: i as u32 + 1,
                    col: m.start() as u32 + 1,
                    end: m.end() as u32 + 1,
                    pre: 1,
                    text: cut(line, MAX_LINE_BYTES).to_string(),
                });
                total += 1;
                if total == MAX_MATCHES {
                    break 'lines;
                }
            }
        }
        if !matches.is_empty() {
            files.push(FoundFile {
                folder: p.folder,
                doc: 0,
                version: 0,
                path: p.path,
                matches,
            });
        }
        if total >= MAX_MATCHES {
            status = "limit";
            break;
        }
    }
    Ok(Outcome {
        files,
        status: status.to_string(),
    })
}

/// A file's text, or None when it is too big, binary or not UTF-8.
fn read_text(path: &str) -> Option<String> {
    if fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The longest prefix of `s` of at most `n` bytes that ends on a character boundary.
pub fn cut(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut i = n;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_split_at_commas_outside_braces() {
        assert_eq!(globs("src/**, *.rs"), vec!["src/**", "*.rs"]);
        assert_eq!(globs("{a,b}/**,  ,x"), vec!["{a,b}/**", "x"]);
        assert!(globs("  ,  ").is_empty());
    }

    #[test]
    fn escape_glob_keeps_paths_literal() {
        assert_eq!(escape_glob("src/a[1]*"), "src/a[[]1[]][*]");
        assert_eq!(escape_glob("src/main"), "src/main");
    }

    #[test]
    fn folder_glob_is_relative_to_its_workspace_folder() {
        let roots = Roots(vec![(1, "/home/u/proj/".into())]);
        assert_eq!(roots.folder_glob("/home/u/proj").as_deref(), Some(""));
        assert_eq!(
            roots.folder_glob("/home/u/proj/src/ui").as_deref(),
            Some("src/ui/**")
        );
        assert_eq!(roots.folder_glob("/elsewhere"), None);
        assert_eq!(roots.display("/home/u/proj/src/a.rs"), "src/a.rs");
    }

    #[test]
    fn abs_joins_relative_paths_to_their_folder() {
        let roots = Roots(vec![(1, "/p".into())]);
        assert_eq!(roots.abs(1, "src/a.rs"), "/p/src/a.rs");
        assert_eq!(roots.abs(1, "/q/b.rs"), "/q/b.rs");
    }

    #[test]
    fn cut_keeps_char_boundaries() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("aé", 2), "a");
        assert_eq!(cut("aé", 3), "aé");
    }
}
