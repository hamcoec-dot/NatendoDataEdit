//! Address, ZIP code, phone and e-mail tidy steps (Milestone 14). Every
//! word list here is built in: nothing is looked up online.
//!
//! Each tidy returns the new text, or `None` when the cell could not be read
//! (it is then left as it is and counted).

use serde::{Deserialize, Serialize};

/// Street words: abbreviated or written out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressStyle {
    /// `123 North Main Street Apartment 4` -> `123 N Main St Apt 4`.
    #[default]
    Short,
    /// `123 N Main St Apt 4` -> `123 North Main Street Apartment 4`.
    Long,
}

/// How a ZIP code is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZipStyle {
    /// Five digits, with the four extra digits when the cell has them
    /// (`37402-1234`).
    #[default]
    Keep,
    /// Five digits only.
    Five,
}

/// How a US phone number is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneStyle {
    /// `423-555-0123`
    #[default]
    Dashes,
    /// `(423) 555-0123`
    Brackets,
    /// `423.555.0123`
    Dots,
    /// `4235550123`
    Digits,
    /// `+1 423-555-0123`
    International,
}

// ---- word lists ----

/// Street types: (short form, long form). The first short form of a long
/// word is the one written; other spellings are read too.
const STREET_TYPES: &[(&str, &str)] = &[
    ("Aly", "Alley"),
    ("Anx", "Annex"),
    ("Arc", "Arcade"),
    ("Ave", "Avenue"),
    ("Av", "Avenue"),
    ("Aven", "Avenue"),
    ("Avn", "Avenue"),
    ("Byu", "Bayou"),
    ("Bch", "Beach"),
    ("Bnd", "Bend"),
    ("Blf", "Bluff"),
    ("Blvd", "Boulevard"),
    ("Boul", "Boulevard"),
    ("Blv", "Boulevard"),
    ("Br", "Branch"),
    ("Brg", "Bridge"),
    ("Brk", "Brook"),
    ("Byp", "Bypass"),
    ("Cyn", "Canyon"),
    ("Cswy", "Causeway"),
    ("Ctr", "Center"),
    ("Cir", "Circle"),
    ("Crcl", "Circle"),
    ("Clf", "Cliff"),
    ("Clb", "Club"),
    ("Cmns", "Commons"),
    ("Cor", "Corner"),
    ("Crse", "Course"),
    ("Ct", "Court"),
    ("Crt", "Court"),
    ("Cts", "Courts"),
    ("Cv", "Cove"),
    ("Crk", "Creek"),
    ("Cres", "Crescent"),
    ("Xing", "Crossing"),
    ("Dl", "Dale"),
    ("Dm", "Dam"),
    ("Dv", "Divide"),
    ("Dr", "Drive"),
    ("Drv", "Drive"),
    ("Est", "Estate"),
    ("Ests", "Estates"),
    ("Expy", "Expressway"),
    ("Ext", "Extension"),
    ("Fls", "Falls"),
    ("Fry", "Ferry"),
    ("Fld", "Field"),
    ("Flds", "Fields"),
    ("Flt", "Flat"),
    ("Frd", "Ford"),
    ("Frst", "Forest"),
    ("Frg", "Forge"),
    ("Frk", "Fork"),
    ("Ft", "Fort"),
    ("Fwy", "Freeway"),
    ("Gdn", "Garden"),
    ("Gdns", "Gardens"),
    ("Gtwy", "Gateway"),
    ("Gln", "Glen"),
    ("Grn", "Green"),
    ("Grv", "Grove"),
    ("Hbr", "Harbor"),
    ("Hvn", "Haven"),
    ("Hts", "Heights"),
    ("Hwy", "Highway"),
    ("Hway", "Highway"),
    ("Hl", "Hill"),
    ("Hls", "Hills"),
    ("Holw", "Hollow"),
    ("Inlt", "Inlet"),
    ("Is", "Island"),
    ("Iss", "Islands"),
    ("Jct", "Junction"),
    ("Ky", "Key"),
    ("Knl", "Knoll"),
    ("Knls", "Knolls"),
    ("Lk", "Lake"),
    ("Lks", "Lakes"),
    ("Lndg", "Landing"),
    ("Ln", "Lane"),
    ("Lgt", "Light"),
    ("Lf", "Loaf"),
    ("Lck", "Lock"),
    ("Ldg", "Lodge"),
    ("Mnr", "Manor"),
    ("Mdw", "Meadow"),
    ("Mdws", "Meadows"),
    ("Ml", "Mill"),
    ("Mls", "Mills"),
    ("Msn", "Mission"),
    ("Mtwy", "Motorway"),
    ("Mt", "Mount"),
    ("Mtn", "Mountain"),
    ("Nck", "Neck"),
    ("Orch", "Orchard"),
    ("Opas", "Overpass"),
    ("Pkwy", "Parkway"),
    ("Pky", "Parkway"),
    ("Psge", "Passage"),
    ("Pne", "Pine"),
    ("Pnes", "Pines"),
    ("Pl", "Place"),
    ("Pln", "Plain"),
    ("Plns", "Plains"),
    ("Plz", "Plaza"),
    ("Pt", "Point"),
    ("Pts", "Points"),
    ("Prt", "Port"),
    ("Pr", "Prairie"),
    ("Radl", "Radial"),
    ("Rnch", "Ranch"),
    ("Rpd", "Rapid"),
    ("Rst", "Rest"),
    ("Rdg", "Ridge"),
    ("Riv", "River"),
    ("Rd", "Road"),
    ("Rds", "Roads"),
    ("Shl", "Shoal"),
    ("Shls", "Shoals"),
    ("Shr", "Shore"),
    ("Shrs", "Shores"),
    ("Skwy", "Skyway"),
    ("Spg", "Spring"),
    ("Spgs", "Springs"),
    ("Sq", "Square"),
    ("Sta", "Station"),
    ("Strm", "Stream"),
    ("St", "Street"),
    ("Str", "Street"),
    ("Sts", "Streets"),
    ("Smt", "Summit"),
    ("Ter", "Terrace"),
    ("Terr", "Terrace"),
    ("Trwy", "Throughway"),
    ("Trce", "Trace"),
    ("Trak", "Track"),
    ("Trfy", "Trafficway"),
    ("Trl", "Trail"),
    ("Trlr", "Trailer"),
    ("Tunl", "Tunnel"),
    ("Tpke", "Turnpike"),
    ("Upas", "Underpass"),
    ("Un", "Union"),
    ("Vly", "Valley"),
    ("Via", "Viaduct"),
    ("Vw", "View"),
    ("Vlg", "Village"),
    ("Vl", "Ville"),
    ("Vis", "Vista"),
    ("Wls", "Wells"),
];

