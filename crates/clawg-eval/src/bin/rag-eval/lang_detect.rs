//! `--lang-detect`: how the language detector clawg writes `memory.lang` with
//! labels the golden rows that carry a `lang`, as counts only: no row's text
//! leaves this module. Needs no model and no store.

use std::collections::BTreeMap;

use clawg_core::memory::language::detect;
use clawg_core::preferences::Lang;
use serde::Serialize;

use crate::golden::CorpusRow;

/// Rows of at least this many words carry the bar: a shorter one can be
/// honestly undecided.
pub const LONG_WORDS: usize = 8;

/// What the detector said of the rows of one label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub en: usize,
    pub es: usize,
    pub undecided: usize,
}

impl Counts {
    fn add(&mut self, said: Option<Lang>) {
        match said {
            Some(Lang::En) => self.en += 1,
            Some(Lang::Es) => self.es += 1,
            None => self.undecided += 1,
        }
    }

    fn total(&self) -> usize {
        self.en + self.es + self.undecided
    }
}

/// Shares of a set of labelled rows: right, the other language, undecided.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Rates {
    pub rows: usize,
    pub right: f64,
    pub crossed: f64,
    pub undecided: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    /// Every labelled row, by its label.
    pub by_label: BTreeMap<String, Counts>,
    /// The rows of [`LONG_WORDS`] words or more, by their label.
    pub long_by_label: BTreeMap<String, Counts>,
    pub all: Rates,
    pub long: Rates,
}

/// The confusion of `detect` over the rows labelled `en` or `es`; other
/// labels and rows without one are left out.
pub fn report(rows: &[CorpusRow]) -> Report {
    let mut out = Report::default();
    for row in rows {
        let Some(label) = row.lang.as_deref().filter(|l| Lang::parse(l).is_some()) else { continue };
        let said = detect(&row.content);
        out.by_label.entry(label.to_string()).or_default().add(said);
        if row.content.split_whitespace().count() >= LONG_WORDS {
            out.long_by_label.entry(label.to_string()).or_default().add(said);
        }
    }
    out.all = rates(&out.by_label);
    out.long = rates(&out.long_by_label);
    out
}

fn rates(by_label: &BTreeMap<String, Counts>) -> Rates {
    let (mut rows, mut right, mut crossed, mut undecided) = (0, 0, 0, 0);
    for (label, c) in by_label {
        rows += c.total();
        undecided += c.undecided;
        let (hit, cross) = if label == "en" { (c.en, c.es) } else { (c.es, c.en) };
        right += hit;
        crossed += cross;
    }
    let share = |n: usize| if rows == 0 { 0.0 } else { n as f64 / rows as f64 };
    Rates { rows, right: share(right), crossed: share(crossed), undecided: share(undecided) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(content: &str, lang: Option<&str>) -> CorpusRow {
        CorpusRow {
            ref_: "r".into(),
            content: content.into(),
            memory_type: "semantic_fact".into(),
            importance: 5.0,
            lang: lang.map(str::to_owned),
            age_days: 0.0,
        }
    }

    const SPANISH: &str = "El daemon guarda cada memoria en una sola transacción, y la búsqueda las encuentra.";
    const ENGLISH: &str = "The daemon stores every memory in one transaction, and the search finds them.";

    #[test]
    fn lang_detect_counts_by_label() {
        let rows = [
            row(SPANISH, Some("es")),
            row(ENGLISH, Some("en")),
            row(ENGLISH, Some("es")),
            row("se rompió", Some("es")),
            row(SPANISH, None),
            row(SPANISH, Some("fr")),
        ];
        let r = report(&rows);
        assert_eq!(r.by_label["es"], Counts { en: 1, es: 1, undecided: 1 });
        assert_eq!(r.by_label["en"], Counts { en: 1, es: 0, undecided: 0 });
        assert_eq!(r.by_label.len(), 2, "only en and es are labels");
        assert_eq!(r.long_by_label["es"], Counts { en: 1, es: 1, undecided: 0 }, "the two-word row is not long");
        assert_eq!((r.all.rows, r.all.right, r.all.crossed, r.all.undecided), (4, 0.5, 0.25, 0.25));
        assert_eq!((r.long.rows, r.long.right), (3, 2.0 / 3.0));
    }

    #[test]
    fn lang_detect_prints_no_text() {
        let json = serde_json::to_string(&report(&[row(SPANISH, Some("es")), row(ENGLISH, Some("en"))])).unwrap();
        assert!(!json.contains("daemon") && !json.contains("memoria"), "{json}");
    }
}
