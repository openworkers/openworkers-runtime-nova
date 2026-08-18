#![allow(dead_code)]

use std::collections::HashMap;

use openworkers_core::HttpMethod;
use openworkers_core::HttpRequest;
use openworkers_core::RequestBody;

pub fn get(url: &str) -> HttpRequest {
    HttpRequest {
        method: HttpMethod::Get,
        url: url.to_string(),
        headers: HashMap::new(),
        body: RequestBody::None,
    }
}

pub fn report(label: &str, samples: &mut [f64]) {
    samples.sort_by(f64::total_cmp);

    let mean = samples.iter().sum::<f64>() / samples.len() as f64;

    println!(
        "{label:<38} min {:8.3} ms   median {:8.3} ms   mean {mean:8.3} ms   (n={})",
        samples[0],
        samples[samples.len() / 2],
        samples.len()
    );
}

/// Resident set size of this process, via ps: nova has no heap accounting.
pub fn rss_kb() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps should run");

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}