/// Street types written the same either way.
const SAME_TYPES: &[&str] = &[
    "Loop", "Mall", "Mews", "Oval", "Park", "Pass", "Path", "Pike", "Ramp", "Row", "Run", "Spur",
    "Walk", "Way",
];

const DIRECTIONS: &[(&str, &str)] = &[
    ("N", "North"),
    ("S", "South"),
    ("E", "East"),
    ("W", "West"),
    ("NE", "Northeast"),
    ("NW", "Northwest"),
    ("SE", "Southeast"),
    ("SW", "Southwest"),
];

/// Unit words: (short, long).
const UNITS: &[(&str, &str)] = &[
    ("Apt", "Apartment"),
    ("Bsmt", "Basement"),
    ("Bldg", "Building"),
    ("Dept", "Department"),
    ("Fl", "Floor"),
    ("Frnt", "Front"),
    ("Hngr", "Hangar"),
    ("Lbby", "Lobby"),
    ("Lowr", "Lower"),
    ("Ofc", "Office"),
    ("Ph", "Penthouse"),
    ("Rm", "Room"),
    ("Spc", "Space"),
    ("Ste", "Suite"),
    ("Trlr", "Trailer"),
    ("Uppr", "Upper"),
];

/// Unit words written the same either way.
const SAME_UNITS: &[&str] = &["Unit", "Lot", "Pier", "Slip", "Stop"];

