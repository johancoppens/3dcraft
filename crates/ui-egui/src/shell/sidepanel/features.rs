//! The Features tab: the history as a list (plan of 2026-11-02, boards
//! 01, 10, 11, 15, 16).
//!
//! - Each row has its kind's icon (a sketch's in the sketch indigo, a
//!   datum's in the datum petrol), its name and its key value at the
//!   right (`Extrude 8 mm`, `R18 × 60 mm`, `− Bore`).
//! - A sketch sits folded under the feature that uses it, when nothing
//!   else lies between them in the history; datums that follow one
//!   another fold into one group in their place ([`outline`]): the order
//!   shown is the history's.
//! - The pointer on a row shows its grip, its eye and its value as a
//!   field (a click edits it: `feature.edit`, one undo step), and marks
//!   what it uses (↑) and what uses it (↓) in indigo, with a strip that
//!   names them and counts the rest ([`dependencies`]).
//! - A feature that failed for a reason of its own is a red row with the
//!   reason, *Show where* and *Edit*; the features made from it are dimmed:
//!   regeneration does not build them (`regen/src/cause.rs`: a feature
//!   whose input failed fails itself with "fix it first", without being
//!   built), while the independent ones after it are built as usual.
//! - The roll-back bar sits after the last feature built; its grip drags
//!   it (`history.rollback`). A row's grip drags it to another place
//!   (`feature.move`); a place before what it is made from, or after what
//!   is made from it, is refused before the drop.
//! - A right click opens the row's menu: Edit, Rename, Show dependencies,
//!   Roll back to here (Roll forward to end on the last row, and on the
//!   bar's own menu), Suppress, Hide, Copy id, Delete, each a command.

use super::{Drag, Editing, Filter, SideTab, matches};
use crate::kit::{Button, Depends, Glyph, Ground, Kit, ListRow, MenuItem, RowMark};
use crate::shell::views::FlyTo;
use crate::shell::{Shell, TreeRow, side_inset, timeline};
use crate::theme::color;
use craft3d_engine::{FeatureState, HistoryEntry, HistoryView};
use egui::{Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// The fewest rows the list keeps in view: open checks that would leave
/// it fewer fold to their header.
const MIN_LIST_ROWS: f32 = 6.0;

/// A line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideLine {
    /// The feature's id; for a group of datums, `datums:` and its first
    /// datum's id.
    pub id: String,
    /// The feature's place in the history; `None` for a group.
    pub row: Option<usize>,
    /// How deep it is nested.
    pub depth: u8,
    /// A group's datums, by id, in history order.
    pub members: Vec<String>,
    /// Something is nested under it: open or folded.
    pub fold: Option<bool>,
    /// The places in the history of what is nested under it, folded or
    /// not (a feature's sketches).
    pub nested: Vec<usize>,
    pub name: String,
    pub key: String,
}

/// A feature as [`outline`] takes it.
#[derive(Clone, Debug)]
pub struct OutlineItem<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub is_sketch: bool,
    pub is_datum: bool,
    pub uses: &'a [String],
    pub key: String,
}

/// The fold key of a group of datums starting with `first`.
pub fn group_key(first: &str) -> String {
    format!("datums:{first}")
}

/// The lines of the list for `items` (the history, in order): a sketch
/// nested under the first later feature made from it when only sketches
/// nested there lie between them; two datums or more in a row folded into
/// a group in their place. `open` says which features and groups are
/// unfolded. With a `filter`, only what holds it is listed, with what it
/// is nested in, opened.
pub fn outline(
    items: &[OutlineItem<'_>],
    open: &dyn Fn(&str) -> bool,
    filter: &str,
) -> Vec<SideLine> {
    let n = items.len();
    // The feature each sketch is made into: the first after it.
    let mut made_into: Vec<Option<usize>> = vec![None; n];
    for (i, (item, into)) in items.iter().zip(made_into.iter_mut()).enumerate() {
        if !item.is_sketch {
            continue;
        }
        *into = items
            .iter()
            .enumerate()
            .skip(i + 1)
            .find(|(_, f)| !f.is_sketch && !f.is_datum && f.uses.iter().any(|u| u == item.id))
            .map(|(j, _)| j);
    }
    let mut parent: Vec<Option<usize>> = vec![None; n];
    for (i, slot) in parent.iter_mut().enumerate() {
        let Some(j) = made_into.get(i).copied().flatten() else {
            continue;
        };
        let between_nested = (i + 1..j).all(|k| {
            items.get(k).is_some_and(|f| f.is_sketch)
                && made_into.get(k).copied().flatten() == Some(j)
        });
        if between_nested {
            *slot = Some(j);
        }
    }
    let mut children: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, p) in parent.iter().enumerate() {
        if let Some(p) = p {
            children.entry(*p).or_default().push(i);
        }
    }
    let filtering = !filter.trim().is_empty();
    let holds = |i: usize| {
        items.get(i).is_some_and(|f| {
            matches(f.name, filter) || matches(f.kind, filter) || matches(&f.key, filter)
        })
    };
    let line = |i: usize, depth: u8, fold: Option<bool>| {
        let f = items.get(i);
        SideLine {
            id: f.map_or_else(String::new, |f| f.id.to_string()),
            row: Some(i),
            depth,
            members: Vec::new(),
            fold,
            nested: Vec::new(),
            name: f.map_or_else(String::new, |f| f.name.to_string()),
            key: f.map_or_else(String::new, |f| f.key.clone()),
        }
    };
    let mut lines = Vec::new();
    let mut i = 0;
    // Each pass moves on by one feature at least.
    while i < n {
        if parent.get(i).copied().flatten().is_some() {
            i += 1;
            continue;
        }
        let datum = items.get(i).is_some_and(|f| f.is_datum);
        if datum {
            let mut last = i;
            while items.get(last + 1).is_some_and(|f| f.is_datum)
                && parent.get(last + 1).copied().flatten().is_none()
            {
                last += 1;
            }
            if last > i {
                let members: Vec<usize> = (i..=last).collect();
                let first = items.get(i).map_or("", |f| f.id);
                let key = group_key(first);
                let shown: Vec<usize> = if filtering {
                    members.iter().copied().filter(|&k| holds(k)).collect()
                } else {
                    members.clone()
                };
                if !shown.is_empty() {
                    let is_open = filtering || open(&key);
                    lines.push(SideLine {
                        id: key,
                        row: None,
                        depth: 0,
                        members: members
                            .iter()
                            .filter_map(|&k| items.get(k).map(|f| f.id.to_string()))
                            .collect(),
                        fold: Some(is_open),
                        nested: Vec::new(),
                        name: "Datums".to_string(),
                        key: members
                            .iter()
                            .filter_map(|&k| items.get(k))
                            .map(|f| f.key.rsplit(" · ").next().unwrap_or("").to_string())
                            .filter(|k| !k.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · "),
                    });
                    if is_open {
                        for k in shown {
                            lines.push(line(k, 1, None));
                        }
                    }
                }
                i = last + 1;
                continue;
            }
        }
        let kids = children.get(&i).cloned().unwrap_or_default();
        let id = items.get(i).map_or("", |f| f.id);
        let shown_kids: Vec<usize> = if filtering {
            kids.iter().copied().filter(|&k| holds(k)).collect()
        } else {
            kids.clone()
        };
        if !filtering || holds(i) || !shown_kids.is_empty() {
            let is_open = (filtering && !shown_kids.is_empty()) || (!filtering && open(id));
            let fold = (!kids.is_empty()).then_some(is_open);
            let mut parent = line(i, 0, fold);
            parent.nested = kids.clone();
            lines.push(parent);
            if is_open {
                for k in shown_kids {
                    lines.push(line(k, 1, None));
                }
            }
        }
        i += 1;
    }
    lines
}

/// What a feature is made from and what is made from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependencies {
    /// What it uses directly, by id, in history order.
    pub uses: Vec<String>,
    /// What uses it directly, by id, in history order.
    pub used_by: Vec<String>,
    /// How many more features are made from it through those.
    pub later: usize,
}

/// What the history's features are made from, indexed once per revision
/// of the history: each feature's place by id and its users, so a hover
/// costs a walk over what depends on it, not over the whole history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    /// Each feature's place among the features, by id.
    pub at: BTreeMap<String, usize>,
    /// What uses each feature directly, by id, in history order.
    pub used_by: BTreeMap<String, Vec<String>>,
}

