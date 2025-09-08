extern crate x11rb;

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use x11rb::connection::RequestConnection as _;
use x11rb::protocol::{
    Event,
    xfixes::SelectionEventMask,
    xproto::{ConnectionExt as _, *},
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;
use x11rb::{connection::Connection, protocol::xfixes};

use crate::{atom_pool::AtomPool, utils::is_plaintext_mime, x11_window::X11Window};

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

#[derive(Debug, Clone)]
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
    atoms: Atoms,
    tasks: HashMap<Atom, Task>,
    transfer_window: u32,
    transfer_atoms: AtomPool<'a>,
}

impl<'a> Selection<'a> {
    pub fn new(window: &'a X11Window, selection_type: SelectionType) -> Result<Self> {
        let conn = &window.conn;
        let root = window.screen.root;

        conn.prefetch_extension_information(xfixes::X11_EXTENSION_NAME)?;
        conn.extension_information(xfixes::X11_EXTENSION_NAME)?
            .context("No XFixes found")?;
        xfixes::query_version(conn, 5, 0)?.reply()?;

        let atoms = Atoms::new(conn)?.reply()?;

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
            CreateWindowAux::default(),
            WindowClass::INPUT_OUTPUT,
        )?;

        xfixes::select_selection_input(
            conn,
            root,
            match selection_type {
                SelectionType::PRIMARY => atoms.PRIMARY,
                SelectionType::CLIPBOARD => atoms.CLIPBOARD,
            },
            SelectionEventMask::SET_SELECTION_OWNER,
        )?;

        Ok(Selection {
            items: VecDeque::new(),
            conn,
            atoms,
            tasks: HashMap::new(),
            transfer_window,
            transfer_atoms: AtomPool::new(conn, "SELECTION_DATA")?,
        })
    }

    pub fn handle_event(&mut self, event: &Event) -> Result<()> {
        let conn = self.conn;
        let atoms = self.atoms;

        match event {
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
                        let Some(mime) = mimes.remove(&ev.target) else {
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
                            data.insert(mime, property.value);
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
}

fn filter_mimes(mimes: HashMap<Atom, String>) -> HashMap<Atom, String> {
    let mut filtered_mimes = HashMap::new();
    let mut plain: Option<(Atom, &str)> = None;
    let mut image: Option<(Atom, &str)> = None;

    for (atom, mime) in mimes.iter() {
        if is_plaintext_mime(mime) {
            if plain.is_none_or(|(_, p)| p.contains(';') && !mime.contains(';')) {
                plain = Some((*atom, mime));
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

fn hash_selection_data(data: &HashMap<String, Vec<u8>>) -> Result<u64> {
    let data_bin = bincode::encode_to_vec(data, BINCODE_CONFIG)?;
    let hash = ahash::RandomState::with_seed(HASH_SEED).hash_one(&data_bin);

    Ok(hash)
}
