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
pub enum ScrollAction {
    ItemUp,
    ItemDown,
    HalfUp,
    HalfDown,
    PageUp,
    PageDown,
}

impl ScrollAction {
    pub fn flipped(self) -> Self {
        use ScrollAction::*;
        match self {
            ItemUp => ItemDown,
            ItemDown => ItemUp,
            HalfUp => HalfDown,
            HalfDown => HalfUp,
            PageUp => PageDown,
            PageDown => PageUp,
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Paste,
    Scroll(ScrollAction),
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
                    let scroll_action = match key {
                        Key::ArrowUp | Key::K => Some(ScrollAction::ItemUp),
                        Key::P if modifiers.ctrl_only() => Some(ScrollAction::ItemUp),
                        Key::Tab if modifiers.shift_only() => Some(ScrollAction::ItemUp),
                        Key::ArrowDown | Key::J => Some(ScrollAction::ItemDown),
                        Key::N if modifiers.ctrl_only() => Some(ScrollAction::ItemDown),
                        Key::Tab => Some(ScrollAction::ItemDown),
                        Key::U if modifiers.ctrl_only() => Some(ScrollAction::HalfUp),
                        Key::D if modifiers.ctrl_only() => Some(ScrollAction::HalfDown),
                        Key::B if modifiers.ctrl_only() => Some(ScrollAction::PageUp),
                        Key::F if modifiers.ctrl_only() => Some(ScrollAction::PageDown),
                        _ => None,
                    };
                    if let Some(scroll_action) = scroll_action {
                        action = Some(Action::Scroll(scroll_action));
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
