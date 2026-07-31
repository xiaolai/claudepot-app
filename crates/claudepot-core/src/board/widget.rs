//! Resolving a widget slot against real rows into a **render plan**.
//!
//! The renderer draws what this module produces and makes no decisions
//! of its own. That split is deliberate: every degenerate-data rule in
//! plan §7 is a correctness question with a right answer, and answering
//! it in TypeScript would mean answering it again in every other
//! surface that ever renders a board.
//!
//! # Schemas do not prevent broken renders
//!
//! `spec.rs` validates placement — unique ids, supported kinds,
//! resolvable column bindings. Every one of the following is *schema
//! valid* and still unrenderable, which is why the guards live here:
//!
//! | Case | Guard |
//! |---|---|
//! | log axis containing zero or negative | [`AxisScale`] falls back, with a reason |
//! | `min == max` | padded synthetic range |
//! | all-null series | [`RenderPlan::Empty`], not an axis with no marks |
//! | very large row counts | downsample for charts, truncate for tables |
//! | high-cardinality categorical | cap plus a `+N more` bucket |
//! | very long labels | truncate, full value carried alongside |
//!
//! The exact numbers are tuning; the existence of a named, tested
//! behavior per case is the design commitment.

use serde::{Deserialize, Serialize};

use super::series::{Row, SeriesDef, Value};
use super::spec::{WidgetKind, WidgetSlot};

/// Points a chart renders before downsampling kicks in.
pub const MAX_CHART_POINTS: usize = 2_000;

/// Rows a table renders before truncating.
pub const MAX_TABLE_ROWS: usize = 500;

/// Distinct categories a bar chart renders before collapsing the tail.
pub const MAX_CATEGORIES: usize = 40;

/// Characters of a label before truncation.
pub const MAX_LABEL_CHARS: usize = 48;

/// A label that is safe to draw, plus the full text for a tooltip.
///
/// Both halves always travel together — per `rules/path-display.md`, a
/// truncated string with no way to see the rest is the anti-pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub text: String,
    /// `Some` only when `text` was shortened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
}

impl Label {
    pub fn new(raw: &str) -> Self {
        let count = raw.chars().count();
        if count <= MAX_LABEL_CHARS {
            return Self {
                text: raw.to_string(),
                full: None,
            };
        }
        let text: String = raw.chars().take(MAX_LABEL_CHARS - 1).collect();
        Self {
            text: format!("{text}…"),
            full: Some(raw.to_string()),
        }
    }
}

/// Which scale an axis ended up with, and why.
///
/// **Internally tagged on purpose.** With the default representation
/// serde emits `"linear"` for a unit variant and
/// `{"log_fell_back_to_linear":{...}}` for the struct variant — two
/// different JSON *types* in one field. The TypeScript consumer then
/// has to `typeof`-check before narrowing, and the obvious
/// `"log_fell_back_to_linear" in scale` throws `TypeError` on a string
/// primitive. That is not hypothetical: it shipped, and the unit test
/// missed it because the fixture encoded the wrong shape.
///
/// `tag = "kind"` makes every variant an object, so one discriminant
/// narrows all three.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AxisScale {
    Linear,
    Log,
    /// Log was asked for but is impossible for this data. Carries the
    /// reason so the UI can say so instead of silently drawing a
    /// different chart than the spec describes.
    LogFellBackToLinear {
        reason: String,
    },
}

/// A numeric axis range, always non-degenerate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisRange {
    pub min: f64,
    pub max: f64,
    pub scale: AxisScale,
    /// True when `min == max` in the data and the range was padded.
    pub padded: bool,
}

impl AxisRange {
    /// Build a drawable range from observed values.
    ///
    /// A single repeated value, or a single point, produces `min == max`
    /// — which renders as a zero-height axis with the line either
    /// invisible or at an arbitrary edge. Pad it.
    pub fn from_values(values: &[f64], want_log: bool) -> Option<Self> {
        let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }
        let mut min = finite.iter().copied().fold(f64::INFINITY, f64::min);
        let mut max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        let scale = if !want_log {
            AxisScale::Linear
        } else if min <= 0.0 {
            // log(0) is -inf and log(negative) is undefined; charting
            // libraries variously drop the point, draw at the axis
            // floor, or render nothing at all.
            AxisScale::LogFellBackToLinear {
                reason: "series contains zero or negative values".to_string(),
            }
        } else {
            AxisScale::Log
        };