impl Graph {
    pub fn of(features: &[HistoryEntry]) -> Self {
        let mut graph = Graph::default();
        for (i, f) in features.iter().enumerate() {
            graph.at.insert(f.id.clone(), i);
            for input in &f.uses {
                let users = graph.used_by.entry(input.clone()).or_default();
                if users.last() != Some(&f.id) {
                    users.push(f.id.clone());
                }
            }
        }
        graph
    }

    /// The dependencies of `id` among `features` (those the graph was
    /// made of).
    pub fn dependencies(&self, features: &[HistoryEntry], id: &str) -> Dependencies {
        let Some(me) = self.at.get(id).and_then(|&i| features.get(i)) else {
            return Dependencies::default();
        };
        let mut uses: Vec<(usize, String)> = me
            .uses
            .iter()
            .filter_map(|u| self.at.get(u).map(|&i| (i, u.clone())))
            .collect();
        uses.sort();
        uses.dedup();
        let users = |of: &str| self.used_by.get(of).map_or(&[][..], Vec::as_slice);
        let used_by: Vec<String> = users(id).to_vec();
        // Everything made from it, through what uses it: each feature
        // seen once, so the walk ends.
        let mut seen: BTreeSet<&str> = used_by.iter().map(String::as_str).collect();
        let mut queue: Vec<&str> = used_by.iter().map(String::as_str).collect();
        while let Some(next) = queue.pop() {
            for user in users(next) {
                if user != id && seen.insert(user.as_str()) {
                    queue.push(user.as_str());
                }
            }
        }
        let later = seen.len().saturating_sub(used_by.len());
        Dependencies {
            uses: uses.into_iter().map(|(_, u)| u).collect(),
            used_by,
            later,
        }
    }
}

/// The dependencies of `id` among `features` (in history order).
pub fn dependencies(features: &[HistoryEntry], id: &str) -> Dependencies {
    Graph::of(features).dependencies(features, id)
}

/// A line of the dependency strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StripMark {
    NotRebuilt,
    Uses,
    UsedBy,
    Later,
}

/// What the outline was made from ([`Shell::cached_outline`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutlineKey {
    view: Option<(u64, u64)>,
    scene: Option<u64>,
    rows: usize,
    scrub: Option<usize>,
    filter: String,
    unfolded: Vec<String>,
}

/// The line at the top of the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Features built.
    pub rebuilt: usize,
    /// Features that failed for a reason of their own.
    pub failed: usize,
    /// Features not built because something they are made from failed.
    pub not_rebuilt: usize,
    /// Rolled back: after which feature, its place and of how many.
    pub rolled_back: Option<(String, usize, usize)>,
    /// Checks, and how many of them fail.
    pub checks: usize,
    pub checks_failing: usize,
}

/// Whether the regeneration skipped `row` because something it is made
/// from failed (it is dimmed, not red).
pub fn not_rebuilt(row: &TreeRow) -> bool {
    row.state == FeatureState::Failed && row.caused_by.is_some()
}

/// Whether `row` failed for a reason of its own (the red row).
pub fn failed_itself(row: &TreeRow) -> bool {
    row.state == FeatureState::Failed && row.caused_by.is_none()
}

impl Shell {
    /// The lines of the Features tab as they are listed now.
    pub fn side_outline(&self) -> Vec<SideLine> {
        let items = self.outline_items();
        let unfolded = &self.state.side_panel;
        outline(
            &items,
            &|key: &str| unfolded.is_open(key),
            &self.state.side_panel.filter,
        )
    }

    /// The dependencies of a feature, by id, as the pointer on its row
    /// shows them.
    pub fn side_dependencies(&self, id: &str) -> Dependencies {
        if let Some((key, cached, deps)) = &self.side.deps_cache
            && *key == self.side.view_for
            && cached == id
        {
            return deps.clone();
        }
        self.side
            .view
            .as_ref()
            .map(|view| self.side.graph.dependencies(&view.features, id))
            .unwrap_or_default()
    }

    /// The dependencies of `id`, kept until the history or the row
    /// changes: the hover asks every frame.
    fn hover_dependencies(&mut self, id: &str) -> Dependencies {
        let deps = self.side_dependencies(id);
        self.side.deps_cache = Some((self.side.view_for, id.to_string(), deps.clone()));
        deps
    }

    /// The lines, kept until what they are made of changes: the history,
    /// the scene's rows (their states), the scrub, the filter, the folds.
    fn cached_outline(&mut self) -> std::sync::Arc<Vec<SideLine>> {
        let key = OutlineKey {
            view: self.side.view_for,
            scene: self.shown_scene.as_ref().map(|s| s.revision),
            rows: self.rows.len(),
            scrub: self.timeline.position.as_ref().map(|p| p.step),
            filter: self.state.side_panel.filter.clone(),
            unfolded: self.state.side_panel.unfolded.clone(),
        };
        if let Some((k, lines)) = &self.side.outline_cache
            && *k == key
        {
            return lines.clone();
        }
        let lines = std::sync::Arc::new(self.side_outline());
        self.side.outline_cache = Some((key, lines.clone()));
        lines
    }

    /// The summary at the top of the Features tab.
    pub fn side_summary(&self) -> Summary {
        use craft3d_engine::AssertionStatus as A;
        let rows = &self.rows;
        let steps = self.timeline.steps;
        let rolled_back = self
            .timeline
            .rollback
            .filter(|&at| at > 0 && at < steps)
            .map(|at| {
                let name = rows
                    .get(at - 1)
                    .map_or_else(String::new, |r| r.name.clone());
                (name, at, steps)
            });
        Summary {
            rebuilt: rows
                .iter()
                .filter(|r| r.state == FeatureState::Ok && r.status != timeline::AFTER_SCRUB)
                .count(),
            failed: rows.iter().filter(|r| failed_itself(r)).count(),
            not_rebuilt: rows.iter().filter(|r| not_rebuilt(r)).count(),
            rolled_back,
            checks: self
                .assertion_rows
                .iter()
                .filter(|a| a.status != A::Inactive)
                .count(),
            checks_failing: self
                .assertion_rows
                .iter()
                .filter(|a| matches!(a.status, A::Failed | A::Error))
                .count(),
        }
    }

