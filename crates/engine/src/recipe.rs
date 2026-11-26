//! Recipes (Milestone 14): the steps done to a table, kept so they can be
//! run again on another file, in the app or with `natendo-cli run`. A
//! recipe is plain JSON; each step is one of the operations below, with
//! its settings, and runs as it would from the menus (plan, then apply).

use serde::{Deserialize, Serialize};

use crate::clean::{CleanMatrix, CleanSpec, SplitName};
use crate::document::{Document, Progress};
use crate::editing::{Arrange, Extract, GroupCalc, MapValues};
use crate::example::ExampleSpec;
use crate::expr::ComputeSpec;
use crate::filter::FilterSpec;
use crate::find::FindSpec;
use crate::ops::{MergeColumns, SplitColumn};
use crate::reshape::{Blanks, Duplicates, MergeRows, SplitRows};
use crate::sort::SortSpec;
use crate::tools::{Breaks, FillBlanks, Repeats};
use crate::Error;

/// The recipe file format's version.
pub const RECIPE_VERSION: u32 = 1;
/// Most steps in a recipe.
pub const MAX_STEPS: usize = 500;
/// Most questions a recipe asks.
pub const MAX_QUESTIONS: usize = 50;
/// Longest answer to a question.
pub const MAX_ANSWER_CHARS: usize = 10_000;

/// A value a recipe asks for each time it runs ("which county?"). Its
/// steps hold `{{name}}` where the answer goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Letters, digits, spaces, `-` and `_` (the command line's
    /// `--set name=value`).
    pub name: String,
    /// The question as asked; the name when empty.
    #[serde(default)]
    pub prompt: String,
    /// The answer filled in at first (the last one given).
    #[serde(default)]
    pub default: String,
}

/// Whether `name` can name a question.
pub fn question_name_ok(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.chars().count() <= 40
        && n.chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
}

/// The placeholder for question `name` in a step's text.
pub fn placeholder(name: &str) -> String {
    format!("{{{{{}}}}}", name.trim())
}

/// Every text in a JSON value, with its path (`spec.with`, `pairs.0.1`).
fn texts_of(v: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
    match v {
        serde_json::Value::String(s) => out.push((path.to_string(), s.clone())),
        serde_json::Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                texts_of(x, &join_path(path, &i.to_string()), out);
            }
        }
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                // The step's kind is not a value.
                if path.is_empty() && k == "op" {
                    continue;
                }
                texts_of(x, &join_path(path, k), out);
            }
        }
        _ => {}
    }
}

fn join_path(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        format!("{a}.{b}")
    }
}

