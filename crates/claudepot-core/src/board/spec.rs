//! The board spec — a named widget grid, revisioned.
//!
//! # Scope of validation here
//!
//! This module validates **placement**: that each widget slot has a
//! unique id, a supported kind, and column bindings that resolve
//! against a real series. That is everything the store and the terminal
//! renderer need.
//!
//! It does **not** implement the widget AST's recursive allowlist,
//! depth and size caps, or the render-time degenerate-data guards.
//! Those belong to the renderer and land with it (plan §14 step 6),
//! because they are gated on the §10.1 trial passing. Until then no
//! renderer consumes a spec, so there is no surface for the escape-hatch
//! class of bug the allowlist exists to prevent.
//!
//! What is fixed here and must not be relaxed later: [`WidgetKind`] is
//! a **closed enum**, not a free string, and there is no field that
//! carries renderer options through. Plan §7 rejects a raw-option
//! escape hatch outright — JSON strips functions but leaves
//! `image://` loaders, HTML-capable tooltip templates, `javascript:`
//! URLs, and `__proto__` keys live against a recursive deep merge.

use serde::{Deserialize, Serialize};

use super::error::{redact_identifier, BoardError};
use super::series::{check_name, SeriesDef};

/// Maximum widget slots on one board. A board is a glance surface; a
/// grid past this is a report, and reports are out of scope (plan §13).
pub const MAX_WIDGETS: usize = 24;

/// The v1 widget set. Four kinds, chosen against the overnight-run
/// case in plan §1: table for rows and anomalies, KPI for totals and
/// deltas, line for trend, bar for comparison across a category.
///
/// Area, scatter, histogram, and heatmap are deliberately absent until
/// the AST and its render guards exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetKind {
    Line,
    Bar,
    Table,
    Kpi,
}

impl WidgetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WidgetKind::Line => "line",
            WidgetKind::Bar => "bar",
            WidgetKind::Table => "table",
            WidgetKind::Kpi => "kpi",
        }
    }

    /// Whether this kind needs an explicit `x_column` binding.
    ///
    /// `table` renders every column, and `kpi` reduces one column to a
    /// scalar — neither has an x axis to bind.
    fn needs_x(self) -> bool {
        matches!(self, WidgetKind::Line | WidgetKind::Bar)
    }

    /// Whether this kind needs an explicit `y_column` binding.
    fn needs_y(self) -> bool {
        matches!(self, WidgetKind::Line | WidgetKind::Bar | WidgetKind::Kpi)
    }
}

/// One widget on the grid, bound to a series.
///
/// Slot order in [`BoardSpec::widgets`] is the layout order. There is
/// no geometry (row / column / span) because nothing renders a grid
/// yet; adding coordinates before a renderer exists would be
/// speculative scaffolding that the renderer then has to honor or
/// migrate away from.
/// `deny_unknown_fields` is load-bearing, not tidiness: plan §7 says
/// unknown keys are **rejected**, and a spec arrives from an agent over
/// MCP or from an export envelope. Silently ignoring an unrecognized
/// key is how a renderer-option passthrough gets reintroduced by
/// accident — the writer thinks the key took effect, and nothing says
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetSlot {
    pub id: String,
    pub kind: WidgetKind,
    /// Name of the series this widget reads.
    pub series: String,
    pub title: String,
    /// Column driving the x axis. Required for `line` and `bar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_column: Option<String>,
    /// Column driving the value. Required for `line`, `bar`, and `kpi`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_column: Option<String>,
}

/// The board's structure. Persisted as JSON, revisioned by the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardSpec {
    #[serde(default)]
    pub widgets: Vec<WidgetSlot>,
}

impl BoardSpec {
    pub fn new(widgets: Vec<WidgetSlot>) -> Self {
        Self { widgets }
    }

    /// An empty spec. Legal: a board can collect series before anyone
    /// decides how to display them, which is exactly what the terminal
    /// trial in plan §10.1 does.
    pub fn empty() -> Self {
        Self {
            widgets: Vec::new(),
        }
    }