    /// The history as [`outline`] takes it: the scene's rows with the
    /// history view's key values (a sketch's with its degrees of
    /// freedom).
    fn outline_items(&self) -> Vec<OutlineItem<'_>> {
        let entries: BTreeMap<&str, &HistoryEntry> = self
            .side
            .view
            .as_ref()
            .map(|v| v.features.iter().map(|f| (f.id.as_str(), f)).collect())
            .unwrap_or_default();
        const NO_USES: &[String] = &[];
        self.rows
            .iter()
            .map(|row| {
                let entry = entries.get(row.id.as_str()).copied();
                let key = match entry {
                    Some(e) if e.is_sketch => sketch_key(&e.curves, &row.status),
                    Some(e) => e.key.clone(),
                    None => row.kind.to_string(),
                };
                OutlineItem {
                    id: &row.id,
                    name: &row.name,
                    kind: row.kind,
                    is_sketch: entry.is_some_and(|e| e.is_sketch) || row.kind == "sketch",
                    is_datum: entry.is_some_and(|e| e.is_datum),
                    uses: entry.map_or(NO_USES, |e| e.uses.as_slice()),
                    key,
                }
            })
            .collect()
    }

    /// Selects the feature of a row, as a click on it does: Shift adds it
    /// or takes it out, a click on the only selected row clears.
    pub(crate) fn click_feature(&mut self, id: String, shift: bool) {
        let only =
            self.state.selected_feature.as_ref() == Some(&id) && self.selection().items.len() <= 1;
        let request = if shift {
            json!({"items": [{"kind": "feature", "feature": id}], "mode": "toggle"})
        } else if only {
            json!({"mode": "clear"})
        } else {
            json!({"items": [{"kind": "feature", "feature": id}]})
        };
        self.state.selected_feature = if only && !shift { None } else { Some(id) };
        self.queue_select(request);
    }

    /// The Features tab.
    pub(super) fn features_tab(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let inset = f32::from(side_inset(&tokens));
        // The timeline at the foot, then the checks and the selection
        // above it.
        egui::Panel::bottom("craft3d-timeline")
            .resizable(false)
            .show(ui, |ui| {
                ui.set_max_width((ui.available_width() - inset).max(40.0));
                self.timeline_ui(ui);
            });
        let selected = !self.selection().items.is_empty();
        // The checks fold to their header while something is selected,
        // and when, open, they would leave the list less than a few rows;
        // what is selected shows only where it leaves the list those rows
        // (the status bar names it anyway).
        let top = kit.hit(tokens.control_height) + tokens.nav_gap + tokens.label_row_height + 4.0;
        let room = ui.available_height() - top;
        let least = MIN_LIST_ROWS * kit.hit(tokens.row_height);
        let folded_h = self.checks_height(ui, true);
        let props_h = if self.side.props.is_some() && room - folded_h - self.props_height() >= least
        {
            self.props_height()
        } else {
            0.0
        };
        let open_h = self.checks_height(ui, false);
        let short = room - props_h - open_h < least;
        let selected = selected || short;
        // As high as what it holds, worked out before it is drawn: a panel
        // that measured itself would be a frame late. What does not fit
        // scrolls inside it.
        let height = (self.checks_height(ui, selected) + props_h)
            .min(ui.available_height() * 0.6)
            .max(0.0);
        let show_props = props_h > 0.0;
        egui::Panel::bottom("craft3d-side-foot")
            .resizable(false)
            .exact_size(height)
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("craft3d-side-foot-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width((ui.available_width() - inset).max(40.0));
                        ui.spacing_mut().item_spacing.y = 0.0;
                        self.checks_ui(ui, selected);
                        if show_props {
                            self.props_ui(ui);
                        }
                    });
            });
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - inset).max(40.0));
            self.side_filter(ui, Filter::Features, "Filter features");
            ui.add_space(tokens.nav_gap);
            self.summary_line(ui, &kit);
            self.side_problem(ui);
        });
        if self.rows.is_empty() {
            let text = egui::RichText::new("No features yet.").color(kit.c(kit.skin.text_dim));
            ui.label(text);
            return;
        }
        self.feature_list(ui);
    }

    /// `25 rebuilt · 7 checks pass`; `1 failed · 4 not rebuilt`;
    /// `Rolled back to Bored · 10 of 25`.
    fn summary_line(&mut self, ui: &mut egui::Ui, kit: &Kit) {
        let s = self.side_summary();
        let t = kit.t;
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), t.label_row_height + 4.0),
            Sense::hover(),
        );
        self.side.hits.drawn.push(rect);
        let painter = ui.painter();
        let mut x = rect.left() + 2.0;
        let y = rect.center().y;
        let mut part = |painter: &egui::Painter, dot: Dot, text: &str, ink: crate::theme::Rgb| {
            let r = t.nav_icon_size * 0.42;
            status_dot(painter, kit, pos2(x + r, y), r, dot);
            x += 2.0 * r + t.nav_icon_gap;
            let g = kit.galley(ui, text, kit.font(), ink);
            let w = g.size().x;
            painter.galley(pos2(x, y - g.size().y * 0.5), g, color(ink));
            x += w + t.field_inset * 1.2;
        };
        let sk = kit.skin;
        if let Some((name, at, of)) = &s.rolled_back {
            part(
                painter,
                Dot::Info,
                &format!("Rolled back to {name} · {at} of {of}"),
                sk.text,
            );
            return;
        }
        if s.failed > 0 {
            part(
                painter,
                Dot::Failed,
                &format!("{} failed", s.failed),
                sk.failed,
            );
            if s.not_rebuilt > 0 {
                part(
                    painter,
                    Dot::Grey,
                    &format!("{} not rebuilt", s.not_rebuilt),
                    sk.text,
                );
            }
            return;
        }
        part(painter, Dot::Ok, &format!("{} rebuilt", s.rebuilt), sk.text);
        if s.checks > 0 {
            if s.checks_failing > 0 {
                let text = if s.checks_failing == 1 {
                    format!("1 of {} checks fails", s.checks)
                } else {
                    format!("{} of {} checks fail", s.checks_failing, s.checks)
                };
                part(painter, Dot::Failed, &text, sk.failed);
            } else {
                let text = if s.checks == 1 {
                    "1 check passes".to_string()
                } else {
                    format!("{} checks pass", s.checks)
                };
                part(painter, Dot::Ok, &text, sk.text);
            }
        }
    }

    /// The list itself, in a scroll area that fills what is left.
    fn feature_list(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let sk = kit.skin;
        let in_use = self.jig_in_use();
        let lines = self.cached_outline();
        let view: Option<std::sync::Arc<HistoryView>> = self.side.view.clone();
        let graph = self.side.graph.clone();
        let entry_of = |id: &str| -> Option<&HistoryEntry> {
            let v = view.as_deref()?;
            graph.at.get(id).and_then(|&i| v.features.get(i))
        };
        // What the row under the pointer (or the one asked for) uses and
        // what uses it.
        let hover = self.state.side_panel.hover.clone();
        let deps = hover
            .as_deref()
            .map(|id| self.hover_dependencies(id))
            .unwrap_or_default();
        let index: BTreeMap<String, usize> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        let steps = self.rows.len();
        // The roll-back bar goes before the first line past the marker.
        let marker = self
            .timeline
            .rollback
            .filter(|&at| at < steps)
            .unwrap_or(steps);
        let mut pointer_on: Option<String> = None;
        let mut clicked: Option<(String, bool)> = None;
        let mut double: Option<String> = None;
        let mut fold_clicked: Option<String> = None;
        let mut eye_clicked: Option<(String, bool)> = None;
        let mut edit_start: Option<(String, String)> = None;
        let mut menu_open: Option<(String, egui::Pos2)> = None;
        let mut commit: Option<(&'static str, serde_json::Value, String)> = None;
        let mut cancel = false;
        let mut show_where: Option<String> = None;
        let mut drag_start: Option<Drag> = None;
        let mut line_rects: Vec<(usize, Rect)> = Vec::new();
        let mut bar_rect: Option<Rect> = None;
        let mut grip_released = false;
        // A click in the view scrolls the list to the feature that made
        // it (to the group or the parent a folded one is in).
        let scroll_to = self.tree_scroll.take();
        // Where the list showed its rows in the last frame: a row there is
        // in view, and the list stays.
        let view_before = self.tree_view;
        let editing = self.side.editing.clone();
        let focus = std::mem::take(&mut self.side.focus_pending);
        let menu_for = self.state.side_panel.menu.clone();
        let mut rows_drawn: Vec<(String, Rect)> = Vec::new();
        let mut zooms_drawn: Vec<(String, Rect)> = Vec::new();
        let errors = self.side.errors.clone();
        // Taken out while the list is drawn (and put back after): no copy
        // of every row each frame.
        let rows = std::mem::take(&mut self.rows);
        let list = egui::ScrollArea::vertical()
            .id_salt(("craft3d-side-features", self.viewing))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: side_inset(&tokens),
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let mut bar_drawn = false;
                        for (n, line) in lines.iter().enumerate() {
                            let first_row = line
                                .row
                                .or_else(|| {
                                    line.members.first().and_then(|m| index.get(m).copied())
                                })
                                .unwrap_or(0);
                            if !bar_drawn && first_row >= marker && !line_is_nested(line) {
                                bar_rect = Some(rollback_bar(ui, &kit, marker, steps));
                                bar_drawn = true;
                            }
                            let row = line.row.and_then(|i| rows.get(i));
                            let entry = row.and_then(|r| entry_of(&r.id));
                            let (glyph, ink) = match (row, entry) {
                                (_, Some(e)) => {
                                    let g = Glyph::for_feature(e.kind, e.op);
                                    (g, g.feature_ink(sk.icon_sketch, sk.icon_datum, sk.text_dim))
                                }
                                (Some(r), None) => {
                                    let g = Glyph::for_feature(r.kind, None);
                                    (g, g.feature_ink(sk.icon_sketch, sk.icon_datum, sk.text_dim))
                                }
                                (None, _) => (Glyph::Datums, sk.icon_datum),
                            };
                            let id = line.id.clone();
                            let failed = row.is_some_and(failed_itself);
                            let dimmed = row.is_some_and(|r| {
                                not_rebuilt(r)
                                    || matches!(
                                        r.state,
                                        FeatureState::RolledBack
                                            | FeatureState::Suppressed
                                            | FeatureState::Inactive
                                    )
                                    || r.status == timeline::AFTER_SCRUB
                            }) || (line.row.is_none()
                                && line.members.iter().all(|m| {
                                    index
                                        .get(m)
                                        .and_then(|&i| rows.get(i))
                                        .is_some_and(|r| r.state != FeatureState::Ok)
                                }));
                            let value = match row {
                                Some(r) if failed_itself(r) => "failed".to_string(),
                                Some(r) if r.state == FeatureState::Suppressed => {
                                    "suppressed".to_string()
                                }
                                Some(r) if r.state == FeatureState::Inactive => "off".to_string(),
                                _ => line.key.clone(),
                            };
                            let hovered_here = hover.as_deref() == Some(id.as_str());
                            let depends = if hovered_here {
                                None
                            } else if deps
                                .uses
                                .iter()
                                .any(|u| *u == id || line.members.contains(u))
                            {
                                Some(Depends::Uses)
                            } else if deps
                                .used_by
                                .iter()
                                .any(|u| *u == id || line.members.contains(u))
                            {
                                Some(Depends::UsedBy)
                            } else {
                                None
                            };
                            let editable = !in_use && entry.is_some_and(|e| e.edit.is_some());
                            let editing_value =
                                matches!(&editing, Some(Editing::Value { id: e, .. }) if *e == id);
                            let editing_name =
                                matches!(&editing, Some(Editing::Name { id: e, .. }) if *e == id);
                            let selected = row.is_some_and(|r| r.selected || r.made_selection)
                                || menu_for.as_deref() == Some(id.as_str());
                            let hidden = row.is_some_and(|r| r.hidden);
                            let name = if editing_name { "" } else { line.name.as_str() };
                            let item = ListRow::new(name)
                                .icon(glyph)
                                .icon_ink(ink)
                                .value(&value)
                                .indent(line.depth.saturating_mul(1))
                                .fold(line.fold)
                                .selected(selected)
                                .dimmed(dimmed)
                                .failed(failed)
                                .hovered(hovered_here)
                                .grip(
                                    hovered_here
                                        && !in_use
                                        && line.depth == 0
                                        && line.row.is_some(),
                                )
                                .eye(hovered_here && !in_use && line.row.is_some())
                                .hidden(hidden)
                                .depends(depends)
                                .mark(agent_mark(row))
                                .zoom(hovered_here && row.is_some_and(|r| r.zoomable))
                                .value_field(
                                    hovered_here
                                        && editable
                                        && !editing_value
                                        && row.is_some_and(|r| r.selected),
                                )
                                .value_hidden(editing_value);
                            // A failed row's fill and bar go under it and
                            // its reason.
                            let back = failed.then(|| ui.painter().add(egui::Shape::Noop));
                            let hits = kit.list_row_hits(ui, item);
                            // A row not rebuilt says why on hover.
                            if let Some(r) = row
                                && not_rebuilt(r)
                                && let Some((_, root)) = &r.caused_by
                            {
                                kit.hover_tip(
                                    ui,
                                    &hits.response,
                                    &format!("Not rebuilt: {root} failed"),
                                );
                            }
                            let rect = hits.response.rect;
                            line_rects.push((n, rect));
                            rows_drawn.push((id.clone(), rect));
                            if hits.response.hovered() {
                                pointer_on = Some(id.clone());
                            }
                            if scroll_to
                                .as_ref()
                                .is_some_and(|t| *t == id || line.members.contains(t))
                                && !view_before.is_some_and(|v| mostly_in(rect, v))
                            {
                                // Only as far as it takes, and at once.
                                hits.response.scroll_to_me_animation(
                                    None,
                                    egui::style::ScrollAnimation::none(),
                                );
                            }
                            let pointer = hits.response.interact_pointer_pos();
                            let inside = |r: Option<Rect>| {
                                r.is_some_and(|r| pointer.is_some_and(|p| r.contains(p)))
                            };
                            if let Some(fold) = hits.fold {
                                self.side.hits.folds.push((id.clone(), fold));
                            }
                            if let Some(eye) = hits.eye {
                                self.side.hits.eyes.push((id.clone(), eye));
                            }
                            if let Some(zoom) = hits.zoom {
                                zooms_drawn.push((id.clone(), zoom));
                            }
                            if editable && line.row.is_some() {
                                self.side.hits.values.push((id.clone(), hits.value));
                            }
                            // The grip drags the row.
                            if let Some(grip) = hits.grip {
                                self.side.hits.targets.push((format!("grip:{id}"), grip));
                                let drag = ui.interact(
                                    grip,
                                    egui::Id::new(("craft3d-side-grip", &id)),
                                    Sense::drag(),
                                );
                                if drag.drag_started() {
                                    drag_start = Some(Drag::Row { id: id.clone() });
                                }
                            }
                            if hits.response.clicked() {
                                if inside(hits.fold) {
                                    fold_clicked = Some(id.clone());
                                } else if inside(hits.zoom) {
                                    double = Some(id.clone());
                                } else if inside(hits.eye) && line.row.is_some() {
                                    eye_clicked = Some((id.clone(), hidden));
                                } else if editable
                                    && row.is_some_and(|r| r.selected)
                                    && inside(Some(hits.value))
                                {
                                    // The value of the selected row: a
                                    // click edits it (the first click on a
                                    // row selects it, wherever it lands).
                                    if let Some(edit) = entry.and_then(|e| e.edit.as_ref()) {
                                        edit_start = Some((id.clone(), edit.expr.clone()));
                                    }
                                } else if line.row.is_none() {
                                    fold_clicked = Some(id.clone());
                                } else {
                                    let shift = ui.input(|i| i.modifiers.shift);
                                    clicked = Some((id.clone(), shift));
                                }
                            }
                            if hits.response.double_clicked() && line.row.is_some() {
                                double = Some(id.clone());
                            }
                            if hits.response.secondary_clicked() && line.row.is_some() {
                                menu_open = Some((id.clone(), pointer.unwrap_or(rect.center())));
                            }
                            // A field over the value, or over the name.
                            if editing_value || editing_name {
                                let (text, place, key) = match &editing {
                                    Some(Editing::Value { text, .. }) if editing_value => {
                                        let w =
                                            (hits.value.width()).max(tokens.control_height * 4.0);
                                        let place = Rect::from_min_max(
                                            pos2(hits.value.right() - w, rect.top()),
                                            pos2(hits.value.right(), rect.bottom()),
                                        );
                                        (text.clone(), place, format!("value:{id}"))
                                    }
                                    Some(Editing::Name { text, .. }) => {
                                        let left = rect.left()
                                            + tokens.nav_spacing * 4.0
                                            + f32::from(line.depth) * tokens.nav_icon_size
                                            + tokens.nav_icon_size * 1.75
                                            + tokens.nav_icon_gap * 1.5;
                                        let place = Rect::from_min_max(
                                            pos2(left, rect.top()),
                                            pos2(
                                                (hits.value.left() - tokens.nav_icon_gap)
                                                    .max(left + 40.0),
                                                rect.bottom(),
                                            ),
                                        );
                                        (text.clone(), place, format!("name:{id}"))
                                    }
                                    _ => (String::new(), rect, String::new()),
                                };
                                let mut text = text;
                                let error = errors.contains_key(&key);
                                let field = ui
                                    .scope_builder(egui::UiBuilder::new().max_rect(place), |ui| {
                                        kit.number_field(
                                            ui,
                                            ("craft3d-side-edit", &key),
                                            &mut text,
                                            "",
                                            place.width(),
                                            error,
                                        )
                                    })
                                    .inner;
                                if focus {
                                    field.request_focus();
                                }
                                if field.changed() {
                                    self.side.editing = match &editing {
                                        Some(Editing::Value { id, .. }) => Some(Editing::Value {
                                            id: id.clone(),
                                            text: text.clone(),
                                        }),
                                        Some(Editing::Name { id, .. }) => Some(Editing::Name {
                                            id: id.clone(),
                                            text: text.clone(),
                                        }),
                                        other => other.clone(),
                                    };
                                }
                                if field.lost_focus() {
                                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                        match &editing {
                                            Some(Editing::Value { .. }) => {
                                                if let Some(edit) =
                                                    entry.and_then(|e| e.edit.as_ref())
                                                {
                                                    let mut params = serde_json::Map::new();
                                                    params.insert(
                                                        "feature".into(),
                                                        id.clone().into(),
                                                    );
                                                    params.insert(
                                                        edit.param.into(),
                                                        text.trim().into(),
                                                    );
                                                    commit = Some((
                                                        "feature.edit",
                                                        params.into(),
                                                        key.clone(),
                                                    ));
                                                }
                                            }
                                            Some(Editing::Name { .. }) => {
                                                commit = Some((
                                                    "feature.rename",
                                                    json!({"feature": id, "name": text.trim()}),
                                                    key.clone(),
                                                ));
                                            }
                                            _ => {}
                                        }
                                    } else {
                                        cancel = true;
                                    }
                                }
                            }
                            if let Some(error) = errors
                                .get(&format!("value:{id}"))
                                .or_else(|| errors.get(&format!("name:{id}")))
                            {
                                let label = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(error)
                                            .color(kit.c(sk.failed))
                                            .size(tokens.secondary_font - 1.0),
                                    )
                                    .wrap(),
                                );
                                self.side.hits.drawn.push(label.rect);
                            }
                            // A failed feature: its reason, Show where and Edit.
                            if let (true, Some(r)) = (failed, row) {
                                let top = rect.top();
                                let reason = r.error.clone().unwrap_or_default();
                                let pad = tokens.nav_spacing * 4.0 + tokens.nav_icon_size;
                                let inner = ui
                                    .horizontal(|ui| {
                                        ui.add_space(pad);
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y =
                                                tokens.nav_spacing * 2.0;
                                            let label = ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&reason)
                                                        .color(kit.c(sk.failed))
                                                        .size(tokens.secondary_font),
                                                )
                                                .wrap(),
                                            );
                                            self.side.hits.drawn.push(label.rect);
                                            ui.horizontal(|ui| {
                                                let show = kit.button(
                                                    ui,
                                                    Button::primary("Show where").icon(Glyph::Nav(
                                                        crate::shell::icons::Icon::ZoomTo,
                                                    )),
                                                );
                                                self.side
                                                    .hits
                                                    .targets
                                                    .push((format!("show where:{id}"), show.rect));
                                                if show.clicked() {
                                                    show_where = Some(id.clone());
                                                }
                                                let edit = kit.button(
                                                    ui,
                                                    Button::secondary("Edit").enabled(!in_use),
                                                );
                                                self.side
                                                    .hits
                                                    .targets
                                                    .push((format!("edit:{id}"), edit.rect));
                                                if edit.clicked() {
                                                    match entry.and_then(|e| e.edit.as_ref()) {
                                                        Some(edit) => {
                                                            edit_start = Some((
                                                                id.clone(),
                                                                edit.expr.clone(),
                                                            ))
                                                        }
                                                        None => {
                                                            menu_open = Some((
                                                                id.clone(),
                                                                edit.rect.left_bottom(),
                                                            ));
                                                        }
                                                    }
                                                }
                                            });
                                            ui.add_space(tokens.nav_spacing * 2.0);
                                        });
                                    })
                                    .response;
                                let whole = Rect::from_min_max(
                                    pos2(rect.left(), top),
                                    pos2(rect.right(), inner.rect.bottom()),
                                );
                                if let Some(back) = back {
                                    ui.painter().set(
                                        back,
                                        egui::Shape::rect_filled(whole, 0.0, kit.c(sk.failed_fill)),
                                    );
                                }
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        whole.min,
                                        vec2(tokens.info_bar_width, whole.height()),
                                    ),
                                    0.0,
                                    kit.c(sk.failed),
                                );
                            }
                        }
                        if !bar_drawn {
                            bar_rect = Some(rollback_bar(ui, &kit, marker, steps));
                        }
                        // Room under the last row for the strip of the
                        // dependencies, which lies over the list's foot.
                        ui.add_space(tokens.row_height * 3.5);
                    });
            });
        self.rows = rows;
        let list_rect = list.inner_rect;
        self.tree_view = Some(list_rect);
        // The roll-back bar's grip drags the marker.
        if let Some(bar) = bar_rect {
            let grip =
                Rect::from_min_size(bar.min, vec2(tokens.control_height * 1.6, bar.height()));
            self.side.hits.targets.push(("rollback".to_string(), grip));
            let drag = ui
                .interact(
                    grip,
                    egui::Id::new("craft3d-side-rollback"),
                    Sense::click_and_drag(),
                )
                .on_hover_text(
                    "The roll-back bar: the model is built up to here, and new features go in \
                     after it. Drag it to another step, or to the end (right-click: Roll \
                     forward to end) to build the whole history again.",
                );
            let rolled_back = self.timeline.rollback.is_some();
            let mut forward = false;
            kit.context_menu(ui, &drag, |plate, ui| {
                let item = plate.menu_item(
                    ui,
                    crate::kit::MenuItem::new(ROLL_FORWARD).enabled(rolled_back && !in_use),
                );
                self.side
                    .hits
                    .targets
                    .push((format!("menu:{ROLL_FORWARD}"), item.rect));
                forward = item.clicked();
                forward
            });
            if forward {
                self.side_queue("history.rollback", json!({"feature": null}), None);
            }
            if drag.hovered() || drag.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if drag.drag_started() && !in_use {
                drag_start = Some(Drag::Rollback);
            }
            grip_released = drag.drag_stopped();
        }
        if let Some(start) = drag_start {
            self.side.drag = Some(start);
        }
        // A drag in progress: where it would land.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let released = ui.input(|i| i.pointer.any_released()) || grip_released;
        if let (Some(drag), Some(at)) = (self.side.drag.clone(), pointer) {
            let slot = drop_slot(&lines, &line_rects, at.y);
            // Dropped outside the list: nothing happens.
            let in_list = list_rect.contains(at);
            match &drag {
                Drag::Rollback => {
                    let y = slot_y(&line_rects, slot, list_rect);
                    draw_bar_at(ui, &kit, y, list_rect);
                    if released && in_list {
                        // The marker after the line above the drop.
                        let above = line_before(&lines, slot).map(last_feature);
                        let last = self.rows.last().map(|r| r.id.clone());
                        let feature = match above {
                            Some(id) if Some(&id) != last.as_ref() => json!(id),
                            _ => serde_json::Value::Null,
                        };
                        self.side_queue("history.rollback", json!({"feature": feature}), None);
                    }
                }
                Drag::Row { id } => {
                    let from = index.get(id).copied();
                    let to = slot_place(&lines, slot, &index, steps);
                    let allowed = match (from, to) {
                        (Some(from), Some(to)) => {
                            move_allowed(view.as_deref(), &self.rows, id, from, to)
                        }
                        _ => false,
                    };
                    if allowed {
                        let y = slot_y(&line_rects, slot, list_rect);
                        ui.painter().line_segment(
                            [
                                pos2(list_rect.left() + 8.0, y),
                                pos2(list_rect.right() - 8.0, y),
                            ],
                            Stroke::new(2.0, kit.c(sk.info_bar)),
                        );
                    }
                    if let (true, true, true, Some(from), Some(to)) =
                        (released, in_list, allowed, from, to)
                    {
                        let position = if to > from { to - 1 } else { to };
                        if position != from {
                            self.side_queue(
                                "feature.move",
                                json!({"feature": id, "position": position}),
                                None,
                            );
                        }
                    }
                }
            }
            ui.ctx().request_repaint();
        }
        if released {
            self.side.drag = None;
        }
        // The pointer on a row shows its dependencies; off the rows, a
        // hover the pointer made goes.
        let pointer_in_list = pointer.is_some_and(|p| list_rect.contains(p));
        match pointer_on {
            Some(id) => {
                if self.state.side_panel.hover.as_deref() != Some(id.as_str()) {
                    ui.ctx().request_repaint();
                }
                self.state.side_panel.hover = Some(id);
                self.side.hover_by_pointer = true;
            }
            None if self.side.hover_by_pointer && !pointer_in_list => {
                self.state.side_panel.hover = None;
                self.side.hover_by_pointer = false;
            }
            None => {}
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape))
            && !self.side.hover_by_pointer
            && self.state.side_panel.menu.is_none()
        {
            self.state.side_panel.hover = None;
        }
        // The strip that names the dependencies, over the list's foot.
        if let Some(id) = self.state.side_panel.hover.clone() {
            self.dependency_strip(ui, &kit, &id, list_rect);
        }
        self.side.hits.rows = rows_drawn;
        self.tree_row_hits = self.side.hits.rows.clone();
        self.tree_zoom_hits = zooms_drawn;
        if let Some((id, at)) = menu_open {
            self.state.side_panel.menu = Some(id);
            self.side.menu_at = Some(at);
        }
        if let Some(id) = &self.state.side_panel.menu.clone() {
            let at = self.side.menu_at.or_else(|| {
                self.side
                    .hits
                    .rows
                    .iter()
                    .find(|(r, _)| r == id)
                    .map(|(_, r)| pos2(r.center().x, r.bottom()))
            });
            if let Some(at) = at {
                self.feature_menu(ui, id, at, in_use);
            } else {
                self.state.side_panel.menu = None;
            }
        }
        if let Some((command, params, key)) = commit {
            self.side_queue(command, params, Some(key));
            self.side.editing = None;
        } else if cancel {
            self.side.editing = None;
        }
        if let Some((id, expr)) = edit_start {
            self.side.editing = Some(Editing::Value { id, text: expr });
            self.side.focus_pending = true;
        }
        if let Some(key) = fold_clicked {
            let open = self.state.side_panel.is_open(&key);
            self.state.side_panel.set_open(&key, !open);
        }
        if let Some((id, hidden)) = eye_clicked {
            self.toggle_visible(id, hidden);
            ui.ctx().request_repaint();
        }
        if let Some(id) = show_where {
            self.show_where(&id);
        }
        if let Some(id) = double {
            self.state.selected_feature = Some(id.clone());
            self.queue_select(json!({"items": [{"kind": "feature", "feature": id}]}));
            self.request_fly(FlyTo::Feature(id));
        } else if let Some((id, shift)) = clicked {
            self.click_feature(id, shift);
        }
        // Keep the tab on Features while a drag is under way.
        let _ = SideTab::Features;
    }

    /// Flies to where a failed feature would be: what it is made from.
    fn show_where(&mut self, id: &str) {
        let deps = self.side_dependencies(id);
        if deps.uses.is_empty() {
            self.request_fly(FlyTo::Feature(id.to_string()));
            return;
        }
        let items: Vec<serde_json::Value> = deps
            .uses
            .iter()
            .map(|u| json!({"kind": "feature", "feature": u}))
            .collect();
        self.queue_select(json!({ "items": items }));
        self.request_fly(FlyTo::Selection);
    }

    /// The strip over the list's foot: what the feature `id` uses, what
    /// uses it, and how many more through them.
    /// The strip's lines for the feature `id`: why it was not rebuilt
    /// (when it was not), what it uses, what uses it, and how many more.
    pub fn side_strip(&self, id: &str) -> Vec<String> {
        self.strip_lines(id).into_iter().map(|(_, t)| t).collect()
    }

    fn strip_lines(&self, id: &str) -> Vec<(StripMark, String)> {
        let deps = self.side_dependencies(id);
        let name = |id: &String| {
            self.rows
                .iter()
                .find(|r| r.id == *id)
                .map_or_else(|| id.clone(), |r| r.name.clone())
        };
        let mut lines = Vec::new();
        // The cause, which the dimmed row does not show itself.
        if let Some(row) = self.rows.iter().find(|r| r.id == id)
            && not_rebuilt(row)
            && let Some((_, root)) = &row.caused_by
        {
            lines.push((StripMark::NotRebuilt, format!("Not rebuilt: {root} failed")));
        }
        if !deps.uses.is_empty() {
            let names: Vec<String> = deps.uses.iter().map(name).collect();
            lines.push((StripMark::Uses, format!("Uses  {}", names.join(", "))));
        }
        if !deps.used_by.is_empty() {
            let names: Vec<String> = deps.used_by.iter().map(name).collect();
            lines.push((StripMark::UsedBy, format!("Used by  {}", names.join(", "))));
        }
        if deps.later > 0 {
            let text = if deps.later == 1 {
                "and 1 later feature through them".to_string()
            } else {
                format!("and {} later features through them", deps.later)
            };
            lines.push((StripMark::Later, text));
        }
        lines
    }

    fn dependency_strip(&mut self, ui: &egui::Ui, kit: &Kit, id: &str, list: Rect) {
        let t = kit.t;
        let sk = kit.skin;
        let lines: Vec<(Option<bool>, String, crate::theme::Rgb)> = self
            .strip_lines(id)
            .into_iter()
            .map(|(mark, text)| match mark {
                StripMark::NotRebuilt => (None, text, sk.failed),
                StripMark::Uses => (Some(true), text, sk.info_ink),
                StripMark::UsedBy => (Some(false), text, sk.info_ink),
                StripMark::Later => (None, text, sk.text_dim),
            })
            .collect();
        if lines.is_empty() {
            return;
        }
        let h = t.label_row_height * lines.len() as f32 + t.nav_spacing * 4.0;
        let strip = Rect::from_min_max(pos2(list.left(), list.bottom() - h), list.right_bottom());
        let painter = ui.painter();
        painter.rect_filled(strip, 0.0, kit.c(sk.info_fill));
        self.side.hits.drawn.push(strip);
        let mut y = strip.top() + t.nav_spacing * 2.0;
        let text_x = strip.left() + t.field_inset + t.nav_icon_size;
        for (arrow, text, ink) in lines {
            let mid = y + t.label_row_height * 0.5;
            if let Some(up) = arrow {
                let x = strip.left() + t.field_inset * 0.5 + t.nav_icon_size * 0.4;
                let h = t.nav_icon_size * 0.4;
                let dir = if up { -1.0 } else { 1.0 };
                let stroke = Stroke::new(t.icon_stroke + 0.2, kit.c(sk.info_ink));
                let tip = pos2(x, mid + dir * h);
                painter.line_segment([pos2(x, mid - dir * h), tip], stroke);
                painter.line_segment([tip, pos2(x - h * 0.6, tip.y - dir * h * 0.6)], stroke);
                painter.line_segment([tip, pos2(x + h * 0.6, tip.y - dir * h * 0.6)], stroke);
            }
            let mut job = egui::text::LayoutJob::simple_singleline(text, kit.font(), kit.c(ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width(
                (strip.right() - text_x - t.field_inset * 0.5).max(0.0),
            );
            let g = painter.layout_job(job);
            painter.galley(pos2(text_x, mid - g.size().y * 0.5), g, kit.c(ink));
            y += t.label_row_height;
        }
    }

    /// A feature's context menu, on the dark plate.
    fn feature_menu(&mut self, ui: &egui::Ui, id: &str, at: egui::Pos2, in_use: bool) {
        let plate = Kit::new(&self.tokens, Ground::Plate);
        let row = self.rows.iter().find(|r| r.id == id).cloned();
        let Some(row) = row else {
            self.state.side_panel.menu = None;
            return;
        };
        let edit = self
            .side
            .view
            .as_ref()
            .and_then(|v| v.features.iter().find(|f| f.id == id))
            .and_then(|f| f.edit.clone());
        let suppressed = row.state == FeatureState::Suppressed;
        let is_last = self.rows.last().is_some_and(|r| r.id == row.id);
        let rolled_back = self.timeline.rollback.is_some();
        let mut chosen: Option<&'static str> = None;
        let mut items_drawn: Vec<(String, Rect)> = Vec::new();
        let area = egui::Area::new(egui::Id::new("craft3d-side-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .constrain(true)
            .show(ui.ctx(), |ui| {
                plate.menu_frame(ui, plate.t.menu_width, |ui| {
                    let mut item = |ui: &mut egui::Ui,
                                    label: &'static str,
                                    keys: Option<&str>,
                                    enabled: bool,
                                    danger: bool| {
                        let mut m = MenuItem::new(label).enabled(enabled);
                        if let Some(keys) = keys {
                            m = m.shortcut(keys);
                        }
                        if danger {
                            m = m.danger();
                        }
                        let r = plate.menu_item(ui, m);
                        items_drawn.push((format!("menu:{label}"), r.rect));
                        if r.clicked() {
                            chosen = Some(label);
                        }
                    };
                    item(ui, "Edit…", None, edit.is_some() && !in_use, false);
                    item(ui, "Rename", None, !in_use, false);
                    item(ui, "Show dependencies", None, true, false);
                    plate.menu_separator(ui);
                    // On the last row the marker can only go: forward,
                    // to the end (none where there is no marker).
                    if is_last {
                        item(ui, ROLL_FORWARD, None, !in_use && rolled_back, false);
                    } else {
                        item(ui, "Roll back to here", None, !in_use, false);
                    }
                    item(
                        ui,
                        if suppressed { "Unsuppress" } else { "Suppress" },
                        None,
                        !in_use,
                        false,
                    );
                    item(
                        ui,
                        if row.hidden { "Show" } else { "Hide" },
                        None,
                        !in_use,
                        false,
                    );
                    plate.menu_separator(ui);
                    item(ui, "Copy id", Some(id), true, false);
                    item(ui, "Delete", None, !in_use, true);
                })
            });
        self.side.hits.targets.extend(items_drawn);
        let elsewhere = ui.input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer()
            && !ui.input(|i| i.pointer.secondary_pressed());
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if chosen.is_some() || elsewhere || escape {
            self.state.side_panel.menu = None;
            self.side.menu_at = None;
        }
        let id = id.to_string();
        match chosen {
            Some("Edit…") => {
                if let Some(edit) = edit {
                    self.side.editing = Some(Editing::Value {
                        id,
                        text: edit.expr,
                    });
                    self.side.focus_pending = true;
                }
            }
            Some("Rename") => {
                self.side.editing = Some(Editing::Name {
                    id,
                    text: row.name.clone(),
                });
                self.side.focus_pending = true;
            }
            Some("Show dependencies") => {
                self.state.side_panel.hover = Some(id);
                self.side.hover_by_pointer = false;
            }
            Some(ROLL_FORWARD) => {
                self.side_queue("history.rollback", json!({"feature": null}), None);
            }
            Some("Roll back to here") => {
                let last = self.rows.last().map(|r| r.id.clone());
                let feature = if Some(&id) == last.as_ref() {
                    serde_json::Value::Null
                } else {
                    json!(id)
                };
                self.side_queue("history.rollback", json!({"feature": feature}), None);
            }
            Some("Suppress") | Some("Unsuppress") => {
                self.side_queue(
                    "feature.suppress",
                    json!({"feature": id, "suppressed": !suppressed}),
                    None,
                );
            }
            Some("Hide") | Some("Show") => self.toggle_visible(id, row.hidden),
            Some("Copy id") => ui.ctx().copy_text(id),
            Some("Delete") => self.side_queue("feature.delete", json!({"feature": id}), None),
            _ => {}
        }
    }
}

/// The agent's mark (✦) on a row of a feature an agent made or last
/// changed: the document's provenance (`madeBy`, `lastChangedBy`).
fn agent_mark(row: Option<&TreeRow>) -> Option<RowMark> {
    row.and_then(|r| r.made_by.as_ref()).map(|_| RowMark::Agent)
}

/// A sketch's key value: its curves and its degrees of freedom.
fn sketch_key(curves: &str, status: &str) -> String {
    // `status_text` writes `2 DOF`, `fully constrained` or a state.
    let dof = if status == "fully constrained" {
        "0 DOF".to_string()
    } else if status.ends_with("DOF") {
        status.to_string()
    } else {
        String::new()
    };
    match (curves.is_empty(), dof.is_empty()) {
        (false, false) => format!("{curves} · {dof}"),
        (false, true) => curves.to_string(),
        (true, false) => dof,
        (true, true) => String::new(),
    }
}

/// Whether most of `row` shows in `view`: a row cut by a pixel or two at
/// the list's edge is in view, and the list stays.
fn mostly_in(row: Rect, view: Rect) -> bool {
    let shown = (row.bottom().min(view.bottom()) - row.top().max(view.top())).max(0.0);
    shown >= row.height() * 0.5
}

fn line_is_nested(line: &SideLine) -> bool {
    line.depth > 0
}

/// The last feature of a line: a group's last datum, else itself.
fn last_feature(line: &SideLine) -> String {
    line.members
        .last()
        .cloned()
        .unwrap_or_else(|| line.id.clone())
}

/// The place a drop at height `y` lands: before the top-level line `n`
/// (a parent with what is nested under it, a group with its datums, kept
/// whole), or `lines.len()` for the end. Never between a parent and what
/// is nested under it.
fn drop_slot(lines: &[SideLine], rects: &[(usize, Rect)], y: f32) -> usize {
    let rect_of = |n: usize| rects.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
    for (n, _) in lines.iter().enumerate().filter(|(_, l)| l.depth == 0) {
        // The block: the line and the nested lines under it.
        let last = lines
            .get(n + 1..)
            .unwrap_or(&[])
            .iter()
            .take_while(|l| l.depth > 0)
            .count();
        let (Some(top), Some(bottom)) = (rect_of(n), rect_of(n + last)) else {
            continue;
        };
        if y < (top.top() + bottom.bottom()) * 0.5 {
            return n;
        }
    }
    lines.len()
}

/// Where the line of a drop at `slot` is drawn: on top of that line, or
/// under the last.
fn slot_y(rects: &[(usize, Rect)], slot: usize, list: Rect) -> f32 {
    let y = rects
        .iter()
        .find(|(n, _)| *n == slot)
        .map(|(_, r)| r.top())
        .or_else(|| rects.last().map(|(_, r)| r.bottom()))
        .unwrap_or(list.top());
    y.clamp(list.top(), list.bottom())
}

/// The top-level line before `slot`: the marker of a roll-back dropped
/// there goes after its last feature.
fn line_before(lines: &[SideLine], slot: usize) -> Option<&SideLine> {
    lines
        .get(..slot.min(lines.len()))
        .unwrap_or(&[])
        .iter()
        .rev()
        .find(|l| l.depth == 0)
}

/// The history place a drop at `slot` puts a feature before: the first
/// feature of the line there (a parent's nested sketches come before it
/// in the history, folded or not; a group's first datum), or the end.
fn slot_place(
    lines: &[SideLine],
    slot: usize,
    index: &BTreeMap<String, usize>,
    steps: usize,
) -> Option<usize> {
    let Some(line) = lines.get(slot) else {
        return Some(steps);
    };
    let first = line
        .members
        .first()
        .and_then(|m| index.get(m).copied())
        .or(line.row)?;
    Some(line.nested.iter().copied().fold(first, usize::min))
}

/// Whether moving feature `id` from `from` to before place `to` keeps it
/// after what it is made from and before what is made from it.
pub fn move_allowed(
    view: Option<&HistoryView>,
    rows: &[TreeRow],
    id: &str,
    from: usize,
    to: usize,
) -> bool {
    let Some(view) = view else {
        return false;
    };
    if to == from || to == from + 1 {
        return false;
    }
    let place = |fid: &str| rows.iter().position(|r| r.id == fid);
    let Some(me) = view.features.iter().find(|f| f.id == id) else {
        return false;
    };
    // Its new place among the others: before `to`.
    let new_place = if to > from { to - 1 } else { to };
    let after_inputs = me.uses.iter().all(|u| {
        place(u).is_none_or(|p| {
            let p = if p > from { p - 1 } else { p };
            p < new_place
        })
    });
    let before_users = view
        .features
        .iter()
        .filter(|f| f.uses.iter().any(|u| u == id))
        .all(|f| {
            place(&f.id).is_none_or(|p| {
                let p = if p > from { p - 1 } else { p };
                p >= new_place
            })
        });
    after_inputs && before_users
}

/// The roll-back bar: its grip, a gold line across, and where it is
/// (`10 of 25`) while it is not at the end. Returns its rectangle.
fn rollback_bar(ui: &mut egui::Ui, kit: &Kit, at: usize, of: usize) -> Rect {
    let t = kit.t;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), t.row_height), Sense::hover());
    let painter = ui.painter();
    let label = (at < of).then(|| format!("{at} of {of}"));
    let right = match &label {
        Some(text) => {
            let g = kit.galley(ui, text.clone(), kit.font(), t.attention);
            let w = g.size().x;
            painter.galley(
                pos2(rect.right() - w - 2.0, rect.center().y - g.size().y * 0.5),
                g,
                color(t.attention),
            );
            rect.right() - w - t.nav_icon_gap - 4.0
        }
        None => rect.right() - 2.0,
    };
    let grip = grip_pill(painter, kit, rect);
    painter.line_segment(
        [
            pos2(grip.right() + t.nav_icon_gap, rect.center().y),
            pos2(right, rect.center().y),
        ],
        Stroke::new(2.0, color(t.accent)),
    );
    rect
}

