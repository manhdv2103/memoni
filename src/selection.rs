extern crate x11rb;

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, VecDeque},
    fmt, mem,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use bincode::{Decode, Encode};
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
    utils::{image_mime_score, is_image_mime, is_plaintext_mime, plaintext_mime_score},
    x11_key_converter::X11KeyConverter,
    x11_window::X11Window,
};

// Heavily modified from https://github.com/SUPERCILEX/clipboard-history/blob/master/x11/src/main.rs

const HASH_SEED: usize = 0xfd9aadcf54cc0f35;
const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();
const OVERDUE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_INCR_SIZE: usize = 10 * 1024 * 1024;
const INCR_CHUNK_SIZE: usize = 1024 * 1024 - 1;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        PRIMARY,
        CLIPBOARD,

        INCR,
        TIMESTAMP,
        TARGETS,
        SAVE_TARGETS,
        MULTIPLE,

        _NET_WM_NAME,
    }
}

type SelectionData = BTreeMap<String, Vec<u8>>;

#[derive(Debug, Encode, Decode)]
pub struct SelectionItem {
    pub id: u64,
    pub data: SelectionData,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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
enum RequestTaskState {
    TargetsRequest {
        owner: u32,
    },
    PendingSelection {
        owner: u32,
        mimes: HashMap<Atom, String>,
        data: SelectionData,
    },
    PendingIncr {
        owner: u32,
        mimes: HashMap<Atom, String>,
        data: SelectionData,
        current_mime_atom: Atom,
        current_mime_name: String,
        buffer: Vec<u8>,
    },
}

#[derive(Debug)]
enum IncrPasteTaskState {
    TransferingIncr {
        target: u32,
        item_id: u64,
        data_atom_name: String,
        offset: usize,
    },
}

#[derive(Debug)]
struct Task<S> {
    state: S,
    last_update: Instant,
}

impl<S> Task<S> {
    fn new(state: S) -> Self {
        Task {
            state,
            last_update: Instant::now(),
        }
    }

    fn set_state(&mut self, state: S) {
        self.state = state;
        self.last_update = Instant::now();
    }
}

pub struct Selection<'a> {
    pub items: VecDeque<SelectionItem>,

    conn: &'a XCBConnection,
    screen: &'a Screen,
    key_converter: Rc<RefCell<X11KeyConverter>>,
    config: &'a Config,
    merge_consecutive_similar_items: bool,
    selection_atom: Atom,
    atoms: Atoms,
    request_tasks: HashMap<Atom, Task<RequestTaskState>>,
    incr_paste_tasks: HashMap<(Window, Atom), Task<IncrPasteTaskState>>,
    transfer_window: Window,
    paste_window: Window,
    transfer_atoms: AtomPool<'a>,
    mime_atoms: RefCell<HashMap<String, Atom>>,
    paste_item_id: Option<u64>,
    prev_item_metadata: Option<(u32, Instant, bool)>,
}

