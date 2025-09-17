extern crate x11rb;

use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    fmt,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use x11rb::protocol::{
    Event,
    xfixes::SelectionEventMask,
    xproto::{ConnectionExt as _, *},
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;
use x11rb::{connection::Connection, protocol::xfixes};
use x11rb::{connection::RequestConnection as _, protocol::xtest::ConnectionExt as _};
use xkeysym::Keysym;

use crate::{
    atom_pool::AtomPool,
    config::{Binding, Config, Modifier},
    utils::plaintext_mime_score,
    x11_key_converter::X11KeyConverter,
    x11_window::X11Window,
};

// Heavily modified from https://github.com/SUPERCILEX/clipboard-history/blob/master/x11/src/main.rs

const HASH_SEED: usize = 0xfd9aadcf54cc0f35;
const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();
const OVERDUE_TIMEOUT: Duration = Duration::from_secs(3);

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        PRIMARY,
        CLIPBOARD,

        INCR,
        TIMESTAMP,
        TARGETS,
        SAVE_TARGETS,
        MULTIPLE,
    }
}

pub struct SelectionItem {
    pub id: u64,
    pub data: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SelectionType {
    PRIMARY,
    CLIPBOARD,
}
impl fmt::Display for SelectionType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug)]
enum TaskState {
    TargetsRequest,
    PendingSelection {
        mimes: HashMap<Atom, String>,
        data: HashMap<String, Vec<u8>>,
    },
}

struct Task {
    state: TaskState,
    last_update: Instant,
}

impl Task {
    fn new(state: TaskState) -> Self {
        Task {
            state,
            last_update: Instant::now(),
        }
    }

    fn set_state(&mut self, state: TaskState) {
        self.state = state;
        self.last_update = Instant::now();
    }
}

pub struct Selection<'a> {
    pub items: VecDeque<SelectionItem>,

    conn: &'a XCBConnection,
    screen: &'a Screen,
    key_converter: &'a X11KeyConverter,
    config: &'a Config,
    selection_atom: Atom,
    atoms: Atoms,
    tasks: HashMap<Atom, Task>,
    transfer_window: u32,
    paste_window: u32,
    transfer_atoms: AtomPool<'a>,
    mime_atoms: Cell<HashMap<String, Atom>>,
    paste_item_id: Option<u64>,
}

impl<'a> Selection<'a> {
    pub fn new(
        window: &'a X11Window,
        key_converter: &'a X11KeyConverter,
        selection_type: SelectionType,
        config: &'a Config,
    ) -> Result<Self> {
        let conn = &window.conn;
        let root = window.screen.root;

        conn.prefetch_extension_information(xfixes::X11_EXTENSION_NAME)?;
        conn.extension_information(xfixes::X11_EXTENSION_NAME)?
            .context("No XFixes found")?;
        xfixes::query_version(conn, 5, 0)?.reply()?;

        let atoms = Atoms::new(conn)?.reply()?;
        let selection_atom = match selection_type {
            SelectionType::PRIMARY => atoms.PRIMARY,
            SelectionType::CLIPBOARD => atoms.CLIPBOARD,
        };

        let create_util_window = |title: &[u8], aux, kind| -> Result<_> {
            let win_id = conn.generate_id()?;
            conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win_id,
                root,
                0,
                0,
                1,
                1,
                0,
                kind,
                x11rb::COPY_FROM_PARENT,
                &aux,
            )?;
            conn.change_property8(
                PropMode::REPLACE,
                win_id,
                window.atoms._NET_WM_NAME,
                window.atoms.UTF8_STRING,
                title,
            )?;
            Ok(win_id)
        };
        let transfer_window = create_util_window(
            b"Memoni transfer window",
            CreateWindowAux::default().event_mask(EventMask::PROPERTY_CHANGE),
            WindowClass::INPUT_ONLY,
        )?;

        xfixes::select_selection_input(
            conn,
            root,
            selection_atom,
            SelectionEventMask::SET_SELECTION_OWNER,
        )?;