/// The menu item that takes the rollback marker away.
const ROLL_FORWARD: &str = "Roll forward to end";

/// The amber pill with the grip's dots, at the left of `row`.
fn grip_pill(painter: &egui::Painter, kit: &Kit, row: Rect) -> Rect {
    let t = kit.t;
    let h = (t.row_height * 0.75).min(row.height());
    let pill = Rect::from_min_size(
        pos2(row.left() + 2.0, row.center().y - h * 0.5),
        vec2(h * 1.7, h),
    );
    painter.rect_filled(
        pill,
        egui::CornerRadius::same(crate::kit::controls::radius_of(h)),
        color(t.accent),
    );
    crate::kit::rows::grip_dots(
        painter,
        Rect::from_center_size(pill.center(), vec2(h * 0.55, h * 0.75)),
        color(t.knob),
    );
    pill
}

/// The bar drawn where a drag of it would put it.
fn draw_bar_at(ui: &egui::Ui, kit: &Kit, y: f32, list: Rect) {
    let row = Rect::from_center_size(
        pos2(list.center().x, y),
        vec2(list.width(), kit.t.row_height),
    );
    let painter = ui.painter();
    let grip = grip_pill(painter, kit, row);
    painter.line_segment(
        [
            pos2(grip.right() + kit.t.nav_icon_gap, y),
            pos2(list.right() - 8.0, y),
        ],
        Stroke::new(2.0, color(kit.t.accent)),
    );
}

