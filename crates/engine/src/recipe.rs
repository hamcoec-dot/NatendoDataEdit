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
        if r.steps.len() > MAX_STEPS {
            return Err(Error::Invalid(format!(
                "a recipe has at most {MAX_STEPS} steps"
            )));
        }
        for s in &r.steps {
            if let RecipeOp::Clean { spec } = s {
                spec.cleaner.check()?;
            }
            if let RecipeOp::SortProblems { spec } | RecipeOp::CheckColumn { spec, .. } = s {
                crate::validate::check_spec(spec)?;
            }
        }
        Ok(r)
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
}