    /// Validate placement against the board's series definitions.
    ///
    /// Every failure names the widget id, because a spec rejected
    /// without saying which slot is wrong is a spec the writer cannot
    /// fix.
    pub fn validate(&self, series: &[SeriesDef]) -> Result<(), BoardError> {
        if self.widgets.len() > MAX_WIDGETS {
            return Err(BoardError::CapExceeded {
                what: "widgets per board",
                limit: MAX_WIDGETS,
                actual: self.widgets.len(),
            });
        }

        let mut ids: Vec<&str> = Vec::with_capacity(self.widgets.len());

        for w in &self.widgets {
            check_name(&w.id)?;
            ids.push(w.id.as_str());

            let def = series.iter().find(|s| s.name == w.series).ok_or_else(|| {
                BoardError::InvalidSpec(format!(
                    "widget `{}` binds series `{}`, which does not exist",
                    redact_identifier(&w.id),
                    redact_identifier(&w.series)
                ))
            })?;

            let has_column = |name: &str| def.columns.iter().any(|c| c.name == name);

            match (&w.x_column, w.kind.needs_x()) {
                (Some(c), _) if !has_column(c) => {
                    return Err(BoardError::InvalidSpec(format!(
                        "widget `{}` binds x_column `{}`, absent from series `{}`",
                        redact_identifier(&w.id),
                        redact_identifier(c),
                        redact_identifier(&w.series)
                    )));
                }
                (None, true) => {
                    return Err(BoardError::InvalidSpec(format!(
                        "widget `{}` is a {} and needs an x_column",
                        redact_identifier(&w.id),
                        w.kind.as_str()
                    )));
                }
                _ => {}
            }

            match (&w.y_column, w.kind.needs_y()) {
                (Some(c), _) if !has_column(c) => {
                    return Err(BoardError::InvalidSpec(format!(
                        "widget `{}` binds y_column `{}`, absent from series `{}`",
                        redact_identifier(&w.id),
                        redact_identifier(c),
                        redact_identifier(&w.series)
                    )));
                }
                (None, true) => {
                    return Err(BoardError::InvalidSpec(format!(
                        "widget `{}` is a {} and needs a y_column",
                        redact_identifier(&w.id),
                        w.kind.as_str()
                    )));
                }
                _ => {}
            }
        }

        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        if ids.len() != before {
            return Err(BoardError::InvalidSpec(
                "widget ids must be unique within a board".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::series::{Column, ColumnType};

    fn series() -> Vec<SeriesDef> {
        vec![SeriesDef::new(
            "runs",
            vec![
                Column::new("at", ColumnType::Timestamp),
                Column::new("cost", ColumnType::Number),
                Column::new("model", ColumnType::String),
            ],
        )]
    }

    fn line(id: &str) -> WidgetSlot {
        WidgetSlot {
            id: id.to_string(),
            kind: WidgetKind::Line,
            series: "runs".to_string(),
            title: "Cost".to_string(),
            x_column: Some("at".to_string()),
            y_column: Some("cost".to_string()),
        }
    }

    #[test]
    fn empty_spec_is_valid() {
        // A board may collect series before anyone decides how to show
        // them — that is the terminal-trial workflow.
        assert!(BoardSpec::empty().validate(&series()).is_ok());
    }

    #[test]
    fn well_formed_line_widget_validates() {
        assert!(BoardSpec::new(vec![line("cost")])
            .validate(&series())
            .is_ok());
    }

    #[test]
    fn widget_bound_to_a_missing_series_is_rejected() {
        let mut w = line("cost");
        w.series = "absent".to_string();
        assert!(matches!(
            BoardSpec::new(vec![w]).validate(&series()),
            Err(BoardError::InvalidSpec(_))
        ));
    }

    #[test]
    fn widget_bound_to_a_missing_column_is_rejected() {
        let mut w = line("cost");
        w.y_column = Some("nope".to_string());
        assert!(matches!(
            BoardSpec::new(vec![w]).validate(&series()),
            Err(BoardError::InvalidSpec(_))
        ));
    }

    #[test]
    fn line_without_an_x_column_is_rejected() {
        let mut w = line("cost");
        w.x_column = None;
        assert!(matches!(
            BoardSpec::new(vec![w]).validate(&series()),
            Err(BoardError::InvalidSpec(_))
        ));
    }

    #[test]
    fn table_needs_no_axis_bindings() {
        let w = WidgetSlot {
            id: "rows".to_string(),
            kind: WidgetKind::Table,
            series: "runs".to_string(),
            title: "Runs".to_string(),
            x_column: None,
            y_column: None,
        };
        assert!(BoardSpec::new(vec![w]).validate(&series()).is_ok());
    }

    #[test]
    fn kpi_needs_a_value_column_but_no_x() {
        let mut w = WidgetSlot {
            id: "total".to_string(),
            kind: WidgetKind::Kpi,
            series: "runs".to_string(),
            title: "Total".to_string(),
            x_column: None,
            y_column: None,
        };
        assert!(BoardSpec::new(vec![w.clone()]).validate(&series()).is_err());
        w.y_column = Some("cost".to_string());
        assert!(BoardSpec::new(vec![w]).validate(&series()).is_ok());
    }

    #[test]
    fn duplicate_widget_ids_are_rejected() {
        let spec = BoardSpec::new(vec![line("same"), line("same")]);
        assert!(matches!(
            spec.validate(&series()),
            Err(BoardError::InvalidSpec(_))
        ));
    }

    #[test]
    fn widget_count_cap_is_enforced() {
        let widgets = (0..MAX_WIDGETS + 1)
            .map(|i| line(&format!("w{i}")))
            .collect();
        assert!(matches!(
            BoardSpec::new(widgets).validate(&series()),
            Err(BoardError::CapExceeded { .. })
        ));
    }

    #[test]
    fn widget_kind_is_a_closed_enum_with_no_raw_option_passthrough() {
        // Plan §7: accepting a renderer option object is not a
        // boundary. This test fails to compile-by-inspection if a
        // free-form field is ever added; it asserts the wire form
        // stays a bare tag.
        let json = serde_json::to_string(&WidgetKind::Line).unwrap();
        assert_eq!(json, "\"line\"");
        assert!(serde_json::from_str::<WidgetKind>("\"vega\"").is_err());
        assert!(serde_json::from_str::<WidgetKind>("{\"raw\":{}}").is_err());
    }
}
