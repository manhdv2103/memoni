use std::mem;

use anyhow::Result;
use egui::{Event, Key, Modifiers, RawInput};
use log::debug;

trait ModifiersExtra {
    fn ctrl_only(&self) -> bool;
}
impl ModifiersExtra for Modifiers {
    fn ctrl_only(&self) -> bool {
        self.ctrl && !(self.alt || self.shift)
    }
}

#[derive(Debug)]
pub enum Action {
    Paste,
    Scroll(isize),
    Remove,
    HideWindow,
}

pub struct KeyAction {}

impl KeyAction {
    pub fn new() -> Result<Self> {
        Ok(KeyAction {})
    }

    pub fn from_input(&self, egui_input: &mut RawInput) -> Vec<Action> {
        let mut actions = vec![];
        for event in mem::take(&mut egui_input.events) {
            if let Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = event
            {
                if pressed {
                    let mut action = None;
                    let scroll_step = match key {
                        Key::ArrowUp | Key::K => -1,
                        Key::P if modifiers.ctrl_only() => -1,
                        Key::Tab if modifiers.shift_only() => -1,
                        Key::ArrowDown | Key::J => 1,
                        Key::N if modifiers.ctrl_only() => 1,
                        Key::Tab => 1,

                        // TODO: proper half-page/full-page step (based on window and item sizes)
                        Key::D if modifiers.ctrl_only() => 5,
                        Key::U if modifiers.ctrl_only() => -5,
                        Key::F if modifiers.ctrl_only() => 10,
                        Key::B if modifiers.ctrl_only() => -10,
                        _ => 0,
                    };
                    if scroll_step != 0 {
                        action = Some(Action::Scroll(scroll_step));
                    }

                    if action.is_none() {
                        action = match key {
                            Key::Enter | Key::Space => Some(Action::Paste),

                            Key::D if modifiers.shift_only() => Some(Action::Remove),
                            Key::Delete => Some(Action::Remove),

                            Key::Escape => Some(Action::HideWindow),
                            Key::Q => Some(Action::HideWindow),

                            _ => None,
                        };
                    }

                    if let Some(action) = action {
                        debug!(
                            "received {key:?} with {modifiers:?}, converting to action {action:?}"
                        );
                        actions.push(action);
                    }
                }
            } else {
                egui_input.events.push(event);
            }
        }

        actions
    }
}
