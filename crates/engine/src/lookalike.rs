//! Group look-alike values by sound and by spelling (the Value editor).
//!
//! Works on the distinct values of a column and their counts, never on every
//! row. Three methods: values that sound alike (Smith / Smyth, Catherine /
//! Kathryn), values spelled almost alike (Jonson / Johnson), or both.
//!
//! * [`sound_key`] is our own small Metaphone-style key for English names.
//! * [`near_spelling`] is a bounded edit distance where swapping two
//!   neighbouring letters is one edit (optimal string alignment).
//! * Close spellings are found with a delete-up-to-k index (every form of a
//!   value with up to k letters deleted; two values within k edits always
//!   share one), built in passes so the index stays small, then each
//!   candidate pair is checked with [`near_spelling`].
//! * A group is a most-common value (the hub) plus the values near it that no
//!   earlier hub took, so a chain of near-misses never merges unrelated names.

use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::Progress;
use crate::editing::{look_alike_key, Tally};
use crate::Error;

/// Most distinct values grouped at once.
pub const MAX_GROUPED: usize = 200_000;
/// Values with fewer letters and digits than this are left alone.
const MIN_LEN: usize = 3;
/// Values shorter than this are not grouped by spelling.
const MIN_SPELL: usize = 4;
/// Values longer than this are not grouped by spelling (the index grows with
/// the square of the length).
const MAX_SPELL: usize = 32;
/// A sound key with fewer letters than this says too little ("Lee" and "Lew"
/// are "L"; "Moore", "Mary" and "Mari" are "MR").
const MIN_SOUND: usize = 3;
/// Two values both seen at least this often are different names (Michael and
/// Michelle, Robert and Roberta), never two spellings of one: by either
/// method, they are not joined. A value seen less often than this can join a
/// common one (Smith 990 and Smyth 12). See [`different_names`] for small
/// files, where every count is small.
const COMMON: u64 = 20;
/// The rarer of two values must be seen at least this often, and at least a
/// quarter as often as the other, to be a name of its own in a small file
/// (Michael 13 and Michelle 10).
const OWN_NAME: u64 = 3;
/// A deleted form shared by more values than this is skipped (it is too
/// general to say anything); counted in [`Grouping::crowded`].
const MAX_CROWD: usize = 3_000;
/// Index entries held at once; more are done in several passes.
const PASS_ENTRIES: usize = 4_000_000;

/// How to decide that two values look alike.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Sound the same.
    #[default]
    Sounds,
    /// Spelled almost the same (one slip; two for long values).
    Spelling,
    /// Either of the two.
    Both,
}

/// The groups found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grouping {
    /// Groups of two or more: indexes into the values given, the suggested
    /// spelling first (most common; ties go to the properly capitalised one),
    /// groups in the order of their first member.
    pub groups: Vec<Vec<u32>>,
    /// Values too long for spelling grouping (left out of it).
    pub too_long: u32,
    /// Deleted forms shared by too many values to use.
    pub crowded: u32,
}

// ---- sound ----

