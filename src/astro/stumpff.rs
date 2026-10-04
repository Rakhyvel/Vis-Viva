/// Below this |z|, use the series to prevent catastrophic cancellation
const SERIES_THRESHOLD: f64 = 1.0;
/// Enough terms that the series is accurate to rounding error for |z| < 1
const SERIES_TERMS: usize = 10;

pub fn stumpff_c2_c3(z: f64) -> (f64, f64) {
    if z.abs() < SERIES_THRESHOLD {
        let (mut t2, mut t3) = (0.5, 1.0 / 6.0);
        let (mut c2, mut c3) = (t2, t3);
        for k in 1..SERIES_TERMS {
            let k = k as f64;
            t2 *= -z / ((2.0 * k + 1.0) * (2.0 * k + 2.0));
            t3 *= -z / ((2.0 * k + 2.0) * (2.0 * k + 3.0));
            c2 += t2;
            c3 += t3;
        }
        (c2, c3)
    } else if z > 0.0 {
        let sz = z.sqrt();
        let sin_sz = sz.sin();

        let half_sin = (0.5 * sz).sin();
        let c2 = 2.0 * half_sin * half_sin / z;
        let c3 = (sz - sin_sz) / (z * sz);

        (c2, c3)
    } else {
        let sz = (-z).sqrt();

        let half_sinh = (0.5 * sz).sinh();
        let c2 = 2.0 * half_sinh * half_sinh / (-z);
        let c3 = (sz.sinh() - sz) / ((-z) * sz);

        (c2, c3)
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;

    const ABS_TOL: f64 = 1e-14;
    const REL_TOL: f64 = 1e-14;

    #[track_caller]
    fn assert_close(actual: f64, expected: f64) {
        let abs_err = (actual - expected).abs();
        let rel_err = abs_err / expected.abs().max(f64::MIN_POSITIVE);
        assert!(
            abs_err < ABS_TOL || rel_err < REL_TOL,
            "expected {}, got {} (abs_err={}, rel_err={})",
            expected,
            actual,
            abs_err,
            rel_err
        );
    }

    #[test]
    fn zero_z() {
        let (c2, c3) = stumpff_c2_c3(0.0);

        assert_close(c2, 0.5);
        assert_close(c3, 1.0 / 6.0);
    }

    #[test]
    fn positive_z() {
        let (c2, c3) = stumpff_c2_c3(1.0);

        assert_close(c2, 1.0 - 1.0_f64.cos());
        assert_close(c3, 1.0 - 1.0_f64.sin());
    }

    #[test]
    fn negative_z() {
        let (c2, c3) = stumpff_c2_c3(-1.0);

        assert_close(c2, 1.0_f64.cosh() - 1.0);
        assert_close(c3, 1.0_f64.sinh() - 1.0);
    }

    #[test]
    fn pi_squared() {
        let (c2, c3) = stumpff_c2_c3(PI * PI);

        assert_close(c2, 2.0 / (PI * PI));
        assert_close(c3, 1.0 / (PI * PI));
    }

    #[test]
    fn negative_pi_squared() {
        let (c2, c3) = stumpff_c2_c3(-PI * PI);

        assert_close(c2, (PI.cosh() - 1.0) / (PI * PI));
        assert_close(c3, (PI.sinh() - PI) / (PI * PI * PI));
    }

    #[test]
    fn four_pi_squared() {
        let (c2, c3) = stumpff_c2_c3(4.0 * PI * PI);

        assert_close(c2, 0.0);
        assert_close(c3, 1.0 / (4.0 * PI * PI));
    }

    #[test]
    fn small_positive_z() {
        for ep in [1e-9, 1e-10, 1e-11, 1e-12, 1e-13] {
            let (c2, c3) = stumpff_c2_c3(ep);

            assert_close(c2, 0.5 - ep / 24.0);
            assert_close(c3, (1.0 / 6.0) - (ep / 120.0));
        }
    }

    #[test]
    fn small_negative_z() {
        for ep in [1e-9, 1e-10, 1e-11, 1e-12, 1e-13] {
            let (c2, c3) = stumpff_c2_c3(-ep);

            assert_close(c2, 0.5 + ep / 24.0);
            assert_close(c3, (1.0 / 6.0) + (ep / 120.0));
        }
    }

    // generated with python
    const GOLDEN: &[(f64, f64, f64)] = &[
        (1e-07, 4.99999995833333347222e-1, 1.66666665833333335317e-1),
        (-1e-07, 5.00000004166666680556e-1, 1.66666667500000001984e-1),
        (1e-05, 4.99999583333472222197e-1, 1.66666583333353174600e-1),
        (-1e-05, 5.00000416666805555580e-1, 1.66666750000019841273e-1),
        (0.001, 4.99958334722197420910e-1, 1.66658333531743276039e-1),
        (-0.001, 5.00041668055580357419e-1, 1.66675000198415454170e-1),
        (0.1, 4.95847197448171374338e-1, 1.65835314707089143571e-1),
        (-0.1, 5.04180580384721064832e-1, 1.67501986885222866937e-1),
        (0.5, 4.79510805848739697493e-1, 1.62549260268863124432e-1),
        (-0.5, 5.21183673042712238954e-1, 1.70883282545214003739e-1),
        // just either side of the series/closed-form switch at |z| = 1
        (0.999, 4.59736657649728775314e-1, 1.58536960058122596395e-1),
        (-0.999, 5.43036116318179336560e-1, 1.75192455323688503900e-1),
        (1.001, 4.59658733246225515625e-1, 1.58521070706672832837e-1),
        (-1.001, 5.43125156242244495572e-1, 1.75209932377578057476e-1),
    ];

    #[test]
    fn matches_high_precision_values() {
        for &(z, c2, c3) in GOLDEN {
            let (actual_c2, actual_c3) = stumpff_c2_c3(z);
            assert_close(actual_c2, c2);
            assert_close(actual_c3, c3);
        }
    }

    #[test]
    fn positive_large_z() {
        for z in [100.0, 1000.0, 10000.0] {
            let (c2, c3) = stumpff_c2_c3(z);

            assert!(c2.is_finite());
            assert!(c3.is_finite());
            assert!(c2 > 0.0);
            assert!(c3 > 0.0);
        }
    }

    #[test]
    fn negative_large_z() {
        for z in [-100.0, -1000.0, -10000.0] {
            let (c2, c3) = stumpff_c2_c3(z);

            assert!(c2.is_finite());
            assert!(c3.is_finite());
            assert!(c2 > 0.0);
            assert!(c3 > 0.0);
        }
    }
}
