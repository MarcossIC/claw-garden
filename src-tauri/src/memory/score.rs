//! Registers the five SQLITE_DETERMINISTIC scalar UDFs used by the score formula.
//! Called from MemoryStore::open() and MemoryStore::new_for_test().

use crate::memory::MemoryError;

/// Registers `exp_decay`, `lambda_for_type`, `importance_factor`, `dispute_factor`,
/// `cosine_from_l2_unit` on the connection. ALL five UDFs share
/// `SQLITE_UTF8 | SQLITE_DETERMINISTIC` flags.
pub(crate) async fn register_score_udfs(
    conn: &tokio_rusqlite::Connection,
) -> Result<(), MemoryError> {
    conn.call(|c| {
        use rusqlite::functions::FunctionFlags;
        let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;

        // UDF #1: exp_decay(age_secs INTEGER, lambda REAL) -> REAL
        c.create_scalar_function("exp_decay", 2, det, |ctx| {
            let age_secs: i64 = ctx.get(0)?;
            let lambda: f64 = ctx.get(1)?;
            let age_days = age_secs as f64 / 86_400.0;
            Ok((-lambda * age_days).exp())
        })?;

        // UDF #2: lambda_for_type(memory_type TEXT) -> REAL
        c.create_scalar_function("lambda_for_type", 1, det, |ctx| {
            let memory_type: String = ctx.get(0)?;
            let lambda = match memory_type.as_str() {
                "user_preference" => 0.0019_f64,
                "architectural_decision" => 0.0039_f64,
                "session_summary" => 0.0077_f64,
                "semantic_fact" => 0.0039_f64,
                "chat_message" => 0.023_f64,
                "tool_error" => 0.099_f64,
                _ => 0.023_f64,
            };
            Ok(lambda)
        })?;

        // UDF #3: importance_factor(importance REAL) -> REAL
        c.create_scalar_function("importance_factor", 1, det, |ctx| {
            let imp: f64 = ctx.get(0)?;
            let imp = imp.clamp(1.0_f64, 10.0);
            Ok(0.5 + (imp / 20.0))
        })?;

        // UDF #4: dispute_factor(status TEXT) -> REAL
        c.create_scalar_function("dispute_factor", 1, det, |ctx| {
            let status: String = ctx.get(0)?;
            match status.as_str() {
                "active" => Ok(1.0_f64),
                "disputed" => Ok(0.6_f64),
                _ => Ok(0.0_f64),
            }
        })?;

        // UDF #5: cosine_from_l2_unit(distance REAL) -> REAL
        // For unit vectors, ||a-b||^2 = 2 - 2*cos(a,b) => cos(a,b) = 1 - d^2/2.
        // Clamped to 0.0: cos ranges [-1, 1], so a distance past sqrt(2) would
        // otherwise go negative — MAX(0.0, ...) keeps every KNN hit's similarity
        // factor inside [0, 1] instead of dragging the whole score product negative.
        //
        // The argument is `f64`, not `i64`: sqlite-vec emits `distance` as REAL
        // (`ValueRef::Real`), and rusqlite's `FromSql for i64` rejects `Real` with
        // `InvalidType` — that mismatch would fail every KNN row at runtime while
        // still leaving the test suite green, since no unit test drives the UDF
        // through an actual `vec0` MATCH query.
        c.create_scalar_function("cosine_from_l2_unit", 1, det, |ctx| {
            let d: f64 = ctx.get(0)?;
            Ok((1.0 - d * d / 2.0).max(0.0))
        })?;

        Ok(())
    })
    .await
    .map_err(|e| match e {
        tokio_rusqlite::Error::Error(rusqlite_err) => MemoryError::Sqlite(rusqlite_err),
        tokio_rusqlite::Error::ConnectionClosed => MemoryError::Backend("connection closed during UDF registration".into()),
        other => MemoryError::Backend(other.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use crate::memory::db::MemoryStore;

    // exp_decay UDF
    #[tokio::test]
    async fn exp_decay_zero_age_returns_one() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT exp_decay(0, 0.023)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 1.0).abs() < 1e-9, "expected 1.0, got {v}");
    }

    #[tokio::test]
    async fn exp_decay_one_day_chat_lambda() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT exp_decay(86400, 0.023)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.977).abs() < 0.001, "expected ~0.977, got {v}");
    }

    #[tokio::test]
    async fn exp_decay_seven_days_tool_error_lambda_approx_half() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT exp_decay(604800, 0.099)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.5).abs() < 0.01, "expected ~0.5, got {v}");
    }

    #[tokio::test]
    async fn register_score_udfs_makes_exp_decay_callable() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT exp_decay(0, 0.023)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 1.0).abs() < 1e-9);
    }

    // lambda_for_type UDF
    #[tokio::test]
    async fn lambda_user_preference() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT lambda_for_type('user_preference')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.0019).abs() < 1e-9, "expected 0.0019, got {v}");
    }

    #[tokio::test]
    async fn lambda_chat_message() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT lambda_for_type('chat_message')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.023).abs() < 1e-9, "expected 0.023, got {v}");
    }

    #[tokio::test]
    async fn lambda_tool_error() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT lambda_for_type('tool_error')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.099).abs() < 1e-9, "expected 0.099, got {v}");
    }

    #[tokio::test]
    async fn lambda_unknown_type_returns_default() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT lambda_for_type('unknown_future_type')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.023).abs() < 1e-9, "expected 0.023 (default), got {v}");
    }

    // importance_factor UDF
    #[tokio::test]
    async fn importance_factor_min_value() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT importance_factor(1.0)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.55).abs() < 1e-9, "expected 0.55, got {v}");
    }

    #[tokio::test]
    async fn importance_factor_max_value() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT importance_factor(10.0)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 1.0).abs() < 1e-9, "expected 1.0, got {v}");
    }

    #[tokio::test]
    async fn importance_factor_mid_value() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT importance_factor(5.0)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.75).abs() < 1e-9, "expected 0.75, got {v}");
    }

    #[tokio::test]
    async fn importance_factor_clamp_below_floor() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT importance_factor(0.0)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.55).abs() < 1e-9, "expected 0.55 (clamped), got {v}");
    }

    #[tokio::test]
    async fn importance_factor_clamp_above_ceiling() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT importance_factor(15.0)", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 1.0).abs() < 1e-9, "expected 1.0 (clamped), got {v}");
    }

    // dispute_factor UDF
    #[tokio::test]
    async fn dispute_factor_active() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT dispute_factor('active')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 1.0).abs() < 1e-9, "expected 1.0, got {v}");
    }

    #[tokio::test]
    async fn dispute_factor_disputed() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT dispute_factor('disputed')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.6).abs() < 1e-9, "expected 0.6, got {v}");
    }

    #[tokio::test]
    async fn dispute_factor_superseded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT dispute_factor('superseded')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.0).abs() < 1e-9, "expected 0.0, got {v}");
    }

    #[tokio::test]
    async fn dispute_factor_invalid() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT dispute_factor('invalid')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.0).abs() < 1e-9, "expected 0.0, got {v}");
    }

    #[tokio::test]
    async fn dispute_factor_unknown_status() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row("SELECT dispute_factor('bogus_future_status')", [], |r| r.get(0)))
            .await
            .unwrap();
        assert!((v - 0.0).abs() < 1e-9, "expected 0.0, got {v}");
    }

    // Every registered UDF callable in one expression. Named without a count
    // deliberately: a name like "all_four" goes stale the moment a UDF is added
    // and then claims a coverage it no longer has.
    #[tokio::test]
    async fn register_score_udfs_makes_every_udf_callable() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row(
                "SELECT cosine_from_l2_unit(0.5) * exp_decay(86400, lambda_for_type('chat_message')) * importance_factor(5.0) * dispute_factor('active')",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert!(v > 0.0 && v <= 1.0, "combined score must be in (0.0, 1.0], got {v}");
    }

    #[tokio::test]
    async fn full_score_expression_in_sql_integration() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // Mirrors the score expression the three KNN queries actually build:
        // cosine_from_l2_unit(distance) * MAX(tier_floor, exp_decay(age, lambda))
        //   * importance_factor(importance) * dispute_factor(status)
        // With distance 0.1 between unit vectors: cos = 1 - 0.01/2 = 0.995,
        // so ≈ 0.995 * MAX(0.2, 0.977) * 0.75 * 1.0 ≈ 0.729.
        let v: f64 = store
            .conn
            .call(|c| c.query_row(
                "SELECT cosine_from_l2_unit(0.1) * MAX(0.2, exp_decay(86400, lambda_for_type('chat_message'))) * importance_factor(5.0) * dispute_factor('active')",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert!(v > 0.7 && v < 0.75, "expected ~0.729 (in range (0.7, 0.75)), got {v}");
        assert!(v > 0.0 && v <= 1.0, "must be in (0.0, 1.0], got {v}");
    }

    // cosine_from_l2_unit UDF — converts L2 distance between unit vectors
    // into exact cosine similarity, clamped so a KNN hit never scores below
    // zero regardless of how far apart the two vectors are.
    //
    // This test crosses the actual SQL/Rust boundary rather than restating
    // the algebra: it manufactures several unit-vector pairs, computes their
    // cosine in Rust via a dot product AND their L2 distance in Rust, feeds
    // ONLY the distance through the SQL UDF, and asserts the UDF's output
    // equals the Rust-computed (clamped) cosine. One pair is obtuse enough
    // that its raw cosine is negative, which exercises the MAX(0.0, ...)
    // clamp across the same boundary instead of asserting it in isolation.
    #[tokio::test]
    async fn cosine_from_l2_unit_matches_rust_cosine() {
        fn normalize(v: [f64; 3]) -> [f64; 3] {
            let norm = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] / norm, v[1] / norm, v[2] / norm]
        }
        fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
            a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
        }
        fn l2(a: [f64; 3], b: [f64; 3]) -> f64 {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        }

        let store = MemoryStore::new_for_test().await.unwrap();

        let pairs = [
            (normalize([1.0, 0.0, 0.0]), normalize([1.0, 0.0, 0.0])), // identical: cos = 1.0
            (normalize([1.0, 0.0, 0.0]), normalize([0.0, 1.0, 0.0])), // orthogonal: cos = 0.0
            (normalize([1.0, 0.0, 0.0]), normalize([1.0, 1.0, 0.0])), // acute: cos ≈ 0.707
            (normalize([1.0, 0.0, 0.0]), normalize([-1.0, 0.0, 0.0])), // opposite: cos = -1.0 → clamp
            (normalize([1.0, 2.0, 3.0]), normalize([-3.0, -2.0, -1.0])), // obtuse: cos ≈ -0.714 → clamp, strictly between -1 and 0
        ];

        for (a, b) in pairs {
            let rust_cosine = dot(a, b);
            let expected = rust_cosine.max(0.0);
            let distance = l2(a, b);

            let sql_value: f64 = store
                .conn
                .call(move |c| c.query_row(&format!("SELECT cosine_from_l2_unit({distance})"), [], |r| r.get(0)))
                .await
                .unwrap();

            assert!(
                (sql_value - expected).abs() < 1e-9,
                "a={a:?} b={b:?}: rust cosine (dot product)={rust_cosine}, expected (clamped)={expected}, \
                 SQL UDF cosine_from_l2_unit(distance={distance})={sql_value}"
            );
        }
    }

    #[tokio::test]
    async fn cosine_from_l2_unit_never_negative() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // d=1.5 clamps to 0.0, not the unclamped magnitude: 1 - 1.5²/2 = -0.125,
        // and d=1.5 exceeds sqrt(2) ≈ 1.414 where cosine similarity for unit
        // vectors turns negative — MAX(0.0, ...) is exactly what floors it.
        for (d, expected) in [(1.0_f64, 0.5_f64), (1.5, 0.0), (2.0, 0.0)] {
            let v: f64 = store
                .conn
                .call(move |c| {
                    c.query_row(&format!("SELECT cosine_from_l2_unit({d})"), [], |r| r.get(0))
                })
                .await
                .unwrap();
            assert!(
                (v - expected).abs() < 1e-9,
                "d={d}: expected {expected}, got {v}"
            );
            assert!(v >= 0.0, "d={d}: score must never be negative, got {v}");
        }
    }
}