/// The state a summary dot says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dot {
    Ok,
    Failed,
    Grey,
    Info,
}

/// A round dot that says a state by its mark, not by its colour alone:
/// a tick, a cross, or none.
pub(crate) fn status_dot(painter: &egui::Painter, kit: &Kit, at: egui::Pos2, r: f32, dot: Dot) {
    let sk = kit.skin;
    let (fill, mark) = match dot {
        Dot::Ok => (sk.ok, Some(Glyph::Check)),
        Dot::Failed => (sk.failed, Some(Glyph::Close)),
        Dot::Grey => (sk.faint, None),
        Dot::Info => (sk.info_bar, None),
    };
    let r = if mark.is_some() { r } else { r * 0.55 };
    painter.circle_filled(at, r, color(fill));
    if let Some(glyph) = mark {
        crate::kit::glyphs::paint(
            painter,
            glyph,
            Rect::from_center_size(at, vec2(r * 1.3, r * 1.3)),
            color(kit.t.knob_fill),
            &kit.t,
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]
    use super::*;

    fn item<'a>(id: &'a str, kind: &'a str, uses: &'a [String]) -> OutlineItem<'a> {
        OutlineItem {
            id,
            name: id,
            kind,
            is_sketch: kind == "sketch",
            is_datum: kind.starts_with("datum"),
            uses,
            key: format!("{kind} key"),
        }
    }

    #[test]
    fn sketches_nest_and_datums_group_in_place() {
        let none: Vec<String> = Vec::new();
        let f2 = vec!["F2".to_string()];
        let f5 = vec!["F5".to_string(), "F6".to_string()];
        let items = vec![
            item("F1", "datumPlane", &none),
            item("F2", "sketch", &none),
            item("F3", "extrude", &f2),
            item("F4", "datumAxis", &none),
            item("F5", "datumPlane", &none),
            item("F6", "sketch", &none),
            item("F7", "extrude", &f5),
            // A sketch with something between it and its user stays.
            item("F8", "sketch", &none),
            item("F9", "block", &none),
        ];
        let closed = outline(&items, &|_| false, "");
        let ids: Vec<&str> = closed.iter().map(|l| l.id.as_str()).collect();
        // F6 lies right before F7, which is made from it: nested.
        assert_eq!(ids, ["F1", "F3", "datums:F4", "F7", "F8", "F9"]);
        assert_eq!(closed[1].fold, Some(false));
        assert_eq!(closed[2].members, ["F4", "F5"]);
        let open = outline(&items, &|_| true, "");
        let ids: Vec<(&str, u8)> = open.iter().map(|l| (l.id.as_str(), l.depth)).collect();
        assert_eq!(
            ids,
            [
                ("F1", 0),
                ("F3", 0),
                ("F2", 1),
                ("datums:F4", 0),
                ("F4", 1),
                ("F5", 1),
                ("F7", 0),
                ("F6", 1),
                ("F8", 0),
                ("F9", 0)
            ]
        );
        // A filter keeps what holds it, opened under its parent.
        let found = outline(&items, &|_| false, "F2");
        let ids: Vec<&str> = found.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["F3", "F2"]);
    }

    fn entry(id: &str, uses: &[&str]) -> HistoryEntry {
        HistoryEntry {
            id: id.to_string(),
            name: id.to_string(),
            kind: "block",
            op: None,
            is_sketch: false,
            is_datum: false,
            key: String::new(),
            edit: None,
            uses: uses.iter().map(|u| u.to_string()).collect(),
            curves: String::new(),
        }
    }

    fn row(id: &str) -> TreeRow {
        TreeRow {
            id: id.to_string(),
            name: id.to_string(),
            kind: "block",
            state: FeatureState::Ok,
            status: "ok".to_string(),
            error: None,
            caused_by: None,
            selected: false,
            hidden: false,
            made_selection: false,
            zoomable: false,
            when: None,
            passes: None,
            instance: None,
            depth: 0,
            made_by: None,
        }
    }

    /// F1, F2 made from F1, F3 alone, F4 made from F2.
    fn four() -> (HistoryView, Vec<TreeRow>) {
        let features = vec![
            entry("F1", &[]),
            entry("F2", &["F1"]),
            entry("F3", &[]),
            entry("F4", &["F2"]),
        ];
        let rows = features.iter().map(|f| row(&f.id)).collect();
        (
            HistoryView {
                features,
                ..HistoryView::default()
            },
            rows,
        )
    }

    #[test]
    fn a_move_keeps_a_feature_after_its_inputs_and_before_its_users() {
        let (view, rows) = four();
        let ok = |id: &str, from: usize, to: usize| move_allowed(Some(&view), &rows, id, from, to);
        // F3, made of nothing and used by nothing, goes anywhere.
        assert!(ok("F3", 2, 0), "up");
        assert!(ok("F3", 2, 4), "down to the end");
        // Not where it already is.
        assert!(!ok("F3", 2, 2) && !ok("F3", 2, 3));
        // F2 not before its input F1.
        assert!(!ok("F2", 1, 0), "past an input");
        // F2 down past F3 is fine, past its user F4 not.
        assert!(ok("F2", 1, 3));
        assert!(!ok("F2", 1, 4), "past a user");
        // F4 up past F3 is fine, past its input F2 not.
        assert!(ok("F4", 3, 2));
        assert!(!ok("F4", 3, 1));
        assert!(!move_allowed(None, &rows, "F3", 2, 0));
    }

    /// A drop is never between a parent and what is nested under it, and
    /// before a folded parent it goes before the parent's sketch.
    #[test]
    fn drops_keep_a_sketch_with_its_feature() {
        let none: Vec<String> = Vec::new();
        let f2 = vec!["F2".to_string()];
        let items = vec![
            item("F1", "block", &none),
            item("F2", "sketch", &none),
            item("F3", "extrude", &f2),
            item("F4", "block", &none),
        ];
        let index: BTreeMap<String, usize> = ["F1", "F2", "F3", "F4"]
            .iter()
            .enumerate()
            .map(|(i, id)| (id.to_string(), i))
            .collect();
        for opened in [false, true] {
            let lines = outline(&items, &|_| opened, "");
            let rects: Vec<(usize, Rect)> = (0..lines.len())
                .map(|n| {
                    (
                        n,
                        Rect::from_min_size(pos2(0.0, n as f32 * 20.0), vec2(100.0, 20.0)),
                    )
                })
                .collect();
            let parent = lines.iter().position(|l| l.id == "F3").unwrap();
            // Every height gives a slot at a top-level line or the end.
            for y in 0..(lines.len() * 20) {
                let slot = drop_slot(&lines, &rects, y as f32);
                assert!(
                    slot == lines.len() || lines[slot].depth == 0,
                    "{opened} {y}"
                );
            }
            // Before the parent: before its sketch in the history.
            assert_eq!(slot_place(&lines, parent, &index, 4), Some(1), "{opened}");
            assert_eq!(slot_place(&lines, lines.len(), &index, 4), Some(4));
            // The marker dropped after the parent goes after the parent.
            let after = drop_slot(&lines, &rects, (lines.len() * 20 - 1) as f32);
            assert_eq!(
                line_before(&lines, after).map(last_feature).as_deref(),
                Some("F4")
            );
        }
    }

    #[test]
    fn a_sketch_key_says_curves_and_freedom() {
        assert_eq!(sketch_key("2 splines", "12 DOF"), "2 splines · 12 DOF");
        assert_eq!(sketch_key("", "18 DOF"), "18 DOF");
        assert_eq!(sketch_key("4 lines", "failed"), "4 lines");
    }
}
