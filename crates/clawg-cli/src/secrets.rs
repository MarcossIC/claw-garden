//! `clawg secrets` — what it prints, kept apart from the command so the
//! wording is under test without a store.

use clawg_core::memory::secrets::Flagged;

/// One line per row that holds a secret, then a line on what to do. Only ids,
/// kinds and whether the row is deleted: the secret itself never reaches the
/// terminal, where scrollback and logs would keep another copy.
pub fn report(flagged: &[Flagged]) -> String {
    let rows: Vec<[String; 3]> = flagged
        .iter()
        .map(|f| {
            let kinds: Vec<&str> = f.kinds.iter().map(|k| k.as_str()).collect();
            let state = if f.deleted { "deleted" } else { "live" };
            [f.id.to_string(), kinds.join(", "), state.to_string()]
        })
        .collect();
    let id_width = rows.iter().map(|r| r[0].len()).max().unwrap_or(0);
    let kinds_width = rows.iter().map(|r| r[1].len()).max().unwrap_or(0);

    let mut out = String::new();
    for [id, kinds, state] in &rows {
        out.push_str(&format!("memory {id:>id_width$}  {kinds:<kinds_width$}  {state}\n"));
    }
    match flagged {
        [] => out.push_str("no stored memory holds a secret of a known format"),
        [one] => out.push_str(&format!(
            "1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge {} --yes`.",
            one.id
        )),
        many => out.push_str(&format!(
            "{} memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`.",
            many.len()
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::memory::redact::SecretKind;

    #[test]
    fn the_secrets_report_lists_rows_and_never_their_content() {
        let flagged = [
            Flagged { id: 7, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
            Flagged { id: 1234, kinds: vec![SecretKind::GithubToken, SecretKind::Jwt], deleted: true },
        ];
        assert_eq!(
            report(&flagged),
            "memory    7  aws_access_key     live\n\
             memory 1234  github_token, jwt  deleted\n\
             2 memories hold a secret. To remove one for good, run `clawg stop`, then `clawg purge <ID> --yes`."
        );
        assert_eq!(
            report(&flagged[..1]),
            "memory 7  aws_access_key  live\n\
             1 memory holds a secret. To remove it for good, run `clawg stop`, then `clawg purge 7 --yes`."
        );
        assert_eq!(report(&[]), "no stored memory holds a secret of a known format");
    }
}
