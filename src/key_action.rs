use std::{fmt, mem};

use anyhow::Result;
use egui::{Event, Key, Modifiers, RawInput};
use log::debug;

trait ModifiersExtra {
    fn ctrl_only(&self) -> bool;
    fn real_shift_only(&self) -> bool;
}
impl ModifiersExtra for Modifiers {
    #[inline]
    fn ctrl_only(&self) -> bool {
        self.ctrl && !(self.alt || self.shift)
    }

    #[inline]
    fn real_shift_only(&self) -> bool {
        self.shift && !(self.alt || self.ctrl)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct KeyChord {
    pub key: Key,
    pub mods: Modifiers,
}
impl KeyChord {
    pub fn only_key(k: Key) -> KeyChord {
        KeyChord {
            key: k,
            mods: Modifiers::NONE,
        }
    }
}
impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut write_part = |str| -> fmt::Result {
            if !first {
                write!(f, "-")?;
            }
            first = false;
            write!(f, "{str}")?;
            Ok(())
        };

        if self.mods.contains(Modifiers::CTRL) {
            write_part("C")?;
        }
        if self.mods.contains(Modifiers::ALT) {
            write_part("M")?;
        }
        if self.mods.contains(Modifiers::SHIFT) {
            write_part("S")?;
        }

        if self.key >= Key::A && self.key <= Key::Z {
            write_part(&self.key.name().to_lowercase())?;
        } else {
            write_part(self.key.symbol_or_name())?;
        }

        Ok(())
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ScrollAction {
    ItemUp,
    ItemDown,
    HalfUp,
    HalfDown,
    PageUp,
    PageDown,
    ToTop,
    ToBottom,
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
            ToTop => ToBottom,
            ToBottom => ToTop,
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

pub struct KeyAction {
    pub pending_keys: Vec<KeyChord>,
}
impl KeyAction {
    pub fn new() -> Result<Self> {
        Ok(KeyAction {
            pending_keys: vec![],
        })
    }

    pub fn process_input(&mut self, egui_input: &mut RawInput) -> Vec<Action> {
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
                    use Key::*;

                    let prev_pending_keys_size = self.pending_keys.len();
                    let mut consumed_pending_keys = None;
                    let mut action = None;

                    let scroll_action = match key {
                        ArrowUp | K => Some(ScrollAction::ItemUp),
                        ArrowDown | J => Some(ScrollAction::ItemDown),

                        P if modifiers.ctrl_only() => Some(ScrollAction::ItemUp),
                        N if modifiers.ctrl_only() => Some(ScrollAction::ItemDown),

                        Tab if modifiers.real_shift_only() => Some(ScrollAction::ItemUp),
                        Tab => Some(ScrollAction::ItemDown),

                        U if modifiers.ctrl_only() => Some(ScrollAction::HalfUp),
                        D if modifiers.ctrl_only() => Some(ScrollAction::HalfDown),

                        B if modifiers.ctrl_only() => Some(ScrollAction::PageUp),
                        F if modifiers.ctrl_only() => Some(ScrollAction::PageDown),

                        G if modifiers.real_shift_only() => Some(ScrollAction::ToBottom),
                        G => {
                            if self.pending_keys.len() == 1
                                && let Some(pending_key) = self.pending_keys.first()
                                && *pending_key == KeyChord::only_key(G)
                            {
                                consumed_pending_keys =
                                    Some(self.pending_keys.drain(..).collect::<Vec<_>>());
                                Some(ScrollAction::ToTop)
                            } else {
                                self.pending_keys.push(KeyChord::only_key(G));
                                None
                            }
                        }

                        _ => None,
                    };
                    if let Some(scroll_action) = scroll_action {
                        action = Some(Action::Scroll(scroll_action));
                    }

                    if action.is_none() {
                        action = match key {
                            Enter | Space => Some(Action::Paste),

                            D => {
                                if self.pending_keys.len() == 1
                                    && let Some(pending_key) = self.pending_keys.first()
                                    && *pending_key == KeyChord::only_key(D)
                                {
                                    consumed_pending_keys =
                                        Some(self.pending_keys.drain(..).collect::<Vec<_>>());
                                    Some(Action::Remove)
                                } else {
                                    self.pending_keys.push(KeyChord::only_key(D));
                                    None
                                }
                            }
                            Delete => Some(Action::Remove),

                            Escape => {
                                if self.pending_keys.is_empty() {
                                    Some(Action::HideWindow)
                                } else {
                                    debug!("received Escape, clearing pending keys");
                                    self.pending_keys.clear();
                                    None
                                }
                            }
                            Q => Some(Action::HideWindow),

                            _ => None,
                        };
                    }

                    if !self.pending_keys.is_empty()
                        && prev_pending_keys_size == self.pending_keys.len()
                    {
                        debug!(
                            "received {key:?} with {modifiers:?}, triggering invalid key sequence: {:?}",
                            self.pending_keys
                        );
                        self.pending_keys.clear();
                    } else if let Some(action) = action {
                        if let Some(pending_keys) = consumed_pending_keys {
                            debug!(
                                "received {key:?} with {modifiers:?}, pending keys: {:?}, converting to action {action:?}",
                                pending_keys
                            );
                        } else {
                            debug!(
                                "received {key:?} with {modifiers:?}, converting to action {action:?}"
                            );
                        }
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