fn find<'a>(list: &'a [(&'a str, &'a str)], w: &str) -> Option<(&'a str, &'a str)> {
    let first_short = |long: &str| list.iter().find(|(_, l)| *l == long).map(|(s, _)| *s);
    list.iter()
        .find(|(s, l)| s.eq_ignore_ascii_case(w) || l.eq_ignore_ascii_case(w))
        .map(|&(_, l)| (first_short(l).unwrap_or(l), l))
}

/// `form` in capitals when the address is written in capitals; short
/// directions are always capitals.
fn styled(form: &str, upper: bool, direction: bool) -> String {
    if upper || (direction && form.len() <= 2) {
        form.to_uppercase()
    } else {
        form.to_string()
    }
}

/// A two-letter US state or territory code.
fn is_state(w: &str) -> bool {
    const STATES: &str = "AL AK AZ AR CA CO CT DE FL GA HI ID IL IN IA KS KY LA ME MD MA MI MN MS MO MT NE NV NH NJ NM NY NC ND OH OK OR PA RI SC SD TN TX UT VT VA WA WV WI WY DC PR VI GU AS MP";
    w.len() == 2 && STATES.split(' ').any(|s| s.eq_ignore_ascii_case(w))
}

fn is_type(w: &str) -> bool {
    find(STREET_TYPES, w).is_some() || SAME_TYPES.iter().any(|s| s.eq_ignore_ascii_case(w))
}

fn unit_word(w: &str) -> Option<(&'static str, &'static str)> {
    find(UNITS, w).or_else(|| {
        SAME_UNITS
            .iter()
            .find(|s| s.eq_ignore_ascii_case(w))
            .map(|&s| (s, s))
    })
}

/// A unit's number or letter: `4`, `4B`, `B`, `12-A`, `#4`.
fn unit_id(w: &str) -> bool {
    let w = w.trim_start_matches('#');
    !w.is_empty()
        && w.len() <= 8
        && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && (w.chars().any(|c| c.is_ascii_digit()) || w.chars().count() == 1)
}

/// Abbreviates or writes out the street type, directions and unit words of
/// a street address. Only the words in their usual places change: the
/// street type is the last such word before the unit (never the first word
/// of the street's name, as in `12 Court St`), directions come right after
/// the house number or right after the street type, and a unit word needs
/// a number or letter after it. Full stops after short words are dropped;
/// the rest of the cell is kept as it is.
pub fn address(t: &str, style: AddressStyle) -> Option<String> {
    if t.trim().is_empty() {
        return None;
    }
    let mut out = String::with_capacity(t.len() + 8);
    let mut first = true;
    for part in t.split(',') {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&address_part(part, style));
    }
    Some(out)
}