/// A key that is the same for names that sound alike: letters only (accents
/// folded, case and spaces ignored), vowels dropped except a first one, silent
/// letters dropped, `PH`=`F`, `C`/`K`/`Q` alike, `TH`=`Q` (so it never reads as a digit), `GH`, `KN`, `WR`,
/// doubled letters merged, and so on. Digits stay as they are at the end, so
/// "Ward 1" and "Ward 2" never match.
pub fn sound_key(s: &str) -> String {
    let folded: Vec<char> = look_alike_key(s).chars().collect();
    let digits: String = folded.iter().filter(|c| c.is_ascii_digit()).collect();
    let w: Vec<char> = folded.into_iter().filter(|c| !c.is_ascii_digit()).collect();
    let n = w.len();
    let at = |i: usize, back: usize| -> char {
        if i < back {
            '\0'
        } else {
            w.get(i - back).copied().unwrap_or('\0')
        }
    };
    let next = |i: usize, ahead: usize| -> char { w.get(i + ahead).copied().unwrap_or('\0') };
    let vowel = |c: char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u');
    let mut out = String::with_capacity(n + digits.len());
    let mut i = 0usize;
    // A first vowel counts (as "A"); odd starts lose a letter.
    match (next(0, 0), next(0, 1)) {
        ('k', 'n') | ('g', 'n') | ('p', 'n') | ('w', 'r') => i = 1,
        ('a', 'e') => {
            out.push('A');
            i = 2;
        }
        ('x', _) => {
            out.push('S');
            i = 1;
        }
        ('w', 'h') => {
            out.push('W');
            i = 2;
        }
        (c, _) if vowel(c) => {
            out.push('A');
            i = 1;
        }
        _ => {}
    }
    while i < n {
        let c = w[i];
        let (p1, n1, n2) = (at(i, 1), next(i, 1), next(i, 2));
        // Doubled letters count once (except C: "Bacci").
        if c == p1 && c != 'c' {
            i += 1;
            continue;
        }
        match c {
            'a' | 'e' | 'i' | 'o' | 'u' => {}
            'b' => {
                // Silent at the end after M ("Plumb").
                if !(p1 == 'm' && i + 1 == n) {
                    out.push('B');
                }
            }
            'c' => {
                if p1 == 's' && matches!(n1, 'i' | 'e' | 'y') {
                    // "Scott"/"Scioli": the S says it.
                } else if n1 == 'i' && n2 == 'a' {
                    out.push('X');
                } else if n1 == 'h' {
                    out.push(if n2 == 'r' { 'K' } else { 'X' });
                    i += 1;
                } else if matches!(n1, 'i' | 'e' | 'y') {
                    out.push('S');
                } else {
                    out.push('K');
                }
            }
            'd' => {
                if n1 == 'g' && matches!(n2, 'e' | 'i' | 'y') {
                    out.push('J');
                    i += 1;
                } else {
                    out.push('T');
                }
            }
            'g' => {
                if n1 == 'h' {
                    // "Leigh", "Knight": silent unless a vowel follows.
                    if vowel(n2) {
                        out.push('K');
                    }
                    i += 1;
                } else if n1 == 'n' && i + 2 >= n {
                    // "Sign" at the end.
                } else if matches!(n1, 'i' | 'e' | 'y') && p1 != 'g' {
                    out.push('J');
                } else {
                    out.push('K');
                }
            }
            'h' => {
                if vowel(n1) && !matches!(p1, 'c' | 's' | 'p' | 't' | 'g') {
                    out.push('H');
                }
            }
            'k' => {
                if p1 != 'c' {
                    out.push('K');
                }
            }
            'p' => {
                if n1 == 'h' {
                    out.push('F');
                    i += 1;
                } else {
                    out.push('P');
                }
            }
            'q' => out.push('K'),
            's' => {
                if n1 == 'h' {
                    out.push('X');
                    i += 1;
                } else if n1 == 'i' && matches!(n2, 'o' | 'a') {
                    out.push('X');
                } else if n1 == 'c' && n2 == 'h' {
                    out.push_str("SK");
                    i += 2;
                } else {
                    out.push('S');
                }
            }
            't' => {
                if n1 == 'i' && matches!(n2, 'a' | 'o') {
                    out.push('X');
                } else if n1 == 'h' {
                    out.push('Q');
                    i += 1;
                } else if !(n1 == 'c' && n2 == 'h') {
                    out.push('T');
                }
            }
            'v' => out.push('F'),
            'w' => {
                if vowel(n1) {
                    out.push('W');
                }
            }
            'x' => out.push_str("KS"),
            'y' => {
                if vowel(n1) {
                    out.push('Y');
                }
            }
            'z' => out.push('S'),
            'f' | 'j' | 'l' | 'm' | 'n' | 'r' => out.push(c.to_ascii_uppercase()),
            other => out.push(other),
        }
        i += 1;
    }
    out.push_str(&digits);
    out
}

// ---- spelling ----

/// How many slips (a letter changed, added, dropped, or two neighbours
/// swapped) a pair may differ by, from the length of the shorter value.
fn spelling_limit(len: usize) -> usize {
    if len >= 8 {
        2
    } else if len >= MIN_SPELL {
        1
    } else {
        0
    }
}

/// True when `a` and `b` are at most `max` slips apart (changing, adding or
/// dropping a letter, or swapping two neighbours, is one slip each). Stops
/// as soon as the answer is known.
pub fn near_spelling(a: &[char], b: &[char], max: usize) -> bool {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max {
        return false;
    }
    let mut prev2 = vec![0usize; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    let mut last_best = 0usize;
    for i in 1..=n {
        cur[0] = i;
        let mut best = cur[0];
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
            best = best.min(v);
        }
        // A swap reaches back two rows, so both must be out of range.
        if best > max && last_best > max {
            return false;
        }
        last_best = best;
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m] <= max
}