        let padded = min == max;
        if padded {
            // Pad proportionally so the shape is readable at any
            // magnitude; fall back to ±1 at exactly zero.
            let pad = if min == 0.0 { 1.0 } else { min.abs() * 0.1 };
            min -= pad;
            max += pad;
        }

        Some(Self {
            min,
            max,
            scale,
            padded,
        })
    }
}

/// One x/y point after resolution.
///
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Category label for `bar`, RFC 3339 or numeric string for `line`.
    pub x: String,
    /// `None` is a **gap**, and the renderer must break the line rather
    /// than joining across it or drawing zero.
    pub y: Option<f64>,
}

/// What the renderer should draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderPlan {
    /// Nothing to draw, with a reason the UI can show verbatim.
    Empty { reason: String },
    Line {
        points: Vec<Point>,
        y_axis: AxisRange,
        /// Points dropped by downsampling. Never silent.
        downsampled_from: Option<usize>,
    },
    Bar {
        points: Vec<Point>,
        y_axis: AxisRange,
        /// Categories folded into a trailing `+N more` bucket.
        collapsed_categories: Option<usize>,
    },
    Kpi {
        value: Option<f64>,
        /// Rows that contributed. A KPI over 3 of 10,000 rows is a
        /// different claim than one over all of them.
        sample_size: usize,
    },
    Table {
        headers: Vec<Label>,
        rows: Vec<Vec<String>>,
        total_rows: usize,
    },
}

/// A widget resolved against its data, ready to draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedWidget {
    pub id: String,
    pub title: Label,
    pub plan: RenderPlan,
}

/// Resolve one widget slot against a series' rows.
///
/// Never fails: every degenerate case has a defined rendering, because
/// a board that shows an error where a chart should be is worse than a
/// board that says "no data yet".
pub fn resolve(slot: &WidgetSlot, def: &SeriesDef, rows: &[Row], want_log: bool) -> ResolvedWidget {
    let title = Label::new(&slot.title);
    let plan = match slot.kind {
        WidgetKind::Table => resolve_table(def, rows),
        WidgetKind::Kpi => resolve_kpi(slot, def, rows),
        WidgetKind::Line => resolve_line(slot, def, rows, want_log),
        WidgetKind::Bar => resolve_bar(slot, def, rows, want_log),
    };
    ResolvedWidget {
        id: slot.id.clone(),
        title,
        plan,
    }
}

fn column_index(def: &SeriesDef, name: &Option<String>) -> Option<usize> {
    let want = name.as_ref()?;
    def.columns.iter().position(|c| &c.name == want)
}