fn address_part(part: &str, style: AddressStyle) -> String {
    // Words with their spans; separators between them are kept.
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in part.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                words.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        words.push((s, part.len()));
    }
    if words.is_empty() {
        return part.to_string();
    }
    let bare = |k: usize| part[words[k].0..words[k].1].trim_end_matches('.');
    // Capitals when the part has letters and none are small.
    let upper = part.chars().any(char::is_alphabetic) && !part.chars().any(char::is_lowercase);
    // All small letters: the other words get a capital first letter
    // (`4410 n oak avenue` -> `4410 N Oak Ave`). Mixed case is left as
    // typed (`McDonald`).
    let lower = part.chars().any(char::is_alphabetic) && !part.chars().any(char::is_uppercase);
    let mut new: Vec<Option<String>> = vec![None; words.len()];
    // Units: a unit word with an id after it (or `#4`), anywhere.
    let mut unit_at = words.len();
    #[allow(clippy::needless_range_loop)] // `bare(k + 1)` looks ahead
    for k in 0..words.len() {
        if let Some((short, long)) = unit_word(bare(k)) {
            if k + 1 < words.len() && unit_id(bare(k + 1)) {
                let form = if style == AddressStyle::Short {
                    short
                } else {
                    long
                };
                new[k] = Some(styled(form, upper, false));
                unit_at = unit_at.min(k);
            }
        } else if bare(k).starts_with('#') && unit_id(bare(k)) {
            unit_at = unit_at.min(k);
        }
    }
    // The street: from the house number (if the part starts with one) to
    // the unit.
    let house = bare(0).chars().next().is_some_and(|c| c.is_ascii_digit());
    let name_from = usize::from(house);
    let end = unit_at;
    if house && end > name_from {
        // Street type: the last type word, not the first word of the name.
        let mut ty = None;
        for k in (name_from + 1..end).rev() {
            if is_type(bare(k)) {
                ty = Some(k);
                break;
            }
            // Only a direction may follow the type.
            if !(k + 1 == end && find(DIRECTIONS, bare(k)).is_some()) {
                break;
            }
        }
        if let Some(k) = ty {
            if let Some((short, long)) = find(STREET_TYPES, bare(k)) {
                let form = if style == AddressStyle::Short {
                    short
                } else {
                    long
                };
                new[k] = Some(styled(form, upper, false));
            } else {
                new[k] = Some(bare(k).to_string());
            }
        }
        let name_end = ty.unwrap_or(end);
        // A direction before the name, when a name follows it.
        if name_end > name_from + 1 {
            if let Some((short, long)) = find(DIRECTIONS, bare(name_from)) {
                let form = if style == AddressStyle::Short {
                    short
                } else {
                    long
                };
                new[name_from] = Some(styled(form, upper, true));
            }
        }
        // A direction after the type.
        if let Some(k) = ty {
            if k + 1 < end {
                if let Some((short, long)) = find(DIRECTIONS, bare(k + 1)) {
                    let form = if style == AddressStyle::Short {
                        short
                    } else {
                        long
                    };
                    new[k + 1] = Some(styled(form, upper, true));
                }
            }
        }
    }
    let next_is_zip = |k: usize| {
        k + 1 < words.len() && {
            let w = bare(k + 1);
            w.len() >= 5 && w.chars().take(5).all(|c| c.is_ascii_digit())
        }
    };
    let mut out = String::with_capacity(part.len() + 8);
    let mut at = 0;
    for (k, &(a, b)) in words.iter().enumerate() {
        out.push_str(&part[at..a]);
        match &new[k] {
            // Full stops after a changed word are dropped.
            Some(w) => out.push_str(w),
            None if lower && is_state(&part[a..b]) && (!house || next_is_zip(k)) => {
                out.push_str(&part[a..b].to_uppercase());
            }
            None if lower && part[a..b].starts_with(char::is_alphabetic) => {
                let w = &part[a..b];
                let mut cs = w.chars();
                if let Some(f) = cs.next() {
                    out.extend(f.to_uppercase());
                    out.push_str(cs.as_str());
                }
            }
            None => out.push_str(&part[a..b]),
        }
        at = b;
    }
    out.push_str(&part[at..]);
    out
}

/// A ZIP code: lost leading zeros put back (`2134` -> `02134`), the four
/// extra digits written `12345-6789`, a trailing `.0` from a spreadsheet
/// dropped. Not 3 to 5 or 7 to 9 digits: `None`.
pub fn zip(t: &str, style: ZipStyle) -> Option<String> {
    let s = t.trim();
    if s.is_empty() {
        return None;
    }
    let s = s.strip_suffix(".0").unwrap_or(s);
    let (five, four) = match s.split_once(['-', ' ']) {
        Some((a, b)) => {
            let b = b.trim();
            if !b.bytes().all(|c| c.is_ascii_digit()) || b.len() != 4 {
                return None;
            }
            (a.trim(), Some(b))
        }
        None => match s.len() {
            9 if s.bytes().all(|c| c.is_ascii_digit()) => (&s[..5], Some(&s[5..])),
            7 | 8 if s.bytes().all(|c| c.is_ascii_digit()) => {
                (&s[..s.len() - 4], Some(&s[s.len() - 4..]))
            }
            _ => (s, None),
        },
    };
    if five.is_empty()
        || five.len() > 5
        || five.len() < 3
        || !five.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let five = format!("{five:0>5}");
    Some(match (four, style) {
        (Some(f), ZipStyle::Keep) => format!("{five}-{f}"),
        _ => five,
    })
}

