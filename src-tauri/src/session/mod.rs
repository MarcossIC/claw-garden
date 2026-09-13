mod manager;

// `SessionError` is a plain data/error enum and lives in `clawg-core` so that
// `clawg_core::models::AppError` can keep its `From<SessionError>` impl. The
// session *manager* itself is desktop-side agent runtime and stays here.
pub use clawg_core::models::SessionError;
pub use manager::SessionManager;
