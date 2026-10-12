//! Snapshots of `memory.db`, and the schema check that decides when one is due.

/// One migration as the schema plan compares it: applied to a database
/// (`refinery_schema_history`) or embedded in this binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationId {
    pub version: i32,
    pub name: String,
    pub checksum: u64,
}

/// What opening a database with this binary would do to its schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaPlan {
    /// No migration applied yet: a new database.
    Fresh,
    /// Exactly this binary's migrations.
    UpToDate,
    /// A prefix of this binary's migrations; `from + 1 ..= to` are pending.
    Pending { from: i32, to: i32 },
    /// Written by a newer binary: version `db` is past the last one this
    /// binary knows, `binary`.
    Newer { db: i32, binary: i32 },
    /// Applied version `version` differs from this binary's (name or
    /// checksum), or is a hole this binary would never fill.
    Diverged { version: i32 },
}

/// Compares a database's applied migrations with this binary's.
///
/// A divergence inside the range this binary knows wins over `Newer`: a
/// binary that disagrees about its own migrations has a different problem
/// than one that is merely older.
pub fn schema_plan(applied: &[MigrationId], known: &[MigrationId]) -> SchemaPlan {
    let Some(db) = applied.iter().map(|m| m.version).max() else {
        return SchemaPlan::Fresh;
    };
    let binary = known.iter().map(|m| m.version).max().unwrap_or(0);

    let mut in_range: Vec<&MigrationId> = applied.iter().filter(|m| m.version <= binary).collect();
    in_range.sort_by_key(|m| m.version);
    let mut expected: Vec<&MigrationId> = known.iter().collect();
    expected.sort_by_key(|m| m.version);

    for (i, a) in in_range.iter().enumerate() {
        match expected.get(i) {
            Some(k) if k.version == a.version => {
                if k.name != a.name || k.checksum != a.checksum {
                    return SchemaPlan::Diverged { version: a.version };
                }
            }
            // The binary has a migration the database skipped, or the
            // database has one the binary never had.
            Some(k) => return SchemaPlan::Diverged { version: k.version.min(a.version) },
            None => return SchemaPlan::Diverged { version: a.version },
        }
    }

    if db > binary {
        SchemaPlan::Newer { db, binary }
    } else if db < binary {
        SchemaPlan::Pending { from: db, to: binary }
    } else {
        SchemaPlan::UpToDate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(version: i32, checksum: u64) -> MigrationId {
        MigrationId { version, name: format!("m{version}"), checksum }
    }

    fn range(to: i32) -> Vec<MigrationId> {
        (1..=to).map(|v| m(v, v as u64 * 10)).collect()
    }

    #[test]
    fn schema_plan_covers_every_history_shape() {
        let known = range(8);

        assert_eq!(schema_plan(&[], &known), SchemaPlan::Fresh);
        assert_eq!(schema_plan(&range(8), &known), SchemaPlan::UpToDate);
        assert_eq!(schema_plan(&range(5), &known), SchemaPlan::Pending { from: 5, to: 8 });
        assert_eq!(schema_plan(&range(9), &known), SchemaPlan::Newer { db: 9, binary: 8 });

        let mut altered = range(8);
        altered[2].checksum = 1;
        assert_eq!(schema_plan(&altered, &known), SchemaPlan::Diverged { version: 3 });

        let mut renamed = range(8);
        renamed[3].name = "other".into();
        assert_eq!(schema_plan(&renamed, &known), SchemaPlan::Diverged { version: 4 });

        let holed: Vec<MigrationId> = range(8).into_iter().filter(|m| m.version != 6).collect();
        assert_eq!(schema_plan(&holed, &known), SchemaPlan::Diverged { version: 6 });
    }

    #[test]
    fn a_divergence_the_binary_knows_about_wins_over_newer() {
        let mut applied = range(9);
        applied[2].checksum = 1;
        assert_eq!(schema_plan(&applied, &range(8)), SchemaPlan::Diverged { version: 3 });
    }

    #[test]
    fn history_order_does_not_matter() {
        let mut applied = range(5);
        applied.reverse();
        assert_eq!(schema_plan(&applied, &range(8)), SchemaPlan::Pending { from: 5, to: 8 });
    }
}
