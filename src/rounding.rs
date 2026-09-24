//! Squircle math for the corner-rounding overlay.
//!
//! This module owns only the geometry: it generates the outline of the black
//! mask region for each screen corner as a closed contour. The contour is the
//! corner square (side `radius`) cut by the quarter superellipse
//! `|u|^p + |v|^p = r^p` inscribed in it, where `p = curvature`
//! (`p = 2` is a circular arc, larger values give a squircle).
//! Filling, anti-aliasing and presenting the pixels are delegated to
//! tiny-skia and the Wayland toolkit.

/// Corner radius in logical pixels (the caller scales it to physical pixels).
pub const CORNER_RADIUS: u32 = 6;

/// Superellipse exponent: 2.0 gives a circular arc, larger values give a
/// squircle with longer straight runs and a tighter turn at the corner.
pub const CORNER_CURVATURE: f64 = 4.0;

/// Outline of the black mask region for each corner, in buffer pixel
/// coordinates. Returns four closed contours in the order top-left,
/// top-right, bottom-right, bottom-left. Each contour is the corner square of
/// side `radius` cut by the superellipse arc, so filling all of them paints
/// exactly the rounded-corner mask. Coordinates use pixel-center sampling
/// (pixel `i` spans `i .. i+1`), so all four corners are exact mirrors.
pub fn corner_contours(
    width: u32,
    height: u32,
    radius: u32,
    curvature: f64,
) -> Vec<Vec<(f32, f32)>> {
    let radius = radius.min(width / 2).min(height / 2);
    if radius < 1 || width == 0 || height == 0 {
        return Vec::new();
    }

    let r = radius as f64;
    let p = curvature.max(2.0);

    // Arc of (r - x)^p + (r - y)^p = r^p in corner-square local coordinates,
    // running from the left edge (0, r) to the top edge (r, 0). Endpoints are
    // clamped into the corner square so the contour stays consistent
    // with its straight edges.
    let steps = ((radius as f64) * 8.0).max(16.0) as usize;
    let mut arc = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = (i as f64 / steps as f64) * std::f64::consts::FRAC_PI_2;
        let u = r * t.cos().powf(2.0 / p);
        let v = r * t.sin().powf(2.0 / p);
        let x = (r - u).clamp(0.0, r);
        let y = (r - v).clamp(0.0, r);
        arc.push((x, y));
    }

    // Contour per corner: two segments flush with the screen edges plus the
    // arc, so the filled region covers the whole corner square up to the
    // boundary pixel (the path edge at 0/w keeps pixel column/row 0 fully
    // covered while the arc itself still anti-aliases).
    let local = |mirror_x: bool, mirror_y: bool| -> Vec<(f32, f32)> {
        let eps = 0.5;
        let lo = 0.0;
        let hi = r;
        let fx = |x: f64| (if mirror_x { r - x } else { x }) as f32;
        let fy = |y: f64| (if mirror_y { r - y } else { y }) as f32;
        let mut pts = Vec::with_capacity(arc.len() + 3);
        pts.push((fx(lo), fy(lo)));
        pts.push((fx(lo), fy(hi - eps)));
        for &(x, y) in &arc {
            pts.push((fx(x), fy(y)));
        }
        pts.push((fx(hi - eps), fy(lo)));
        pts
    };

    let (w, h) = (width as f32, height as f32);
    let place = |pts: Vec<(f32, f32)>, ox: f32, oy: f32| {
        pts.into_iter().map(|(x, y)| (x + ox, y + oy)).collect()
    };

    vec![
        place(local(false, false), 0.0, 0.0),
        place(local(true, false), w - r as f32, 0.0),
        place(local(true, true), w - r as f32, h - r as f32),
        place(local(false, true), 0.0, h - r as f32),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist_to_corner(p: (f32, f32), corner: (f32, f32)) -> f32 {
        ((p.0 - corner.0).powi(2) + (p.1 - corner.1).powi(2)).sqrt()
    }

    #[test]
    fn contours_touch_screen_edges_and_stay_in_corner_squares() {
        let (w, h, r) = (200u32, 100u32, 20u32);
        let contours = corner_contours(w, h, r, CORNER_CURVATURE);
        assert_eq!(contours.len(), 4);

        let squares = [
            (0.0f32, 0.0),
            (w as f32 - r as f32, 0.0),
            (w as f32 - r as f32, h as f32 - r as f32),
            (0.0, h as f32 - r as f32),
        ];
        for (contour, &(sx, sy)) in contours.iter().zip(&squares) {
            assert!(contour.len() > 4);
            for &(x, y) in contour {
                assert!(x >= sx - 1e-4 && x <= sx + r as f32 + 1e-4);
                assert!(y >= sy - 1e-4 && y <= sy + r as f32 + 1e-4);
            }
            let min_x = contour.iter().fold(f32::MAX, |m, p| m.min(p.0));
            let min_y = contour.iter().fold(f32::MAX, |m, p| m.min(p.1));
            // Edges are inset half a pixel, so "reaching the edge" means
            // coming within one pixel of it.
            assert!(min_x <= sx + 0.51, "contour must reach the screen edge");
            assert!(min_y <= sy + 0.51, "contour must reach the screen edge");
        }
    }

    #[test]
    fn all_four_corners_align() {
        let (w, h, r) = (101u32, 73u32, 8u32);
        let contours = corner_contours(w, h, r, CORNER_CURVATURE);
        let tl = &contours[0];

        for (i, other) in contours.iter().enumerate().skip(1) {
            assert_eq!(tl.len(), other.len(), "corner {i} point count differs");
            for &(x, y) in tl {
                let mx = if i == 1 || i == 2 { w as f32 - x } else { x };
                let my = if i == 2 || i == 3 { h as f32 - y } else { y };
                assert!(
                    other
                        .iter()
                        .any(|&(ox, oy)| (ox - mx).abs() < 1e-3 && (oy - my).abs() < 1e-3),
                    "corner {i} missing mirrored point ({mx}, {my})"
                );
            }
        }
    }

    #[test]
    fn higher_curvature_recedes_from_corner() {
        // A higher exponent makes the inner superellipse bulge toward the
        // screen corner (bigger kept region, smaller black mask), so its arc
        // midpoint sits closer to the corner point than the circular arc's.
        let (w, h, r) = (200u32, 200u32, 20u32);
        let arc_mid = |p: f64| {
            let c = &corner_contours(w, h, r, p)[0];
            c[c.len() / 2]
        };
        let d = |pt: (f32, f32)| dist_to_corner(pt, (0.0, 0.0));
        assert!(
            d(arc_mid(4.0)) < d(arc_mid(2.0)),
            "squircle arc midpoint should sit closer to the corner than circle"
        );
    }

    #[test]
    fn degenerate_sizes_are_empty() {
        assert!(corner_contours(0, 10, 5, 2.0).is_empty());
        // Width 1 clamps the radius to 0 -> nothing is rounded.
        assert!(corner_contours(1, 10, 5, 2.0).is_empty());
        // Radius clamped by a small output still yields 4 tiny contours.
        assert_eq!(corner_contours(4, 4, 5, 2.0).len(), 4);
    }
}
