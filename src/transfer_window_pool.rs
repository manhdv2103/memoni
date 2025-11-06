use std::collections::VecDeque;

use anyhow::Result;
use x11rb::{
    connection::Connection,
    protocol::xproto::{
        Atom, ConnectionExt, CreateWindowAux, EventMask, PropMode, Window, WindowClass,
    },
    wrapper::ConnectionExt as _,
    xcb_ffi::XCBConnection,
};

const WIN_TITLE_PREFIX: &str = "Memoni transfer window ";
const ATOM_PREFIX: &str = "TRANSFER_SELECTION_DATA_";

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
    atoms: Atoms,
    windows: VecDeque<TransferWindow>,
    counter: u8,
}

impl<'a> TransferWindowPool<'a> {
    pub fn new(conn: &'a XCBConnection, parent_win: u32) -> Result<Self> {
        let atoms = Atoms::new(conn)?.reply()?;
        let mut window_pool = TransferWindowPool {
            conn,
            parent_win,
            atoms,
            windows: VecDeque::new(),
            counter: 0,
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
        match self.windows.pop_front() {
            Some(w) => Ok(w),
            None => self.create_window(),
        }
    }

    pub fn release(&mut self, window: TransferWindow) {
        self.windows.push_back(window);
    }

    fn create_window(&mut self) -> Result<TransferWindow> {
        let counter_str = self.counter.to_string();

        let transfer_window = self.create_util_window(
            &format!("{WIN_TITLE_PREFIX}{counter_str}").into_bytes(),
            &CreateWindowAux::default().event_mask(EventMask::PROPERTY_CHANGE),
            WindowClass::INPUT_ONLY,
        )?;
        let atom = self
            .conn
            .intern_atom(false, &format!("{ATOM_PREFIX}{counter_str}").into_bytes())?
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
