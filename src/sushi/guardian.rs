//! Grades how a route's price impact grows as trade size increases, instead
//! of judging a single quote's impact against one flat threshold. A pool can
//! look fine at the size someone is actually asking for and still be a bad
//! idea to lean on — this exists to catch that curve, not just the one point
//! on it the base preview already shows.
//!
//! Pure grading logic only — no network here. The three `Option<f64>` impact
//! readings (1x/10x/100x the requested amount) are gathered elsewhere via
//! `api::quote` and handed in.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grade {
    Fine,
    Caution,
    ThinSizeDown,
}

pub struct Reading {
    pub grade: Grade,
    pub impact_1x: Option<f64>,
    pub impact_10x: Option<f64>,
    pub impact_100x: Option<f64>,
}

impl Reading {
    pub fn message(&self) -> &'static str {
        match self.grade {
            Grade::Fine => "liquidity looks solid at this size",
            Grade::Caution => "liquidity's a bit thin here — worth a second look before sizing up",
            Grade::ThinSizeDown => "thin pool — that's a real haircut, maybe size down",
        }
    }
}

/// Fractions, not percent: 0.005 means 0.5%. A missing leg (a larger size
/// simply couldn't be quoted — typical of a thin pool) counts toward
/// `Caution`/`ThinSizeDown`, never toward "no data, assume fine".
pub fn grade(impact_1x: Option<f64>, impact_10x: Option<f64>, impact_100x: Option<f64>) -> Grade {
    let i1 = impact_1x.map(f64::abs);
    let i10 = impact_10x.map(f64::abs);
    let i100 = impact_100x.map(f64::abs);

    // Never less cautious than the flat threshold this replaces.
    if i1.is_some_and(|v| v > 0.03) {
        return Grade::ThinSizeDown;
    }
    // A larger size that couldn't even be quoted is itself a thin-liquidity
    // signal, regardless of how clean the small quote looked.
    if impact_10x.is_none() || impact_100x.is_none() {
        return Grade::Caution;
    }
    // Steep curve even when the small size looks clean: the 100x impact is
    // wildly out of proportion to the 1x one.
    if let (Some(a), Some(b)) = (i1, i100) {
        if a > 0.0 && b / a > 8.0 {
            return Grade::ThinSizeDown;
        }
    }
    if i1.is_some_and(|v| v >= 0.01) || i10.is_some_and(|v| v >= 0.05) {
        return Grade::Caution;
    }
    Grade::Fine
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fine_when_the_curve_stays_shallow() {
        assert_eq!(grade(Some(0.002), Some(0.008), Some(0.015)), Grade::Fine);
    }

    #[test]
    fn caution_when_1x_is_a_bit_elevated() {
        assert_eq!(grade(Some(0.015), Some(0.03), Some(0.06)), Grade::Caution);
    }

    #[test]
    fn thin_size_down_matches_the_old_flat_3pct_threshold() {
        assert_eq!(grade(Some(0.031), None, None), Grade::ThinSizeDown);
    }

    #[test]
    fn thin_size_down_on_a_steep_curve_even_with_a_clean_1x() {
        // 1x looks totally fine (0.2%) but 100x is 40x worse — a real cliff.
        assert_eq!(grade(Some(0.002), Some(0.02), Some(0.08)), Grade::ThinSizeDown);
    }

    #[test]
    fn missing_larger_legs_is_caution_not_fine() {
        assert_eq!(grade(Some(0.001), None, None), Grade::Caution);
    }

    #[test]
    fn all_missing_is_caution_not_a_panic() {
        assert_eq!(grade(None, None, None), Grade::Caution);
    }
}