/// FNV-1a over the letters left after deleting the positions in `skip`.
fn form_hash(key: &[char], skip: &[usize]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, c) in key.iter().enumerate() {
        if skip.contains(&i) {
            continue;
        }
        h ^= u64::from(*c as u32);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Mix in the length so the empty and short forms do not collide.
    h ^ ((key.len() - skip.len()) as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// Visit the hash of every form of `key` with up to `k` letters deleted.
fn each_form(key: &[char], k: usize, mut f: impl FnMut(u64)) {
    let l = key.len();
    f(form_hash(key, &[]));
    if k >= 1 {
        for p in 0..l {
            f(form_hash(key, &[p]));
        }
    }
    if k >= 2 {
        for p in 0..l {
            for q in p + 1..l {
                f(form_hash(key, &[p, q]));
            }
        }
    }
}

/// How many forms [`each_form`] visits.
fn form_count(l: usize, k: usize) -> usize {
    1 + if k >= 1 { l } else { 0 } + if k >= 2 { l * (l - 1) / 2 } else { 0 }
}

/// The pairs `(i, j)`, `i < j`, of `keys` (letters and digits, folded) that
/// are within the spelling limit of the shorter one. `eligible` lists the
/// indexes of `keys` to look at. Returns the pairs and the count of crowded
/// forms skipped.
fn spelling_pairs(
    keys: &[Vec<char>],
    eligible: &[u32],
    progress: &Progress,
    ok: &(dyn Fn(u32, u32) -> bool + Sync),
) -> Result<(Vec<(u32, u32)>, u32), Error> {
    let total: usize = eligible
        .iter()
        .map(|&i| {
            let l = keys[i as usize].len();
            form_count(l, spelling_limit(l))
        })
        .sum();
    let passes = total.div_ceil(PASS_ENTRIES).max(1);
    progress.start((passes * eligible.len()) as u64);
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut crowded = 0u32;
    for pass in 0..passes {
        let parts: Vec<Vec<(u64, u32)>> = eligible
            .par_chunks(1024)
            .map(|chunk| -> Result<Vec<(u64, u32)>, Error> {
                let mut part = Vec::new();
                for &i in chunk {
                    let key = &keys[i as usize];
                    each_form(key, spelling_limit(key.len()), |h| {
                        if passes == 1 || (h % passes as u64) as usize == pass {
                            part.push((h, i));
                        }
                    });
                }
                progress.step(chunk.len() as u64)?;
                Ok(part)
            })
            .collect::<Result<_, _>>()?;
        let mut entries: Vec<(u64, u32)> = Vec::with_capacity(parts.iter().map(Vec::len).sum());
        for part in parts {
            entries.extend(part);
        }
        entries.par_sort_unstable();
        entries.dedup();
        // Runs of equal hashes are the candidate sets.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut s = 0;
        while s < entries.len() {
            let mut e = s + 1;
            while e < entries.len() && entries[e].0 == entries[s].0 {
                e += 1;
            }
            if e - s >= 2 {
                if e - s > MAX_CROWD {
                    crowded += 1;
                } else {
                    runs.push((s, e));
                }
            }
            s = e;
        }
        let found: Vec<Vec<(u32, u32)>> = runs
            .par_iter()
            .map(|&(s, e)| {
                let mut out = Vec::new();
                let run = &entries[s..e];
                for x in 0..run.len() {
                    for y in x + 1..run.len() {
                        let (a, b) = (run[x].1, run[y].1);
                        let (ka, kb) = (&keys[a as usize], &keys[b as usize]);
                        let limit = spelling_limit(ka.len().min(kb.len()));
                        if limit > 0 && ok(a, b) && near_spelling(ka, kb, limit) {
                            out.push((a, b));
                        }
                    }
                }
                out
            })
            .collect();
        for f in found {
            pairs.extend(f);
        }
        if progress.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
    }
    pairs.par_sort_unstable();
    pairs.dedup();
    Ok((pairs, crowded))
}

// ---- grouping ----

/// How properly capitalised a value is: 2 for "Smith" or "Mary Ann" (each word
/// capital then small), 1 for other mixed case, 0 for all capitals or all
/// small letters.
fn capital_score(v: &str) -> u8 {
    let letters: Vec<char> = v.chars().filter(|c| c.is_alphabetic()).collect();
    let upper = letters.iter().filter(|c| c.is_uppercase()).count();
    if upper == 0 || upper == letters.len() {
        return 0;
    }
    let mut titled = true;
    let mut word_start = true;
    for c in v.chars() {
        if c.is_alphabetic() {
            if word_start != c.is_uppercase() {
                titled = false;
                break;
            }
            word_start = false;
        } else {
            word_start = true;
        }
    }
    if titled {
        2
    } else {
        1
    }
}

fn first_letter(key: &[char]) -> Option<char> {
    key.iter().copied().find(|c| c.is_alphabetic())
}

/// Two values so alike in how often they are seen that they are two names,
/// not a name and its misspelling: both seen [`COMMON`] times or more, or the
/// rarer seen [`OWN_NAME`] times or more and at least a quarter as often as
/// the other. A typo is a small share of its name; two real names are of
/// similar size, in a precinct as in a county. (Smith 19 with Smyth 2, or
/// Philip 29,871 with Phillip 4, are one name.) Values written with the same
/// letters (case, spaces and accents aside) are one name whatever their
/// counts.
fn different_names(a: &Tally, b: &Tally, same_letters: bool) -> bool {
    if same_letters {
        return false;
    }
    let (lo, hi) = (a.count.min(b.count), a.count.max(b.count));
    lo >= COMMON || (lo >= OWN_NAME && lo.saturating_mul(4) >= hi)
}

/// Can two values that sound or are spelled alike start with these letters?
/// The same letter, or a respelling of the first sound we know (Catherine and
/// Kathryn, Philip and Fillip, George and Jorge). "Garcia" and "Karisha"
/// share a sound key but not a start; "Jean" and "Sean", "Zack" and "Sack",
/// "Allen" and "Ellen" are one slip apart but are different names.
fn same_start(a: Option<char>, b: Option<char>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a == b
                || matches!(
                    (a.min(b), a.max(b)),
                    ('c', 'k') | ('c', 'q') | ('k', 'q') | ('c', 's') | ('f', 'p') | ('g', 'j')
                )
        }
        (None, None) => true,
        _ => false,
    }
}

