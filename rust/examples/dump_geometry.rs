//! Dump geometry results as JSON so the Rust port can be diffed against the
//! Python implementation it replaces. A rewrite is only trustworthy if it
//! agrees with the thing it is replacing, number for number.
use onionskin::geometry::*;

fn main() {
    let pages = [
        PageSize::new(210.0, 297.0),
        PageSize::new(297.0, 210.0),
        PageSize::new(215.9, 355.6),
    ];
    let transforms = [
        Similarity {
            dx_mm: 0.4,
            dy_mm: -0.2,
            rotation_deg: 0.0,
            scale: 1.0,
        },
        Similarity {
            dx_mm: 0.0,
            dy_mm: 0.0,
            rotation_deg: 0.35,
            scale: 1.0,
        },
        Similarity {
            dx_mm: -1.2,
            dy_mm: 0.9,
            rotation_deg: -0.22,
            scale: 0.997,
        },
        Similarity {
            dx_mm: 2.0,
            dy_mm: 3.0,
            rotation_deg: 1.5,
            scale: 1.0021,
        },
    ];
    let points = [(20.0, 20.0), (105.0, 148.5), (190.0, 280.0), (0.0, 0.0)];

    let mut rows: Vec<String> = Vec::new();
    for page in pages.iter() {
        for t in transforms.iter() {
            for p in points.iter() {
                let a = t.apply(*p, page);
                let inv = t.inverse();
                let m = t.to_pdf_matrix(page);
                rows.push(format!(
                    "{{\"page\":[{},{}],\"t\":[{},{},{},{}],\"p\":[{},{}],\
                     \"apply\":[{:.12},{:.12}],\"inv\":[{:.12},{:.12},{:.12},{:.12}],\
                     \"m\":[{:.12},{:.12},{:.12},{:.12},{:.9},{:.9}]}}",
                    page.width_mm,
                    page.height_mm,
                    t.dx_mm,
                    t.dy_mm,
                    t.rotation_deg,
                    t.scale,
                    p.0,
                    p.1,
                    a.0,
                    a.1,
                    inv.dx_mm,
                    inv.dy_mm,
                    inv.rotation_deg,
                    inv.scale,
                    m.a,
                    m.b,
                    m.c,
                    m.d,
                    m.e,
                    m.f
                ));
            }
        }
    }

    // A calibration fit, the other place the numbers must agree exactly.
    let page = pages[0];
    let nominal = [
        (25.0, 25.0),
        (185.0, 25.0),
        (25.0, 272.0),
        (185.0, 272.0),
        (105.0, 148.5),
    ];
    let truth = Similarity {
        dx_mm: 0.42,
        dy_mm: -0.31,
        rotation_deg: 0.18,
        scale: 1.0021,
    };
    let noise = [
        (0.1, -0.1),
        (-0.1, 0.1),
        (0.12, 0.08),
        (-0.08, -0.12),
        (0.05, 0.05),
    ];
    let observed: Vec<(f64, f64)> = nominal
        .iter()
        .zip(noise.iter())
        .map(|(p, n)| {
            let m = truth.apply(*p, &page);
            (m.0 + n.0, m.1 + n.1)
        })
        .collect();
    let fit = solve_similarity(&nominal, &observed, &page).unwrap();
    rows.push(format!(
        "{{\"fit\":[{:.12},{:.12},{:.12},{:.12}],\"rms\":{:.12},\"max\":{:.12}}}",
        fit.transform.dx_mm,
        fit.transform.dy_mm,
        fit.transform.rotation_deg,
        fit.transform.scale,
        fit.rms_residual_mm,
        fit.max_residual_mm
    ));

    println!("[{}]", rows.join(","));
}
