//! search-panel: a Find in Files panel in rhun's sidebar. A form (search text, files to include
//! and exclude, Match Case, Whole Word, Regex) and a results tree under it; Ctrl+Alt+Shift+F
//! focuses the form, and the Explorer's "Find in Folder" fills in the folder as the files to
//! include. Literal search runs in rhun; regex search runs in search.rs.

mod search;

use std::collections::HashMap;

use rhun_ext::files::{FoundFile, SearchMatch};
use rhun_ext::resources::parse_folders;
use rhun_ext::views::{Field, FieldType, Icon, Kids, Node, Patch, RetainedEvent, State, ViewOpen};
use rhun_ext::{run, Event, Ext, Feature, Handler, Invocation, View};

use search::{Options, Outcome, Roots};

const SEARCH: &str = "search";

#[derive(Default)]
struct Panel {
    search: Option<View>,
    results: Option<View>,
    /// The user's last value of each form field, with the field's sequence (echoed back so rhun
    /// applies what we send only where it is current).
    values: HashMap<String, (u64, String)>,
    roots: Roots,
}

impl Handler for Panel {
    fn event(&mut self, _ext: &Ext, ev: &Event) {
        if let Some(folders) = parse_folders(ev) {
            self.roots = Roots::new(&folders);
        }
    }

    fn view_open(&mut self, ext: &Ext, open: &ViewOpen) {
        if open.name == SEARCH {
            self.search = Some(open.view);
            self.patch(ext, open.view, Patch::new().form(&self.form()));
        }
    }

    fn view_event(&mut self, ext: &Ext, view: View, ev: &RetainedEvent) {
        match ev {
            RetainedEvent::Change { key, seq, value, .. } => {
                self.values.insert(
                    key.clone(),
                    (*seq, String::from_utf8_lossy(value).into_owned()),
                );
            }
            // the results tab was closed: the next search opens a new one
            RetainedEvent::Closed if self.results == Some(view) => self.results = None,
            RetainedEvent::Submit { .. } => self.search(ext),
            RetainedEvent::Press { key, .. } if key == "run" => self.search(ext),
            _ => {}
        }
    }

    fn invoke(&mut self, ext: &Ext, inv: &Invocation) {
        match inv.name.as_str() {
            "focus" => self.focus(ext),
            "in_folder" => self.in_folder(ext, inv),
            "search" => {
                // a command's arguments (a key's, or a test's) become the query
                let query = String::from_utf8_lossy(&inv.body).trim().to_string();
                if !query.is_empty() {
                    self.set_value("query", &query);
                    if let Some(view) = self.search {
                        self.patch(ext, view, Patch::new().form(&self.form()));
                    }
                }
                self.search(ext);
            }
            _ => {}
        }
    }
}

impl Panel {
    fn focus(&self, ext: &Ext) {
        if let Err(e) = ext.vshow(SEARCH, true) {
            eprintln!("search-panel: focus: {e}");
        }
    }

    /// "Find in Folder": the folder becomes the files to include, the form takes the focus. The
    /// search itself is left to the user (Enter), as in VS Code.
    fn in_folder(&mut self, ext: &Ext, inv: &Invocation) {
        let Some(path) = inv
            .target()
            .and_then(|(t, _)| t.fields.last().cloned())
        else {
            return;
        };
        let Some(glob) = self.roots.folder_glob(&path) else {
            let _ = ext.toast("Find in Folder: the folder is not in the workspace");
            return;
        };
        let Some(view) = self.search else {
            let _ = ext.toast("Find in Files is not open");
            return;
        };
        self.set_value("include", &glob);
        self.patch(ext, view, Patch::new().form(&self.form()));
        self.focus(ext);
    }

    /// Runs the search and shows its results in the results tab, which opens (an editor tab of its
    /// own, closable) when there is none.
    fn search(&mut self, ext: &Ext) {
        let o = self.options();
        if o.query.is_empty() {
            let _ = ext.toast("Type a search first");
            return;
        }
        let view = match self.results {
            Some(view) => view,
            None => match ext.vtab("tree", "Find in Files: results") {
                Ok(view) => {
                    self.results = Some(view);
                    view
                }
                Err(e) => {
                    let _ = ext.toast(&format!("Find in Files: {e}"));
                    return;
                }
            },
        };
        self.patch(ext, view, Patch::new().busy(true));
        let result = if o.regex {
            search::regex(ext, &o, &self.roots)
        } else {
            search::literal(ext, &o).map_err(|e| e.to_string())
        };
        let mut p = match result {
            Ok(outcome) => self.results_patch(&o, outcome),
            Err(e) => {
                let mut p = Patch::new();
                p.state(State::Error(e, vec![]));
                p
            }
        };
        p.busy(false);
        self.patch(ext, view, &p);
    }

