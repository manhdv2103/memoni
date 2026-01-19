use std::collections::VecDeque;

use anyhow::Result;
use log::{trace, warn};
use x11rb::{
    connection::Connection,
    protocol::xproto::{
        Atom, ConnectionExt, CreateWindowAux, EventMask, PropMode, Window, WindowClass,
    },
    wrapper::ConnectionExt as _,
    xcb_ffi::XCBConnection,
};

use crate::selection::SelectionType;

const WIN_TITLE_PREFIX: &str = "Memoni transfer window";
const ATOM_PREFIX: &str = "TRANSFER_SELECTION_DATA";

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

#[derive(Debug)]
pub struct TransferWindow {
    pub id: Window,
    pub atom: Atom,
}

pub struct TransferWindowPool<'a> {
    conn: &'a XCBConnection,
    parent_win: u32,
    selection_type: SelectionType,
    atoms: Atoms,
    windows: VecDeque<TransferWindow>,
    counter: u8,
    get_count: usize,
    release_count: usize,
}

impl<'a> TransferWindowPool<'a> {
    pub fn new(
        conn: &'a XCBConnection,
        parent_win: u32,
        selection_type: SelectionType,
    ) -> Result<Self> {
        let atoms = Atoms::new(conn)?.reply()?;
        let mut window_pool = TransferWindowPool {
            conn,
            parent_win,
            selection_type,
            atoms,
            windows: VecDeque::new(),
            counter: 0,
            get_count: 0,
            release_count: 0,
        };

        let initial_windows = vec![
            window_pool.create_window()?,
            window_pool.create_window()?,
            window_pool.create_window()?,
            window_pool.create_window()?,
        ];
        window_pool.windows.extend(initial_windows);

        Ok(window_pool)
    }

    pub fn get(&mut self) -> Result<TransferWindow> {
        let in_use = self.get_count - self.release_count;
        if in_use > 100 {
            warn!("transfer window pool might be leaking, in use items: {in_use}");
        }

        self.get_count += 1;
        let window = match self.windows.pop_front() {
            Some(w) => Ok(w),
            None => self.create_window(),
        };
        trace!("getting transfer window: {window:?}");

        window
    }

    pub fn release(&mut self, window: TransferWindow) {
        trace!("releasing transfer window: {window:?}");
        self.release_count += 1;
        self.windows.push_back(window);
    }

    fn create_window(&mut self) -> Result<TransferWindow> {
        let counter_str = self.counter.to_string();

        let transfer_window = self.create_util_window(
            &format!("{WIN_TITLE_PREFIX} {counter_str} - {}", self.selection_type).into_bytes(),
            &CreateWindowAux::default().event_mask(EventMask::PROPERTY_CHANGE),
            WindowClass::INPUT_ONLY,
        )?;
        let atom = self
            .conn
            .intern_atom(
                false,
                &format!("{ATOM_PREFIX}_{}_{counter_str}", self.selection_type).into_bytes(),
            )?
            .reply()?
            .atom;

        self.counter += 1;

        Ok(TransferWindow {
            id: transfer_window,
            atom,
        })
    }

    fn create_util_window(
        &self,
        title: &[u8],
        aux: &CreateWindowAux,
        kind: WindowClass,
    ) -> Result<Window> {
        let win_id = self.conn.generate_id()?;
        self.conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win_id,
            self.parent_win,
            0,
            0,
            1,
            1,
            0,
            kind,
            x11rb::COPY_FROM_PARENT,
            aux,
        )?;
        self.conn.change_property8(
            PropMode::REPLACE,
            win_id,
            self.atoms._NET_WM_NAME,
            self.atoms.UTF8_STRING,
            title,
        )?;

        Ok(win_id)
    }
}
