// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Note properties, ported from the Michael Lane Edition: double-clicking a note opens a form of its controls; only
//! the fields the user changed apply, to every selected note of the note's channel, as one undo step. The form is a
//! dialog of the app's own (`dialog::Body::NoteProperties`), painted by `views::dialog`.

use std::time::Instant;

use os_model::Note;
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, State};
use crate::dialog::{Body, Field, Modal};
use crate::piano;

/// The form's fields, in the order `values` returns them.
pub const LABELS: [&str; 8] = [
    "Velocity (0-128)",
    "Pan (64 = center)",
    "Release (0-128)",
    "Fine pitch (120 = center)",
    "Cutoff (0-255)",
    "Resonance (0-255)",
    "Color group (1-16)",
    "Length (ticks)",
];

/// What a click on the form asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Put the caret in field `i`.
    Field(usize),
    Slide,
    Porta,
    Apply,
    Cancel,
}

/// The open form: the notes it was opened on, the note it shows, and the fields' text.
pub struct Form {
    pub pattern: u16,
    pub channel: u16,
    /// The notes it applies to, with their values when it opened (it refuses to apply if any has changed since).
    pub original: Vec<(usize, Note)>,
    /// The note double-clicked: the one whose values the fields show and whose type the switches change.
    pub anchor: Note,
    pub fields: [Field; 8],
    pub focus: usize,
    pub slide: bool,
    pub porta: bool,
    /// Why the last Apply was refused; empty when there is nothing to say.
    pub message: String,
}

fn values(n: Note) -> [u32; 8] {
    [n.velocity.into(), n.pan.into(), n.release.into(), n.fine_pitch.into(), n.mod_x.into(), n.mod_y.into(), u32::from(n.group & 15) + 1, n.length]
}

fn form(s: &mut State) -> Option<&mut Form> {
    match &mut s.dialogs.last_mut()?.body {
        Body::NoteProperties(f) => Some(f),
        _ => None,
    }
}

/// A press on note `i` at `at`: true when it is the second press of a double-click, which the caller opens the form
/// for. A note that moved or was redrawn between the presses is not the same note (`old == n`), so a drag never turns
/// the next press into a double-click.
pub fn double_click(s: &mut State, i: usize, at: (f32, f32)) -> bool {
    let Some(&n) = piano::notes(s).get(i) else { return false };
    if Some(n.channel) != piano::channel(s) {
        return false;
    }
    let now = Instant::now();
    let first = s.piano.properties_click.take();
    let double = first.is_some_and(|(p, j, old, t, xy)| {
        p == s.pattern && j == i && old == n && now.duration_since(t).as_millis() < 400 && (xy.0 - at.0).abs() <= 4.0 && (xy.1 - at.1).abs() <= 4.0
    });
    if !double {
        s.piano.properties_click = Some((s.pattern, i, n, now, at));
    }
    double
}

/// Opens the form on note `i`: the selection's notes when it is selected, else just it.
pub fn open(s: &mut State, i: usize) {
    let Some(&anchor) = piano::notes(s).get(i) else { return };
    let selected = s.piano.selection(s.pattern, Some(anchor.channel));
    let ids = if selected.contains(&i) { selected } else { vec![i] };
    let original: Vec<(usize, Note)> =
        ids.into_iter().filter_map(|j| piano::notes(s).get(j).copied().filter(|n| n.channel == anchor.channel).map(|n| (j, n))).collect();
    let fields = values(anchor).map(|v| {
        let mut f = Field::default();
        f.set(&v.to_string());
        f
    });
    s.piano.properties_click = None;
    let form = Form {
        pattern: s.pattern,
        channel: anchor.channel,
        original,
        anchor,
        fields,
        focus: 0,
        slide: anchor.slide(),
        porta: anchor.porta(),
        message: String::new(),
    };
    s.dialogs.push(Modal::new(Body::NoteProperties(form)));
}

/// A click on the form.
pub fn command(app: &mut App, c: Command) {
    match c {
        Command::Cancel => {
            app.state.dialogs.pop();
        }
        Command::Apply => return apply(app),
        Command::Field(i) => {
            if let Some(f) = form(&mut app.state) {
                if i < f.fields.len() {
                    f.focus = i;
                    f.fields[i].select_all();
                }
            }
        }
        Command::Slide | Command::Porta => {
            if let Some(f) = form(&mut app.state) {
                if c == Command::Slide {
                    f.slide = !f.slide;
                } else {
                    f.porta = !f.porta;
                }
            }
        }
    }
    app.invalidate();
}

