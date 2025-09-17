use anyhow::Result;
use x11rb::{
    connection::Connection as _,
    protocol::xproto::{ConnectionExt as _, GetKeyboardMappingReply},
    xcb_ffi::XCBConnection,
};
use xkeysym::{KeyCode, Keysym, keysym as xkeysym_keycode_to_keysym};

pub struct X11KeyConverter {
    min_keycode: u8,
    mapping: GetKeyboardMappingReply,
}

impl X11KeyConverter {
    pub fn new(conn: &XCBConnection) -> Result<Self> {
        let setup = conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;
        let mapping_reply = conn
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;

        Ok(Self {
            min_keycode,
            mapping: mapping_reply,
        })
    }

    pub fn keycode_to_keysym(&self, keycode: KeyCode) -> Option<Keysym> {
        xkeysym_keycode_to_keysym(
            keycode,
            0,
            self.min_keycode.into(),
            self.mapping.keysyms_per_keycode,
            &self.mapping.keysyms,
        )
    }

    pub fn keysym_to_keycode(&self, keysym: Keysym) -> Option<KeyCode> {
        let Self {
            min_keycode,
            mapping,
        } = self;

        for (i, keysyms) in mapping
            .keysyms
            .chunks(mapping.keysyms_per_keycode as usize)
            .enumerate()
        {
            for &ks in keysyms {
                if ks == keysym.into() {
                    let keycode = min_keycode + i as u8;
                    return Some(keycode.into());
                }
            }
        }

        None
    }
}
