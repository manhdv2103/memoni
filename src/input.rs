use crate::{
    utils::{is_printable_char, keysym_to_egui_key},
    x11_key_converter::X11KeyConverter,
    x11_window::X11Window,
};
use anyhow::Result;
use egui::{Event, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, Vec2};
use x11rb::protocol::Event as X11Event;
use xkeysym::Keysym;

pub struct Input<'a> {
    pub egui_input: RawInput,
    window: &'a X11Window<'a>,
    key_converter: &'a X11KeyConverter,
}

impl<'a> Input<'a> {
    pub fn new(window: &'a X11Window, key_converter: &'a X11KeyConverter) -> Result<Self> {
        let egui_input = RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(
                Pos2::new(0.0, 0.0),
                Vec2::new(
                    window.config.layout.window_dimensions.width as f32,
                    window.config.layout.window_dimensions.height as f32,
                ),
            )),
            ..Default::default()
        };

        Ok(Input {
            egui_input,
            window,
            key_converter,
        })
    }

    pub fn handle_event(&mut self, event: &X11Event) {
        let modifiers = &mut self.egui_input.modifiers;

        let egui_event = match event {
            X11Event::ButtonPress(ev) | X11Event::ButtonRelease(ev) if ev.detail <= 3 => {
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
            X11Event::ButtonPress(ev) | X11Event::ButtonRelease(ev) => {
                let delta = match ev.detail {
                    4 => Some(egui::vec2(0.0, 1.0)),
                    5 => Some(egui::vec2(0.0, -1.0)),
                    6 => Some(egui::vec2(1.0, 0.0)),
                    7 => Some(egui::vec2(-1.0, 0.0)),
                    _ => None,
                };

                delta.map(|d| Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: d,
                    modifiers: *modifiers,
                })
            }
            X11Event::KeyPress(ev) | X11Event::KeyRelease(ev) => 'blk: {
                let pressed = matches!(event, X11Event::KeyPress(_));
                let keycode = ev.detail;

                if let Some(keysym) = self.key_converter.keycode_to_keysym(keycode.into()) {
                    if keysym.is_modifier_key() {
                        if keysym == Keysym::Alt_L || keysym == Keysym::Alt_R {
                            modifiers.alt = pressed;
                        }
                        if keysym == Keysym::Control_L || keysym == Keysym::Control_R {
                            modifiers.ctrl = pressed;
                        }
                        if keysym == Keysym::Shift_L || keysym == Keysym::Shift_R {
                            modifiers.shift = pressed;
                        }
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
            X11Event::MotionNotify(ev) => {
                let (x, y) = self.window.get_current_win_pos();
                Some(Event::PointerMoved(Pos2::new(
                    (ev.root_x - x) as f32,
                    (ev.root_y - y) as f32,
                )))
            }
            _ => None,
        };

        if let Some(egui_event) = egui_event {
            self.egui_input.events.push(egui_event);
        }
    }
}
