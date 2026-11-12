//! The vector-space invariant shared by both edges of retrieval: every embedding
//! stored or queried against `memory_vectors` is a finite, non-zero, unit-length
//! `f32` vector of exactly [`EMBEDDING_DIM`] components. Insertion (`store_insert.rs`)
//! and query (`search.rs`'s `search_all_memories`) both funnel through [`to_unit`] so
//! the space they write into and the space they read from are the same space by
//! construction, not by convention.

pub(crate) const EMBEDDING_DIM: usize = 384;

/// Why a vector was rejected. Every variant maps to a discard-with-`warn!`, never a
/// panic — an untrustworthy embedding is a caller mistake or a degenerate corpus
/// entry, not a store-integrity failure.
#[derive(Debug, PartialEq)]
pub enum VectorError {
    Dimension { expected: usize, actual: usize },
    NonFinite { index: usize },
    /// Zero magnitude, or a magnitude small enough that dividing by it produces a
    /// non-finite output component — both mean "no direction to normalize".
    Degenerate,
}

impl std::fmt::Display for VectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VectorError::Dimension { expected, actual } => {
                write!(f, "expected {expected}-dim vector, got {actual}")
            }
            VectorError::NonFinite { index } => {
                write!(f, "non-finite component at index {index}")
            }
            VectorError::Degenerate => write!(f, "zero or non-normalizable magnitude"),
        }
    }
}

/// Normalizes `v` to unit length. The norm is accumulated in `f64` — `f32` can
/// overflow or underflow at the extremes of a 384-dim sum of squares — and the
/// division back to `f32` happens only after the accumulator is known finite and
/// non-zero. Rejects any output component that is still non-finite after division,
/// which catches denormal magnitudes without a magic threshold.
pub fn to_unit(v: &[f32]) -> Result<Vec<f32>, VectorError> {
    if v.len() != EMBEDDING_DIM {
        return Err(VectorError::Dimension { expected: EMBEDDING_DIM, actual: v.len() });
    }
    for (index, x) in v.iter().enumerate() {
        if !x.is_finite() {
            return Err(VectorError::NonFinite { index });
        }
    }

    // f64 accumulator: a 384-dim sum of squared f32 components can overflow or
    // underflow f32 at the extremes, and this sum is the one place that matters.
    let norm_sq: f64 = v.iter().map(|x| (*x as f64) * (*x as f64)).sum();
    let norm = norm_sq.sqrt();
    if norm == 0.0 {
        return Err(VectorError::Degenerate);
    }

    let mut out = Vec::with_capacity(v.len());
    for x in v {
        let unit = (*x as f64 / norm) as f32;
        if !unit.is_finite() {
            return Err(VectorError::Degenerate);
        }
        out.push(unit);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_unit_is_idempotent() {
        let v: Vec<f32> = (0..EMBEDDING_DIM).map(|i| (i as f32 + 1.0) * 0.37).collect();
        let once = to_unit(&v).unwrap();
        let twice = to_unit(&once).unwrap();
        for (a, b) in once.iter().zip(twice.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "to_unit(to_unit(v)) must equal to_unit(v) within f32 eps: {a} vs {b}"
            );
        }
    }
}
