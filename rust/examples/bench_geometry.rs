use onionskin::geometry::*;
use std::time::Instant;
fn main() {
    let page = PageSize::new(210.0, 297.0);
    let t = Similarity { dx_mm: 0.42, dy_mm: -0.31, rotation_deg: 0.18, scale: 1.0021 };
    let n = 2_000_000;
    let start = Instant::now();
    let mut acc = 0.0f64;
    for i in 0..n {
        let p = ((i % 200) as f64, (i % 280) as f64);
        let a = t.apply(p, &page);
        acc += a.0 + a.1;
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!("{:.4} {:.1}", elapsed, acc);
}