fn refuse(app: &mut App, message: &str, focus: Option<usize>) {
    if let Some(f) = form(&mut app.state) {
        f.message = message.into();
        if let Some(i) = focus {
            f.focus = i;
        }
    }
    app.invalidate();
}

/// Reads the fields back, checks them, and applies only what changed to the notes the form opened on.
fn apply(app: &mut App) {
    let Some((texts, slide, porta, anchor, original)) = form(&mut app.state).map(|f| {
        let texts: Vec<String> = f.fields.iter().map(|x| x.text.clone()).collect();
        (texts, f.slide, f.porta, f.anchor, f.original.clone())
    }) else {
        return;
    };
    let baseline = values(anchor);
    // Velocity, pan and release go to 128; fine pitch to 240 (its 120 is the centre); the colour groups are 1-16.
    let limits = [128u32, 128, 128, 240, 255, 255, 16, u32::MAX];
    let mut v = baseline;
    for j in 0..v.len() {
        match texts[j].trim().parse::<u32>() {
            Ok(n) if n == baseline[j] || (n <= limits[j] && (j != 6 || n >= 1)) => v[j] = n,
            _ => return refuse(app, &format!("Check {}.", LABELS[j]), Some(j)),
        }
    }
    if v[7] != baseline[7] && original.iter().any(|(_, n)| n.position.checked_add(v[7]).is_none()) {
        return refuse(app, "Length exceeds the project timeline.", Some(7));
    }
    // The notes may have changed under the form (an undo, another edit): it closes without applying.
    let stale = match app.state.dialogs.last().map(|d| &d.body) {
        Some(Body::NoteProperties(f)) => {
            app.state.pattern != f.pattern
                || piano::channel(&app.state) != Some(f.channel)
                || !f.original.iter().all(|(i, n)| piano::notes(&app.state).get(*i) == Some(n))
        }
        _ => true,
    };
    app.state.dialogs.pop();
    if stale {
        app.state.message = Some("The notes changed: reopen Note properties.".into());
        app.invalidate();
        return;
    }
    let changed = v != baseline || slide != anchor.slide() || porta != anchor.porta();
    if !changed {
        app.invalidate();
        return;
    }
    let s = &mut app.state;
    piano::snapshot_as(s, "Edit note properties");
    let mut anchor_after = None;
    for (i, old) in original {
        let updated = {
            let p = piano::pattern_mut(s);
            let Some(n) = p.notes.get_mut(i) else { continue };
            if v[0] != baseline[0] {
                n.velocity = v[0] as u8;
            }
            if v[1] != baseline[1] {
                n.pan = v[1] as u8;
            }
            if v[2] != baseline[2] {
                n.release = v[2] as u8;
            }
            if v[3] != baseline[3] {
                n.fine_pitch = v[3] as u8;
            }
            if v[4] != baseline[4] {
                n.mod_x = v[4] as u8;
            }
            if v[5] != baseline[5] {
                n.mod_y = v[5] as u8;
            }
            if v[6] != baseline[6] {
                n.group = (n.group & 0xf0) | ((v[6] - 1) as u8);
            }
            if v[7] != baseline[7] {
                n.length = v[7];
            }
            if slide != anchor.slide() {
                n.set_slide(slide);
            }
            if porta != anchor.porta() {
                n.set_porta(porta);
            }
            *n
        };
        if old == anchor {
            anchor_after = Some(updated);
        }
    }
    // The note double-clicked becomes the last one touched: the next note drawn takes its controls.
    if let Some(u) = anchor_after {
        s.piano.last_controls = Some(u);
        if u.length > 0 {
            s.piano.last_len = u.length;
        }
    }
    s.piano.dirty = true;
    piano::finish_edit(s);
    app.invalidate();
}