/// Replace every placeholder in the texts of `v`.
fn fill_texts(v: &mut serde_json::Value, answers: &[(String, String)]) {
    match v {
        serde_json::Value::String(s) => {
            if s.contains("{{") {
                for (k, a) in answers {
                    if s.contains(k.as_str()) {
                        *s = s.replace(k.as_str(), a);
                    }
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| fill_texts(x, answers)),
        serde_json::Value::Object(m) => m.values_mut().for_each(|x| fill_texts(x, answers)),
        _ => {}
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// What a recorded "Fix all like this" says about the rows left unticked.
pub fn unticked_note(unticked: u32, fields: u32) -> String {
    let rows = if unticked == 1 {
        "1 row you unticked is".to_string()
    } else {
        format!("{unticked} rows you unticked are")
    };
    format!("{rows} not in the recipe; running it fixes every row of {fields} fields")
}

/// One step. The names match the app's own operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RecipeOp {
    Sort {
        spec: SortSpec,
    },
    Filter {
        spec: FilterSpec,
    },
    ClearFilter,
    InsertCols {
        at: u32,
        count: u32,
    },
    DeleteCols {
        at: u32,
        count: u32,
    },
    MoveCols {
        at: u32,
        count: u32,
        to: u32,
    },
    ReplaceAll {
        spec: FindSpec,
        with: String,
    },
    /// Many find → replace pairs at once.
    BatchReplace {
        spec: crate::find::BatchReplace,
    },
    RemoveDuplicates {
        spec: Duplicates,
    },
    RemoveBlankRows {
        spec: Blanks,
    },
    RemoveBlankCols {
        spec: Blanks,
    },
    SplitColumn {
        spec: SplitColumn,
    },
    MergeColumns {
        spec: MergeColumns,
    },
    SplitRows {
        spec: SplitRows,
    },
    MergeRows {
        spec: MergeRows,
    },
    SortColumns {
        col: u32,
        count: u32,
        descending: bool,
    },
    Clean {
        spec: CleanSpec,
    },
    /// A clean matrix: each column (found by title) with its own steps, in
    /// the order stored.
    CleanMatrix {
        spec: CleanMatrix,
    },
    SplitName {
        spec: SplitName,
    },
    FillExample {
        spec: ExampleSpec,
    },
    Compute {
        spec: ComputeSpec,
    },
    FillBlanks {
        spec: FillBlanks,
    },
    CountRepeats {
        spec: Repeats,
    },
    BlankRepeats {
        spec: Duplicates,
    },
    Breaks {
        spec: Breaks,
    },
    DeleteHidden,
    MapValues {
        spec: MapValues,
    },
    GroupCalc {
        spec: GroupCalc,
    },
    Arrange {
        spec: Arrange,
    },
    Extract {
        spec: Extract,
    },
    /// Data checks: rows that fail first.
    SortProblems {
        spec: crate::validate::CheckSpec,
        /// Left out (older recipes): every row.
        #[serde(default)]
        scope: crate::sort::SortScope,
    },
    /// Data checks: a "Check result" column.
    CheckColumn {
        spec: crate::validate::CheckSpec,
        #[serde(default)]
        title: String,
    },
    /// Rename a column: its cell in the first row (the header row).
    RenameColumn {
        col: u32,
        name: String,
    },
    /// Tidy every column title (spaces, case, odd characters, repeats).
    TidyTitles {
        spec: crate::titles::TitleSpec,
    },
    /// Compare two columns: a column saying Same, Different, Only in first
    /// or Only in second.
    CompareColumn {
        spec: crate::colcompare::CompareCols,
    },
    /// What the columns hold (Text, Number, Date, Id), by column TITLE so it still fits a file whose columns
    /// moved: exactly these columns get these types and every other column is Auto. A column named here that
    /// the file does not have stops the run ("no column titled ...").
    ColumnTypes {
        types: Vec<ColumnTypeAt>,
    },
    /// "Fix all like this": join the fields `groups` in every row of exactly
    /// `fields` fields, in a file whose usual number is `usual`. Counts and
    /// places only, never a cell's text.
    JoinFields {
        usual: u32,
        fields: u32,
        groups: Vec<u32>,
        /// The first row holds titles and is never changed.
        #[serde(default)]
        header: bool,
        /// Rows left unticked when it was done by hand. Which ones is not
        /// kept: running the step changes every row of that width.
        #[serde(default, skip_serializing_if = "is_zero")]
        unticked: u32,
    },
    /// Set the first `head` and last `foot` rows aside as report notes
    /// (lines above and below the table). Counts only: the lines themselves
    /// are never in a recipe.
    SetAside {
        #[serde(default)]
        head: u32,
        #[serde(default)]
        foot: u32,
    },
}

/// One typed column of a [`RecipeOp::ColumnTypes`] step: its title (and where it was, used only when the
/// table has no titles) and its type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnTypeAt {
    pub title: String,
    pub col: u32,
    #[serde(rename = "type")]
    pub kind: crate::coltype::ColumnType,
}

impl RecipeOp {
    /// What the step does, in a few words.
    pub fn describe(&self) -> String {
        match self {
            RecipeOp::Sort { spec } => match spec.scope {
                crate::sort::SortScope::All => "Sort all the rows, hidden ones too".into(),
                crate::sort::SortScope::Shown => {
                    "Sort the shown rows (hidden rows stay where they are)".into()
                }
            },
            RecipeOp::Filter { .. } => "Filter the rows".into(),
            RecipeOp::ClearFilter => "Show all rows (no filter)".into(),
            RecipeOp::InsertCols { at, count } => {
                format!("Insert {count} column(s) at column {}", at + 1)
            }
            RecipeOp::DeleteCols { at, count } => {
                format!("Delete {count} column(s) from column {}", at + 1)
            }
            RecipeOp::MoveCols { at, count, to } => {
                format!(
                    "Move {count} column(s) from column {} to column {}",
                    at + 1,
                    to + 1
                )
            }
            RecipeOp::ReplaceAll { spec, with } => {
                format!("Replace \"{}\" with \"{with}\"", spec.pattern.text)
            }
            RecipeOp::BatchReplace { spec } => {
                format!("Replace from a list of {} pairs", spec.pairs.len())
            }
            RecipeOp::RemoveDuplicates { .. } => "Remove duplicate rows".into(),
            RecipeOp::RemoveBlankRows { .. } => "Remove blank rows".into(),
            RecipeOp::RemoveBlankCols { .. } => "Remove blank columns".into(),
            RecipeOp::SplitColumn { .. } => "Split a column".into(),
            RecipeOp::MergeColumns { .. } => "Merge columns".into(),
            RecipeOp::SplitRows { .. } => "Split cells into rows".into(),
            RecipeOp::MergeRows { .. } => "Merge rows".into(),
            RecipeOp::SortColumns { .. } => "Sort columns".into(),
            RecipeOp::Clean { spec } => {
                let steps: Vec<String> = spec.cleaner.steps.iter().map(|s| s.describe()).collect();
                format!("Clean: {}", steps.join(", "))
            }
            RecipeOp::CleanMatrix { spec } => {
                let columns: Vec<String> = spec
                    .entries
                    .iter()
                    .map(|e| {
                        let name = if e.title.trim().is_empty() {
                            format!("column {}", e.col + 1)
                        } else {
                            e.title.trim().to_string()
                        };
                        let steps: Vec<String> = e.steps.iter().map(|s| s.describe()).collect();
                        let from = if e.from_file.is_empty() {
                            String::new()
                        } else {
                            format!("from {}: ", e.from_file)
                        };
                        format!("{name} ({from}{})", steps.join(", "))
                    })
                    .collect();
                format!("Clean matrix: {}", columns.join("; "))
            }
            RecipeOp::SplitName { .. } => "Split names".into(),
            RecipeOp::FillExample { .. } => "Fill by example".into(),
            RecipeOp::Compute { spec } if spec.target.is_none() && !spec.hidden => format!(
                "Computed column: {} (shown rows only; hidden rows left empty)",
                spec.expr
            ),
            RecipeOp::Compute { spec } => format!("Computed column: {}", spec.expr),
            RecipeOp::FillBlanks { spec } => {
                let mut stops = Vec::new();
                if let Some(g) = spec.guard {
                    stops.push(match &spec.guard_title {
                        Some(t) if !t.trim().is_empty() => format!("rows with no \u{201c}{t}\u{201d}"),
                        _ => format!("rows with column {} blank", g + 1),
                    });
                }
                if spec.stop_blank_rows {
                    stops.push("blank rows".to_string());
                }
                if spec.stop_total_rows {
                    stops.push("total rows".to_string());
                }
                if !spec.has_stops() || stops.is_empty() {
                    "Fill blank cells".into()
                } else {
                    let last = stops.pop().unwrap_or_default();
                    let head = stops.join(", ");
                    if head.is_empty() {
                        format!("Fill blank cells, stopping at {last}")
                    } else {
                        format!("Fill blank cells, stopping at {head} and {last}")
                    }
                }
            }
            RecipeOp::CountRepeats { .. } => "Count repeats".into(),
            RecipeOp::BlankRepeats { .. } => "Blank out repeats".into(),
            RecipeOp::Breaks { .. } => "Add breaks between groups".into(),
            RecipeOp::DeleteHidden => "Delete hidden rows".into(),
            RecipeOp::JoinFields {
                fields, unticked, ..
            } => {
                let mut text = format!("Join fields in every row of {fields} fields");
                if *unticked > 0 {
                    text.push_str(&format!(" ({})", unticked_note(*unticked, *fields)));
                }
                text
            }
            RecipeOp::RenameColumn { col, name } => {
                format!("Rename column {} to \"{}\"", col + 1, name)
            }
            RecipeOp::TidyTitles { .. } => "Tidy column titles".into(),
            RecipeOp::CompareColumn { spec } => {
                format!("Compare columns {} and {}", spec.a + 1, spec.b + 1)
            }
            RecipeOp::ColumnTypes { types } => {
                if types.is_empty() {
                    "Treat every column as Auto".into()
                } else {
                    format!("Treat {} column(s) as typed", types.len())
                }
            }
            RecipeOp::MapValues { .. } => "Change values from a list".into(),
            RecipeOp::GroupCalc { .. } => "Group figures".into(),
            RecipeOp::Arrange { .. } => "Arrange columns".into(),
            RecipeOp::Extract { .. } => "Extract a piece into a column".into(),
            RecipeOp::SortProblems { scope, .. } => match scope {
                crate::sort::SortScope::All => {
                    "Put rows that fail the checks first (all rows, hidden ones too)".into()
                }
                crate::sort::SortScope::Shown => {
                    "Put rows that fail the checks first (shown rows; hidden rows stay)".into()
                }
            },
            RecipeOp::CheckColumn { .. } => "Add a check result column".into(),
            RecipeOp::SetAside { head, foot } => {
                let rows = |n: u32| {
                    if n == 1 {
                        "row".to_string()
                    } else {
                        format!("{n} rows")
                    }
                };
                match (*head, *foot) {
                    (0, f) => format!("Set aside the last {} as report notes", rows(f)),
                    (h, 0) => format!("Set aside the first {} as report notes", rows(h)),
                    (h, f) => format!(
                        "Set aside the first {} and the last {} as report notes",
                        rows(h),
                        rows(f)
                    ),
                }
            }
        }
    }
}

/// A recipe: the steps, and the column titles the table had when the first
/// step was recorded (a file with other columns is refused unless the user
/// runs it anyway).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub columns: Vec<String>,
    /// The first row holds the column titles.
    #[serde(default)]
    pub header: bool,
    pub steps: Vec<RecipeOp>,
    /// Steps done that a recipe cannot repeat (a join with another tab,
    /// a paste), in words, so the user knows they are missing.
    #[serde(default)]
    pub left_out: Vec<String>,
    /// Values asked for each time the recipe runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<Question>,
}

impl RecipeOp {
    /// Whether the step holds values that may have come from the file
    /// (ticked filter values, texts to find or put in, examples, lists),
    /// so a saved recipe can carry them.
    pub fn holds_values(&self) -> bool {
        matches!(
            self,
            RecipeOp::Filter { .. }
                | RecipeOp::ReplaceAll { .. }
                | RecipeOp::BatchReplace { .. }
                | RecipeOp::FillExample { .. }
                | RecipeOp::MapValues { .. }
                | RecipeOp::FillBlanks { .. }
                | RecipeOp::Compute { .. }
                | RecipeOp::Clean { .. }
                | RecipeOp::CleanMatrix { .. }
                | RecipeOp::Extract { .. }
                | RecipeOp::MergeRows { .. }
                | RecipeOp::GroupCalc { .. }
                | RecipeOp::SortProblems { .. }
                | RecipeOp::CheckColumn { .. }
        )
    }
}

impl Recipe {
    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        serde_json::to_vec_pretty(self).map_err(|e| Error::Invalid(e.to_string()))
    }

    pub fn from_json(b: &[u8]) -> Result<Recipe, Error> {
        if b.len() > 16 << 20 {
            return Err(Error::Invalid("the recipe file is too large".into()));
        }
        let r: Recipe = serde_json::from_slice(b)
            .map_err(|e| Error::Invalid(format!("not a recipe this app can read: {e}")))?;
        if r.version > RECIPE_VERSION {
            return Err(Error::Invalid(
                "this recipe was saved by a newer version of the app".into(),
            ));
        }
        r.check()?;
        Ok(r)
    }

    /// The limits and settings every recipe must keep.
    pub fn check(&self) -> Result<(), Error> {
        let r = self;
        if r.steps.len() > MAX_STEPS {
            return Err(Error::Invalid(format!(
                "a recipe has at most {MAX_STEPS} steps"
            )));
        }
        if r.questions.len() > MAX_QUESTIONS {
            return Err(Error::Invalid(format!(
                "a recipe asks at most {MAX_QUESTIONS} questions"
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for q in &r.questions {
            if !question_name_ok(&q.name) {
                return Err(Error::Invalid(format!(
                    "\"{}\" cannot name a question: use letters, digits, spaces, - or _ (up to 40)",
                    q.name.chars().take(60).collect::<String>()
                )));
            }
            if !seen.insert(q.name.trim().to_lowercase()) {
                return Err(Error::Invalid(format!(
                    "the recipe asks \"{}\" twice",
                    q.name.trim()
                )));
            }
            if q.default.chars().count() > MAX_ANSWER_CHARS || q.prompt.chars().count() > 500 {
                return Err(Error::Invalid(
                    "a question or its answer is too long".into(),
                ));
            }
        }
        for s in &r.steps {
            if let RecipeOp::Clean { spec } = s {
                spec.cleaner.check()?;
            }
            if let RecipeOp::CleanMatrix { spec } = s {
                spec.check()?;
            }
            if let RecipeOp::SortProblems { spec, .. } | RecipeOp::CheckColumn { spec, .. } = s {
                crate::validate::check_spec(spec)?;
            }
        }
        Ok(())
    }

    /// The text values of step `i` (where a question can go), with their
    /// place in the step's settings.
    pub fn step_texts(&self, i: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(Ok(v)) = self.steps.get(i).map(serde_json::to_value) {
            texts_of(&v, "", &mut out);
        }
        out
    }

    /// Turn the text at `path` in step `i` into question `name` (asked
    /// with `prompt`; the text there now is its first answer).
    pub fn ask(&mut self, i: usize, path: &str, name: &str, prompt: &str) -> Result<(), Error> {
        if !question_name_ok(name) {
            return Err(Error::Invalid(
                "name the question with letters, digits, spaces, - or _ (up to 40)".into(),
            ));
        }
        let name = name.trim();
        let step = self.steps.get(i).ok_or(Error::OutOfRange)?;
        let mut v = serde_json::to_value(step).map_err(|e| Error::Invalid(e.to_string()))?;
        let mut at = &mut v;
        for part in path.split('.') {
            at = match at {
                serde_json::Value::Object(m) => m.get_mut(part),
                serde_json::Value::Array(a) => {
                    part.parse::<usize>().ok().and_then(|k| a.get_mut(k))
                }
                _ => None,
            }
            .ok_or_else(|| Error::Invalid("that value is not in the step".into()))?;
        }
        let serde_json::Value::String(old) = at else {
            return Err(Error::Invalid("only a text value can be a question".into()));
        };
        let first = std::mem::replace(old, placeholder(name));
        let new: RecipeOp = serde_json::from_value(v).map_err(|e| Error::Invalid(e.to_string()))?;
        let mut next = self.clone();
        next.steps[i] = new;
        match next
            .questions
            .iter_mut()
            .find(|q| q.name.trim().eq_ignore_ascii_case(name))
        {
            Some(q) => {
                if !prompt.trim().is_empty() {
                    q.prompt = prompt.trim().to_string();
                }
            }
            None => next.questions.push(Question {
                name: name.to_string(),
                prompt: prompt.trim().to_string(),
                default: first,
            }),
        }
        next.check()?;
        *self = next;
        Ok(())
    }

    /// The recipe with its questions answered: each `{{name}}` in its steps
    /// becomes the answer given (by name, any case), else the question's
    /// last answer. Answers to questions it does not ask are refused, so a
    /// typo in `--set` is not silently ignored.
    pub fn answered(&self, answers: &[(String, String)]) -> Result<Recipe, Error> {
        for (k, a) in answers {
            if !self
                .questions
                .iter()
                .any(|q| q.name.trim().eq_ignore_ascii_case(k.trim()))
            {
                return Err(Error::Invalid(format!(
                    "the recipe does not ask \"{}\"{}",
                    k.trim(),
                    if self.questions.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " (it asks: {})",
                            self.questions
                                .iter()
                                .map(|q| q.name.trim())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                )));
            }
            if a.chars().count() > MAX_ANSWER_CHARS {
                return Err(Error::Invalid("an answer is too long".into()));
            }
        }
        if self.questions.is_empty() {
            return Ok(self.clone());
        }
        let fill: Vec<(String, String)> = self
            .questions
            .iter()
            .map(|q| {
                let a = answers
                    .iter()
                    .rev()
                    .find(|(k, _)| k.trim().eq_ignore_ascii_case(q.name.trim()))
                    .map_or(q.default.clone(), |(_, a)| a.clone());
                (placeholder(&q.name), a)
            })
            .collect();
        let mut out = self.clone();
        for s in &mut out.steps {
            let mut v = serde_json::to_value(&*s).map_err(|e| Error::Invalid(e.to_string()))?;
            fill_texts(&mut v, &fill);
            *s = serde_json::from_value(v)
                .map_err(|e| Error::Invalid(format!("an answer does not fit its step: {e}")))?;
        }
        for (q, (_, a)) in out.questions.iter_mut().zip(&fill) {
            q.default = a.clone();
        }
        out.check()?;
        Ok(out)
    }

    /// Column titles the table has that differ from the recipe's, in words;
    /// empty when they match (case and spaces around them ignored).
    pub fn column_problems(&self, doc: &Document) -> Vec<String> {
        let mut out = self.type_problems(doc);
        for s in &self.steps {
            if let RecipeOp::CleanMatrix { spec } = s {
                if let Err(e) = doc.matrix_by_title(spec) {
                    out.push(e.to_string());
                }
            }
        }
        if self.columns.is_empty() {
            return out;
        }
        let cols: Vec<u32> = (0..doc.col_count()).collect();
        let now = match self.title_row() {
            0 => doc.titles(&cols, self.header),
            r => doc.row_values(r).unwrap_or_default(),
        };
        let norm = |s: &str| s.trim().to_lowercase();
        for (i, want) in self.columns.iter().enumerate() {
            match now.get(i) {
                Some(have) if norm(have) == norm(want) => {}
                Some(have) => out.push(format!("column {} is \"{have}\", not \"{want}\"", i + 1)),
                None => out.push(format!("there is no column {} (\"{want}\")", i + 1)),
            }
        }
        if now.len() > self.columns.len() {
            out.push(format!(
                "the file has {} columns, the recipe {}",
                now.len(),
                self.columns.len()
            ));
        }
        out
    }
}

/// The column whose title is `title` (case and spaces around it ignored); a table with no titles falls back
/// to the position `col`.
fn column_by_title(titles: &[String], title: &str, col: u32, width: u32) -> Option<u32> {
    let norm = |s: &str| s.trim().to_lowercase();
    if title.trim().is_empty() {
        return (col < width).then_some(col);
    }
    titles
        .iter()
        .position(|t| norm(t) == norm(title))
        .map(|i| i as u32)
}

impl Recipe {
    /// The row that holds the column titles before the recipe runs: below
    /// the lines a first "set aside" step takes out of the table.
    fn title_row(&self) -> u64 {
        match self.steps.first() {
            Some(RecipeOp::SetAside { head, .. }) if self.header => u64::from(*head),
            _ => 0,
        }
    }

    /// A typed column the file has no title for, in words (empty when every one is there).
    fn type_problems(&self, doc: &Document) -> Vec<String> {
        let titles = doc.row_values(self.title_row()).unwrap_or_default();
        let mut out = Vec::new();
        for s in &self.steps {
            if let RecipeOp::ColumnTypes { types } = s {
                for t in types {
                    if column_by_title(&titles, &t.title, t.col, doc.col_count()).is_none() {
                        out.push(format!(
                            "there is no column titled \"{}\" to treat as {:?}",
                            t.title, t.kind
                        ));
                    }
                }
            }
        }
        out
    }
}

impl Document {
    /// The column types a [`RecipeOp::ColumnTypes`] step asks for in this table: each titled column found by its
    /// title, every other column Auto. A title the table does not have is an error.
    pub fn types_from_step(
        &self,
        types: &[ColumnTypeAt],
    ) -> Result<crate::coltype::ColumnTypes, Error> {
        let titles = self.row_values(0).unwrap_or_default();
        let mut set = crate::coltype::ColumnTypes::default();
        for t in types {
            let c = column_by_title(&titles, &t.title, t.col, self.col_count())
                .ok_or_else(|| Error::Invalid(format!("no column titled \"{}\"", t.title)))?;
            let p = self.phys_of(c).ok_or(Error::OutOfRange)?;
            set = set.with(p, t.kind);
        }
        Ok(set)
    }

    /// Run one recipe step as one undo step. `now` is the local date and
    /// time (days since 1970-01-01), `seed` for steps with random parts.
    pub fn run_recipe_step(
        &mut self,
        op: &RecipeOp,
        now: f64,
        seed: u64,
        progress: &Progress,
    ) -> Result<(), Error> {
        match op {
            RecipeOp::Sort { spec } => {
                let p = self.plan_sort(spec, progress)?;
                self.apply_sort(p)
            }
            RecipeOp::Filter { spec } => {
                let p = self.plan_filter(spec, progress)?;
                self.apply_filter(p)
            }
            RecipeOp::ClearFilter => {
                self.clear_filter();
                Ok(())
            }
            RecipeOp::InsertCols { at, count } => self.insert_cols(*at, *count),
            RecipeOp::DeleteCols { at, count } => self.delete_cols(*at, *count),
            RecipeOp::MoveCols { at, count, to } => self.move_cols(*at, *count, *to),
            RecipeOp::ReplaceAll { spec, with } => {
                let p = self.plan_replace(spec, with, 0, progress)?;
                self.apply_replace(p)
            }
            RecipeOp::BatchReplace { spec } => {
                let p = self.plan_batch_replace(spec, 0, progress)?;
                self.apply_replace(p)
            }
            RecipeOp::RemoveDuplicates { spec } => {
                let p = self.plan_remove_duplicates(spec, progress)?;
                self.apply_rows(p)
            }
            RecipeOp::RemoveBlankRows { spec } => {
                let p = self.plan_remove_blank_rows(spec, progress)?;
                self.apply_rows(p)
            }
            RecipeOp::RemoveBlankCols { spec } => {
                let p = self.plan_remove_blank_cols(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::SplitColumn { spec } => {
                let p = self.plan_split_column(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::MergeColumns { spec } => {
                let p = self.plan_merge_columns(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::SplitRows { spec } => {
                let p = self.plan_split_rows(spec, progress)?;
                self.apply_rows(p)
            }
            RecipeOp::MergeRows { spec } => {
                let p = self.plan_merge_rows(spec, progress)?;
                self.apply_rows(p)
            }
            RecipeOp::SortColumns {
                col,
                count,
                descending,
            } => {
                let p = self.plan_sort_columns(*col, *count, *descending)?;
                self.apply_cols(p)
            }
            RecipeOp::Clean { spec } => {
                let p = self.plan_clean(spec, now, seed, 0, progress)?;
                self.apply_clean(p)
            }
            RecipeOp::CleanMatrix { spec } => {
                let spec = self.matrix_by_title(spec)?;
                let p = self.plan_clean_matrix(&spec, now, seed, progress)?;
                self.apply_clean(p.plan)
            }
            RecipeOp::SplitName { spec } => {
                let p = self.plan_split_name(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::FillExample { spec } => {
                let (p, _) = self.plan_fill_example(spec, progress)?;
                self.apply_cells(p)
            }
            RecipeOp::Compute { spec } => {
                let (p, _) = self.plan_compute(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::FillBlanks { spec } => {
                // The guard column is found by its title (columns may be in another order).
                let mut spec = spec.clone();
                if let (Some(t), true) = (&spec.guard_title, spec.keep_first) {
                    let titles = self.row_values(0).unwrap_or_default();
                    let g = spec.guard.unwrap_or(0);
                    spec.guard = Some(
                        column_by_title(&titles, t, g, self.col_count())
                            .ok_or_else(|| Error::Invalid(format!("no column titled \"{t}\"")))?,
                    );
                }
                let p = self.plan_fill_blanks(&spec, progress)?;
                self.apply_rows(p)
            }
            RecipeOp::CountRepeats { spec } => {
                let p = self.plan_count_repeats(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::BlankRepeats { spec } => {
                let p = self.plan_blank_repeats(spec, progress)?;
                self.apply_rows(p)
            }
            RecipeOp::Breaks { spec } => {
                let p = self.plan_breaks(spec, progress)?;
                self.apply_rows(p)
            }
            RecipeOp::DeleteHidden => {
                let p = self.plan_delete_hidden()?;
                self.apply_rows(p)
            }
            RecipeOp::SetAside { head, foot } => {
                self.set_aside_ends(u64::from(*head), u64::from(*foot))
            }
            RecipeOp::JoinFields {
                usual,
                fields,
                groups,
                header,
                ..
            } => self
                .replay_join_fields(*usual, *fields, groups, *header, progress)
                .map(|_| ()),
            RecipeOp::RenameColumn { col, name } => self.set_first_row_cell(*col, name.clone()),
            RecipeOp::TidyTitles { spec } => self.tidy_titles(spec).map(|_| ()),
            RecipeOp::CompareColumn { spec } => {
                let p = self.plan_compare_column(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::ColumnTypes { types } => {
                let set = self.types_from_step(types)?;
                self.set_column_types(set);
                Ok(())
            }
            RecipeOp::MapValues { spec } => {
                let p = self.plan_map_values(spec, progress)?;
                self.apply_cells(p)
            }
            RecipeOp::GroupCalc { spec } => {
                let (p, _) = self.plan_group_calc(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::Arrange { spec } => {
                let p = self.plan_arrange_columns(spec)?;
                self.apply_cols(p)
            }
            RecipeOp::Extract { spec } => {
                let p = self.plan_extract(spec, progress)?;
                self.apply_cols(p)
            }
            RecipeOp::SortProblems { spec, scope } => {
                let spec = crate::validate::CheckSpec {
                    now,
                    ..spec.clone()
                };
                let rep = self.run_checks(&spec, progress)?;
                let p = self.plan_sort_problems(&rep, spec.keep_first, *scope)?;
                self.apply_sort(p)
            }
            RecipeOp::CheckColumn { spec, title } => {
                let spec = crate::validate::CheckSpec {
                    now,
                    ..spec.clone()
                };
                let rep = self.run_checks(&spec, progress)?;
                let p = self.plan_check_column(&spec, &rep, title, progress)?;
                self.apply_cols(p)
            }
        }
    }

    /// Run every step of a recipe, in order. Stops at the first step that
    /// fails, saying which (the steps before it stay done; undo takes
    /// them back one by one).
    pub fn run_recipe(
        &mut self,
        recipe: &Recipe,
        now: f64,
        seed: u64,
        progress: &Progress,
    ) -> Result<usize, Error> {
        // Questions not answered by the caller take their last answers.
        let recipe = &recipe.answered(&[])?;
        for (i, s) in recipe.steps.iter().enumerate() {
            self.run_recipe_step(
                s,
                now,
                seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
                progress,
            )
            .map_err(|e| match e {
                Error::Cancelled => Error::Cancelled,
                e => Error::Invalid(format!("step {} ({}): {e}", i + 1, s.describe())),
            })?;
        }
        Ok(recipe.steps.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::{CaseTo, Cleaner, Step};
    use crate::OpenOptions;

    fn doc(b: &str) -> Document {
        Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
    }

    /// C5 review 6: the guard is named by title, found by title on replay
    /// (columns in another order), and a missing title is refused.
    #[test]
    fn fill_guard_found_by_title() {
        let spec = crate::tools::FillBlanks {
            cols: vec![0],
            guard: Some(1),
            guard_title: Some("id".into()),
            stop_total_rows: true,
            spaces: true,
            keep_first: true,
            ..Default::default()
        };
        let step = RecipeOp::FillBlanks { spec };
        assert_eq!(
            step.describe(),
            "Fill blank cells, stopping at rows with no \u{201c}id\u{201d} and total rows"
        );
        let p = Progress::new();
        // Same order as recorded.
        let mut d = doc("k,id\na,1\n,2\n,\n,3\n");
        d.run_recipe_step(&step, 0.0, 0, &p).unwrap();
        assert_eq!((0..5).map(|r| d.cell(r, 0).unwrap()).collect::<Vec<_>>(), ["k", "a", "a", "", ""]);
        // Columns in another order: the guard follows its title, not its old place.
        let mut d = doc("id,k\n1,a\n2,\n,\n3,\n");
        let moved = RecipeOp::FillBlanks {
            spec: crate::tools::FillBlanks {
                cols: vec![1],
                guard: Some(1),
                guard_title: Some("ID ".into()),
                spaces: true,
                keep_first: true,
                ..Default::default()
            },
        };
        d.run_recipe_step(&moved, 0.0, 0, &p).unwrap();
        assert_eq!((0..5).map(|r| d.cell(r, 1).unwrap()).collect::<Vec<_>>(), ["k", "a", "a", "", ""]);
        // No such title: refused in the existing words.
        let mut d = doc("k,code\na,1\n,2\n");
        let e = d.run_recipe_step(&step, 0.0, 0, &p).err().unwrap().to_string();
        assert!(e.contains("no column titled \"id\""), "{e}");
    }

    fn recipe() -> Recipe {
        Recipe {
            version: RECIPE_VERSION,
            name: "tidy".into(),
            columns: vec!["name".into(), "city".into(), "junk".into()],
            header: true,
            steps: vec![
                RecipeOp::DeleteCols { at: 2, count: 1 },
                RecipeOp::Clean {
                    spec: serde_json::from_value(serde_json::json!({
                        "scope": { "kind": "columns", "cols": [1] },
                        "skip_first": true,
                        "cleaner": Cleaner {
                            version: 1,
                            name: String::new(),
                            steps: vec![Step::Case { to: CaseTo::Upper }],
                        },
                    }))
                    .unwrap(),
                },
                RecipeOp::Sort {
                    spec: SortSpec {
                        keys: vec![crate::sort::SortKey {
                            col: 0,
                            descending: false,
                            kind: Default::default(),
                            match_case: false,
                        }],
                        keep_first: true,
                        ..Default::default()
                    },
                },
            ],
            left_out: vec![],
            questions: vec![],
        }
    }

    #[test]
    fn runs_saved_steps_on_another_file() {
        let r = recipe();
        let json = r.to_json().unwrap();
        let back = Recipe::from_json(&json).unwrap();
        assert_eq!(back, r);
        let mut d = doc("name,city,junk\r\nbo,reno,x\r\nann,oslo,y\r\n");
        assert!(r.column_problems(&d).is_empty());
        assert_eq!(d.run_recipe(&back, 0.0, 1, &Progress::new()).unwrap(), 3);
        let rows: Vec<Vec<String>> = (0..3).map(|i| d.row_values(i).unwrap()).collect();
        assert_eq!(rows[0], ["name", "city"]);
        assert_eq!(rows[1], ["ann", "OSLO"]);
        assert_eq!(rows[2], ["bo", "RENO"]);
        // Each step is one undo step.
        assert!(d.undo() && d.undo() && d.undo());
        assert!(!d.undo());
    }

    /// B1: a Sanitize step (`#[serde(flatten)]` inside an internally tagged enum) is saved in a recipe, read back
    /// and run on another file.
    #[test]
    fn a_sanitize_step_round_trips_in_a_recipe_and_runs() {
        let r = Recipe {
            version: RECIPE_VERSION,
            name: "clean names".into(),
            columns: vec!["name".into()],
            header: true,
            steps: vec![RecipeOp::Clean {
                spec: serde_json::from_value(serde_json::json!({
                    "scope": { "kind": "columns", "cols": [0] },
                    "skip_first": true,
                    "cleaner": Cleaner {
                        version: 1,
                        name: String::new(),
                        steps: vec![Step::Sanitize { options: crate::xray::SanitizeOptions::usual() }],
                    },
                }))
                .unwrap(),
            }],
            left_out: vec![],
            questions: vec![],
        };
        let json = r.to_json().unwrap();
        let text = String::from_utf8(json.clone()).unwrap();
        assert!(
            text.contains("\"tool\": \"sanitize\"") || text.contains("\"tool\":\"sanitize\""),
            "{text}"
        );
        let back = Recipe::from_json(&json).unwrap();
        assert_eq!(back, r);
        let mut d = doc("name\r\nP\u{430}ul\r\nO\u{2019}Neil\r\nplain\r\n");
        assert!(r.column_problems(&d).is_empty());
        d.run_recipe(&back, 0.0, 1, &Progress::new()).unwrap();
        assert_eq!(d.cell(1, 0).unwrap(), "Paul");
        assert_eq!(d.cell(2, 0).unwrap(), "O'Neil");
        assert_eq!(d.cell(3, 0).unwrap(), "plain");
        assert!(d.undo());
        assert_eq!(d.cell(1, 0).unwrap(), "P\u{430}ul");
    }

    #[test]
    fn other_columns_and_bad_files_are_caught() {
        let r = recipe();
        let d = doc("id,town\r\n1,a\r\n");
        let p = r.column_problems(&d);
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(Recipe::from_json(b"{}").is_err());
        let newer = br#"{"version": 99, "steps": []}"#;
        assert!(Recipe::from_json(newer).is_err());
        // A failing step names itself.
        let mut d = doc("a\r\n1\r\n");
        let bad = Recipe {
            columns: vec![],
            steps: vec![RecipeOp::DeleteCols { at: 5, count: 1 }],
            ..recipe()
        };
        let e = d
            .run_recipe(&bad, 0.0, 1, &Progress::new())
            .unwrap_err()
            .to_string();
        assert!(e.contains("step 1"), "{e}");
    }

    #[test]
    fn questions_are_asked_and_answered() {
        let mut r = recipe();
        r.steps.push(RecipeOp::ReplaceAll {
            spec: FindSpec {
                pattern: crate::find::Pattern {
                    text: "RENO".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            with: "Hamilton".into(),
        });
        let texts = r.step_texts(3);
        let (path, _) = texts.iter().find(|(_, v)| v == "Hamilton").unwrap().clone();
        assert_eq!(path, "with");
        r.ask(3, &path, "county", "Which county?").unwrap();
        assert_eq!(r.questions[0].default, "Hamilton");
        assert!(matches!(&r.steps[3], RecipeOp::ReplaceAll { with, .. } if with == "{{county}}"));
        // Saved and read back, then answered.
        let back = Recipe::from_json(&r.to_json().unwrap()).unwrap();
        let a = back.answered(&[("County".into(), "Knox".into())]).unwrap();
        assert!(matches!(&a.steps[3], RecipeOp::ReplaceAll { with, .. } if with == "Knox"));
        assert_eq!(a.questions[0].default, "Knox");
        // Unanswered: the last answer.
        let a = back.answered(&[]).unwrap();
        assert!(matches!(&a.steps[3], RecipeOp::ReplaceAll { with, .. } if with == "Hamilton"));
        // A question it does not ask, and a bad name, are refused.
        assert!(back.answered(&[("state".into(), "TN".into())]).is_err());
        assert!(r.clone().ask(3, "with", "a{b}", "").is_err());
        assert!(r.clone().ask(0, "at", "n", "").is_err());
        // Run with the answer.
        let mut d = doc("name,city,junk
bo,reno,x
");
        d.run_recipe(&a, 0.0, 1, &Progress::new()).unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["bo", "Hamilton"]);
    }
}
