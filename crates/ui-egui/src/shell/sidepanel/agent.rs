//! The side panel's Agent tab (board 06 of `plan/mockups/sidepanel`): what
//! agents did to the active document, newest first, from its log
//! (`agent.log`, `Session::agent_log_json`): when, the command in its
//! label's words, what it made (the features, by name), a failure's
//! reason, and the intent the agent gave, in italics. Above it, who is
//! connected. Offered while agents are allowed or the document has a log;
//! never in the browser, where agents cannot connect.
//!
//! *Undo to here* (on the board) is not built: the log does not say which
//! undo step each entry made, and counting entries would undo the
//! person's own edits in between. It is owed to an engine command that
//! keys the log to the undo history.

use super::super::Shell;
use crate::kit::Kit;
use crate::theme::color;
use serde_json::Value;

/// How many of the log's entries the tab reads (newest first).
pub const LOG_ROWS: usize = 100;

/// One entry of the log as the tab shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub client: String,
    pub time_ms: u64,
    pub label: String,
    pub ok: bool,
    pub message: Option<String>,
    pub created: Vec<String>,
    pub intent: Option<String>,
}

/// The rows of `log` (as `agent_log_json` gives it), newest first; what
/// cannot be read is left out.
pub fn rows_of(log: &Value) -> Vec<LogRow> {
    let text = |entry: &Value, key: &str| {
        entry
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    log.get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .take(LOG_ROWS)
                .map(|entry| LogRow {
                    client: text(entry, "client").unwrap_or_default(),
                    time_ms: entry.get("timeMs").and_then(Value::as_u64).unwrap_or(0),
                    label: text(entry, "label")
                        .or_else(|| text(entry, "command"))
                        .unwrap_or_default(),
                    ok: entry.get("ok").and_then(Value::as_bool).unwrap_or(true),
                    message: text(entry, "message"),
                    created: entry
                        .get("created")
                        .and_then(Value::as_array)
                        .map(|ids| {
                            ids.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    intent: text(entry, "intent"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// "just now", "3 min ago", "2 h ago": how long before `now_ms`.
pub fn ago(time_ms: u64, now_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(time_ms) / 1000;
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", seconds / 60),
        3600..=172_799 => format!("{} h ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

impl Shell {
    /// Whether the side panel offers the Agent tab: on the desktop, while
    /// agents are allowed or the document has a log.
    pub(crate) fn agent_tab_offered(&self) -> bool {
        if cfg!(target_arch = "wasm32") {
            return false;
        }
        let logged = self
            .side
            .agent_log
            .as_ref()
            .and_then(|log| log.get("total"))
            .and_then(Value::as_u64)
            .is_some_and(|n| n > 0);
        self.agents_status().allowed || logged
    }

    /// The Agent tab: who is connected, then the log, newest first.
    pub(super) fn agent_tab(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let s = kit.skin;
        let now = self.agents_now();
        let status = self.agents_status().clone();
        let names: std::collections::BTreeMap<String, String> = self
            .tree_rows()
            .iter()
            .map(|row| (row.id.clone(), row.name.clone()))
            .collect();
        let rows = self
            .side
            .agent_log
            .as_deref()
            .map(rows_of)
            .unwrap_or_default();
        let total = self
            .side
            .agent_log
            .as_ref()
            .and_then(|log| log.get("total"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        egui::ScrollArea::vertical()
            .id_salt("craft3d-agent-tab")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = tokens.nav_spacing;
                // Who is connected, on the info card.
                let (title, detail) = match status.clients.as_slice() {
                    [] if status.allowed => (
                        "Waiting for an agent".to_string(),
                        "Agents may connect to this window".to_string(),
                    ),
                    [] => (
                        "Agents off".to_string(),
                        "The pill in the status bar turns them on".to_string(),
                    ),
                    [one] => (
                        format!("{} connected", one.name),
                        format!("{} · {total} commands", ago(one.since_ms, now)),
                    ),
                    many => (
                        format!("{} agents connected", many.len()),
                        format!("{total} commands"),
                    ),
                };
                kit.banner(
                    ui,
                    crate::kit::BannerKind::Info,
                    Some(&title),
                    &detail,
                    |_, _| {},
                );
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Latest")
                            .font(kit.medium_font(ui))
                            .color(color(s.text)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("newest first")
                                .font(kit.small_font())
                                .color(color(s.text_dim)),
                        );
                    });
                });
                if rows.is_empty() {
                    kit.paragraph(
                        ui,
                        "Nothing yet. What an agent does to this document shows here, with what it meant by it.",
                        true,
                    );
                }
                let mut hits = Vec::new();
                for row in &rows {
                    let response = log_row(ui, &kit, row, now, &names);
                    hits.push(response.rect);
                }
                self.side.hits.agent_rows = hits;
            });
    }
}

/// One entry: a dot on the time line (indigo, red for a failure), when
/// and who, what it did, what it made, why it failed, the intent.
fn log_row(
    ui: &mut egui::Ui,
    kit: &Kit,
    row: &LogRow,
    now: u64,
    names: &std::collections::BTreeMap<String, String>,
) -> egui::Response {
    let t = kit.t;
    let s = kit.skin;
    let dot_ink = if row.ok { s.info_bar } else { s.failed };
    let gutter = t.nav_icon_size + t.nav_icon_gap;
    let shown = ui.horizontal_top(|ui| {
        let (rail, _) =
            ui.allocate_exact_size(egui::vec2(gutter, t.row_height), egui::Sense::hover());
        let centre = egui::pos2(rail.left() + t.nav_icon_size * 0.5, rail.center().y);
        ui.painter().circle_filled(centre, 4.0, color(dot_ink));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let when = if row.client.is_empty() {
                ago(row.time_ms, now)
            } else {
                format!("{} · {}", ago(row.time_ms, now), row.client)
            };
            ui.label(
                egui::RichText::new(when)
                    .font(kit.small_font())
                    .color(color(s.text_dim)),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&row.label)
                        .font(kit.font())
                        .color(color(s.text)),
                )
                .wrap(),
            );
            for id in &row.created {
                let name = names.get(id).map_or(id.as_str(), String::as_str);
                ui.label(
                    egui::RichText::new(format!("made {name} ({id})"))
                        .font(kit.font())
                        .color(color(s.info_ink)),
                );
            }
            if let (false, Some(message)) = (row.ok, &row.message) {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(message)
                            .font(kit.small_font())
                            .color(color(s.failed)),
                    )
                    .wrap(),
                );
            }
            if let Some(intent) = &row.intent {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("“{intent}”"))
                            .font(kit.font())
                            .italics()
                            .color(color(s.text_dim)),
                    )
                    .wrap(),
                );
            }
        });
    });
    // The time line: a faint rail from the dot down the entry.
    let rect = shown.response.rect;
    let x = rect.left() + t.nav_icon_size * 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(x, rect.top() + t.row_height * 0.5 + 6.0),
            egui::pos2(x, rect.bottom()),
        ],
        egui::Stroke::new(2.0, crate::theme::color_alpha(s.info_bar, 70)),
    );
    shown.response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_log_reads_newest_first_and_skips_what_it_cannot_read() {
        let log = json!({"total": 2, "entries": [
            {"client": "Claude Code", "timeMs": 2000, "command": "feature.fillet",
             "label": "Fillet", "ok": true, "created": ["F24"], "intent": "Round the base"},
            {"client": "Claude Code", "timeMs": 1000, "command": "feature.edit",
             "ok": false, "message": "no such edge", "created": []},
            "nonsense"
        ]});
        let rows = rows_of(&log);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "Fillet");
        assert_eq!(rows[0].created, ["F24"]);
        assert_eq!(rows[0].intent.as_deref(), Some("Round the base"));
        // No label: the command's id.
        assert_eq!(rows[1].label, "feature.edit");
        assert!(!rows[1].ok);
        assert_eq!(rows[1].message.as_deref(), Some("no such edge"));
        assert_eq!(rows[2].label, "");
        assert!(rows_of(&json!({})).is_empty());
        assert!(rows_of(&json!({"entries": 3})).is_empty());
    }

    #[test]
    fn how_long_ago() {
        assert_eq!(ago(0, 30_000), "just now");
        assert_eq!(ago(0, 180_000), "3 min ago");
        assert_eq!(ago(0, 7_200_000), "2 h ago");
        assert_eq!(ago(0, 3 * 86_400_000), "3 days ago");
        assert_eq!(ago(5_000, 1_000), "just now");
    }
}