/// A key press while the form is open: Tab and Shift+Tab walk the fields, Enter applies, Esc cancels, the rest types.
pub fn key(app: &mut App, key: &Key, ctrl: bool, alt: bool) {
    match key {
        Key::Named(NamedKey::Escape) => return command(app, Command::Cancel),
        Key::Named(NamedKey::Enter) => return command(app, Command::Apply),
        Key::Named(NamedKey::Tab) => {
            let back = app.shift;
            if let Some(f) = form(&mut app.state) {
                let n = f.fields.len();
                f.focus = if back { (f.focus + n - 1) % n } else { (f.focus + 1) % n };
                f.fields[f.focus].select_all();
            }
        }
        _ => {
            if let Some(f) = form(&mut app.state) {
                let i = f.focus;
                // A field holds a number at most a few characters long; the rest of the keys are left alone.
                if f.fields[i].len() < 12 || !matches!(key, Key::Character(_)) {
                    f.fields[i].key(key, ctrl, alt);
                }
            }
        }
    }
    app.invalidate();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Win};

    fn app() -> App {
        let mut a = App::new(None);
        a.state.project = crate::app::test_project();
        a.state.selected_channel = 0;
        a.state.z = vec![Win::PianoRoll];
        a
    }

    /// Press note `i` in the piano roll once (the hit its note registers).
    fn press(a: &mut App, c: &mut os_ui::Canvas, i: usize) {
        a.paint(c, None);
        let r = a.hits.iter().find(|h| h.action == Action::PianoNote(i)).expect("the note's hit").rect;
        a.mouse_move(r.x + 5.0, r.cy());
        a.mouse_down();
        a.mouse_up();
    }

    #[test]
    fn a_double_click_opens_the_form_and_apply_edits_the_selection() {
        let mut a = app();
        let n = Note { length: 96, key: 60, ..Note::step(0, 0) };
        let mut other = n;
        other.position = 192;
        other.velocity = 33;
        other.key = 64;
        piano::pattern_mut(&mut a.state).notes = vec![n, other, Note { channel: 1, position: 384, ..n }];
        a.state.piano.select(a.state.pattern, 0, vec![0, 1]);
        let before = piano::notes(&a.state).to_vec();
        let Ok(mut c) = os_ui::Canvas::new(1280, 800, 1.0) else { return };
        press(&mut a, &mut c, 0);
        assert!(a.state.dialogs.is_empty(), "one press is a move, not a double-click");
        press(&mut a, &mut c, 0);
        assert!(a.modal_open(), "the second press opens Note properties");
        assert_eq!(form(&mut a.state).expect("the form").original.len(), 2, "both selected notes");
        command(&mut a, Command::Slide);
        command(&mut a, Command::Field(6));
        key(&mut a, &Key::Character("6".into()), false, false);
        assert!(form(&mut a.state).expect("still open").slide);
        command(&mut a, Command::Apply);
        assert!(!a.modal_open(), "Apply closes it");
        for i in 0..2 {
            let got = piano::notes(&a.state)[i];
            assert!(got.slide(), "note {i} took the switch");
            assert_eq!(got.group & 15, 5, "and the colour group");
            assert_eq!((got.position, got.velocity), (before[i].position, before[i].velocity), "what was not changed stays");
        }
        assert_eq!(piano::notes(&a.state)[2], before[2], "another channel's note is left alone");
        assert_eq!(a.state.piano.last_controls.expect("the last note").group & 15, 5);
        assert!(crate::history::undo(&mut a.state).is_some());
        assert_eq!(piano::notes(&a.state), before, "one undo step");
    }

    #[test]
    fn cancel_changes_nothing_and_a_bad_value_is_refused() {
        let mut a = app();
        let n = Note { length: 48, key: 60, ..Note::step(0, 0) };
        piano::pattern_mut(&mut a.state).notes = vec![n];
        let before = piano::notes(&a.state).to_vec();
        open(&mut a.state, 0);
        command(&mut a, Command::Porta);
        key(&mut a, &Key::Named(NamedKey::Escape), false, false);
        assert!(!a.modal_open());
        assert_eq!(piano::notes(&a.state), before, "Esc cancels");
        open(&mut a.state, 0);
        form(&mut a.state).expect("the form").fields[6].set("17");
        command(&mut a, Command::Apply);
        assert!(a.modal_open(), "out of range: the form stays open");
        assert!(!form(&mut a.state).expect("still open").message.is_empty());
        assert_eq!(piano::notes(&a.state), before);
        command(&mut a, Command::Cancel);
    }

    #[test]
    fn a_drawn_note_is_not_the_first_press_of_a_double_click() {
        let mut a = app();
        let Ok(mut c) = os_ui::Canvas::new(1280, 800, 1.0) else { return };
        a.paint(&mut c, None);
        let g = piano::Geom::new(&a.state, a.hits.iter().find(|h| h.action == Action::PianoGrid).expect("the grid").rect);
        a.mouse_move(g.x_of(96.0) + 4.0, g.y_of(60) + 4.0);
        a.mouse_down();
        a.mouse_up();
        assert_eq!(piano::notes(&a.state).len(), 1);
        press(&mut a, &mut c, 0);
        assert!(!a.modal_open(), "the click that drew it is not a first press");
    }
}
