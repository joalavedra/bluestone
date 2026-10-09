//! Demand forecasting for reorder decisions, after the statsforecast playbook:
//! simple exponential smoothing (SES) for regular sellers, TSB for intermittent ones
//! (Syntetos–Boylan: average demand interval > 1.32 days). Smoothing parameters are picked
//! by in-sample one-step squared error; the residual RMSE drives safety stock.

/// z-score for a 95% cycle service level.
pub const SERVICE_Z: f64 = 1.65;
const INTERMITTENT_ADI: f64 = 1.32;
const GRID: [f64; 5] = [0.1, 0.2, 0.3, 0.4, 0.5];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forecast {
    /// Expected units per day.
    pub rate: f64,
    /// `ses`, `tsb` or `none` (no sales in the window).
    pub method: &'static str,
    /// RMSE of one-step daily forecasts.
    pub sigma: f64,
}

fn ses(y: &[f64], alpha: f64) -> (f64, f64) {
    let warm = y.len().min(7);
    let mut level = y[..warm].iter().sum::<f64>() / warm as f64;
    let mut sse = 0.0;
    for &v in y {
        let e = v - level;
        sse += e * e;
        level += alpha * e;
    }
    (level, sse)
}

/// Teunter–Syntetos–Babai: smooth demand size on demand days and demand probability every day.
fn tsb(y: &[f64], a: f64, b: f64) -> (f64, f64) {
    let hits: Vec<f64> = y.iter().copied().filter(|v| *v > 0.0).collect();
    let mut p = hits.len() as f64 / y.len() as f64;
    let mut z = hits.iter().sum::<f64>() / hits.len() as f64;
    let mut sse = 0.0;
    for &v in y {
        let e = v - p * z;
        sse += e * e;
        if v > 0.0 {
            z += a * (v - z);
            p += b * (1.0 - p);
        } else {
            p -= b * p;
        }
    }
    (p * z, sse)
}

/// Forecast from a daily series (oldest first). Days before the first sale are ignored so new
/// products are not dragged down by their pre-launch zeros.
pub fn forecast(daily: &[f64]) -> Forecast {
    let Some(first) = daily.iter().position(|v| *v > 0.0) else {
        return Forecast {
            rate: 0.0,
            method: "none",
            sigma: 0.0,
        };
    };
    let y = &daily[first..];
    let demand_days = y.iter().filter(|v| **v > 0.0).count();
    let adi = y.len() as f64 / demand_days as f64;
    let (rate, sse, method) = if adi > INTERMITTENT_ADI {
        GRID.iter()
            .flat_map(|a| GRID.iter().map(move |b| tsb(y, *a, *b)))
            .map(|(r, e)| (r, e, "tsb"))
            .min_by(|x, z| x.1.total_cmp(&z.1))
            .unwrap()
    } else {
        GRID.iter()
            .map(|a| ses(y, *a))
            .map(|(r, e)| (r, e, "ses"))
            .min_by(|x, z| x.1.total_cmp(&z.1))
            .unwrap()
    };
    Forecast {
        rate: rate.max(0.0),
        method,
        sigma: (sse / y.len() as f64).sqrt(),
    }
}

/// Units to hold beyond expected lead-time demand.
pub fn safety_stock(f: &Forecast, lead_time_days: i64) -> i64 {
    (SERVICE_Z * f.sigma * (lead_time_days.max(1) as f64).sqrt()).ceil() as i64
}

/// Reorder when on-hand drops to expected lead-time demand plus safety stock.
pub fn reorder_point(f: &Forecast, lead_time_days: i64) -> i64 {
    (f.rate * lead_time_days as f64).ceil() as i64 + safety_stock(f, lead_time_days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_sellers_use_ses_and_track_the_level() {
        let f = forecast(&[4.0; 60]);
        assert_eq!(f.method, "ses");
        assert!((f.rate - 4.0).abs() < 1e-9);
        assert_eq!(safety_stock(&f, 14), 0);
        assert_eq!(reorder_point(&f, 14), 56);
    }

    #[test]
    fn level_shifts_are_followed_not_averaged() {
        let mut y = vec![2.0; 60];
        y.extend([8.0; 30]);
        let f = forecast(&y);
        assert!(f.rate > 7.0, "{f:?}");
        assert!(f.rate > y.iter().sum::<f64>() / 90.0);
    }

    #[test]
    fn intermittent_sellers_use_tsb() {
        let y: Vec<f64> = (0..90)
            .map(|d| if d % 5 == 0 { 10.0 } else { 0.0 })
            .collect();
        let f = forecast(&y);
        assert_eq!(f.method, "tsb");
        assert!((f.rate - 2.0).abs() < 0.6, "{f:?}");
        assert!(safety_stock(&f, 14) > 0);
    }

    #[test]
    fn no_sales_and_new_products() {
        assert_eq!(forecast(&[0.0; 90]).method, "none");
        let mut y = vec![0.0; 80];
        y.extend([3.0; 10]);
        assert!((forecast(&y).rate - 3.0).abs() < 1e-9);
    }
}
