//! The checks under the feature list (plan of 2026-11-02, boards 01 and
//! 12): a header that folds them (`Checks 7 · all pass`, `1 failing`),
//! then a row per check with its measure (`7 mm`; under a check that does
//! not pass, what it must be, `must be ≥ 5 mm`, and why; all of it in the
//! tooltip) and
//! a dot that carries a tick or a cross, so the state does not hang on
//! colour alone. A failing check is a red row with its reason under it. A
//! click on a check flies to what it measures. While something is
//! selected the checks fold to their header, to make room for its
//! properties.

use super::Shell;
use super::features::{Dot, status_dot};
use crate::kit::Kit;
use crate::shell::views::FlyTo;
use craft3d_engine::AssertionStatus as A;
use egui::{Rect, Sense, pos2, vec2};
use serde_json::{Value, json};

/// Most checks listed; the header counts them all.
const MAX_LISTED: usize = 200;

/// A check as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckLine {
    pub name: String,
    /// What was measured (`7 mm`), or the state when nothing was.
    pub measured: String,
    /// What it must be (`≥ 5 mm`).
    pub limit: String,
    /// Passed; failed (or the measure could not be taken); not proven
    /// either way; being measured; switched off.
    pub status: CheckStatus,
    /// Under a failing check: why.
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    Passed,
    Failed,
    NotProven,
    Pending,
    Off,
}

impl CheckLine {
    /// The lines under a check that does not pass: what it must be, then
    /// why where that says more than the measure (not proven, or nothing
    /// measured); on the row, as a phone has no tooltip.
    pub fn under(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if matches!(self.status, CheckStatus::Failed | CheckStatus::NotProven) {
            if !self.limit.is_empty() {
                lines.push(format!("must be {}", self.limit));
            }
            // The reason only where it says more than the measure and the
            // limit: not proven, or nothing measured (an error).
            let says_more = self.status == CheckStatus::NotProven
                || self.measured.is_empty()
                || self.measured == "—";
            if let Some(note) = self.note.as_ref().filter(|_| says_more) {
                lines.push(note.clone());
            }
        }
        lines
    }
}

impl Shell {
    /// The checks as the list shows them, and the header's note
    /// (`all pass`, `1 failing`).
    pub fn side_checks(&self) -> (Vec<CheckLine>, String) {
        let lines: Vec<CheckLine> = self
            .assertion_rows
            .iter()
            .map(|a| {
                let status = match a.status {
                    A::Passed => CheckStatus::Passed,
                    A::Failed | A::Error => CheckStatus::Failed,
                    A::NotProven => CheckStatus::NotProven,
                    A::Pending => CheckStatus::Pending,
                    A::Inactive => CheckStatus::Off,
                };
                CheckLine {
                    name: a.name.clone(),
                    measured: a.measured.as_deref().map(measured_text).unwrap_or_else(|| {
                        match status {
                            CheckStatus::Pending => "measuring".to_string(),
                            CheckStatus::Off => "off".to_string(),
                            _ => "—".to_string(),
                        }
                    }),
                    limit: limit_text(&a.expected),
                    status,
                    // Which step made it fail is not known yet: its reason.
                    note: matches!(status, CheckStatus::Failed | CheckStatus::NotProven)
                        .then(|| a.message.clone()),
                }
            })
            .collect();
        let failing = lines
            .iter()
            .filter(|l| l.status == CheckStatus::Failed)
            .count();
        let unproven = lines
            .iter()
            .filter(|l| l.status == CheckStatus::NotProven)
            .count();
        let pending = lines
            .iter()
            .filter(|l| l.status == CheckStatus::Pending)
            .count();
        let note = if lines.is_empty() {
            String::new()
        } else if failing > 0 {
            format!("{failing} failing")
        } else if unproven > 0 {
            format!("{unproven} not proven")
        } else if pending > 0 {
            "measuring".to_string()
        } else {
            "all pass".to_string()
        };
        (lines, note)
    }

