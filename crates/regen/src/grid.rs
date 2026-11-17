//! Grid surfaces in regeneration (docs/design/surfacing.md › *A sheet
//! through a grid of points*): the points from the feature's expressions,
//! or a regular grid whose heights are a table's numbers times a factor;
//! the sheet from `craft3d_geom::grid::interpolate`, made into a body of
//! one face named by the borders of the grid.

use crate::{Names, length_or_number, to_document};
use craft3d_doc::{
    Document, FeatureId, GridPoints, GridSpacing, NameRole, PersistentName, SurfaceGrid, TableCell,
};
use craft3d_expr::{Dimension, Evaluation, Quantity};
use craft3d_geom::grid::{self, Spacing};
use craft3d_geom::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Portable, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge};

/// What a grid surface made beside its body, for its feature's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Gridded {
    pub rows: usize,
    pub columns: usize,
    /// The largest distance found between a point and the sheet at the
    /// point's parameters.
    pub deviation: f64,
    /// What rounding may leave there (`craft3d_geom::grid::Grid::bound`).
    pub bound: f64,
    /// What may be wrong with the sheet, in words: rounding beyond the
    /// tolerance, heights far steeper than the steps.
    pub warnings: Vec<String>,
}

// Every grid's sheet is taken by a boolean: it has fewer Bézier spans than
// points (`(rows − 3) (columns − 3)` for a cubic), and a boolean takes as
// many spans as the largest grid has points (2026-11-17; until then 32 768,
// and a sheet of more was warned of). A grid cap raised past it brings
// that warning back.
const _: () = assert!(grid::MAX_GRID_POINTS <= craft3d_kernel::boolean::MAX_FREEFORM_SPANS);

// The document's caps are the geometry's: one number each, written twice
// because the document does not depend on the geometry.
const _: () = assert!(
    craft3d_doc::MAX_GRID_POINTS == grid::MAX_GRID_POINTS
        && craft3d_doc::MAX_GRID_SIDE == grid::MAX_GRID_SIDE
);