fn numeric(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

fn resolve_table(def: &SeriesDef, rows: &[Row]) -> RenderPlan {
    if rows.is_empty() {
        return RenderPlan::Empty {
            reason: "no rows yet".to_string(),
        };
    }
    let headers = def.columns.iter().map(|c| Label::new(&c.name)).collect();
    let out: Vec<Vec<String>> = rows
        .iter()
        .take(MAX_TABLE_ROWS)
        .map(|r| r.values.iter().map(|v| v.to_display()).collect())
        .collect();
    RenderPlan::Table {
        headers,
        rows: out,
        // The renderer shows "N of M" from this; truncation is never
        // silent.
        total_rows: rows.len(),
    }
}

fn resolve_kpi(slot: &WidgetSlot, def: &SeriesDef, rows: &[Row]) -> RenderPlan {
    let Some(yi) = column_index(def, &slot.y_column) else {
        return RenderPlan::Empty {
            reason: "no value column bound".to_string(),
        };
    };
    // Last non-null wins: a KPI is "where is it now", and a trailing
    // gap must not read as a drop to zero.
    let mut sample = 0usize;
    let mut value = None;
    for r in rows {
        if let Some(n) = r.values.get(yi).and_then(numeric) {
            value = Some(n);
            sample += 1;
        }
    }
    if value.is_none() {
        return RenderPlan::Empty {
            reason: "every value in this series is empty".to_string(),
        };
    }
    RenderPlan::Kpi {
        value,
        sample_size: sample,
    }
}

/// Evenly-spaced decimation.
///
/// Keeps the first and last point so the span is honest, which
/// `chunks().first()` alone does not guarantee.
fn downsample(points: Vec<Point>, target: usize) -> (Vec<Point>, Option<usize>) {
    let n = points.len();
    if n <= target {
        return (points, None);
    }
    let mut out = Vec::with_capacity(target);
    for i in 0..target {
        let idx = i * (n - 1) / (target - 1);
        out.push(points[idx].clone());
    }
    (out, Some(n))
}

fn resolve_line(slot: &WidgetSlot, def: &SeriesDef, rows: &[Row], want_log: bool) -> RenderPlan {
    let (Some(xi), Some(yi)) = (
        column_index(def, &slot.x_column),
        column_index(def, &slot.y_column),
    ) else {
        return RenderPlan::Empty {
            reason: "axis columns are not bound".to_string(),
        };
    };
    if rows.is_empty() {
        return RenderPlan::Empty {
            reason: "no rows yet".to_string(),
        };
    }

    let points: Vec<Point> = rows
        .iter()
        .map(|r| Point {
            x: r.values.get(xi).map(|v| v.to_display()).unwrap_or_default(),
            // Null stays null. The renderer breaks the line here; it
            // does not draw zero and it does not interpolate across.
            y: r.values.get(yi).and_then(numeric),
        })
        .collect();

    let ys: Vec<f64> = points.iter().filter_map(|p| p.y).collect();
    let Some(y_axis) = AxisRange::from_values(&ys, want_log) else {
        return RenderPlan::Empty {
            reason: "every value in this series is empty".to_string(),
        };
    };

    let (points, downsampled_from) = downsample(points, MAX_CHART_POINTS);
    RenderPlan::Line {
        points,
        y_axis,
        downsampled_from,
    }
}

fn resolve_bar(slot: &WidgetSlot, def: &SeriesDef, rows: &[Row], want_log: bool) -> RenderPlan {
    let (Some(xi), Some(yi)) = (
        column_index(def, &slot.x_column),
        column_index(def, &slot.y_column),
    ) else {
        return RenderPlan::Empty {
            reason: "axis columns are not bound".to_string(),
        };
    };
    if rows.is_empty() {
        return RenderPlan::Empty {
            reason: "no rows yet".to_string(),
        };
    }

    // Sum by category, preserving first-seen order so the chart is
    // stable across refreshes.
    let mut order: Vec<String> = Vec::new();
    let mut totals: std::collections::HashMap<String, f64> = Default::default();
    let mut any_value = false;
    for r in rows {
        let key = r.values.get(xi).map(|v| v.to_display()).unwrap_or_default();
        if !totals.contains_key(&key) {
            order.push(key.clone());
        }
        let entry = totals.entry(key).or_insert(0.0);
        if let Some(n) = r.values.get(yi).and_then(numeric) {
            *entry += n;
            any_value = true;
        }
    }
    if !any_value {
        return RenderPlan::Empty {
            reason: "every value in this series is empty".to_string(),
        };
    }

    // Largest first, so the collapsed tail is the least interesting
    // part rather than an arbitrary alphabetical slice.
    let mut pairs: Vec<(String, f64)> = order
        .into_iter()
        .map(|k| {
            let v = totals.get(&k).copied().unwrap_or(0.0);
            (k, v)
        })
        .collect();
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1));

    let collapsed_categories = if pairs.len() > MAX_CATEGORIES {
        let tail: f64 = pairs[MAX_CATEGORIES..].iter().map(|(_, v)| *v).sum();
        let n = pairs.len() - MAX_CATEGORIES;
        pairs.truncate(MAX_CATEGORIES);
        pairs.push((format!("+{n} more"), tail));
        Some(n)
    } else {
        None
    };

    let ys: Vec<f64> = pairs.iter().map(|(_, v)| *v).collect();
    let y_axis = AxisRange::from_values(&ys, want_log).unwrap_or(AxisRange {
        min: 0.0,
        max: 1.0,
        scale: AxisScale::Linear,
        padded: true,
    });

    RenderPlan::Bar {
        points: pairs
            .into_iter()
            .map(|(k, v)| Point {
                x: Label::new(&k).text,
                y: Some(v),
            })
            .collect(),
        y_axis,
        collapsed_categories,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::series::{Column, ColumnType, Provenance, WriterId, WriterKind};
    use chrono::Utc;

    fn def() -> SeriesDef {
        SeriesDef::new(
            "s",
            vec![
                Column::new("at", ColumnType::String),
                Column::new("v", ColumnType::Number),
            ],
        )
    }

    fn row(x: &str, y: Option<f64>) -> Row {
        Row {
            values: vec![
                Value::String(x.to_string()),
                y.map(Value::Number).unwrap_or(Value::Null),
            ],
            writer_seq: 1,
            provenance: Provenance {
                writer: WriterId::new(WriterKind::Cli, "t"),
                pushed_at: Utc::now(),
                verified: false,
            },
        }
    }

    fn slot(kind: WidgetKind) -> WidgetSlot {
        WidgetSlot {
            id: "w".into(),
            kind,
            series: "s".into(),
            title: "T".into(),
            x_column: Some("at".into()),
            y_column: Some("v".into()),
        }
    }

    #[test]
    fn an_empty_series_renders_an_empty_state_not_an_axis() {
        let r = resolve(&slot(WidgetKind::Line), &def(), &[], false);
        assert!(matches!(r.plan, RenderPlan::Empty { .. }));
    }

    #[test]
    fn an_all_null_series_renders_empty_rather_than_a_flat_zero_line() {
        // Drawing zeros here reports an outage that did not happen.
        let rows = vec![row("a", None), row("b", None)];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, false);
        assert!(matches!(r.plan, RenderPlan::Empty { .. }));
    }

    #[test]
    fn a_null_inside_a_series_stays_a_gap() {
        let rows = vec![row("a", Some(1.0)), row("b", None), row("c", Some(3.0))];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, false);
        match r.plan {
            RenderPlan::Line { points, .. } => {
                assert_eq!(points[1].y, None, "gap became a number");
            }
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_flat_series_gets_a_padded_range_instead_of_zero_height() {
        let rows = vec![row("a", Some(5.0)), row("b", Some(5.0))];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, false);
        match r.plan {
            RenderPlan::Line { y_axis, .. } => {
                assert!(y_axis.padded);
                assert!(y_axis.min < y_axis.max, "min == max survived");
            }
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_single_point_also_gets_a_padded_range() {
        let rows = vec![row("a", Some(2.0))];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, false);
        match r.plan {
            RenderPlan::Line { y_axis, .. } => assert!(y_axis.min < y_axis.max),
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_flat_zero_series_pads_without_dividing_by_zero() {
        let rows = vec![row("a", Some(0.0)), row("b", Some(0.0))];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, false);
        match r.plan {
            RenderPlan::Line { y_axis, .. } => {
                assert!(y_axis.min.is_finite() && y_axis.max.is_finite());
                assert!(y_axis.min < y_axis.max);
            }
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_log_axis_containing_zero_falls_back_and_says_why() {
        let rows = vec![row("a", Some(0.0)), row("b", Some(10.0))];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, true);
        match r.plan {
            RenderPlan::Line { y_axis, .. } => assert!(matches!(
                y_axis.scale,
                AxisScale::LogFellBackToLinear { .. }
            )),
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_log_axis_over_positive_values_stays_log() {
        let rows = vec![row("a", Some(1.0)), row("b", Some(100.0))];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, true);
        match r.plan {
            RenderPlan::Line { y_axis, .. } => assert_eq!(y_axis.scale, AxisScale::Log),
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_negative_value_also_disqualifies_a_log_axis() {
        let rows = vec![row("a", Some(-1.0)), row("b", Some(10.0))];
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, true);
        match r.plan {
            RenderPlan::Line { y_axis, .. } => assert!(matches!(
                y_axis.scale,
                AxisScale::LogFellBackToLinear { .. }
            )),
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_huge_series_is_downsampled_and_says_so() {
        let rows: Vec<Row> = (0..10_000)
            .map(|i| row(&i.to_string(), Some(i as f64)))
            .collect();
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, false);
        match r.plan {
            RenderPlan::Line {
                points,
                downsampled_from,
                ..
            } => {
                assert_eq!(points.len(), MAX_CHART_POINTS);
                assert_eq!(downsampled_from, Some(10_000));
                // First and last must survive or the span is a lie.
                assert_eq!(points.first().unwrap().x, "0");
                assert_eq!(points.last().unwrap().x, "9999");
            }
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn a_small_series_is_not_downsampled() {
        let rows: Vec<Row> = (0..10).map(|i| row(&i.to_string(), Some(1.0))).collect();
        let r = resolve(&slot(WidgetKind::Line), &def(), &rows, false);
        match r.plan {
            RenderPlan::Line {
                downsampled_from, ..
            } => assert_eq!(downsampled_from, None),
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn high_cardinality_bars_collapse_into_a_more_bucket() {
        let rows: Vec<Row> = (0..200)
            .map(|i| row(&format!("cat{i}"), Some(i as f64)))
            .collect();
        let r = resolve(&slot(WidgetKind::Bar), &def(), &rows, false);
        match r.plan {
            RenderPlan::Bar {
                points,
                collapsed_categories,
                ..
            } => {
                assert_eq!(points.len(), MAX_CATEGORIES + 1);
                assert_eq!(collapsed_categories, Some(200 - MAX_CATEGORIES));
                assert!(points.last().unwrap().x.starts_with("+"));
            }
            other => panic!("expected bar, got {other:?}"),
        }
    }

    #[test]
    fn bars_sum_duplicate_categories() {
        let rows = vec![
            row("a", Some(1.0)),
            row("a", Some(2.0)),
            row("b", Some(5.0)),
        ];
        let r = resolve(&slot(WidgetKind::Bar), &def(), &rows, false);
        match r.plan {
            RenderPlan::Bar { points, .. } => {
                assert_eq!(points.len(), 2);
                // Sorted largest first.
                assert_eq!(points[0].x, "b");
                assert_eq!(points[1].y, Some(3.0));
            }
            other => panic!("expected bar, got {other:?}"),
        }
    }

    #[test]
    fn a_table_reports_its_true_row_count_when_truncated() {
        let rows: Vec<Row> = (0..MAX_TABLE_ROWS + 100)
            .map(|i| row(&i.to_string(), Some(1.0)))
            .collect();
        let r = resolve(&slot(WidgetKind::Table), &def(), &rows, false);
        match r.plan {
            RenderPlan::Table {
                rows: out,
                total_rows,
                ..
            } => {
                assert_eq!(out.len(), MAX_TABLE_ROWS);
                // Never a silent truncation.
                assert_eq!(total_rows, MAX_TABLE_ROWS + 100);
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn a_kpi_takes_the_last_non_null_not_the_last_row() {
        // A trailing gap must not read as a drop to zero.
        let rows = vec![row("a", Some(7.0)), row("b", None)];
        let r = resolve(&slot(WidgetKind::Kpi), &def(), &rows, false);
        match r.plan {
            RenderPlan::Kpi { value, sample_size } => {
                assert_eq!(value, Some(7.0));
                assert_eq!(sample_size, 1);
            }
            other => panic!("expected kpi, got {other:?}"),
        }
    }

    #[test]
    fn long_labels_truncate_but_keep_the_full_text() {
        let long = "x".repeat(200);
        let l = Label::new(&long);
        assert!(l.text.chars().count() <= MAX_LABEL_CHARS);
        assert_eq!(l.full.as_deref(), Some(long.as_str()));
    }

    #[test]
    fn axis_scale_serializes_as_a_tagged_object_for_every_variant() {
        // src/types/board.ts narrows on `kind`. Under serde's default
        // representation the unit variants emit bare STRINGS, and
        // `"..." in scale` then throws TypeError on every ordinary
        // chart. That shipped once; this locks the shape.
        for scale in [
            AxisScale::Linear,
            AxisScale::Log,
            AxisScale::LogFellBackToLinear {
                reason: "x".to_string(),
            },
        ] {
            let v = serde_json::to_value(&scale).unwrap();
            assert!(
                v.is_object(),
                "AxisScale serialized as a non-object: {v} — the TS union cannot narrow it"
            );
            assert!(
                v.get("kind").and_then(|k| k.as_str()).is_some(),
                "AxisScale is missing its `kind` discriminant: {v}"
            );
        }
    }

    #[test]
    fn short_labels_carry_no_redundant_full_text() {
        let l = Label::new("cost");
        assert_eq!(l.text, "cost");
        assert_eq!(l.full, None);
    }
}
