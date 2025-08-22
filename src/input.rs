use crate::{
    utils::{is_printable_char, keysym_to_egui_key},
    x11_window::X11Window,
};
use anyhow::Result;
use egui::{Event, PointerButton, Pos2, RawInput};
use x11rb::{
    connection::Connection as _,
    protocol::{
        Event as X11Event,
        xproto::{ConnectionExt as _, GetKeyboardMappingReply},
    },
};
use xkeysym::{KeyCode, Keysym, keysym};

pub struct Input {
    pub egui_input: RawInput,
    min_keycode: u8,
    max_keycode: u8,
    mapping: GetKeyboardMappingReply,
}

impl Input {
    pub fn new(window: &X11Window) -> Result<Self> {
        let window_setup = window.conn.setup();
        let min_keycode = window_setup.min_keycode;
        let max_keycode = window_setup.max_keycode;
        let mapping_reply = window
            .conn
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;

        let egui_input = RawInput {
            focused: true,
            ..Default::default()
        };

        Ok(Input {
            egui_input,
            min_keycode,
            max_keycode,
            mapping: mapping_reply,
        })
    }

    pub fn handle_event(&mut self, event: &X11Event) {
        let modifiers = &mut self.egui_input.modifiers;

        let egui_event = match event {
            X11Event::ButtonPress(ev) | X11Event::ButtonRelease(ev) => {
                let pressed = matches!(event, X11Event::ButtonPress(_));
                let pointer_button = match ev.detail {
                    1 => Some(PointerButton::Primary),
                    2 => Some(PointerButton::Middle),
                    3 => Some(PointerButton::Secondary),
                    _ => None,
                };

                pointer_button.map(|button| Event::PointerButton {
                    pos: Pos2::new(ev.event_x as f32, ev.event_y as f32),
                    button,
                    pressed,
                    modifiers: *modifiers,
                })
            }
            X11Event::KeyPress(ev) | X11Event::KeyRelease(ev) => 'blk: {
                let pressed = matches!(event, X11Event::KeyPress(_));
                let keycode = ev.detail;

                if let Some(keysym) = keysym(
                    KeyCode::new(keycode as u32),
                    0,
                    KeyCode::new(self.min_keycode as u32),
                    self.mapping.keysyms_per_keycode,
                    &self.mapping.keysyms,
                ) {
                    if keysym.is_modifier_key() {
                        modifiers.alt =
                            pressed && (keysym == Keysym::Alt_L || keysym == Keysym::Alt_R);
                        modifiers.ctrl =
                            pressed && (keysym == Keysym::Control_L || keysym == Keysym::Control_R);
                        modifiers.shift =
                            pressed && (keysym == Keysym::Shift_L || keysym == Keysym::Shift_R);
                        break 'blk None;
                    }

                    if let Some(key) = keysym_to_egui_key(Keysym::new(keysym.into())) {
                        if pressed
                            && key.name().len() == 1
                            && is_printable_char(*key.name().as_bytes().first().unwrap() as _)
                        {
                            break 'blk Some(Event::Text(key.name().to_owned()));
                        }

                        break 'blk Some(Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false, // egui will fill this in for us!
                            modifiers: *modifiers,
                        });
                    }

                    break 'blk None;
                }

                None
            }
            X11Event::MotionNotify(ev) => Some(Event::PointerMoved(Pos2::new(
                ev.event_x as f32,
                ev.event_y as f32,
            ))),
            _ => None,
        };

        if let Some(egui_event) = egui_event {
            self.egui_input.events.push(egui_event);
        }
    }
}
