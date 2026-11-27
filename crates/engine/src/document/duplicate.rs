//! Duplicate: an unsaved copy of a document as it is now (edits, column map,
//! column types, sort, filter and hidden rows included), with no file and an
//! empty undo history.
//!
//! The copy owns its bytes. A file-backed document maps its file, and a map
//! shared with the copy would keep the file mapped after the original closes,
//! which on Windows stops a save of the original replacing its own file.

use super::*;

/// Bytes copied between two looks at Cancel.
const CHUNK: usize = 32 << 20;
/// Edited rows copied between two looks at Cancel.
const ROWS_CHUNK: usize = 65_536;

impl Document {
    /// Memory a duplicate takes: the bytes, the row index and the edited rows.
    pub fn duplicate_bytes(&self) -> u64 {
        let edited: usize = self.overlay.rows.values().map(|e| 48 + e.cost()).sum();
        (self.source.bytes().len() + self.index.memory_bytes() + edited) as u64
    }

    /// An unsaved copy (see the module notes). Cancel through `progress`
    /// stops it with [`Error::Cancelled`] and nothing kept.
    pub fn duplicate(&self, progress: &Progress) -> Result<Document, Error> {
        if !self.is_indexed() {
            return Err(Error::StillLoading);
        }
        let rows = self.total_rows().max(1);
        progress.total.store(rows, Ordering::Relaxed);
        let stop = || progress.cancel.load(Ordering::Relaxed);
        let src = self.source.bytes();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(src.len())
            .map_err(|_| Error::Io(std::io::Error::from(std::io::ErrorKind::OutOfMemory)))?;
        for part in src.chunks(CHUNK) {
            if stop() {
                return Err(Error::Cancelled);
            }
            bytes.extend_from_slice(part);
            let share = bytes.len() as f64 / src.len().max(1) as f64;
            progress
                .done
                .store((share * rows as f64 * 0.9) as u64, Ordering::Relaxed);
        }
        let mut rows_map = HashMap::with_capacity(self.overlay.rows.len());
        for (i, (k, v)) in self.overlay.rows.iter().enumerate() {
            if i % ROWS_CHUNK == 0 && stop() {
                return Err(Error::Cancelled);
            }
            rows_map.insert(*k, v.clone());
        }
        let overlay = Overlay {
            rows: rows_map,
            widths: self.overlay.widths.clone(),
        };
        let mut doc = Document {
            source: Source::from_vec(bytes),
            data_start: self.data_start,
            preamble_len: self.preamble_len,
            preamble_lines: self.preamble_lines,
            data_end: self.data_end,
            notes: self.notes.clone(),
            detection: self.detection.clone(),
            format: self.format,
            lossy: self.lossy,
            invalid_bytes: self.invalid_bytes,
            index: self.index.clone(),
            order: self.order.clone(),
            file_width: self.file_width,
            overlay,
            next_id: self.next_id,
            cols: self.cols.clone(),
            column_types: self.column_types.clone(),
            next_col: self.next_col,
            swapped: self.swapped.clone(),
            map_identity: AtomicU8::new(0),
            view: self.view.clone(),
            rule_cache: crate::chips::new_cache(),
            json_shape: self.json_shape.clone(),
            revision: 0,
            data_revision: 0,
            rows_revision: 0,
            touched: Vec::new(),
            touch_floor: 0,
            undo_bytes: 0,
            redo_bytes: 0,
            undo_budget: self.undo_budget,
            rebase_work_share: self.rebase_work_share,
            last_history: None,
            undo: Vec::new(),
            redo: Vec::new(),
            undo_ids: Default::default(),
            redo_ids: Vec::new(),
            uid: next_step_id(),
            base_mark: next_step_id(),
            clean_depth: None,
            history_pos: 0,
            protected: self.protected.clone(),
            protected_skipped: AtomicU64::new(0),
            formulas: self.formulas.clone(),
        };
        // Unsaved from the start: closing it asks to save.
        doc.mark_modified();
        if stop() {
            return Err(Error::Cancelled);
        }
        progress.done.store(rows, Ordering::Relaxed);
        Ok(doc)
    }
}
