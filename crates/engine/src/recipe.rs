//! Recipes (Milestone 14): the steps done to a table, kept so they can be
//! run again on another file, in the app or with `natendo-cli run`. A
//! recipe is plain JSON; each step is one of the operations below, with
//! its settings, and runs as it would from the menus (plan, then apply).

use serde::{Deserialize, Serialize};

use crate::clean::{CleanSpec, SplitName};
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
}

impl RecipeOp {
    /// What the step does, in a few words.
    pub fn describe(&self) -> String {
        match self {
            RecipeOp::Sort { .. } => "Sort the rows".into(),
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
            RecipeOp::SplitName { .. } => "Split names".into(),
            RecipeOp::FillExample { .. } => "Fill by example".into(),
            RecipeOp::Compute { spec } => format!("Computed column: {}", spec.expr),
            RecipeOp::FillBlanks { .. } => "Fill blank cells".into(),
            RecipeOp::CountRepeats { .. } => "Count repeats".into(),
            RecipeOp::BlankRepeats { .. } => "Blank out repeats".into(),
            RecipeOp::Breaks { .. } => "Add breaks between groups".into(),
            RecipeOp::DeleteHidden => "Delete hidden rows".into(),
            RecipeOp::RenameColumn { col, name } => {
                format!("Rename column {} to \"{}\"", col + 1, name)
            }
            RecipeOp::TidyTitles { .. } => "Tidy column titles".into(),
            RecipeOp::CompareColumn { spec } => {
                format!("Compare columns {} and {}", spec.a + 1, spec.b + 1)
            }
            RecipeOp::MapValues { .. } => "Change values from a list".into(),
            RecipeOp::GroupCalc { .. } => "Group figures".into(),
            RecipeOp::Arrange { .. } => "Arrange columns".into(),
            RecipeOp::Extract { .. } => "Extract a piece into a column".into(),
            RecipeOp::SortProblems { .. } => "Put rows that fail the checks first".into(),
            RecipeOp::CheckColumn { .. } => "Add a check result column".into(),
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
            if let RecipeOp::SortProblems { spec } | RecipeOp::CheckColumn { spec, .. } = s {
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
        if self.columns.is_empty() {
            return Vec::new();
        }
        let cols: Vec<u32> = (0..doc.col_count()).collect();
        let now = doc.titles(&cols, self.header);
        let norm = |s: &str| s.trim().to_lowercase();
        let mut out = Vec::new();
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

impl Document {
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
                let p = self.plan_fill_blanks(spec, progress)?;
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
            RecipeOp::RenameColumn { col, name } => self.set_first_row_cell(*col, name.clone()),
            RecipeOp::TidyTitles { spec } => self.tidy_titles(spec).map(|_| ()),
            RecipeOp::CompareColumn { spec } => {
                let p = self.plan_compare_column(spec, progress)?;
                self.apply_cols(p)
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
            RecipeOp::SortProblems { spec } => {
                let spec = crate::validate::CheckSpec {
                    now,
                    ..spec.clone()
                };
                let rep = self.run_checks(&spec, progress)?;
                let p = self.plan_sort_problems(&rep, spec.keep_first)?;
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
