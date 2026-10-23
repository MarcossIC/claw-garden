//! What an export from another memory tool becomes before it is stored: a
//! list of notes and the relations between them, in terms that belong to no
//! tool. An adapter reads its tool's file into this model and nothing else;
//! it opens no store and writes nothing.

use std::collections::BTreeMap;

use crate::memory::backup::days_from_civil;

/// Everything an adapter read out of one export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignExport {
    /// The tool the export came from.
    pub source: &'static str,
    /// The format version the file declared.
    pub version: String,
    /// In the order the file gave them.
    pub notes: Vec<ForeignNote>,
    pub relations: Vec<ForeignRelation>,
    /// What the file holds that has no place in a clawg row, counted by the
    /// name of the field (the entries of a list, one by one).
    pub unplaced: BTreeMap<String, u64>,
    /// Notes the source rewrote in place, whose earlier versions the file
    /// does not carry.
    pub rewritten: u64,
    /// Sessions that give no note because they have no summary.
    pub discarded_sessions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Observation,
    Prompt,
    SessionSummary,
}

/// A date the source wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Milliseconds since the epoch, UTC.
    At(i64),
    /// Missing or not a date: it is not a reason to refuse the file.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignNote {
    /// Names the note in its source, stable across exports of the same data.
    pub origin: String,
    pub kind: NoteKind,
    /// The type the source gave an observation; prompts and session
    /// summaries have none.
    pub source_type: Option<String>,
    pub title: String,
    pub content: String,
    /// The project name as the source wrote it.
    pub project: Option<String>,
    /// The source keeps it out of what its projects share.
    pub personal: bool,
    pub topic_key: Option<String>,
    pub session: Option<String>,
    /// When the text was written.
    pub written: When,
    /// How many times the source saw the same text again, as written.
    pub seen: u64,
    pub last_seen: Option<When>,
    pub deleted: Option<When>,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignRelation {
    /// The origins of the two notes.
    pub source: String,
    pub target: String,
    pub verb: String,
    pub judgment: String,
    pub by_user: bool,
    pub reason: Option<String>,
    pub evidence: Option<String>,
    pub created: When,
    pub updated: When,
    pub superseded: bool,
}

