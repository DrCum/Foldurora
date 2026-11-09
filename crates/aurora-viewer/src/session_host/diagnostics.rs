//! Opt-in bounded timings, without chat, account identities or image data.
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) enum Metric {
    Paint,
    Input,
    Geometry,
    Fit,
    Show,
    Hide,
    Focus,
    MonitorFirst,
}

pub(super) fn enabled() -> bool {
    *OnceLock::get_or_init(&ENABLED, || std::env::var_os("AURORA_HOST_DIAGNOSTICS").is_some_and(|v| v == "1"))
}
static ENABLED: OnceLock<bool> = OnceLock::new();

pub(super) fn record(metric: Metric, elapsed: Duration) {
    if !enabled() {
        return;
    }
    struct Sample {
        count: u64,
        max_us: u128,
        total_us: u128,
    }
    struct Counters {
        since: Instant,
        values: [Sample; 8],
    }
    static COUNTERS: OnceLock<Mutex<Counters>> = OnceLock::new();
    let counters = COUNTERS.get_or_init(|| {
        Mutex::new(Counters {
            since: Instant::now(),
            values: std::array::from_fn(|_| Sample {
                count: 0,
                max_us: 0,
                total_us: 0,
            }),
        })
    });
    let Ok(mut counters) = counters.try_lock() else { return };
    let sample = &mut counters.values[metric as usize];
    sample.count = sample.count.saturating_add(1);
    sample.max_us = sample.max_us.max(elapsed.as_micros());
    sample.total_us = sample.total_us.saturating_add(elapsed.as_micros());
    if counters.since.elapsed() >= Duration::from_secs(1) {
        for (name, value) in [
            "paint",
            "input-to-present",
            "geometry",
            "fit",
            "show",
            "hide",
            "focus",
            "monitor-first",
        ]
        .into_iter()
        .zip(&mut counters.values)
        {
            if value.count != 0 {
                log::info!(
                    "host timing {name}: count={} mean_us={} max_us={}",
                    value.count,
                    value.total_us / u128::from(value.count),
                    value.max_us
                );
            }
            *value = Sample {
                count: 0,
                max_us: 0,
                total_us: 0,
            };
        }
        counters.since = Instant::now();
    }
}