/// The points of the grid, row by row, in the document's unit; with the
/// number of rows and of points per row, and whether the face is to be
/// turned over so that it looks along the heights (a table whose steps
/// have opposite signs).
fn points(
    doc: &Document,
    variables: &Evaluation,
    feature: &SurfaceGrid,
) -> Result<(Vec<Vec3>, usize, usize, bool), String> {
    match &feature.points {
        GridPoints::Given { rows } => {
            let count = rows.len();
            let width = rows.first().map_or(0, Vec::len);
            if count < 2 || width < 2 {
                return Err(format!(
                    "points: a grid has at least 2 rows of at least 2 points; this one has {count} rows of {width}"
                ));
            }
            if let Some(r) = rows.iter().position(|row| row.len() != width) {
                return Err(format!(
                    "points: row {} has {} points and row 1 has {width}: every row of a grid has as many points",
                    r + 1,
                    rows.get(r).map_or(0, Vec::len)
                ));
            }
            let total = count.saturating_mul(width);
            if total > grid::MAX_GRID_POINTS {
                return Err(format!(
                    "points: a grid holds at most {} points; {count} rows of {width} are {total}",
                    grid::MAX_GRID_POINTS
                ));
            }
            let mut out = Vec::with_capacity(total);
            for (r, row) in rows.iter().enumerate() {
                for (c, [x, y, z]) in row.iter().enumerate() {
                    let at = |axis: &str| format!("points[{r}][{c}].{axis}");
                    out.push(Vec3::new(
                        coordinate(doc, variables, &at("x"), x)?,
                        coordinate(doc, variables, &at("y"), y)?,
                        coordinate(doc, variables, &at("z"), z)?,
                    ));
                }
            }
            Ok((out, count, width, false))
        }
        GridPoints::Table {
            table,
            origin,
            step,
            factor,
        } => {
            let Some(found) = doc.table(table) else {
                let known: Vec<&str> = doc.tables.iter().map(|t| t.name.as_str()).collect();
                return Err(if known.is_empty() {
                    format!("table: there is no table named `{table}` (the document has no tables)")
                } else {
                    format!(
                        "table: there is no table named `{table}` (the tables are {})",
                        known.join(", ")
                    )
                });
            };
            // Its columns other than the key, in order: the grid's columns.
            let columns: Vec<(usize, Option<craft3d_expr::Unit>, &str)> = found
                .columns
                .iter()
                .enumerate()
                .filter(|(_, c)| c.name != found.key)
                .map(|(i, c)| {
                    (
                        i,
                        c.unit.as_deref().and_then(craft3d_expr::Unit::from_symbol),
                        c.name.as_str(),
                    )
                })
                .collect();
            let (count, width) = (found.rows.len(), columns.len());
            if count < 2 || width < 2 {
                return Err(format!(
                    "table: `{table}` has {count} rows and {width} columns besides its key `{}`; a grid needs at least 2 of each (its rows are the grid's rows, its columns other than the key the grid's columns)",
                    found.key
                ));
            }
            let total = count.saturating_mul(width);
            if total > grid::MAX_GRID_POINTS {
                return Err(format!(
                    "table: a grid holds at most {} points; `{table}` has {count} rows of {width} heights, {total}",
                    grid::MAX_GRID_POINTS
                ));
            }
            let [ox, oy, oz] = origin;
            let o = Vec3::new(
                coordinate(doc, variables, "origin.x", ox)?,
                coordinate(doc, variables, "origin.y", oy)?,
                coordinate(doc, variables, "origin.z", oz)?,
            );
            let [sx, sy] = step;
            let (dx, dy) = (
                coordinate(doc, variables, "step.x", sx)?,
                coordinate(doc, variables, "step.y", sy)?,
            );
            for (name, d, e) in [("step.x", dx, sx), ("step.y", dy, sy)] {
                if !(d.is_finite() && d != 0.0) {
                    return Err(format!(
                        "{name}: `{}` is {d}; a step of the grid is a length other than zero (negative runs the other way)",
                        e.source()
                    ));
                }
            }
            let scale = variables
                .evaluate(factor.source())
                .map_err(|e| format!("factor: `{}`: {e}", factor.source()))?;
            let mut out = Vec::with_capacity(total);
            for (r, row) in found.rows.iter().enumerate() {
                for (c, (index, unit, name)) in columns.iter().enumerate() {
                    let value = match row.get(*index) {
                        Some(TableCell::Number(v)) => *v,
                        Some(TableCell::Text(text)) => {
                            return Err(format!(
                                "table: `{table}` row {}, column `{name}` holds the text \"{text}\"; every cell of a grid's columns is a number",
                                r + 1
                            ));
                        }
                        None => {
                            return Err(format!(
                                "table: `{table}` row {} has no cell in column `{name}`",
                                r + 1
                            ));
                        }
                    };
                    let cell = match unit {
                        Some(unit) => Quantity::from_unit(value, *unit),
                        None => Quantity::dimensionless(value),
                    };
                    let dimension = cell
                        .dimension
                        .checked_mul(scale.dimension)
                        .unwrap_or(Dimension::ANGLE);
                    let product = Quantity {
                        value: cell.value * scale.value,
                        dimension,
                    };
                    let height = if dimension == Dimension::LENGTH {
                        to_document(product, doc).value
                    } else if dimension.is_dimensionless() {
                        product.value
                    } else {
                        return Err(format!(
                            "factor: a height of `{table}` (column `{name}`, {}) times `{}` ({}) is {}, but a length is needed",
                            if cell.dimension.is_dimensionless() {
                                "a plain number".to_string()
                            } else {
                                cell.dimension.described()
                            },
                            factor.source(),
                            scale.dimension.described(),
                            dimension.described()
                        ));
                    };
                    out.push(Vec3::new(
                        o.x + c as f64 * dx,
                        o.y + r as f64 * dy,
                        o.z + height,
                    ));
                }
            }
            Ok((out, count, width, (dx > 0.0) != (dy > 0.0)))
        }
    }
}

/// One coordinate: a length, a bare number in the document's unit.
fn coordinate(
    doc: &Document,
    variables: &Evaluation,
    name: &str,
    expr: &craft3d_doc::Expr,
) -> Result<f64, String> {
    let (value, _) = length_or_number(doc, variables, name, expr)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!(
            "{name}: `{}` is {}; a point of a grid is finite numbers",
            expr.source(),
            if value.is_nan() {
                "not a number"
            } else {
                "infinite"
            }
        ))
    }
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    id: FeatureId,
    feature: &SurfaceGrid,
) -> Result<(Body, Names, Gridded), String> {
    let tol = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let (pts, rows, columns, turned) = points(doc, variables, feature)?;
    let spacing = match feature.spacing {
        GridSpacing::Greville => Spacing::Greville,
        GridSpacing::Chord => Spacing::Chord,
        GridSpacing::Even => Spacing::Even,
    };
    // A table's heights stand along +z: the proof that the sheet does not
    // fold tries that direction first.
    let up = matches!(feature.points, GridPoints::Table { .. }).then_some(Vec3::Z);
    let steep = match up {
        Some(_) => steepest(&pts, rows, columns),
        None => 0.0,
    };
    let made = grid::interpolate_with(
        &pts,
        rows,
        columns,
        &grid::Options {
            spacing,
            tolerance: tol.linear,
            angular: tol.angular,
            up,
        },
    )
    .map_err(|e| format!("points: {e}"))?;
    drop(pts);
    let (deviation, bound, reach) = (made.deviation, made.bound, made.reach);
    let mut warnings = Vec::new();
    if bound > tol.linear {
        warnings.push(format!(
            "the grid reaches {reach:.3e} from the origin, where rounding may take the sheet up to {bound:.1e} from its points, more than the tolerance {:.1e} (found {deviation:.1e} at the points): work nearer the origin (and move the sheet there with feature.transform) or in a larger unit",
            tol.linear
        ));
    }
    if steep > STEEP {
        warnings.push(format!(
            "the heights rise up to {steep:.0} times the step between neighbours: is the factor right (heights in metres over steps of millimetres want the map's scale)?"
        ));
    }
    let body = sheet(made.surface, rows, columns, turned)?;
    // Numbered as a primitive's entities are, in a fixed order: the edges
    // along the first row (0), the last row (1), the first column (2) and
    // the last column (3); the corners at the first row's first point (0)
    // and last point (1), and the last row's first (2) and last (3). In
    // the order `sheet` adds them.
    let face = PersistentName::new(id, NameRole::Face, vec![0]);
    let edges = [0, 3, 1, 2]
        .into_iter()
        .map(|k| PersistentName::new(id, NameRole::Edge, vec![k]))
        .collect();
    let vertices = (0..4)
        .map(|k| PersistentName::new(id, NameRole::Vertex, vec![k]))
        .collect();
    let names = Names::listed(vec![face], edges, vertices);
    Ok((
        body,
        names,
        Gridded {
            rows,
            columns,
            deviation,
            bound,
            warnings,
        },
    ))
}