/// Group the look-alikes among `values` (a column's distinct values, most
/// common first, as the Value editor lists them).
pub fn group_lookalikes(
    values: &[Tally],
    method: Method,
    progress: &Progress,
) -> Result<Grouping, Error> {
    if values.len() > MAX_GROUPED {
        return Err(Error::OutOfMemory(
            "Too many different values; filter first",
        ));
    }
    let n = values.len();
    let keys: Vec<Vec<char>> = values
        .iter()
        .map(|t| look_alike_key(&t.value).chars().collect())
        .collect();
    let digits: Vec<String> = keys
        .iter()
        .map(|k| k.iter().filter(|c| c.is_ascii_digit()).collect())
        .collect();
    let mut result = Grouping::default();
    // Sound buckets: values with one key.
    let mut bucket_of: Vec<u32> = vec![u32::MAX; n];
    let mut buckets: Vec<Vec<u32>> = Vec::new();
    if matches!(method, Method::Sounds | Method::Both) {
        let mut by_key: HashMap<String, u32> = HashMap::new();
        for i in 0..n {
            if keys[i].len() < MIN_LEN {
                continue;
            }
            let sk = sound_key(&values[i].value);
            let letters = sk.chars().filter(|c| !c.is_ascii_digit()).count();
            if letters < MIN_SOUND {
                continue;
            }
            let id = *by_key.entry(sk).or_insert_with(|| {
                buckets.push(Vec::new());
                (buckets.len() - 1) as u32
            });
            buckets[id as usize].push(i as u32);
            bucket_of[i] = id;
        }
        progress.step(n as u64)?;
    }
    // Spelling neighbours.
    let mut adj: Vec<Vec<u32>> = Vec::new();
    if matches!(method, Method::Spelling | Method::Both) {
        let mut eligible = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            if k.len() > MAX_SPELL {
                result.too_long += 1;
            } else if k.len() >= MIN_SPELL {
                eligible.push(i as u32);
            }
        }
        let ok = |a: u32, b: u32| -> bool {
            let (a, b) = (a as usize, b as usize);
            if digits[a] != digits[b] {
                return false;
            }
            // Two names are two names, and a name starts as it starts.
            !different_names(&values[a], &values[b], keys[a] == keys[b])
                && same_start(first_letter(&keys[a]), first_letter(&keys[b]))
        };
        let (pairs, crowded) = spelling_pairs(&keys, &eligible, progress, &ok)?;
        result.crowded = crowded;
        adj = vec![Vec::new(); n];
        for (a, b) in pairs {
            adj[a as usize].push(b);
            adj[b as usize].push(a);
        }
    }
    // Hubs in order of how common they are; each takes its unclaimed
    // neighbours.
    let mut taken = vec![false; n];
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for hub in 0..n {
        if taken[hub] {
            continue;
        }
        let mut members = vec![hub as u32];
        let near = adj.get(hub).map(Vec::as_slice).unwrap_or(&[]);
        let same_sound = match bucket_of[hub] {
            u32::MAX => &[][..],
            b => buckets[b as usize].as_slice(),
        };
        let hub_start = first_letter(&keys[hub]);
        let mut take = |m: u32| {
            let mi = m as usize;
            if mi != hub
                && !taken[mi]
                && !different_names(&values[hub], &values[mi], keys[hub] == keys[mi])
                && same_start(hub_start, first_letter(&keys[mi]))
                && !members.contains(&m)
            {
                members.push(m);
            }
        };
        for &m in near {
            take(m);
        }
        for &m in same_sound {
            take(m);
        }
        if members.len() < 2 {
            continue;
        }
        for &m in &members {
            taken[m as usize] = true;
        }
        members.sort_unstable_by(|&a, &b| {
            let (x, y) = (&values[a as usize], &values[b as usize]);
            y.count
                .cmp(&x.count)
                .then_with(|| capital_score(&y.value).cmp(&capital_score(&x.value)))
                .then_with(|| x.value.cmp(&y.value))
        });
        groups.push(members);
    }
    groups.sort_unstable_by_key(|g| g[0]);
    result.groups = groups;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn tallies(v: &[(&str, u64)]) -> Vec<Tally> {
        let mut t: Vec<Tally> = v
            .iter()
            .map(|(s, c)| Tally {
                value: s.to_string(),
                count: *c,
            })
            .collect();
        t.sort_by_key(|a| std::cmp::Reverse(a.count));
        t
    }

    fn names(t: &[Tally], g: &Grouping) -> Vec<Vec<String>> {
        g.groups
            .iter()
            .map(|m| m.iter().map(|&i| t[i as usize].value.clone()).collect())
            .collect()
    }

    /// Plain optimal-string-alignment distance, no limit.
    fn osa(a: &[char], b: &[char]) -> usize {
        let (n, m) = (a.len(), b.len());
        let mut d = vec![vec![0usize; m + 1]; n + 1];
        for (i, row) in d.iter_mut().enumerate() {
            row[0] = i;
        }
        for (j, cell) in d[0].iter_mut().enumerate() {
            *cell = j;
        }
        for i in 1..=n {
            for j in 1..=m {
                let cost = usize::from(a[i - 1] != b[j - 1]);
                d[i][j] = (d[i - 1][j] + 1)
                    .min(d[i][j - 1] + 1)
                    .min(d[i - 1][j - 1] + cost);
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
                }
            }
        }
        d[n][m]
    }

    #[test]
    fn names_that_sound_alike_share_a_key() {
        let k = sound_key;
        assert_eq!(k("Smith"), k("Smyth"));
        assert_eq!(k("Smith"), k("Smythe"));
        assert_eq!(k("Catherine"), k("Kathryn"));
        assert_eq!(k("Catherine"), k("Katherine"));
        assert_eq!(k("Catherine"), k("Cathryn"));
        assert_eq!(k("Johnson"), k("Jonson"));
        assert_eq!(k("Stephen"), k("Steven"));
        assert_eq!(k("Philip"), k("Fillip"));
        assert_eq!(k("Knight"), k("Night"));
        assert_ne!(k("Smith"), k("Jones"));
        assert_ne!(k("Anderson"), k("Andrews"));
        // Case, accents, spaces and punctuation do not matter.
        assert_eq!(k("O'Brien"), k("obrien"));
        assert_eq!(k("José"), k("Jose"));
        // Digits stay.
        assert_ne!(k("Ward 1"), k("Ward 2"));
        assert_eq!(k("Ward 1"), k("WARD 1"));
        // The "th" sound is not a digit: "Smith" is not "Smi" plus a 0.
        assert_ne!(k("Smith"), k("Smi0"));
    }

    #[test]
    fn sound_groups_leave_short_and_different_values_alone() {
        let t = tallies(&[
            ("Smith", 50),
            ("Smyth", 5),
            ("Smythe", 3),
            ("Lee", 40),
            ("Lew", 30),
            ("Jones", 20),
        ]);
        let g = group_lookalikes(&t, Method::Sounds, &Progress::new()).unwrap();
        assert_eq!(names(&t, &g), vec![vec!["Smith", "Smyth", "Smythe"]]);
    }

    #[test]
    fn spelling_groups_close_values() {
        let t = tallies(&[
            ("Johnson", 90),
            ("Jonson", 4),
            ("Jonhson", 2),
            ("Anne", 30),
            ("Annie", 3),
            ("Ann", 20),
            ("Lee", 40),
            ("Lew", 30),
            ("Smith", 60),
            ("Smyth", 6),
            ("Ward 1", 10),
            ("Ward 2", 9),
        ]);
        let g = group_lookalikes(&t, Method::Spelling, &Progress::new()).unwrap();
        let n = names(&t, &g);
        assert!(
            n.contains(&vec![
                "Johnson".to_string(),
                "Jonson".into(),
                "Jonhson".into()
            ]),
            "{n:?}"
        );
        assert!(
            n.contains(&vec!["Anne".to_string(), "Annie".into()]),
            "{n:?}"
        );
        assert!(
            n.contains(&vec!["Smith".to_string(), "Smyth".into()]),
            "{n:?}"
        );
        // Under four letters is left alone; digits alone never join.
        let flat: Vec<&String> = n.iter().flatten().collect();
        assert!(!flat.contains(&&"Lee".to_string()) && !flat.contains(&&"Ann".to_string()));
        assert!(!flat.contains(&&"Ward 1".to_string()));
    }

    #[test]
    fn swapped_letters_are_one_slip() {
        let t = tallies(&[("Cathy", 30), ("Cahty", 2)]);
        let g = group_lookalikes(&t, Method::Spelling, &Progress::new()).unwrap();
        assert_eq!(names(&t, &g), vec![vec!["Cathy", "Cahty"]]);
    }

    #[test]
    fn two_common_names_with_other_first_letters_are_not_joined() {
        // One slip apart, both common, first letters differ: two names.
        let t = tallies(&[("Mason", 80), ("Jason", 70), ("Jaso", 1)]);
        let g = group_lookalikes(&t, Method::Spelling, &Progress::new()).unwrap();
        assert!(names(&t, &g)
            .iter()
            .all(|m| !(m.contains(&"Mason".to_string()) && m.contains(&"Jason".to_string()))));
    }

    /// Two names that are both common are two people, by either method, even
    /// when they sound or are spelled alike.
    #[test]
    fn common_names_are_never_joined() {
        let sounds = [
            ("Michael", "Michelle"),
            ("Michael", "Mitchell"),
            ("Michelle", "Mitchell"),
            ("Robert", "Roberta"),
            ("Daniel", "Danielle"),
            ("Brown", "Byron"),
            ("Karen", "Green"),
            ("Martin", "Morton"),
            ("Allen", "Ellen"),
        ];
        let spellings = [("John", "Joan"), ("Mary", "Mark"), ("Robert", "Roberta")];
        for (a, b) in sounds.iter().chain(spellings.iter()) {
            for (ca, cb) in [(900, 40), (40, 900), (900, 800), (25, 20)] {
                let t = tallies(&[(a, ca), (b, cb)]);
                for method in [Method::Sounds, Method::Spelling, Method::Both] {
                    let g = group_lookalikes(&t, method, &Progress::new()).unwrap();
                    assert!(
                        g.groups.is_empty(),
                        "{a} ({ca}) + {b} ({cb}) joined by {method:?}: {:?}",
                        names(&t, &g)
                    );
                }
            }
        }
        // All three at once: Michael, Michelle and Mitchell stay three.
        let t = tallies(&[("Michael", 900), ("Michelle", 800), ("Mitchell", 40)]);
        assert!(group_lookalikes(&t, Method::Both, &Progress::new())
            .unwrap()
            .groups
            .is_empty());
    }

    /// The same in a small file (one precinct), where every count is small:
    /// two names of similar size are two names; a typo is a small share.
    #[test]
    fn small_files_keep_names_apart_too() {
        let pairs = [
            ("Michael", 13, "Michelle", 10),
            ("Robert", 12, "Roberta", 6),
            ("Daniel", 9, "Danielle", 7),
            ("Brown", 11, "Byron", 3),
            ("Cindy", 4, "Sandy", 3),
            ("John", 18, "Joan", 8),
            ("Mary", 16, "Mark", 14),
            ("Allen", 5, "Ellen", 4),
            ("Jean", 4, "Sean", 4),
            ("Zack", 3, "Sack", 2),
        ];
        for (a, ca, b, cb) in pairs {
            let t = tallies(&[(a, ca), (b, cb)]);
            for method in [Method::Sounds, Method::Spelling, Method::Both] {
                let g = group_lookalikes(&t, method, &Progress::new()).unwrap();
                assert!(
                    g.groups.is_empty(),
                    "{a} ({ca}) + {b} ({cb}) joined by {method:?}: {:?}",
                    names(&t, &g)
                );
            }
        }
        // A typo is a small share, however small the file.
        let t = tallies(&[
            ("Smith", 19),
            ("Smyth", 2),
            ("Philip", 30),
            ("Phillip", 4),
            ("Johnson", 9),
            ("Jonson", 2),
        ]);
        let g = group_lookalikes(&t, Method::Both, &Progress::new()).unwrap();
        let n = names(&t, &g);
        for pair in [
            ("Smith", "Smyth"),
            ("Philip", "Phillip"),
            ("Johnson", "Jonson"),
        ] {
            assert!(
                n.iter()
                    .any(|m| m.contains(&pair.0.to_string()) && m.contains(&pair.1.to_string())),
                "{pair:?} not joined: {n:?}"
            );
        }
    }

    /// One slip apart but different first letters: different names, whatever
    /// the counts (spelling as well as sound).
    #[test]
    fn spelling_pairs_need_the_same_start() {
        let t = tallies(&[
            ("Jean", 500),
            ("Sean", 1),
            ("Zack", 400),
            ("Sack", 1),
            ("Allen", 300),
            ("Ellen", 1),
        ]);
        for method in [Method::Spelling, Method::Both] {
            let g = group_lookalikes(&t, method, &Progress::new()).unwrap();
            assert!(g.groups.is_empty(), "{method:?}: {:?}", names(&t, &g));
        }
    }

    /// Case, spacing and accents make no new name, however common.
    #[test]
    fn the_same_letters_are_one_name() {
        let t = tallies(&[("Michael", 900), ("MICHAEL", 400), ("michael", 100)]);
        let g = group_lookalikes(&t, Method::Both, &Progress::new()).unwrap();
        assert_eq!(names(&t, &g), vec![vec!["Michael", "MICHAEL", "michael"]]);
    }

    /// A rare value still joins a common one.
    #[test]
    fn a_rare_value_joins_a_common_one() {
        let t = tallies(&[
            ("Smith", 990),
            ("Smyth", 12),
            ("Johnson", 880),
            ("Jonson", 3),
            ("Michael", 900),
            ("Micheal", 2),
            ("Catherine", 500),
            ("Kathryn", 3),
            ("Philip", 300),
            ("Fillip", 2),
        ]);
        let g = group_lookalikes(&t, Method::Both, &Progress::new()).unwrap();
        let n = names(&t, &g);
        for pair in [
            ("Smith", "Smyth"),
            ("Johnson", "Jonson"),
            ("Michael", "Micheal"),
            ("Catherine", "Kathryn"),
            ("Philip", "Fillip"),
        ] {
            assert!(
                n.iter()
                    .any(|m| m.contains(&pair.0.to_string()) && m.contains(&pair.1.to_string())),
                "{pair:?} not joined: {n:?}"
            );
        }
    }

    /// Sounding alike is not enough across first letters (Garcia and
    /// Karisha share a key): only known respellings of the first sound join.
    #[test]
    fn sound_alike_with_another_start_is_not_joined() {
        let t = tallies(&[
            ("Garcia", 34661),
            ("Karisha", 3),
            ("Keorrisha", 3),
            ("Kerisha", 1),
        ]);
        assert_eq!(sound_key("Garcia"), sound_key("Karisha"));
        let g = group_lookalikes(&t, Method::Sounds, &Progress::new()).unwrap();
        assert!(
            !names(&t, &g)
                .iter()
                .any(|m| m.contains(&"Garcia".to_string())),
            "{:?}",
            names(&t, &g)
        );
        let t = tallies(&[("Karen", 500), ("Green", 4), ("Koren", 2)]);
        let g = group_lookalikes(&t, Method::Sounds, &Progress::new()).unwrap();
        assert!(
            !names(&t, &g)
                .iter()
                .any(|m| m.contains(&"Green".to_string())),
            "{:?}",
            names(&t, &g)
        );
    }

    #[test]
    fn winner_is_the_most_common_then_the_capitalised() {
        let t = tallies(&[("SMITH", 10), ("Smith", 10), ("smith", 10), ("Smyth", 2)]);
        let g = group_lookalikes(&t, Method::Both, &Progress::new()).unwrap();
        assert_eq!(g.groups.len(), 1);
        let first = &t[g.groups[0][0] as usize].value;
        assert_eq!(first, "Smith");
        let g = group_lookalikes(
            &tallies(&[("Smyth", 40), ("Smith", 3)]),
            Method::Sounds,
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(g.groups[0][0], 0);
    }

    #[test]
    fn a_chain_of_near_names_does_not_merge() {
        // Marin ~ Martin ~ Marvin ~ Melvin ~ Kelvin: each one slip from the
        // next, but only the first hub's neighbours join it.
        let t = tallies(&[
            ("Martin", 50),
            ("Marin", 5),
            ("Marvin", 4),
            ("Melvin", 3),
            ("Kelvin", 2),
        ]);
        let g = group_lookalikes(&t, Method::Spelling, &Progress::new()).unwrap();
        assert_eq!(names(&t, &g)[0], vec!["Martin", "Marin", "Marvin"]);
        assert!(names(&t, &g)
            .iter()
            .all(|m| !m.contains(&"Kelvin".to_string()) || !m.contains(&"Martin".to_string())));
    }

    #[test]
    fn empty_one_and_odd_inputs() {
        let p = Progress::new();
        assert!(group_lookalikes(&[], Method::Both, &p)
            .unwrap()
            .groups
            .is_empty());
        let one = tallies(&[("Smith", 1)]);
        assert!(group_lookalikes(&one, Method::Both, &p)
            .unwrap()
            .groups
            .is_empty());
        let odd = tallies(&[
            ("", 3),
            ("日本語", 2),
            ("日本誌", 1),
            ("Ünïcödé", 1),
            ("Unicode", 4),
            ("12345", 2),
            ("12346", 1),
        ]);
        let g = group_lookalikes(&odd, Method::Both, &p).unwrap();
        // Accented letters fold, so these two are the same look-alike.
        assert!(names(&odd, &g)
            .iter()
            .any(|m| m.contains(&"Unicode".to_string()) && m.contains(&"Ünïcödé".to_string())));
        // Numbers that differ only in digits stay apart.
        assert!(!names(&odd, &g)
            .iter()
            .any(|m| m.contains(&"12345".to_string())));
    }

    #[test]
    fn too_many_values_say_so() {
        let many: Vec<Tally> = (0..MAX_GROUPED + 1)
            .map(|i| Tally {
                value: format!("v{i}"),
                count: 1,
            })
            .collect();
        let e = group_lookalikes(&many, Method::Sounds, &Progress::new()).unwrap_err();
        assert!(e.to_string().contains("Too many different values"));
    }

    #[test]
    fn cancel_stops_it() {
        let t: Vec<Tally> = (0..5000)
            .map(|i| Tally {
                value: format!("Name{i:05}x"),
                count: 1,
            })
            .collect();
        let p = Progress::new();
        p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            group_lookalikes(&t, Method::Spelling, &p),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn long_values_are_counted_not_grouped_by_spelling() {
        let long = "a".repeat(40);
        let t = tallies(&[(&long, 3), ("Smith", 2)]);
        let g = group_lookalikes(&t, Method::Spelling, &Progress::new()).unwrap();
        assert_eq!(g.too_long, 1);
    }

    fn small_word() -> impl Strategy<Value = String> {
        // A tiny alphabet makes near pairs common.
        proptest::collection::vec(proptest::sample::select(vec!['a', 'b', 'c', 'd']), 0..11)
            .prop_map(|v| v.into_iter().collect())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        /// The bounded check says what the plain distance says, up to 2.
        #[test]
        fn near_spelling_matches_the_full_distance(a in small_word(), b in small_word(), max in 0usize..3) {
            let (x, y) = (chars(&a), chars(&b));
            prop_assert_eq!(near_spelling(&x, &y, max), osa(&x, &y) <= max);
        }

        /// The delete index finds every pair that checking all pairs finds,
        /// including 2-slip pairs of long values and pairs of mixed length.
        #[test]
        fn index_finds_every_close_pair(words in proptest::collection::vec(small_word(), 2..40)) {
            let mut uniq: Vec<String> = words;
            uniq.sort();
            uniq.dedup();
            let keys: Vec<Vec<char>> = uniq.iter().map(|w| chars(w)).collect();
            let eligible: Vec<u32> = (0..keys.len() as u32)
                .filter(|&i| keys[i as usize].len() >= MIN_SPELL)
                .collect();
            let (found, _) = spelling_pairs(&keys, &eligible, &Progress::new(), &|_, _| true).unwrap();
            let mut all = Vec::new();
            for (x, &i) in eligible.iter().enumerate() {
                for &j in &eligible[x + 1..] {
                    let (a, b) = (&keys[i as usize], &keys[j as usize]);
                    let limit = spelling_limit(a.len().min(b.len()));
                    if osa(a, b) <= limit {
                        all.push((i.min(j), i.max(j)));
                    }
                }
            }
            all.sort_unstable();
            let mut found: Vec<(u32, u32)> = found.into_iter().map(|(a, b)| (a.min(b), a.max(b))).collect();
            found.sort_unstable();
            prop_assert_eq!(found, all);
        }
    }

    /// Speed gate: 200,000 distinct made-up names, every method, under 3 s
    /// each. Run in release: `cargo test --release -p natendo-engine speed_
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn speed_200k_names() {
        const SYL: [&str; 24] = [
            "ma", "ri", "an", "son", "ber", "ton", "el", "la", "ke", "dro", "vin", "sha", "ne",
            "ha", "li", "or", "ka", "tho", "wen", "pe", "sul", "mi", "gar", "den",
        ];
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut seen = std::collections::HashSet::new();
        let mut vals: Vec<Tally> = Vec::new();
        while vals.len() < 200_000 {
            let n = 2 + (rnd() % 4) as usize;
            let mut name = String::new();
            for _ in 0..n {
                name.push_str(SYL[(rnd() % 24) as usize]);
            }
            let mut c = name.chars();
            let name = c.next().unwrap().to_uppercase().collect::<String>() + c.as_str();
            if seen.insert(name.clone()) {
                let count = 1 + (rnd() % 2) + if rnd() % 500 == 0 { 40 } else { 0 };
                vals.push(Tally { value: name, count });
            }
        }
        vals.sort_by_key(|a| std::cmp::Reverse(a.count));
        for method in [Method::Sounds, Method::Spelling, Method::Both] {
            let t = std::time::Instant::now();
            let g = group_lookalikes(&vals, method, &Progress::new()).unwrap();
            let took = t.elapsed();
            println!(
                "{method:?}: {took:?}, {} groups, {} crowded",
                g.groups.len(),
                g.crowded
            );
            assert!(took.as_secs_f64() < 3.0, "{method:?} took {took:?}");
        }
    }

    #[test]
    fn index_finds_two_slip_pairs_of_long_values() {
        // Exactly two slips apart, 10 letters: a sample the one-deletion
        // index would miss.
        let keys = vec![
            chars("abcdefghij"),
            chars("abXdefgYij"),
            chars("abcdefghijk"),
            chars("bcdefghij"),
        ];
        let eligible: Vec<u32> = (0..4).collect();
        let (found, _) = spelling_pairs(&keys, &eligible, &Progress::new(), &|_, _| true).unwrap();
        assert!(found.contains(&(0, 1)));
        assert!(found.contains(&(0, 2)));
        assert!(found.contains(&(0, 3)));
    }
}