    /// How high the checks are drawn: the rule, the header and, open,
    /// their rows (scrolled past a third of the window).
    pub(super) fn checks_height(&self, ui: &egui::Ui, folded: bool) -> f32 {
        let folded = folded && !self.side.checks_asked;
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let mut h = t.nav_gap + 1.0 + kit.hit(t.row_height);
        if self.assertion_rows.is_empty() {
            let width = (ui.available_width() - f32::from(crate::shell::side_inset(&t))).max(40.0);
            let hint = ui.painter().layout(
                crate::shell::NO_ASSERTIONS_HINT.to_string(),
                kit.small_font(),
                kit.c(kit.skin.text_dim),
                width,
            );
            return h + hint.size().y + t.nav_spacing * 2.0;
        }
        if self.state.side_panel.checks_open && !folded {
            h += self.checks_rows_height().min(self.checks_max_height(ui));
        }
        h
    }

    fn checks_rows_height(&self) -> f32 {
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let (lines, _) = self.side_checks();
        lines
            .iter()
            .take(MAX_LISTED)
            .map(|l| kit.hit(t.row_height + t.label_row_height * l.under().len() as f32))
            .sum()
    }

    fn checks_max_height(&self, ui: &egui::Ui) -> f32 {
        (ui.ctx().content_rect().height() * 0.3).max(self.tokens.row_height * 3.0)
    }