/// A table's rises between neighbours above this many times the step are
/// warned of: a factor left out, most likely.
const STEEP: f64 = 1e3;

/// The largest rise in z between neighbours of a table's grid over their
/// distance in x and y.
fn steepest(pts: &[Vec3], rows: usize, columns: usize) -> f64 {
    let at = |r: usize, c: usize| pts.get(r * columns + c).copied();
    let mut most = 0.0_f64;
    for r in 0..rows {
        for c in 0..columns {
            let right = (c + 1 < columns).then(|| at(r, c + 1)).flatten();
            let up = (r + 1 < rows).then(|| at(r + 1, c)).flatten();
            for (a, b) in [(at(r, c), right), (at(r, c), up)] {
                if let (Some(a), Some(b)) = (a, b) {
                    let run = (b.x - a.x).phypot(b.y - a.y);
                    if run > 0.0 {
                        most = most.max((b.z - a.z).abs() / run);
                    }
                }
            }
        }
    }
    most
}

/// The body of one face on `surface`, bounded by its four border curves:
/// vertices at (u, v) = (0, 0), (1, 0), (0, 1), (1, 1) (the first row's
/// first and last point, the last row's first and last), edges along the
/// first row (v = 0), the last column (u = 1), the last row (v = 1) and
/// the first column (u = 0), in that order, counter-clockwise in the
/// parameters; `turned` turns the face over (its normal `−S_u × S_v`, its
/// loop the other way).
pub(crate) fn sheet(
    surface: NurbsSurface,
    rows: usize,
    columns: usize,
    turned: bool,
) -> Result<Body, String> {
    let (pu, pv) = surface.degrees();
    let ctrl = surface.control_points();
    let lost = || craft3d_math::bug_text!("a grid's sheet lost a control point").to_string();
    // A border of the net: control point (c, r) at c * rows + r.
    let border = |along_u: bool, at: usize| -> Result<NurbsCurve, String> {
        let (degree, knots, points): (usize, Vec<f64>, Vec<Vec3>) = if along_u {
            (
                pu,
                surface.knots_u().to_vec(),
                (0..columns)
                    .map(|c| ctrl.get(c * rows + at).copied().ok_or_else(lost))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            (
                pv,
                surface.knots_v().to_vec(),
                (0..rows)
                    .map(|r| ctrl.get(at * rows + r).copied().ok_or_else(lost))
                    .collect::<Result<_, _>>()?,
            )
        };
        NurbsCurve::polynomial(degree, knots, points).map_err(|e| e.to_string())
    };
    let first_row = border(true, 0)?;
    let last_row = border(true, rows - 1)?;
    let first_column = border(false, 0)?;
    let last_column = border(false, columns - 1)?;
    let corner = |c: usize, r: usize| ctrl.get(c * rows + r).copied().ok_or_else(lost);
    let corners = [
        corner(0, 0)?,
        corner(columns - 1, 0)?,
        corner(0, rows - 1)?,
        corner(columns - 1, rows - 1)?,
    ];
    let mut body = Body::new(BodyKind::Sheet);
    let [c00, c10, c01, c11] = corners.map(|p| body.add_vertex(p));
    let mut coedges = Vec::with_capacity(4);
    for (curve, start, end, back) in [
        (first_row, c00, c10, false),
        (last_column, c10, c11, false),
        (last_row, c01, c11, true),
        (first_column, c00, c01, true),
    ] {
        let range = curve.domain();
        let edge = body.add_edge(Curve3::Nurbs(curve), range, start, end);
        coedges.push(Coedge {
            edge,
            reversed: back,
        });
    }
    if turned {
        coedges.reverse();
        for coedge in &mut coedges {
            coedge.reversed = !coedge.reversed;
        }
    }
    let lp = body.add_loop(coedges);
    let face = body.add_face(Surface::Nurbs(surface), turned, vec![lp]);
    body.add_shell(vec![face]);
    Ok(body)
}
