//! Registers the four SQLITE_DETERMINISTIC scalar UDFs used by the score formula.
//! Called from MemoryStore::open() and MemoryStore::new_for_test() (D6).

use crate::memory::MemoryError;

/// Registers `exp_decay`, `lambda_for_type`, `importance_factor`, `dispute_factor`
/// on the connection. ALL four UDFs share `SQLITE_UTF8 | SQLITE_DETERMINISTIC` flags.
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

    // T03-test: exp_decay UDF
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

    // T04-test: lambda_for_type UDF
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

    // T05-test: importance_factor UDF
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

    // T06-test: dispute_factor UDF
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

    // T07-test: all four UDFs callable together
    #[tokio::test]
    async fn register_score_udfs_makes_all_four_callable() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let v: f64 = store
            .conn
            .call(|c| c.query_row(
                "SELECT exp_decay(86400, lambda_for_type('chat_message')) * importance_factor(5.0) * dispute_factor('active')",
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
        // Full score: (1.0 - 0.1) * MAX(0.2, exp_decay(86400, lambda_for_type('chat_message'))) * importance_factor(5.0) * dispute_factor('active')
        // ≈ 0.9 * MAX(0.2, 0.977) * 0.75 * 1.0 ≈ 0.9 * 0.977 * 0.75 ≈ 0.659
        let v: f64 = store
            .conn
            .call(|c| c.query_row(
                "SELECT (1.0 - 0.1) * MAX(0.2, exp_decay(86400, lambda_for_type('chat_message'))) * importance_factor(5.0) * dispute_factor('active')",
                [],
                |r| r.get(0),
            ))
            .await
            .unwrap();
        assert!(v > 0.5 && v < 0.75, "expected ~0.66 (in range (0.5, 0.75)), got {v}");
        assert!(v > 0.0 && v <= 1.0, "must be in (0.0, 1.0], got {v}");
    }
}
