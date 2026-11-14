//! Bounded resource-publication probe. Run with:
//! cargo run --release -p caliber-ffi --example resource_slots -- --check-scaling

use std::hint::black_box;
use std::ptr;
use std::time::{Duration, Instant};

use caliber_core::ResourceRegistry;
use caliber_ffi::{
    CaliberContextConfig, CaliberStatus, caliber_context_create, caliber_context_destroy,
    caliber_context_publish_resource, caliber_context_release_resource,
};

const COUNTS: [usize; 2] = [4_096, 16_384];

#[derive(Clone, Copy)]
struct Sample {
    fill: Duration,
    retire: Duration,
    refill: Duration,
}

fn core_sample(count: usize, tile: &[u8], cycles: u32) -> Sample {
    let registry = ResourceRegistry::new(count, tile.len()).unwrap();
    let mut handles = Vec::with_capacity(count);
    let start = Instant::now();
    for _ in 0..count {
        handles.push(registry.insert(black_box(tile)).unwrap());
    }
    let fill = start.elapsed();
    let mut retire = Duration::ZERO;
    let mut refill = Duration::ZERO;
    for _ in 0..cycles {
        let start = Instant::now();
        for handle in handles.iter().step_by(2) {
            registry.release(*handle).unwrap();
        }
        retire += start.elapsed();
        let start = Instant::now();
        for handle in handles.iter_mut().step_by(2) {
            *handle = registry.insert(black_box(tile)).unwrap();
        }
        refill += start.elapsed();
    }
    Sample {
        fill,
        retire: retire / cycles,
        refill: refill / cycles,
    }
}

fn ffi_sample(count: usize, tile: &[u8], cycles: u32) -> Sample {
    let config = CaliberContextConfig {
        max_resources: count,
        max_resource_bytes: tile.len(),
        ..CaliberContextConfig::default()
    };
    let mut context = ptr::null_mut();
    // SAFETY: config and output are valid for the duration of the call.
    assert_eq!(
        unsafe { caliber_context_create(&config, &mut context) },
        CaliberStatus::Ok
    );
    let mut handles = Vec::with_capacity(count);
    let start = Instant::now();
    for _ in 0..count {
        let mut handle = (0, 0);
        // SAFETY: context is live, tile is readable, and outputs are writable.
        let status = unsafe {
            caliber_context_publish_resource(
                context,
                black_box(tile).as_ptr(),
                tile.len(),
                &mut handle.0,
                &mut handle.1,
            )
        };
        assert_eq!(status, CaliberStatus::Ok);
        handles.push(handle);
    }
    let fill = start.elapsed();
    let mut retire = Duration::ZERO;
    let mut refill = Duration::ZERO;
    for _ in 0..cycles {
        let start = Instant::now();
        for &(id, generation) in handles.iter().step_by(2) {
            // SAFETY: context remains live and is used on this thread only.
            let status = unsafe { caliber_context_release_resource(context, id, generation) };
            assert_eq!(status, CaliberStatus::Ok);
        }
        retire += start.elapsed();
        let start = Instant::now();
        for handle in handles.iter_mut().step_by(2) {
            // SAFETY: context is live, tile is readable, and outputs are writable.
            let status = unsafe {
                caliber_context_publish_resource(
                    context,
                    black_box(tile).as_ptr(),
                    tile.len(),
                    &mut handle.0,
                    &mut handle.1,
                )
            };
            assert_eq!(status, CaliberStatus::Ok);
        }
        refill += start.elapsed();
    }
    // SAFETY: all calls have completed and this thread owns the context.
    unsafe { caliber_context_destroy(context) };
    Sample {
        fill,
        retire: retire / cycles,
        refill: refill / cycles,
    }
}

fn median(samples: &[Sample], field: impl Fn(&Sample) -> Duration) -> f64 {
    let mut values: Vec<_> = samples
        .iter()
        .map(|sample| field(sample).as_secs_f64() * 1_000.0)
        .collect();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let check_scaling = args.iter().any(|arg| arg == "--check-scaling");
    let tile_bytes = if args.iter().any(|arg| arg == "--tiny") {
        1
    } else {
        1_024
    };
    let samples = 5;
    let cycles = 3;
    let tile: Vec<u8> = (0..tile_bytes).map(|i| (i % 251) as u8).collect();
    println!("tile_bytes={tile_bytes} samples={samples} cycles={cycles} (milliseconds; median)");
    println!("path,count,fill,retire_half,refill_half");
    let mut ffi_fill = Vec::new();
    for (path, run) in [
        ("core", core_sample as fn(_, _, _) -> _),
        ("ffi", ffi_sample),
    ] {
        let _ = run(256, &tile, 1);
        for count in COUNTS {
            let timings: Vec<_> = (0..samples).map(|_| run(count, &tile, cycles)).collect();
            let fill = median(&timings, |sample| sample.fill);
            let retire = median(&timings, |sample| sample.retire);
            let refill = median(&timings, |sample| sample.refill);
            println!("{path},{count},{fill:.3},{retire:.3},{refill:.3}");
            if path == "ffi" {
                ffi_fill.push(fill);
            }
        }
    }
    let ratio = ffi_fill[1] / ffi_fill[0];
    println!("ffi_fill_4x_count_ratio={ratio:.2}");
    if check_scaling && ratio >= 8.0 {
        eprintln!("FAIL: quadrupling resource count grew FFI fill time by at least 8x");
        std::process::exit(1);
    }
}
