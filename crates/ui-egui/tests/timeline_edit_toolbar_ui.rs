//! The compact Main Edit toolbar dispatches the existing undoable timeline commands.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::ClipId;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    first: ClipId,
    frame: i64,
}

impl Driver {
    fn demo() -> Self {
        Self::with_width(1600.0)
    }

    fn with_width(width: f32) -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).unwrap();
        session.execute("file.newSequence", json!({"name": "Toolbar", "video": 1, "audio": 1})).unwrap();
        let rate = session.sequence_rate();
        let item = session.project.items.values().find(|i| i.has_video() && i.as_media().is_some()).unwrap().id;
        for start in [0, 96] {
            session.execute("timeline.place", json!({"item": item.0, "frame": start, "duration": rate.tick_of(48).0})).unwrap();
        }
        session.execute("edit.deselectAll", json!({})).unwrap();
        let first = session.active_sequence().unwrap().video_tracks[0].items[0].id;
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(width, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut driver = Self { harness, tx, first, frame: rate.tick_of(24).0 };
        driver.frames(4);
        driver
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (request, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(request).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], json!(true), "{method} {params}: {value}");
                return value["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn exec(&mut self, command: &str, params: Value) {
        self.ok("engine.execute", json!({"command": command, "params": params}));
        self.frames(2);
    }

    fn click(&mut self, action: &str) {
        self.ok("ui.click", json!({"id": format!("timeline.editToolbar.{action}")}));
        self.frames(2);
    }

    fn enabled(&mut self, action: &str) -> bool {
        let id = format!("timeline.editToolbar.{action}");
        let elements = self.ok("ui.elements", json!({"prefix": id}));
        elements.as_array().unwrap().iter().find(|element| element["id"] == id).unwrap()["enabled"].as_bool().unwrap()
    }

    fn select_first(&mut self) {
        self.exec("timeline.select", json!({"clips": [self.first.0]}));
    }

    fn at_first_middle(&mut self) {
        self.exec("playhead.set", json!({"time": self.frame}));
    }

    fn video_items(&self) -> usize {
        self.harness.state().session.active_sequence().unwrap().video_tracks[0].items.len()
    }

    fn first_bounds(&self) -> (i64, i64) {
        let seq = self.harness.state().session.active_sequence().unwrap();
        let item = &seq.video_tracks[0].items[0];
        (item.start.0, item.end().0)
    }

    fn first_source_in(&self) -> i64 {
        self.harness.state().session.active_sequence().unwrap().video_tracks[0].items[0].source_in.0
    }

    fn undo_steps(&self) -> usize {
        self.harness.state().session.history.undo.len()
    }
}

#[test]
fn the_toolbar_keeps_all_actions_visible_in_a_narrow_timeline() {
    let mut d = Driver::with_width(1000.0);
    let right = d.harness.state().tl.layout.as_ref().unwrap().content.max.x;
    let elements = d.ok("ui.elements", json!({"prefix": "timeline.editToolbar."}));
    assert_eq!(elements.as_array().unwrap().len(), 5);
    for element in elements.as_array().unwrap() {
        let bounds = element["rect"].as_array().unwrap();
        assert_eq!(bounds[2].as_f64().unwrap(), 26.0, "the toolbar uses icon-only buttons in a narrow panel");
        assert!(
            bounds[0].as_f64().unwrap() + bounds[2].as_f64().unwrap() <= right as f64,
            "button should remain within the timeline (right={right}): {element}"
        );
    }
}

#[test]
fn the_toolbar_actions_edit_and_undo_through_existing_commands() {
    let mut d = Driver::demo();
    d.at_first_middle();
    d.select_first();
    let before = d.first_bounds();
    let source_in = d.first_source_in();
    let initial_undo = d.undo_steps();
    for action in ["split", "deleteLeft", "deleteRight", "deleteSelected", "addMarker"] {
        d.at_first_middle();
        d.select_first();
        assert!(d.enabled(action), "{action} should be available");
        let undo = d.undo_steps();
        d.click(action);
        assert_eq!(d.undo_steps(), undo + 1, "{action} must create one undo step");
        match action {
            "split" => assert_eq!(d.video_items(), 3),
            "deleteLeft" => {
                assert_eq!(d.first_bounds(), (0, d.frame));
                assert_eq!(d.first_source_in(), source_in + d.frame, "the left side of the source was removed");
            }
            "deleteRight" => {
                assert_eq!(d.first_bounds(), (0, d.frame));
                assert_eq!(d.first_source_in(), source_in, "the right side was removed without moving the source in-point");
            }
            "deleteSelected" => assert_eq!(d.video_items(), 1),
            "addMarker" => assert!(d.harness.state().session.active_sequence().unwrap().markers.iter().any(|m| m.start.0 == d.frame)),
            _ => unreachable!(),
        }
        d.exec("edit.undo", json!({}));
        assert_eq!(d.video_items(), 2);
        assert_eq!(d.first_bounds(), before);
    }
    assert_eq!(d.undo_steps(), initial_undo);
}

#[test]
fn the_toolbar_disables_actions_without_a_valid_target() {
    let mut d = Driver::demo();
    d.exec("edit.deselectAll", json!({}));
    d.exec("playhead.set", json!({"frame": 72}));
    for action in ["split", "deleteLeft", "deleteRight", "deleteSelected"] {
        assert!(!d.enabled(action), "{action} should be disabled over a gap");
    }
    assert!(d.enabled("addMarker"), "markers may be placed over a gap");
    let undo = d.undo_steps();
    d.click("split");
    d.click("deleteLeft");
    d.click("deleteRight");
    d.click("deleteSelected");
    assert_eq!(d.undo_steps(), undo);

    d.at_first_middle();
    assert!(d.enabled("split"));
    assert!(d.enabled("deleteLeft"));
    assert!(d.enabled("deleteRight"));
    assert!(!d.enabled("deleteSelected"));
    d.select_first();
    assert!(d.enabled("deleteSelected"));

    let tracks: Vec<u64> = d.harness.state().session.active_sequence().unwrap().all_tracks().map(|track| track.id.0).collect();
    for track in tracks {
        d.exec("timeline.setTrack", json!({"track": track, "locked": true}));
    }
    for action in ["split", "deleteLeft", "deleteRight", "deleteSelected"] {
        assert!(!d.enabled(action), "{action} should be disabled when the selected tracks are locked");
    }
    assert!(d.enabled("addMarker"));

    let ruler_top = d.harness.state().tl.layout.as_ref().unwrap().ruler.min.y;
    let elements = d.ok("ui.elements", json!({"prefix": "timeline.editToolbar."}));
    assert_eq!(elements.as_array().unwrap().len(), 5);
    for element in elements.as_array().unwrap() {
        let bounds = element["rect"].as_array().unwrap();
        assert!(bounds[1].as_f64().unwrap() + bounds[3].as_f64().unwrap() < ruler_top as f64);
        assert!(!element["label"].as_str().unwrap().is_empty(), "every button has a tooltip");
    }
}