    fn results_patch(&self, o: &Options, outcome: Outcome) -> Patch {
        let mut p = Patch::new();
        let total: usize = outcome.files.iter().map(|f| f.matches.len()).sum();
        let files = outcome.files.len();
        let limited = outcome.status != "complete";
        if outcome.files.is_empty() {
            p.state(State::Empty(format!("No matches for \"{}\"", o.query), vec![]));
            return p;
        }
        p.message(&format!(
            "{total} matches in {files} files{}",
            if limited { " (limit reached)" } else { "" }
        ));
        let file_nodes: Vec<Node> = outcome.files.iter().map(|f| self.file_node(f)).collect();
        p.kids(b"", &file_nodes, false);
        for f in &outcome.files {
            let abs = self.roots.abs(f.folder, &f.path);
            let children: Vec<Node> = f
                .matches
                .iter()
                .map(|m| match_node(&abs, m))
                .collect();
            p.kids(file_id(&abs).as_bytes(), &children, false);
        }
        p
    }

    fn file_node(&self, f: &FoundFile) -> Node {
        let abs = self.roots.abs(f.folder, &f.path);
        let base = abs.rsplit('/').next().unwrap_or(&abs).to_string();
        Node::new(file_id(&abs), &self.roots.display(&abs))
            .kids(Kids::Open)
            .icon(Icon::file(&base), None)
            .badge(f.matches.len() as u32)
    }

    fn options(&self) -> Options {
        Options {
            query: self.text("query"),
            include: self.text("include"),
            exclude: self.text("exclude"),
            case: self.flag("case"),
            word: self.flag("word"),
            regex: self.flag("regex"),
        }
    }

    fn form(&self) -> Vec<Field> {
        vec![
            self.text_field("query", 1, 12, "Search", "text or regex to find"),
            self.text_field("include", 2, 12, "Files to include", "src/**, *.rs"),
            self.text_field("exclude", 3, 12, "Files to exclude", "target/**, *.lock"),
            self.check_field("case", 4, "Match Case"),
            self.check_field("word", 4, "Whole Word"),
            self.check_field("regex", 4, "Regex"),
            Field::new("run", FieldType::Button, 5, 12, "Search").primary(),
        ]
    }

    fn text_field(&self, key: &str, row: u32, weight: u32, label: &str, hint: &str) -> Field {
        Field::new(key, FieldType::Text, row, weight, label)
            .value(Some(self.seq(key)), &self.text(key))
            .placeholder(hint)
    }

    fn check_field(&self, key: &str, weight: u32, label: &str) -> Field {
        let on = if self.flag(key) { "yes" } else { "no" };
        Field::new(key, FieldType::Check, 4, weight, label).value(Some(self.seq(key)), on)
    }

    fn seq(&self, key: &str) -> u64 {
        self.values.get(key).map_or(0, |(seq, _)| *seq)
    }

    fn text(&self, key: &str) -> String {
        self.values
            .get(key)
            .map_or_else(String::new, |(_, v)| v.clone())
    }

    fn flag(&self, key: &str) -> bool {
        self.values.get(key).is_some_and(|(_, v)| v == "yes")
    }

    /// Sets a field's text, keeping the sequence the user last had (rhun applies it only then).
    fn set_value(&mut self, key: &str, value: &str) {
        let seq = self.seq(key);
        self.values.insert(key.to_string(), (seq, value.to_string()));
    }

    fn patch(&self, ext: &Ext, view: View, patch: &Patch) {
        if let Err(e) = ext.patch(view, patch) {
            eprintln!("search-panel: patch: {e}");
        }
    }
}

/// The node ids: a file by its path, a match by its path, line and column (stable across runs of
/// a search, so the tree keeps what the user expanded).
fn file_id(abs: &str) -> String {
    format!("f {abs}")
}

fn match_node(abs: &str, m: &SearchMatch) -> Node {
    let prefix = format!("{}: ", m.line);
    let text: String = m
        .text
        .chars()
        .map(|c| if c == '\t' || c == '\r' { ' ' } else { c })
        .collect();
    let label = format!("{prefix}{text}");
    let mut node = Node::new(format!("m {abs} {} {}", m.line, m.col), &label)
        .open(abs, m.line, m.col);
    // the match in the label: columns are bytes of the text, which starts at column `pre`
    let start = prefix.len() + m.col.saturating_sub(m.pre) as usize;
    let end = prefix.len() + m.end.saturating_sub(m.pre) as usize;
    if start < end
        && end <= label.len()
        && label.is_char_boundary(start)
        && label.is_char_boundary(end)
    {
        node = node.highlight(start..end);
    }
    node
}

fn main() {
    let features = [
        Feature::CommandArgs,
        Feature::ViewTree,
        Feature::ViewForm,
        Feature::Files,
        Feature::WorkspaceFolders,
    ];
    if let Err(e) = run(&features, Panel::default()) {
        eprintln!("search-panel: {e}");
    }
}
