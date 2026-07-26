//! Emit test PDFs so their ink position can be measured independently.
use onionskin::geometry::PageSize;
use onionskin::pdf::{write_delta, Font, LineFont, PlacedLine};
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).expect("output dir");
    let dir = Path::new(&dir);
    let a4 = PageSize::new(210.0, 297.0);

    let mk = |text: &str, x: f64, y: f64, size: f64, rot: f64| PlacedLine {
        text: text.into(),
        x_mm: x,
        y_mm: y,
        size_pt: size,
        font: LineFont::Builtin(Font::Helvetica),
        rotation_deg: rot,
        colour: (0.0, 0.0, 0.0),
    };

    write_delta(
        &dir.join("plain.pdf"),
        &[a4],
        &[vec![mk("Approved", 60.0, 150.0, 12.0, 0.0)]],
        "t",
        None,
    )
    .unwrap();
    write_delta(
        &dir.join("two.pdf"),
        &[a4, a4],
        &[
            vec![mk("first", 30.0, 60.0, 12.0, 0.0)],
            vec![mk("second", 90.0, 200.0, 12.0, 0.0)],
        ],
        "t",
        None,
    )
    .unwrap();
    write_delta(
        &dir.join("rot90.pdf"),
        &[a4],
        &[vec![mk("Turned", 100.0, 100.0, 14.0, 90.0)]],
        "t",
        None,
    )
    .unwrap();
    write_delta(
        &dir.join("landscape.pdf"),
        &[PageSize::new(297.0, 210.0)],
        &[vec![mk("Wide", 40.0, 80.0, 12.0, 0.0)]],
        "t",
        None,
    )
    .unwrap();
    write_delta(
        &dir.join("accents.pdf"),
        &[a4],
        &[vec![mk("café — naïve €20", 25.0, 120.0, 12.0, 0.0)]],
        "t",
        None,
    )
    .unwrap();
    println!("ok");
}