        Ok(Selection {
            items: VecDeque::new(),
            conn,
            screen: &window.screen,
            key_converter,
            config,
            selection_atom,
            atoms,
            tasks: HashMap::new(),
            transfer_window,
            paste_window: window.win_id,
            transfer_atoms: AtomPool::new(conn, "SELECTION_DATA")?,
            mime_atoms: Cell::new(HashMap::new()),
            paste_item_id: None,
        })
    }

    pub fn handle_event(&mut self, event: &Event) -> Result<()> {
        let conn = self.conn;
        let atoms = self.atoms;

        match event {
            // Captures copied data
            Event::SelectionNotify(ev) => {
                if ev.requestor != self.transfer_window {
                    return Ok(());
                }

                // Conversion request failed
                let transfer_atom = ev.property;
                if transfer_atom == x11rb::NONE {
                    return Ok(());
                }

                let Some(task) = self.tasks.get_mut(&transfer_atom) else {
                    return Ok(());
                };

                let property = conn.get_property(
                    true,
                    ev.requestor,
                    ev.property,
                    GetPropertyType::ANY,
                    0,
                    u32::MAX,
                )?;
                conn.flush()?;

                return match task.state {
                    TaskState::TargetsRequest => {
                        let property = property.reply()?;
                        if property.type_ == atoms.INCR {
                            // Ignoring abusive TARGETS property
                            return Ok(());
                        }

                        let Some(value) = property.value32() else {
                            // Invalid TARGETS property value format
                            return Ok(());
                        };

                        let mut atom_cookies = Vec::new();
                        for atom in value {
                            // Special atoms
                            if [
                                atoms.TIMESTAMP,
                                atoms.TARGETS,
                                atoms.SAVE_TARGETS,
                                atoms.MULTIPLE,
                            ]
                            .contains(&atom)
                            {
                                continue;
                            }

                            atom_cookies.push((conn.get_atom_name(atom)?, atom));
                        }
                        if atom_cookies.is_empty() {
                            return Ok(());
                        }

                        let mut mimes = HashMap::new();
                        for (cookie, atom) in atom_cookies {
                            let reply = cookie.reply()?;
                            let name = str::from_utf8(&reply.name)?.to_string();
                            mimes.insert(atom, name);
                        }

                        let mimes = filter_mimes(mimes);
                        if mimes.is_empty() {
                            return Ok(());
                        }

                        if let Some(&target_atom) = mimes.keys().next() {
                            conn.convert_selection(
                                self.transfer_window,
                                ev.selection,
                                target_atom,
                                transfer_atom,
                                x11rb::CURRENT_TIME,
                            )?
                            .check()?;
                        }

                        task.set_state(TaskState::PendingSelection {
                            mimes,
                            data: HashMap::new(),
                        });

                        Ok(())
                    }
                    TaskState::PendingSelection {
                        ref mut mimes,
                        ref mut data,
                    } => {
                        let Some(mime_name) = mimes.remove(&ev.target) else {
                            // Not a pending target
                            return Ok(());
                        };

                        let property = property.reply()?;
                        if property.type_ == atoms.INCR {
                            // TODO: Support INCR;
                            return Ok(());
                        }

                        // Dropping empty or blank selection
                        if !(property.value.is_empty()
                            || property.value.iter().all(u8::is_ascii_whitespace))
                        {
                            data.insert(mime_name.clone(), property.value);
                            self.mime_atoms.get_mut().insert(mime_name, ev.target);
                        }

                        if let Some(&atom) = mimes.keys().next() {
                            conn.convert_selection(
                                self.transfer_window,
                                ev.selection,
                                atom,
                                transfer_atom,
                                x11rb::CURRENT_TIME,
                            )?
                            .check()?;
                        } else {
                            let (_, task) = self.tasks.remove_entry(&transfer_atom).unwrap();
                            let TaskState::PendingSelection { data, .. } = task.state else {
                                unreachable!();
                            };

                            self.items.push_front(SelectionItem {
                                id: hash_selection_data(&data)?,
                                data,
                            });
                        }

                        Ok(())
                    }
                };
            }
            Event::XfixesSelectionNotify(ev) => {
                if ev.owner == self.paste_window {
                    return Ok(());
                }

                let transfer_atom = self.transfer_atoms.get()?;
                conn.convert_selection(
                    self.transfer_window,
                    ev.selection,
                    atoms.TARGETS,
                    transfer_atom,
                    x11rb::CURRENT_TIME,
                )?
                .check()?;

                self.tasks
                    .insert(transfer_atom, Task::new(TaskState::TargetsRequest));
            }

            // Handles paste requests
            Event::SelectionRequest(ev) => {
                let reply = |property| {
                    conn.send_event(
                        false,
                        ev.requestor,
                        EventMask::NO_EVENT,
                        SelectionNotifyEvent {
                            response_type: SELECTION_NOTIFY_EVENT,
                            sequence: ev.sequence,
                            time: ev.time,
                            requestor: ev.requestor,
                            selection: ev.selection,
                            target: ev.target,
                            property,
                        },
                    )?
                    .check()?;
                    Ok(())
                };

                let property = if ev.property == x11rb::NONE {
                    // Obsolete client
                    ev.target
                } else {
                    ev.property
                };
                if property == x11rb::NONE {
                    return reply(x11rb::NONE);
                }

                let reply = |reply_property| {
                    if reply_property == x11rb::NONE {
                        conn.delete_property(ev.requestor, property)?.check()?;
                    }
                    reply(reply_property)
                };

                if ![self.atoms.CLIPBOARD, self.atoms.PRIMARY].contains(&ev.selection) {
                    return reply(x11rb::NONE);
                }
                let Some(item_id) = self.paste_item_id else {
                    return reply(x11rb::NONE);
                };
                let Some(item) = &self.items.iter().find(|i| i.id == item_id) else {
                    return reply(x11rb::NONE);
                };

                let mut supported_atoms = Vec::new();
                supported_atoms.push(self.atoms.TARGETS);
                let mut requested_data: Option<&Vec<u8>> = None;
                for (atom_name, data) in &item.data {
                    let atom =
                        get_or_create_mime_atom(self.conn, self.mime_atoms.get_mut(), atom_name)?;
                    if atom != x11rb::NONE {
                        supported_atoms.push(atom);
                    }

                    if atom == ev.target {
                        requested_data = Some(data);
                    }
                }

                if !supported_atoms.contains(&ev.target) {
                    return reply(x11rb::NONE);
                }

                if ev.target == self.atoms.TARGETS {
                    conn.change_property32(
                        PropMode::REPLACE,
                        ev.requestor,
                        property,
                        AtomEnum::ATOM,
                        &supported_atoms,
                    )?
                    .check()?;
                    return reply(property);
                }

                conn.change_property8(
                    PropMode::REPLACE,
                    ev.requestor,
                    property,
                    ev.target,
                    requested_data.unwrap(),
                )?
                .check()?;
                reply(property)?;
            }
            _ => {}
        }

        self.purge_overdue_tasks();

        Ok(())
    }

    fn purge_overdue_tasks(&mut self) {
        let now = Instant::now();
        self.tasks
            .retain(|_, task| now.duration_since(task.last_update) < OVERDUE_TIMEOUT);
    }

    pub fn paste(&mut self, item_id: u64, cursor_original_pos: (i16, i16)) -> Result<()> {
        self.conn
            .set_selection_owner(self.paste_window, self.selection_atom, x11rb::CURRENT_TIME)?
            .check()?;

        let focused_window = self.conn.get_input_focus()?.reply()?.focus;
        if focused_window == self.paste_window {
            return Ok(());
        }

        let key = |type_, code| {
            self.conn
                .xtest_fake_input(type_, code, x11rb::CURRENT_TIME, self.screen.root, 1, 1, 0)
        };
        let move_cursor = |x, y| {
            self.conn.xtest_fake_input(
                MOTION_NOTIFY_EVENT,
                0,
                x11rb::CURRENT_TIME,
                self.screen.root,
                x,
                y,
                0,
            )
        };
        let keycode = |keysym| {
            self.key_converter
                .keysym_to_keycode(keysym)
                .map(|kc| kc.raw() as u8)
                .ok_or_else(|| {
                    anyhow!(
                        "invalid key provided: {}",
                        keysym.name().unwrap_or(&format!("<code {}>", keysym.raw()))
                    )
                })
        };

        if self.selection_atom == self.atoms.CLIPBOARD {
            let paste_bindings = &self.config.paste_bindings;
            let bindings = if let Some((instance_name, class_name)) =
                get_window_class(self.conn, focused_window)?
                && let Some(bindings) = paste_bindings
                    .get(&instance_name)
                    .or_else(|| paste_bindings.get(&class_name))
            {
                bindings
            } else {
                &vec![Binding {
                    key: 'v' as u32,
                    modifiers: vec![Modifier::Control],
                }]
            };

            for binding in bindings {
                for modifier in &binding.modifiers {
                    key(KEY_PRESS_EVENT, keycode((*modifier).into())?)?;
                }
                key(KEY_PRESS_EVENT, keycode(Keysym::new(binding.key))?)?;
                key(KEY_RELEASE_EVENT, keycode(Keysym::new(binding.key))?)?;
                for modifier in binding.modifiers.iter().rev() {
                    key(KEY_RELEASE_EVENT, keycode((*modifier).into())?)?;
                }
            }
        } else if self.selection_atom == self.atoms.PRIMARY {
            let cursor_current_pos = self.conn.query_pointer(self.screen.root)?.reply()?;
            move_cursor(cursor_original_pos.0, cursor_original_pos.1)?;

            // middle mouse button
            key(BUTTON_PRESS_EVENT, 2)?;
            key(BUTTON_RELEASE_EVENT, 2)?;

            move_cursor(cursor_current_pos.root_x, cursor_current_pos.root_y)?;
        }
        self.conn.flush()?;

        self.paste_item_id = Some(item_id);

        Ok(())
    }
}

