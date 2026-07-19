//! Import-side corner geometry for Figma smooth corners and rounded
//! vector-network vertices.

const CIRCLE_CUBIC: f64 = 0.552_284_749_830_793_6;
const SUPERELLIPSE_CUBIC: f64 = 0.9;

fn coord(value: f64) -> String {
    if value.abs() < 5e-5 {
        return "0".to_string();
    }
    let rounded: f64 = format!("{value:.4}").parse().unwrap_or(value);
    format!("{rounded}")
}

fn point_command(command: &str, point: (f64, f64)) -> String {
    format!("{command}{} {}", coord(point.0), coord(point.1))
}

fn cubic_command(cp1: (f64, f64), cp2: (f64, f64), end: (f64, f64)) -> String {
    format!(
        "C{} {} {} {} {} {}",
        coord(cp1.0),
        coord(cp1.1),
        coord(cp2.0),
        coord(cp2.1),
        coord(end.0),
        coord(end.1)
    )
}

/// Bake a per-corner rounded rectangle into cubic path data. Figma's
/// smoothing factor blends the circle control distance toward a
/// superellipse-like, edge-continuous control distance while keeping
/// every control point inside the ordinary rounded-rect bounds.
pub(crate) fn smoothed_rect_path(
    width: f64,
    height: f64,
    mut radii: [f64; 4],
    smoothing: f64,
) -> Option<String> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    for radius in &mut radii {
        *radius = radius.max(0.0);
    }
    let [tl, tr, br, bl] = radii;
    let mut scale = 1.0_f64;
    for (extent, sum) in [
        (width, tl + tr),
        (width, bl + br),
        (height, tl + bl),
        (height, tr + br),
    ] {
        if sum > 0.0 {
            scale = scale.min(extent / sum);
        }
    }
    if scale < 1.0 {
        for radius in &mut radii {
            *radius *= scale;
        }
    }
    let [tl, tr, br, bl] = radii;
    let smoothing = smoothing.clamp(0.0, 1.0);
    let k = CIRCLE_CUBIC + (SUPERELLIPSE_CUBIC - CIRCLE_CUBIC) * smoothing;
    let mut parts = vec![point_command("M", (tl, 0.0))];

    parts.push(point_command("L", (width - tr, 0.0)));
    if tr > 0.0 {
        parts.push(cubic_command(
            (width - tr + k * tr, 0.0),
            (width, tr - k * tr),
            (width, tr),
        ));
    }
    parts.push(point_command("L", (width, height - br)));
    if br > 0.0 {
        parts.push(cubic_command(
            (width, height - br + k * br),
            (width - br + k * br, height),
            (width - br, height),
        ));
    }
    parts.push(point_command("L", (bl, height)));
    if bl > 0.0 {
        parts.push(cubic_command(
            (bl - k * bl, height),
            (0.0, height - bl + k * bl),
            (0.0, height - bl),
        ));
    }
    parts.push(point_command("L", (0.0, tl)));
    if tl > 0.0 {
        parts.push(cubic_command(
            (0.0, tl - k * tl),
            (tl - k * tl, 0.0),
            (tl, 0.0),
        ));
    }
    parts.push("Z".to_string());
    Some(parts.join(" "))
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn rounded_corner(
    previous: (f64, f64),
    corner: (f64, f64),
    next: (f64, f64),
    radius: f64,
) -> ((f64, f64), (f64, f64)) {
    let incoming = distance(previous, corner);
    let outgoing = distance(next, corner);
    if incoming < 1e-6 || outgoing < 1e-6 {
        return (corner, corner);
    }
    let prev_unit = (
        (previous.0 - corner.0) / incoming,
        (previous.1 - corner.1) / incoming,
    );
    let next_unit = (
        (next.0 - corner.0) / outgoing,
        (next.1 - corner.1) / outgoing,
    );
    let cross = prev_unit.0 * next_unit.1 - prev_unit.1 * next_unit.0;
    if cross.abs() < 1e-6 {
        return (corner, corner);
    }
    let trim = radius.min(incoming * 0.5).min(outgoing * 0.5);
    (
        (corner.0 + prev_unit.0 * trim, corner.1 + prev_unit.1 * trim),
        (corner.0 + next_unit.0 * trim, corner.1 + next_unit.1 * trim),
    )
}

fn corner_command(
    start: (f64, f64),
    corner: (f64, f64),
    end: (f64, f64),
    smoothing: f64,
) -> String {
    if smoothing <= 0.0 {
        return format!(
            "Q{} {} {} {}",
            coord(corner.0),
            coord(corner.1),
            coord(end.0),
            coord(end.1)
        );
    }
    let factor = (2.0 / 3.0) + (0.9 - 2.0 / 3.0) * smoothing.clamp(0.0, 1.0);
    cubic_command(
        (
            start.0 + (corner.0 - start.0) * factor,
            start.1 + (corner.1 - start.1) * factor,
        ),
        (
            end.0 + (corner.0 - end.0) * factor,
            end.1 + (corner.1 - end.1) * factor,
        ),
        end,
    )
}

/// Round straight-network joins. Closed chains round every vertex;
/// open chains preserve both endpoints. Positive smoothing promotes
/// the quadratic circular join to a superellipse-biased cubic.
pub(crate) fn rounded_polyline_path(
    points: &[(f64, f64)],
    closed: bool,
    radius: f64,
    smoothing: f64,
) -> Option<String> {
    if points.len() < 2 || radius <= 0.0 {
        return None;
    }
    if closed && points.len() < 3 {
        return None;
    }

    if closed {
        let corners: Vec<_> = (0..points.len())
            .map(|index| {
                rounded_corner(
                    points[(index + points.len() - 1) % points.len()],
                    points[index],
                    points[(index + 1) % points.len()],
                    radius,
                )
            })
            .collect();
        let mut parts = vec![point_command("M", corners[0].1)];
        for index in 1..points.len() {
            parts.push(point_command("L", corners[index].0));
            parts.push(corner_command(
                corners[index].0,
                points[index],
                corners[index].1,
                smoothing,
            ));
        }
        parts.push(point_command("L", corners[0].0));
        parts.push(corner_command(
            corners[0].0,
            points[0],
            corners[0].1,
            smoothing,
        ));
        parts.push("Z".to_string());
        return Some(parts.join(" "));
    }

    let mut parts = vec![point_command("M", points[0])];
    for index in 1..points.len() - 1 {
        let (entry, exit) =
            rounded_corner(points[index - 1], points[index], points[index + 1], radius);
        parts.push(point_command("L", entry));
        parts.push(corner_command(entry, points[index], exit, smoothing));
    }
    parts.push(point_command("L", *points.last()?));
    Some(parts.join(" "))
}