    /// The checks' header and, open, their rows.
    pub(super) fn checks_ui(&mut self, ui: &mut egui::Ui, folded: bool) {
        // Folded for room (something selected, a short panel) unless they
        // were opened since by hand.
        let folded = folded && !self.side.checks_asked;
        let kit = Kit::panel(&self.tokens);
        let t = kit.t;
        let sk = kit.skin;
        let (lines, note) = self.side_checks();
        let note_ink = if lines.iter().any(|l| l.status == CheckStatus::Failed) {
            sk.failed
        } else if lines.iter().any(|l| l.status == CheckStatus::NotProven) {
            sk.attention
        } else {
            sk.ok
        };
        kit.divider(ui);
        let mut open = self.state.side_panel.checks_open && !folded;
        let header = kit.fold_header(
            ui,
            &mut open,
            "Checks",
            Some(lines.len()),
            Some((&note, note_ink)),
        );
        self.side
            .hits
            .targets
            .push(("checks".to_string(), header.rect));
        self.side.hits.drawn.push(header.rect);
        if header.changed() {
            self.state.side_panel.checks_open = open;
            self.side.checks_asked = open;
        }
        if lines.is_empty() {
            // How to add one.
            let width = ui.available_width();
            let hint = ui.painter().layout(
                crate::shell::NO_ASSERTIONS_HINT.to_string(),
                kit.small_font(),
                kit.c(sk.text_dim),
                width,
            );
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(width, hint.size().y + t.nav_spacing * 2.0),
                Sense::hover(),
            );
            self.side.hits.drawn.push(rect);
            ui.painter().galley(rect.min, hint, kit.c(sk.text_dim));
            return;
        }
        if !open {
            return;
        }
        let max_height = self.checks_rows_height().min(self.checks_max_height(ui));
        let mut fly: Option<String> = None;
        let mut pending = false;
        egui::ScrollArea::vertical()
            .id_salt("craft3d-side-checks")
            .max_height(max_height)
            .min_scrolled_height(max_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for line in lines.iter().take(MAX_LISTED) {
                    pending |= line.status == CheckStatus::Pending;
                    let failing = line.status == CheckStatus::Failed;
                    let under = line.under();
                    let height = t.row_height + t.label_row_height * under.len() as f32;
                    let (rect, response) = ui.allocate_exact_size(
                        vec2(ui.available_width(), kit.hit(height)),
                        Sense::click(),
                    );
                    self.side.hits.checks.push((line.name.clone(), rect));
                    self.side.hits.drawn.push(rect);
                    let painter = ui.painter();
                    if failing {
                        painter.rect_filled(rect, 0.0, kit.c(sk.failed_fill));
                        painter.rect_filled(
                            Rect::from_min_size(rect.min, vec2(t.info_bar_width, rect.height())),
                            0.0,
                            kit.c(sk.failed),
                        );
                    } else if response.hovered() {
                        painter.rect_filled(rect, kit.radius(), kit.c(sk.hover));
                    }
                    let y = rect.top() + t.row_height * 0.5;
                    let r = t.nav_icon_size * 0.42;
                    let dot = match line.status {
                        CheckStatus::Passed => Dot::Ok,
                        CheckStatus::Failed => Dot::Failed,
                        CheckStatus::NotProven | CheckStatus::Pending | CheckStatus::Off => {
                            Dot::Grey
                        }
                    };
                    let x0 = rect.left() + t.nav_spacing * 2.0 + if failing { 4.0 } else { 0.0 };
                    status_dot(painter, &kit, pos2(x0 + r, y), r, dot);
                    let text_x = x0 + 2.0 * r + t.nav_icon_gap + 2.0;
                    // At the right the measure, whole (the limit and the
                    // reason go under a check that does not pass). The name
                    // gives way first, down to a share of its own; only
                    // then is the measure cut.
                    let right = rect.right() - 2.0;
                    let room = (right - text_x).max(0.0);
                    let galley =
                        |text: String, ink| painter.layout_no_wrap(text, kit.font(), kit.c(ink));
                    let measured_ink = if failing { sk.failed } else { sk.text };
                    let gap = t.nav_icon_gap * 1.5;
                    let widths = check_columns(
                        room,
                        gap,
                        galley(line.name.clone(), sk.text).size().x,
                        galley(line.measured.clone(), measured_ink).size().x,
                        0.0,
                    );
                    let cut = |text: String, ink, width: f32| {
                        let mut job =
                            egui::text::LayoutJob::simple_singleline(text, kit.font(), kit.c(ink));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
                        painter.layout_job(job)
                    };
                    let mut x = right;
                    let measured = cut(line.measured.clone(), measured_ink, widths.measured);
                    x -= measured.size().x;
                    painter.galley(
                        pos2(x, y - measured.size().y * 0.5),
                        measured,
                        kit.c(measured_ink),
                    );
                    let name = cut(line.name.clone(), sk.text, widths.name);
                    painter.galley(pos2(text_x, y - name.size().y * 0.5), name, kit.c(sk.text));
                    let mut tip = line.name.clone();
                    tip.push_str(": ");
                    tip.push_str(&line.measured);
                    if !line.limit.is_empty() {
                        tip.push_str(&format!(" (must be {})", line.limit));
                    }
                    if let Some(note) = &line.note {
                        tip.push('\n');
                        tip.push_str(note);
                    }
                    let response = response.on_hover_text(tip);
                    for (k, text) in under.iter().enumerate() {
                        let mut job = egui::text::LayoutJob::simple_singleline(
                            text.clone(),
                            kit.font(),
                            kit.c(sk.failed),
                        );
                        job.wrap =
                            egui::text::TextWrapping::truncate_at_width((right - text_x).max(0.0));
                        let galley = painter.layout_job(job);
                        let ny = rect.top() + t.row_height + t.label_row_height * (k as f32 + 0.5);
                        painter.galley(
                            pos2(text_x, ny - galley.size().y * 0.5),
                            galley,
                            kit.c(sk.failed),
                        );
                    }
                    if response.clicked() {
                        fly = Some(line.name.clone());
                    }
                }
            });
        // Measured on another thread: look again soon.
        if pending {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        if let Some(name) = fly {
            self.fly_to_check(&name);
        }
    }

    /// Flies to what the check `name` measures: a feature or body it names,
    /// else the faces or edges it names, selected.
    fn fly_to_check(&mut self, name: &str) {
        let subject = self
            .side
            .checks_list
            .as_deref()
            .and_then(|list| subject_of(list, name));
        let Some(subject) = subject else {
            // Nothing named: the whole model.
            self.request_fly(FlyTo::Selection);
            return;
        };
        if let Some(row) = self
            .rows
            .iter()
            .find(|r| r.id == subject || r.name == subject)
        {
            let id = row.id.clone();
            self.state.selected_feature = Some(id.clone());
            self.queue_select(json!({"items": [{"kind": "feature", "feature": id}]}));
            self.request_fly(FlyTo::Feature(id));
            return;
        }
        self.queue_select(json!({ "names": [subject] }));
        self.request_fly(FlyTo::Selection);
    }
}