fn get_or_create_mime_atom(
    conn: &XCBConnection,
    mime_atoms: &mut HashMap<String, Atom>,
    name: &str,
) -> Result<Atom> {
    if let Some(atom) = mime_atoms.get(name) {
        return Ok(*atom);
    }

    let atom = conn.intern_atom(false, name.as_bytes())?.reply()?.atom;
    mime_atoms.insert(name.to_string(), atom);
    Ok(atom)
}

fn filter_mimes(mimes: HashMap<Atom, String>) -> HashMap<Atom, String> {
    let mut filtered_mimes = HashMap::new();
    let mut plain: Option<(Atom, &str)> = None;
    let mut plain_score = 0;
    let mut image: Option<(Atom, &str)> = None;

    for (atom, mime) in mimes.iter() {
        if let Some(score) = plaintext_mime_score(mime) {
            if plain.is_none_or(|_| score > plain_score) {
                plain = Some((*atom, mime));
                plain_score = score;
            }
        } else if mime.starts_with("image/") {
            if image.is_none() {
                image = Some((*atom, mime));
            }
        } else if mime == "x-kde-passwordManagerHint" {
            filtered_mimes.drain();
            return filtered_mimes;
        } else {
            filtered_mimes.insert(*atom, mime.to_string());
        }
    }

    if let Some((atom, mime)) = plain {
        filtered_mimes.insert(atom, mime.to_string());
    }
    if let Some((atom, mime)) = image {
        filtered_mimes.insert(atom, mime.to_string());
    }

    filtered_mimes
}

fn get_window_class(conn: &XCBConnection, window: u32) -> Result<Option<(String, String)>> {
    let reply: GetPropertyReply = conn
        .get_property(
            false,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            0,
            u32::MAX,
        )?
        .reply()?;

    if reply.value_len == 0 {
        return Ok(None);
    }

    let value = String::from_utf8_lossy(&reply.value).into_owned();
    let mut parts = value.split('\0');

    let instance_name = parts.next().unwrap_or("").to_string();
    let class_name = parts.next().unwrap_or("").to_string();

    Ok(Some((instance_name, class_name)))
}

fn hash_selection_data(data: &HashMap<String, Vec<u8>>) -> Result<u64> {
    let data_bin = bincode::encode_to_vec(data, BINCODE_CONFIG)?;
    let hash = ahash::RandomState::with_seed(HASH_SEED).hash_one(&data_bin);

    Ok(hash)
}