/// A US phone number in one style, an extension kept (` x123`). Needs 10
/// digits, or 11 starting with 1; anything else: `None`.
pub fn phone(t: &str, style: PhoneStyle) -> Option<String> {
    let s = t.trim();
    if s.is_empty() {
        return None;
    }
    // Extension: x, ext, ext., extension, #.
    let lower = s.to_ascii_lowercase();
    let mut cut = s.len();
    let mut ext = "";
    for key in ["extension", "ext.", "ext", "x", "#"] {
        if let Some(i) = lower.rfind(key) {
            let tail = s[i + key.len()..]
                .trim()
                .trim_start_matches(['.', ':'])
                .trim();
            if !tail.is_empty()
                && tail.len() <= 6
                && tail.bytes().all(|c| c.is_ascii_digit())
                && i < cut
            {
                cut = i;
                ext = &s[i + key.len()..];
            }
        }
    }
    let ext: String = ext.chars().filter(char::is_ascii_digit).collect();
    let body = &s[..cut];
    if body.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    let digits: String = body.chars().filter(char::is_ascii_digit).collect();
    let d = match digits.len() {
        10 => digits.as_str(),
        11 if digits.starts_with('1') => &digits[1..],
        _ => return None,
    };
    let (a, b, c) = (&d[..3], &d[3..6], &d[6..]);
    let mut out = match style {
        PhoneStyle::Dashes => format!("{a}-{b}-{c}"),
        PhoneStyle::Brackets => format!("({a}) {b}-{c}"),
        PhoneStyle::Dots => format!("{a}.{b}.{c}"),
        PhoneStyle::Digits => d.to_string(),
        PhoneStyle::International => format!("+1 {a}-{b}-{c}"),
    };
    if !ext.is_empty() {
        out.push_str(" x");
        out.push_str(&ext);
    }
    Some(out)
}

/// Common misspellings of large mail providers' domains.
const DOMAIN_FIXES: &[(&str, &str)] = &[
    ("gmial.com", "gmail.com"),
    ("gmai.com", "gmail.com"),
    ("gamil.com", "gmail.com"),
    ("gmal.com", "gmail.com"),
    ("gmaill.com", "gmail.com"),
    ("gmail.co", "gmail.com"),
    ("gmail.cm", "gmail.com"),
    ("gmail.con", "gmail.com"),
    ("hotmial.com", "hotmail.com"),
    ("hotmal.com", "hotmail.com"),
    ("hotmail.co", "hotmail.com"),
    ("hotmail.con", "hotmail.com"),
    ("yaho.com", "yahoo.com"),
    ("yahooo.com", "yahoo.com"),
    ("yahoo.co", "yahoo.com"),
    ("yahoo.con", "yahoo.com"),
    ("outlok.com", "outlook.com"),
    ("outloo.com", "outlook.com"),
    ("outlook.co", "outlook.com"),
    ("aol.co", "aol.com"),
    ("icloud.co", "icloud.com"),
    ("comcast.ent", "comcast.net"),
];

/// Whether `s` is an e-mail address with a sensible shape: one `@`, a name
/// of allowed characters, a domain of labels with a 2+ letter ending.
pub fn email_ok(s: &str) -> bool {
    let Some((name, domain)) = s.split_once('@') else {
        return false;
    };
    if name.is_empty()
        || name.len() > 64
        || domain.len() > 253
        || name.starts_with('.')
        || name.ends_with('.')
        || name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || "!#$%&'*+/=?^_`{|}~.-".contains(c))
    {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
        && labels
            .last()
            .is_some_and(|l| l.chars().count() >= 2 && l.chars().all(char::is_alphabetic))
}