impl<'a> Selection<'a> {
    pub fn new(
        initial_items: VecDeque<SelectionItem>,
        window: &'a X11Window,
        key_converter: Rc<RefCell<X11KeyConverter>>,
        selection_type: SelectionType,
        config: &'a Config,
        merge_consecutive_similar_items: bool,
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
            items: initial_items,
            conn,
            screen: &window.screen,
            key_converter,
            config,
            merge_consecutive_similar_items,
            selection_atom,
            atoms,
            request_tasks: HashMap::new(),
            incr_paste_tasks: HashMap::new(),
            transfer_window,
            paste_window: window.win_id,
            transfer_atoms: AtomPool::new(conn, "SELECTION_DATA")?,
            mime_atoms: RefCell::new(HashMap::new()),
            paste_item_id: None,
            prev_item_metadata: None,
        })
    }

    pub fn handle_event(
        &mut self,
        event: &Event,
    ) -> Result<Option<(Option<&SelectionItem>, Vec<SelectionItem>)>> {
        let conn = self.conn;
        let atoms = self.atoms;

        'blk: {
            match event {
                // Capture copied data
                Event::SelectionNotify(ev) => {
                    if ev.requestor != self.transfer_window {
                        break 'blk;
                    }

                    // Conversion request failed
                    let transfer_atom = ev.property;
                    if transfer_atom == x11rb::NONE {
                        break 'blk;
                    }

                    let Some(task) = self.request_tasks.get_mut(&transfer_atom) else {
                        break 'blk;
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

                    match task.state {
                        RequestTaskState::TargetsRequest { owner } => {
                            let property = property.reply()?;
                            if property.type_ == atoms.INCR {
                                // Ignoring abusive TARGETS property
                                break 'blk;
                            }

                            let Some(value) = property.value32() else {
                                // Invalid TARGETS property value format
                                break 'blk;
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
                                break 'blk;
                            }

                            let mut mimes = HashMap::new();
                            for (cookie, atom) in atom_cookies {
                                let reply = cookie.reply()?;
                                let name = str::from_utf8(&reply.name)?.to_string();
                                mimes.insert(atom, name);
                            }

                            let mimes = filter_mimes(mimes);
                            if mimes.is_empty() {
                                break 'blk;
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

                            task.set_state(RequestTaskState::PendingSelection {
                                owner,
                                mimes,
                                data: BTreeMap::new(),
                            });
                        }
                        RequestTaskState::PendingSelection {
                            owner,
                            ref mut mimes,
                            ref mut data,
                        } => {
                            let Some(mime_name) = mimes.remove(&ev.target) else {
                                // Not a pending target
                                break 'blk;
                            };

                            let property = property.reply()?;
                            if property.type_ == atoms.INCR {
                                let mimes = mem::take(mimes);
                                let data = mem::take(data);

                                task.set_state(RequestTaskState::PendingIncr {
                                    owner,
                                    mimes,
                                    data,
                                    current_mime_atom: ev.target,
                                    current_mime_name: mime_name,
                                    buffer: Vec::new(),
                                });

                                break 'blk;
                            }

                            return self.process_selection_data(
                                transfer_atom,
                                property.value,
                                mime_name,
                                ev.target,
                            );
                        }
                        RequestTaskState::PendingIncr { .. } => {
                            unreachable!("PendingIncr should only be handled in PropertyNotify");
                        }
                    };
                }
                Event::XfixesSelectionNotify(ev) => {
                    if ev.owner == self.paste_window {
                        break 'blk;
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

                    self.request_tasks.insert(
                        transfer_atom,
                        Task::new(RequestTaskState::TargetsRequest { owner: ev.owner }),
                    );
                }

                // Handle receiving INCR selection
                Event::PropertyNotify(ev)
                    if !self.incr_paste_tasks.contains_key(&(ev.window, ev.atom)) =>
                {
                    if ev.atom == self.atoms._NET_WM_NAME {
                        // Ignoring window name property change.
                        break 'blk;
                    }

                    if ev.state != Property::NEW_VALUE {
                        // Ignoring irrelevant property state change
                        break 'blk;
                    }

                    if ev.window != self.transfer_window {
                        break 'blk;
                    }

                    let transfer_atom = ev.atom;
                    let Some(task) = self.request_tasks.get_mut(&transfer_atom) else {
                        break 'blk;
                    };

                    let incr_task_state = if let RequestTaskState::PendingIncr {
                        owner,
                        mimes,
                        data,
                        current_mime_atom,
                        current_mime_name,
                        buffer,
                    } = &mut task.state
                    {
                        Some((
                            *owner,
                            mem::take(mimes),
                            mem::take(data),
                            mem::take(buffer),
                            mem::take(current_mime_name),
                            *current_mime_atom,
                        ))
                    } else {
                        None
                    };

                    if let Some((
                        owner,
                        mimes,
                        data,
                        mut buffer,
                        current_mime_name,
                        current_mime_atom,
                    )) = incr_task_state
                    {
                        let property = conn.get_property(
                            true,
                            ev.window,
                            ev.atom,
                            GetPropertyType::ANY,
                            0,
                            u32::MAX,
                        )?;
                        conn.flush()?;

                        let property = property.reply()?;

                        // Empty property signals completion
                        if property.value.is_empty() {
                            self.conn.delete_property(ev.window, transfer_atom)?;
                            task.state = RequestTaskState::PendingSelection { owner, mimes, data };

                            return self.process_selection_data(
                                transfer_atom,
                                buffer,
                                current_mime_name,
                                current_mime_atom,
                            );
                        }

                        if buffer.len() + property.value.len() > MAX_INCR_SIZE {
                            eprintln!("Warning: INCR transfer exceeds size limit, aborting");
                            self.request_tasks.remove(&transfer_atom);
                            break 'blk;
                        }

                        buffer.extend_from_slice(&property.value);
                        task.state = RequestTaskState::PendingIncr {
                            owner,
                            mimes,
                            data,
                            buffer,
                            current_mime_name,
                            current_mime_atom,
                        };
                    }
                }

                // Handle paste request
                Event::SelectionRequest(ev) => {
                    let reply = |property| -> Result<()> {
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
                        break 'blk reply(x11rb::NONE)?;
                    }

                    let reply = |reply_property| {
                        if reply_property == x11rb::NONE {
                            conn.delete_property(ev.requestor, property)?.check()?;
                        }
                        reply(reply_property)
                    };

                    if ![self.atoms.CLIPBOARD, self.atoms.PRIMARY].contains(&ev.selection) {
                        break 'blk reply(x11rb::NONE)?;
                    }
                    let Some(item_id) = self.paste_item_id else {
                        break 'blk reply(x11rb::NONE)?;
                    };
                    let Some(item) = &self.items.iter().find(|i| i.id == item_id) else {
                        break 'blk reply(x11rb::NONE)?;
                    };

                    let mut supported_atoms = Vec::new();
                    supported_atoms.push(self.atoms.TARGETS);
                    let mut requested_data = None;
                    for (atom_name, data) in &item.data {
                        let atom = get_or_create_mime_atom(
                            self.conn,
                            self.mime_atoms.get_mut(),
                            atom_name,
                        )?;
                        if atom != x11rb::NONE {
                            supported_atoms.push(atom);
                        }

                        if atom == ev.target {
                            requested_data = Some((data, atom_name));
                        }
                    }

                    if !supported_atoms.contains(&ev.target) {
                        break 'blk reply(x11rb::NONE)?;
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
                        break 'blk reply(property)?;
                    }

                    let (data, atom_name) = requested_data.unwrap();

                    if data.len() > INCR_CHUNK_SIZE {
                        conn.change_window_attributes(
                            ev.requestor,
                            &ChangeWindowAttributesAux::new()
                                .event_mask(EventMask::PROPERTY_CHANGE),
                        )?;
                        conn.change_property32(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            atoms.INCR,
                            &[u32::try_from(data.len()).unwrap_or(u32::MAX)],
                        )?
                        .check()?;

                        self.incr_paste_tasks.insert(
                            (ev.requestor, property),
                            Task::new(IncrPasteTaskState::TransferingIncr {
                                target: ev.target,
                                item_id,
                                data_atom_name: atom_name.to_string(),
                                offset: 0,
                            }),
                        );

                        break 'blk reply(property)?;
                    }

                    conn.change_property8(
                        PropMode::REPLACE,
                        ev.requestor,
                        property,
                        ev.target,
                        data,
                    )?
                    .check()?;
                    reply(property)?;
                }

                // Handle INCR paste request
                Event::PropertyNotify(ev) => {
                    if ev.state != Property::DELETE {
                        // Ignoring irrelevant property state change
                        break 'blk;
                    }

                    let Some(task) = self.incr_paste_tasks.get_mut(&(ev.window, ev.atom)) else {
                        break 'blk;
                    };

                    match task.state {
                        IncrPasteTaskState::TransferingIncr {
                            target,
                            item_id,
                            ref data_atom_name,
                            ref mut offset,
                        } => {
                            let end_transfering =
                                |incr_paste_tasks: &mut HashMap<_, _>| -> Result<()> {
                                    incr_paste_tasks.remove(&(ev.window, ev.atom));
                                    conn.change_window_attributes(
                                        ev.window,
                                        &ChangeWindowAttributesAux::new()
                                            .event_mask(EventMask::NO_EVENT),
                                    )?;
                                    Ok(())
                                };

                            if let Some(data) = &self
                                .items
                                .iter()
                                .find(|i| i.id == item_id)
                                .and_then(|item| item.data.get(data_atom_name))
                            {
                                let end = offset.saturating_add(INCR_CHUNK_SIZE).min(data.len());
                                let chunk = &data[*offset..end];

                                if *offset == end {
                                    end_transfering(&mut self.incr_paste_tasks)?;
                                } else {
                                    *offset = end;
                                }

                                conn.change_property8(
                                    PropMode::REPLACE,
                                    ev.window,
                                    ev.atom,
                                    target,
                                    chunk,
                                )?
                                .check()?;
                            } else {
                                // The item has disappeared somehow, stops transfering
                                end_transfering(&mut self.incr_paste_tasks)?;
                                conn.change_property8(
                                    PropMode::REPLACE,
                                    ev.window,
                                    ev.atom,
                                    target,
                                    &[],
                                )?
                                .check()?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        self.purge_overdue_tasks();

        Ok(None)
    }

    fn process_selection_data(
        &mut self,
        transfer_atom: Atom,
        value: Vec<u8>,
        mime_name: String,
        mime_atom: Atom,
    ) -> Result<Option<(Option<&SelectionItem>, Vec<SelectionItem>)>> {
        let mut task = self.request_tasks.remove(&transfer_atom).unwrap();

        let RequestTaskState::PendingSelection {
            owner,
            ref mut data,
            ref mimes,
        } = task.state
        else {
            panic!(
                "Expected task.state of transfer_atom {transfer_atom} to be PendingSelection, but was {:?}",
                task.state
            );
        };

        // Dropping empty selection
        if !value.is_empty() {
            data.insert(mime_name.clone(), value);
            self.mime_atoms
                .borrow_mut()
                .insert(mime_name.clone(), mime_atom);
        }

        if let Some(&next_atom) = mimes.keys().next() {
            self.request_tasks.insert(transfer_atom, task);
            self.conn
                .convert_selection(
                    self.transfer_window,
                    self.selection_atom,
                    next_atom,
                    transfer_atom,
                    x11rb::CURRENT_TIME,
                )?
                .check()?;
            return Ok(None);
        }

        self.transfer_atoms.release(transfer_atom);

        // Ignoring selection with empty data
        if data.is_empty() {
            return Ok(None);
        }

        let prev_item = self.items.front();
        let new_item_id = hash_selection_data(data)?;
        let mut removed = Vec::new();

        // We only support merge plaintext items without any other type of data
        if self.merge_consecutive_similar_items
            && let Some((prev_owner, prev_time, is_previously_seen)) = self.prev_item_metadata
            && prev_owner == owner
            && prev_time.elapsed() < Duration::from_secs(1)
            // ---
            // If the item has existed before, we should not merge it
            && !is_previously_seen
            // ---
            && data.len() == 1
            && let (mime, new_text) = data.iter().next().unwrap()
            && is_plaintext_mime(mime)
            // ---
            && let Some(prev_item) = prev_item
            && prev_item.data.len() == 1
            && let Some(prev_text) = prev_item.data.get(mime)
            // ---
            && (contains(new_text, prev_text) || contains(prev_text, new_text))
        {
            removed.push(self.items.pop_front().unwrap());
        }

        let mut is_previously_seen = false;
        let mut new_item = None;
        if let Some(idx) = self.items.iter().position(|i| i.id == new_item_id) {
            let previous_seen_item = self.items.remove(idx).unwrap();
            self.items.push_front(previous_seen_item);

            is_previously_seen = true;
        } else {
            self.items.push_front(SelectionItem {
                id: new_item_id,
                data: mem::take(data),
            });

            if self.items.len() > self.config.item_limit {
                removed.extend(self.items.split_off(self.config.item_limit));
            };

            new_item = self.items.front();
        }

        self.prev_item_metadata = Some((owner, Instant::now(), is_previously_seen));
        Ok(Some((new_item, removed)))
    }

    fn purge_overdue_tasks(&mut self) {
        let now = Instant::now();

        let (kept, removed): (HashMap<_, _>, HashMap<_, _>) = self
            .request_tasks
            .drain()
            .partition(|(_, task)| now.duration_since(task.last_update) < OVERDUE_TIMEOUT);
        self.request_tasks = kept;

        for transfer_atom in removed.keys() {
            self.transfer_atoms.release(*transfer_atom);
        }

        self.incr_paste_tasks
            .retain(|_, task| now.duration_since(task.last_update) < OVERDUE_TIMEOUT);
    }

    pub fn paste(&mut self, item_id: u64, pointer_original_pos: (i16, i16)) -> Result<()> {
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
        let move_pointer = |x, y| {
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
                .borrow()
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
            let pointer_current_pos = self.conn.query_pointer(self.screen.root)?.reply()?;
            move_pointer(pointer_original_pos.0, pointer_original_pos.1)?;

            // middle mouse button
            key(BUTTON_PRESS_EVENT, 2)?;
            key(BUTTON_RELEASE_EVENT, 2)?;

            move_pointer(pointer_current_pos.root_x, pointer_current_pos.root_y)?;
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
    let mut image_score = 0;

    for (atom, mime) in mimes.iter() {
        if let Some(score) = plaintext_mime_score(mime) {
            if plain.is_none_or(|_| score > plain_score) {
                plain = Some((*atom, mime));
                plain_score = score;
            }
        } else if is_image_mime(mime) {
            let score = image_mime_score(mime);
            if image.is_none_or(|_| score > image_score) {
                image = Some((*atom, mime));
                image_score = score;
            }
        } else if mime == "x-kde-passwordManagerHint" {
            filtered_mimes.drain();
            return filtered_mimes;

        // Weird mimes prevent copying image on firefox
        } else if mime == "text/ico" || mime == "application/ico" {
            continue;
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

fn get_window_class(conn: &XCBConnection, window: Window) -> Result<Option<(String, String)>> {
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

fn hash_selection_data(data: &SelectionData) -> Result<u64> {
    let data_bin = bincode::encode_to_vec(data, BINCODE_CONFIG)?;
    let hash = ahash::RandomState::with_seed(HASH_SEED).hash_one(&data_bin);

    Ok(hash)
}

// Dumb algorithm here is fine I guess
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