/// Why an export was refused whole.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForeignError {
    #[error("the {format} export is not JSON: {cause}")]
    NotJson { format: &'static str, cause: String },
    #[error("the {format} export is not a JSON object")]
    NotAnObject { format: &'static str },
    #[error("the {format} export has no \"version\" text")]
    NoVersion { format: &'static str },
    #[error("the {format} export is version {found}; this clawg reads {}", join_versions(.known))]
    UnknownVersion {
        format: &'static str,
        found: String,
        known: &'static [&'static str],
    },
    #[error("the {format} export does not have the shape of version {version}: {cause}")]
    Malformed {
        format: &'static str,
        version: String,
        cause: String,
    },
}

fn join_versions(known: &[&str]) -> String {
    match known {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// `YYYY-MM-DD HH:MM:SS` (UTC) or RFC 3339, either with a fraction of 1 to 9
/// digits, to milliseconds since the epoch. The fraction is truncated, never
/// rounded, so a time never reads as the next second.
pub(crate) fn parse_utc(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 19 || !text.is_ascii() {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> {
        let digits = b.get(from..to)?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| digits.iter().fold(0, |n, d| n * 10 + i64::from(d - b'0')))
    };
    if (b[4], b[7], b[13], b[16]) != (b'-', b'-', b':', b':') {
        return None;
    }
    let zoned = match b[10] {
        b' ' => false,
        b'T' => true,
        _ => return None,
    };
    let (y, mo, d) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (h, mi, s) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let mut end = 19;
    let mut milli = 0;
    if b.get(end) == Some(&b'.') {
        let digits = b[end + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        milli = (0..3).fold(0, |n, i| n * 10 + if i < digits { i64::from(b[end + 1 + i] - b'0') } else { 0 });
        end += 1 + digits;
    }
    let offset = match (zoned, &b[end..]) {
        (false, []) | (true, b"Z") => 0,
        (true, [sign @ (b'+' | b'-'), _, _, b':', _, _]) => {
            let (oh, om) = (number(end + 1, end + 3)?, number(end + 4, end + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let minutes = oh * 60 + om;
            if *sign == b'-' { -minutes } else { minutes }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo as u32, d as u32);
    Some(days * 86_400_000 + h * 3_600_000 + (mi - offset) * 60_000 + s * 1000 + milli)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON: i64 = 1_705_320_000_000; // 2024-01-15 12:00:00 UTC

    #[test]
    fn a_date_reads_with_or_without_a_fraction() {
        assert_eq!(parse_utc("2024-01-15 12:00:00"), Some(NOON));
        assert_eq!(parse_utc("2024-01-15 12:00:00.5"), Some(NOON + 500));
        assert_eq!(parse_utc("2024-01-15 12:00:00.000000000"), Some(NOON));
        assert_eq!(parse_utc("2024-01-15 12:00:00.123456789"), Some(NOON + 123));
        assert_eq!(parse_utc("2024-01-15T12:00:00Z"), Some(NOON));
        assert_eq!(parse_utc("2024-01-15T12:00:00.25Z"), Some(NOON + 250));
        assert_eq!(parse_utc("1970-01-01 00:00:00"), Some(0));
    }

    #[test]
    fn the_fraction_is_truncated_to_milliseconds() {
        assert_eq!(parse_utc("2024-01-15 12:00:00.999999999"), Some(NOON + 999));
        assert_eq!(parse_utc("2024-01-15T12:00:00.9999Z"), Some(NOON + 999));
        assert_eq!(parse_utc("2024-01-15 12:00:00.0019"), Some(NOON + 1));
    }

    #[test]
    fn an_offset_is_taken_out() {
        assert_eq!(parse_utc("2024-01-15T14:00:00+02:00"), Some(NOON));
        assert_eq!(parse_utc("2024-01-15T07:00:00-05:00"), Some(NOON));
        assert_eq!(parse_utc("2024-01-15T12:30:00+00:30"), Some(NOON));
    }

    #[test]
    fn a_leap_day_exists_only_in_a_leap_year() {
        assert_eq!(parse_utc("2024-02-29 00:00:00"), Some(1_709_164_800_000));
        assert_eq!(parse_utc("2000-02-29 00:00:00"), Some(951_782_400_000));
        assert_eq!(parse_utc("2023-02-29 00:00:00"), None);
        assert_eq!(parse_utc("1900-02-29 00:00:00"), None);
    }

    #[test]
    fn a_malformed_date_reads_as_nothing() {
        for text in [
            "",
            "yesterday",
            "2024-01-15",
            "2024-01-15 12:00",
            "2024-1-15 12:00:00",
            "2024-13-15 12:00:00",
            "2024-00-15 12:00:00",
            "2024-04-31 12:00:00",
            "2024-01-00 12:00:00",
            "2024-01-15 24:00:00",
            "2024-01-15 12:60:00",
            "2024-01-15 12:00:60",
            "2024-01-15 12:00:00.",
            "2024-01-15 12:00:00.1234567890",
            "2024-01-15 12:00:00 ",
            "2024-01-15 12:00:00Z",
            "2024-01-15 12:00:00+02:00",
            "2024-01-15T12:00:00",
            "2024-01-15T12:00:00+0200",
            "2024-01-15T12:00:00+24:00",
            "2024-01-15T12:00:00+02:60",
            "2024-01-15T12:00:00Zjunk",
            "2024-01-15_12:00:00",
            "+024-01-15 12:00:00",
            "2024-01-15 12:00:00.é",
        ] {
            assert_eq!(parse_utc(text), None, "{text:?}");
        }
    }

    #[test]
    fn the_known_versions_are_named_in_the_error() {
        let error = ForeignError::UnknownVersion {
            format: "engram",
            found: "0.3.0".into(),
            known: &["0.1.0", "0.2.0"],
        };
        assert_eq!(
            error.to_string(),
            "the engram export is version 0.3.0; this clawg reads 0.1.0 and 0.2.0"
        );
    }
}
