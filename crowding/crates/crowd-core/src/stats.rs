//! Numerical helpers: log-gamma, the regularized incomplete gamma function and a gamma
//! distribution parameterised by mean and coefficient of variation.

/// Natural log of the gamma function (Lanczos approximation, |error| < 1e-13 for x > 0).
pub fn ln_gamma(x: f64) -> f64 {
    const G: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularized lower incomplete gamma function P(a, x).
pub fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if a <= 0.0 {
        return 1.0;
    }
    let gln = ln_gamma(a);
    if x < a + 1.0 {
        // Series expansion.
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..500 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-12 {
                break;
            }
        }
        (sum.ln() - x + a * x.ln() - gln).exp().clamp(0.0, 1.0)
    } else {
        // Continued fraction (modified Lentz).
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..500 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < tiny {
                d = tiny;
            }
            c = b + an / c;
            if c.abs() < tiny {
                c = tiny;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-12 {
                break;
            }
        }
        (1.0 - (-x + a * x.ln() - gln).exp() * h).clamp(0.0, 1.0)
    }
}

/// Gamma distribution with the given mean and coefficient of variation.
#[derive(Debug, Clone, Copy)]
pub struct GammaMeanCv {
    shape: f64,
    scale: f64,
}

impl GammaMeanCv {
    pub fn new(mean: f64, cv: f64) -> Self {
        let cv = cv.max(1e-3);
        let shape = 1.0 / (cv * cv);
        let scale = mean.max(1e-9) / shape;
        Self { shape, scale }
    }

    /// P(X <= x).
    pub fn cdf(&self, x: f64) -> f64 {
        gamma_p(self.shape, x / self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ln_gamma_known_values() {
        assert!((ln_gamma(1.0)).abs() < 1e-10);
        assert!((ln_gamma(5.0) - 24f64.ln()).abs() < 1e-10);
        assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-10);
    }

    #[test]
    fn gamma_p_matches_exponential() {
        // Shape 1 is the exponential distribution: P(1, x) = 1 - e^-x.
        for x in [0.1, 0.5, 1.0, 2.0, 5.0] {
            assert!((gamma_p(1.0, x) - (1.0 - (-x).exp())).abs() < 1e-9);
        }
    }

    #[test]
    fn gamma_cdf_is_centred_on_the_mean() {
        let g = GammaMeanCv::new(50.0, 0.1);
        assert!(g.cdf(40.0) < 0.05);
        assert!(g.cdf(60.0) > 0.95);
        let m = g.cdf(50.0);
        assert!((0.45..0.56).contains(&m), "{m}");
    }
}