/// An e-mail address tidied: spaces, `mailto:`, angle brackets, quotes and
/// trailing punctuation taken off, written in small letters, and (with
/// `fix_domains`) common misspellings of big providers' domains fixed. Not
/// an address afterwards: `None`.
pub fn email(t: &str, fix_domains: bool) -> Option<String> {
    let mut s = t.trim();
    if s.is_empty() {
        return None;
    }
    // "Name <a@b.com>": the address.
    if let (Some(a), Some(b)) = (s.rfind('<'), s.rfind('>')) {
        if a < b {
            s = &s[a + 1..b];
        }
    }
    // Words left: the one word with an `@` when nothing but `mailto:`
    // comes before it (`ann@x.org (work)`). A word before it may be part
    // of the address (`john smith@x.org`), so such values are left as they
    // are; words are never joined.
    let s = s.trim();
    let s = if s.contains(char::is_whitespace) {
        let words: Vec<&str> = s.split_whitespace().collect();
        let mut at = words.iter().enumerate().filter(|(_, w)| w.contains('@'));
        match (at.next(), at.next()) {
            (Some((i, w)), None)
                if !w.starts_with('@')
                    && !w.ends_with('@')
                    && words[..i].iter().all(|b| b.eq_ignore_ascii_case("mailto:")) =>
            {
                *w
            }
            _ => return None,
        }
    } else {
        s
    };
    let s = s.trim_matches(['"', '\'', '<', '>', '(', ')', ',', ';']);
    let s = if s.len() >= 7 && s[..7].eq_ignore_ascii_case("mailto:") {
        &s[7..]
    } else {
        s
    };
    let s = s.trim_end_matches(['.', ',', ';', ':']);
    let mut out = s.to_lowercase();
    while out.contains("@@") {
        out = out.replace("@@", "@");
    }
    if fix_domains {
        if let Some((name, domain)) = out.split_once('@') {
            if let Some((_, good)) = DOMAIN_FIXES.iter().find(|(bad, _)| *bad == domain) {
                out = format!("{name}@{good}");
            }
        }
    }
    email_ok(&out).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        let s = |t: &str| address(t, AddressStyle::Short).unwrap();
        let l = |t: &str| address(t, AddressStyle::Long).unwrap();
        assert_eq!(
            s("123 North Main Street Apartment 4"),
            "123 N Main St Apt 4"
        );
        assert_eq!(s("123 NORTH MAIN STREET"), "123 N MAIN ST");
        assert_eq!(s("12 Court Street"), "12 Court St");
        assert_eq!(s("500 Broad St. NW, Suite 200"), "500 Broad St NW, Ste 200");
        assert_eq!(s("100 North St"), "100 North St");
        assert_eq!(s("9 Park Ave"), "9 Park Ave");
        assert_eq!(s("44 Lee Highway Unit B"), "44 Lee Hwy Unit B");
        assert_eq!(s("PO Box 12"), "PO Box 12");
        assert_eq!(s("Chattanooga, TN 37402"), "Chattanooga, TN 37402");
        assert_eq!(
            l("123 N Main St Apt 4"),
            "123 North Main Street Apartment 4"
        );
        assert_eq!(l("7 E 4th Blvd SW"), "7 East 4th Boulevard Southwest");
        assert_eq!(l("1 Ste St"), "1 Ste Street");
        // Words that are not in their places stay.
        assert_eq!(s("The Avenue Cafe"), "The Avenue Cafe");
        assert!(address("  ", AddressStyle::Short).is_none());
        // Review L1: all small letters get capitals.
        assert_eq!(s("4410 n oak avenue apt 2"), "4410 N Oak Ave Apt 2");
        assert_eq!(s("chattanooga, tn 37402"), "Chattanooga, TN 37402");
        assert_eq!(s("12 mcdonald rd"), "12 Mcdonald Rd");
        // Mixed case stays as typed.
        assert_eq!(s("12 McDonald road"), "12 McDonald Rd");
    }

    #[test]
    fn zips() {
        assert_eq!(zip("2134", ZipStyle::Keep).as_deref(), Some("02134"));
        assert_eq!(zip("37402.0", ZipStyle::Keep).as_deref(), Some("37402"));
        assert_eq!(
            zip("374021234", ZipStyle::Keep).as_deref(),
            Some("37402-1234")
        );
        assert_eq!(
            zip("21341234", ZipStyle::Keep).as_deref(),
            Some("02134-1234")
        );
        assert_eq!(zip("37402 1234", ZipStyle::Five).as_deref(), Some("37402"));
        assert_eq!(zip("37402-1234", ZipStyle::Five).as_deref(), Some("37402"));
        assert_eq!(zip("TN 37402", ZipStyle::Keep), None);
        assert_eq!(zip("12", ZipStyle::Keep), None);
        assert_eq!(zip("123456", ZipStyle::Keep), None);
    }

    #[test]
    fn phones() {
        let p = |t: &str, s| phone(t, s);
        assert_eq!(
            p("(423) 555-0123", PhoneStyle::Dashes).as_deref(),
            Some("423-555-0123")
        );
        assert_eq!(
            p("1-423-555-0123", PhoneStyle::Brackets).as_deref(),
            Some("(423) 555-0123")
        );
        assert_eq!(
            p("423.555.0123 ext. 45", PhoneStyle::Dashes).as_deref(),
            Some("423-555-0123 x45")
        );
        assert_eq!(
            p("4235550123x7", PhoneStyle::International).as_deref(),
            Some("+1 423-555-0123 x7")
        );
        assert_eq!(
            p("+1 (423) 555 0123", PhoneStyle::Digits).as_deref(),
            Some("4235550123")
        );
        assert_eq!(p("555-0123", PhoneStyle::Dashes), None);
        assert_eq!(p("call me", PhoneStyle::Dashes), None);
        assert_eq!(p("+44 20 7946 0958", PhoneStyle::Dashes), None);
    }

    #[test]
    fn emails() {
        assert_eq!(
            email("  Ann.Lee@Example.COM ", false).as_deref(),
            Some("ann.lee@example.com")
        );
        assert_eq!(
            email("mailto:bo@x.org.", false).as_deref(),
            Some("bo@x.org")
        );
        assert_eq!(
            email("Bo Smith <BO@X.ORG>", false).as_deref(),
            Some("bo@x.org")
        );
        assert_eq!(email("cy@gmial.com", true).as_deref(), Some("cy@gmail.com"));
        assert_eq!(
            email("cy@gmial.com", false).as_deref(),
            Some("cy@gmial.com")
        );
        assert_eq!(email("dee@@x.net", false).as_deref(), Some("dee@x.net"));
        // Review M1: words are never joined.
        assert_eq!(email("dee @@ x.net", false), None);
        assert_eq!(
            email("Ann Smith <ann@x.org>", false).as_deref(),
            Some("ann@x.org")
        );
        assert_eq!(
            email("ann@x.org (work)", false).as_deref(),
            Some("ann@x.org")
        );
        // M14 round 2: a word before it may be part of the address.
        assert_eq!(email("john smith@x.org", false), None);
        assert_eq!(email("Ann Smith ann@x.org", false), None);
        assert_eq!(email("a b@x.org", false), None);
        assert_eq!(email("a@x.org b@y.org", false), None);
        assert_eq!(
            email("mailto: ann@x.org", false).as_deref(),
            Some("ann@x.org")
        );
        assert_eq!(email("not an address", false), None);
        assert_eq!(email("a@b", false), None);
        assert_eq!(email("a..b@c.com", false), None);
    }
}