/// A measure as a row has room for: what comes before a parenthesis
/// (`58 mm (at most 58.0036 mm)` is `58 mm`), its number to four
/// significant digits (`0.302732308 kg` is `0.3027 kg`).
pub fn compact(text: &str) -> String {
    let head = text.split(" (").next().unwrap_or(text).trim();
    let (number, rest) = head.split_once(' ').unwrap_or((head, ""));
    let Ok(v) = number.parse::<f64>() else {
        return head.to_string();
    };
    if !v.is_finite() {
        return head.to_string();
    }
    let magnitude = if v == 0.0 { 0 } else { v.abs().log10_floor() };
    let decimals = (3 - magnitude).clamp(0, 6) as usize;
    let mut n = format!("{v:.decimals$}");
    if n.contains('.') {
        n = n.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if rest.is_empty() {
        n
    } else {
        format!("{n} {rest}")
    }
}

/// A check's measure as its row shows it, whole: what comes before a
/// parenthesis, every number in it as [`number_text`] writes it
/// (`6491.20578 mm^3` is `6491.2 mm³`, `0.000000153 mm` is `1.53e-7 mm`),
/// powers of units written as superscripts. A check's limit is written the
/// same way, so the two compare as they read.
pub fn measured_text(text: &str) -> String {
    let head = text.split(" (").next().unwrap_or(text).trim();
    pretty_units(
        &head
            .split(' ')
            .map(|word| match word.parse::<f64>() {
                Ok(v) if v.is_finite() => number_text(v),
                _ => word.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A number to five significant digits, written plainly from 1e-4 to
/// below 1e9 (`6491.2`, `0.30273`; a whole number of more digits than five
/// stays whole there, `123457`), in scientific notation beyond (`1.5e-7`,
/// `-1e-5`, `1.2346e12`); trailing zeros dropped, never a negative zero.
pub fn number_text(v: f64) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    if v == 0.0 {
        return "0".to_string();
    }
    let magnitude = v.abs().log10_floor();
    let trim = |n: String| {
        if n.contains('.') {
            n.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            n
        }
    };
    if !(-4..9).contains(&magnitude) {
        // d.dddde±x: the mantissa to five digits.
        let text = format!("{v:.4e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
        return format!("{}e{exponent}", trim(mantissa.to_string()));
    }
    let decimals = (4 - magnitude).clamp(0, 8) as usize;
    let n = trim(format!("{v:.decimals$}"));
    if n == "-0" { "0".to_string() } else { n }
}

/// A check's limit as its row writes it ([`measured_text`]), the
/// tolerance of an `eq` given the unit of its value when `assert.list`
/// writes it bare (`5000 mm^3 ± 1` is `5000 mm³ ± 1 mm³`).
pub fn limit_text(expected: &str) -> String {
    let text = match expected.split_once(" ± ") {
        Some((value, tol)) if tol.trim().parse::<f64>().is_ok() && value.trim().contains(' ') => {
            let unit = value.trim().split_once(' ').map_or("", |(_, u)| u);
            format!("{value} ± {} {unit}", tol.trim())
        }
        _ => expected.to_string(),
    };
    measured_text(&text)
}

/// Unit powers as superscripts: `mm^3` is `mm³`, `kg mm^2` is `kg mm²`,
/// `m^-2` is `m⁻²`, any whole power.
pub fn pretty_units(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let power = c == '^'
            && chars
                .peek()
                .is_some_and(|n| n.is_ascii_digit() || *n == '-');
        if !power {
            out.push(c);
            continue;
        }
        let mut digits = String::new();
        if chars.peek() == Some(&'-') {
            chars.next();
            digits.push('⁻');
        }
        let mut any = false;
        while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
            chars.next();
            any = true;
            digits.push(match d {
                '0' => '⁰',
                '1' => '¹',
                '2' => '²',
                '3' => '³',
                '4' => '⁴',
                '5' => '⁵',
                '6' => '⁶',
                '7' => '⁷',
                '8' => '⁸',
                _ => '⁹',
            });
        }
        if any {
            out.push_str(&digits);
        } else {
            // `^-` without a digit: as it was.
            out.push('^');
            out.push('-');
        }
    }
    out
}

/// How wide each part of a check's row may be drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckColumns {
    pub name: f32,
    pub measured: f32,
    pub limit: f32,
}

/// The widths of a check's row in `room` (parts `gap` apart) for a name,
/// a measure and a limit (0: none) of these natural widths: the measure
/// and the limit whole while they fit beside at least a share of the
/// name; else the limit goes first, then the measure is cut, the name
/// keeping its share. The name takes what is left.
pub fn check_columns(room: f32, gap: f32, name: f32, measured: f32, limit: f32) -> CheckColumns {
    let room = room.max(0.0);
    // The least the name keeps before the numbers give way.
    let name_least = name.min(room * 0.4);
    let mut limit = limit;
    if limit > 0.0 && measured + gap + limit + gap + name_least > room {
        limit = 0.0;
    }
    let measured = measured.min((room - gap - name_least).max(0.0));
    let used = (measured + if limit > 0.0 { gap + limit } else { 0.0 }).min(room);
    CheckColumns {
        name: (room - used - gap).max(0.0),
        measured,
        limit,
    }
}

/// Every number of `text` written short as [`compact`] writes a measure
/// (`491.20578 mm^3 ± 0.00649120578` is `491.2 mm^3 ± 0.006491`).
pub fn compact_all(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            if word.parse::<f64>().is_ok_and(f64::is_finite) {
                compact(word)
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The power of ten of `v`'s first digit, without a float logarithm
/// (`craft3d_math::Portable` keeps those for geometry): by counting.
trait Log10Floor {
    fn log10_floor(self) -> i32;
}

impl Log10Floor for f64 {
    fn log10_floor(self) -> i32 {
        let mut v = self;
        let mut p = 0;
        // Bounded: an f64 spans about 600 powers of ten.
        while v >= 10.0 && p < 400 {
            v /= 10.0;
            p += 1;
        }
        while v < 1.0 && v > 0.0 && p > -400 {
            v *= 10.0;
            p -= 1;
        }
        p
    }
}

/// What the check `name` of `assert.list` measures, when it names one
/// thing: `of`, `face`, `edge` or `from`, as written.
pub fn subject_of(list: &Value, name: &str) -> Option<String> {
    let checks = list
        .get("assertions")
        .and_then(Value::as_array)
        .or_else(|| list.as_array())?;
    let check = checks
        .iter()
        .find(|c| c.get("name").and_then(Value::as_str) == Some(name))?;
    let measure = check.get("measure")?;
    for key in ["of", "face", "edge", "from"] {
        match measure.get(key) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(Value::Array(a)) => {
                if let Some(Value::String(s)) = a.first() {
                    return Some(s.clone());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_are_written_short() {
        assert_eq!(compact("0.302732308 kg"), "0.3027 kg");
        assert_eq!(compact("58 mm (at most 58.0035698 mm)"), "58 mm");
        assert_eq!(compact("7 mm"), "7 mm");
        assert_eq!(compact("22.0001 mm"), "22 mm");
        assert_eq!(compact("true"), "true");
        assert_eq!(compact("12345.6 mm"), "12346 mm");
    }

    #[test]
    fn a_check_shows_its_measure_whole_to_five_digits() {
        assert_eq!(measured_text("6491.20578 mm^3"), "6491.2 mm³");
        assert_eq!(measured_text("7 mm"), "7 mm");
        assert_eq!(measured_text("5.5 mm"), "5.5 mm");
        assert_eq!(measured_text("0.302732308 kg"), "0.30273 kg");
        assert_eq!(measured_text("123456.7 mm"), "123457 mm");
        assert_eq!(measured_text("58 mm (at most 58.0035698 mm)"), "58 mm");
        assert_eq!(measured_text("true"), "true");
        // Small values keep their digits (the review: 5e-5 mm read 0 mm).
        assert_eq!(measured_text("0.00005 mm"), "5e-5 mm");
        assert_eq!(measured_text("1.5e-7 mm"), "1.5e-7 mm");
        assert_eq!(measured_text("-0.00001 mm"), "-1e-5 mm");
        assert_eq!(measured_text("0.000123456 mm"), "0.00012346 mm");
        assert_eq!(measured_text("-0.0 deg"), "0 deg");
        assert_eq!(measured_text("2.5e12 mm"), "2.5e12 mm");
        // The limit the same way as the measure.
        assert_eq!(measured_text("≥ 0.00001 mm"), "≥ 1e-5 mm");
        assert_eq!(
            measured_text("491.20578 mm^3 ± 0.00649120578"),
            "491.21 mm³ ± 0.0064912"
        );
    }

    #[test]
    fn a_check_that_does_not_pass_says_its_limit_and_why_on_the_row() {
        let line = |status, note: Option<&str>| CheckLine {
            name: "wall".to_string(),
            measured: "1 mm".to_string(),
            limit: "≥ 1.5 mm".to_string(),
            status,
            note: note.map(str::to_string),
        };
        // A failing measure: what it must be (the reason would repeat it).
        assert_eq!(
            line(CheckStatus::Failed, Some("too thin")).under(),
            vec!["must be ≥ 1.5 mm".to_string()]
        );
        assert_eq!(
            line(CheckStatus::NotProven, Some("sampled")).under().len(),
            2
        );
        assert!(line(CheckStatus::Passed, None).under().is_empty());
        // Nothing measured (an error): the reason says what went wrong.
        let mut error = line(CheckStatus::Failed, Some("names nothing yet"));
        error.measured = "—".to_string();
        assert_eq!(error.under().len(), 2);
    }

    #[test]
    fn a_limits_tolerance_carries_the_unit_of_its_value() {
        assert_eq!(limit_text("5000 mm^3 ± 1"), "5000 mm³ ± 1 mm³");
        assert_eq!(limit_text("0.25 mm ± 0.00001"), "0.25 mm ± 1e-5 mm");
        assert_eq!(limit_text("2 ± 0"), "2 ± 0");
        assert_eq!(limit_text("≥ 1.5 mm"), "≥ 1.5 mm");
    }

    #[test]
    fn unit_powers_are_superscripts() {
        assert_eq!(pretty_units("mm^4"), "mm⁴");
        assert_eq!(pretty_units("m^-2"), "m⁻²");
        assert_eq!(pretty_units("kg mm^2"), "kg mm²");
        assert_eq!(pretty_units("x^12"), "x¹²");
        assert_eq!(pretty_units("a ^ b"), "a ^ b");
        assert_eq!(pretty_units("^-"), "^-");
    }

    #[test]
    fn a_check_row_cuts_the_name_before_the_measure() {
        // Room for all: each whole.
        let c = check_columns(230.0, 6.0, 60.0, 70.0, 0.0);
        assert_eq!((c.measured, c.limit), (70.0, 0.0));
        assert!(c.name >= 60.0);
        // A long name gives way; the measure stays whole.
        let c = check_columns(230.0, 6.0, 200.0, 70.0, 0.0);
        assert_eq!(c.measured, 70.0);
        assert!((c.name - (230.0 - 70.0 - 6.0)).abs() < 1e-3);
        // A failing check's limit goes before the measure is cut.
        let c = check_columns(230.0, 6.0, 120.0, 70.0, 120.0);
        assert_eq!((c.measured, c.limit), (70.0, 0.0));
        let c = check_columns(230.0, 6.0, 40.0, 70.0, 90.0);
        assert_eq!((c.measured, c.limit), (70.0, 90.0));
        // Only a measure wider than the room less the name's share is cut.
        let c = check_columns(100.0, 6.0, 80.0, 90.0, 0.0);
        assert!((c.measured - (100.0 - 6.0 - 40.0)).abs() < 1e-3);
        assert!(c.name >= 39.0);
    }

    #[test]
    fn long_expected_values_are_written_short() {
        assert_eq!(
            compact_all("491.20578 mm^3 ± 0.00649120578"),
            "491.2 mm^3 ± 0.006491"
        );
        assert_eq!(compact_all("≥ 2"), "≥ 2");
        assert_eq!(compact_all("true"), "true");
        assert_eq!(compact_all(""), "");
    }

    #[test]
    fn the_subject_of_a_check() {
        let list = json!({"assertions": [
            {"name": "Wall", "measure": {"measure": "thickness", "of": "Sleeve"}},
            {"name": "Gap", "measure": {"measure": "distance", "from": ["F3:side(e1)"], "to": "x"}},
            {"name": "Mass", "measure": {"measure": "mass"}}
        ]});
        assert_eq!(subject_of(&list, "Wall").as_deref(), Some("Sleeve"));
        assert_eq!(subject_of(&list, "Gap").as_deref(), Some("F3:side(e1)"));
        assert_eq!(subject_of(&list, "Mass"), None);
        assert_eq!(subject_of(&list, "Nope"), None);
    }
}
