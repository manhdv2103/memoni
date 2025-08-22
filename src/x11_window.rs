extern crate x11rb;

use std::os::unix::ffi::OsStringExt as _;
use std::{cmp, ffi::OsString};

use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, *};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        WM_PROTOCOLS,
        WM_CLIENT_MACHINE,
        UTF8_STRING,
        _NET_CURRENT_DESKTOP,
        _NET_DESKTOP_VIEWPORT,
        _NET_WM_NAME,
        _NET_WM_PID,
        _NET_WM_STATE,
        _NET_WM_STATE_ABOVE,
    }
}

#[derive(Debug)]
struct Viewport {
    x: u32,
    y: u32,
}

pub struct X11Window {
    pub conn: XCBConnection,
    pub screen: Screen,
    pub screen_num: usize,
    pub atoms: Atoms,
    pub win_id: u32,
    pub width: u16,
    pub height: u16,
    pub background_color: u32,
}

impl X11Window {
    pub fn new(width: u16, height: u16, background_color: u32) -> Result<Self> {
        let (conn, screen_num) = XCBConnection::connect(None)?;
        let setup = conn.setup();
        let screen = setup.roots[screen_num].to_owned();
        let win_id = conn.generate_id()?;

        let atoms = Atoms::new(&conn)?.reply()?;

        let win_aux = CreateWindowAux::new()
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION,
            )
            .background_pixel(background_color)
            .win_gravity(Gravity::NORTH_WEST)
            .override_redirect(1);

        let (x, y) = calculate_window_pos(&conn, &screen, &atoms, width, height)?;

        conn.create_window(
            screen.root_depth,
            win_id,
            screen.root,
            x,
            y,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &win_aux,
        )?;

        conn.change_property8(
            PropMode::REPLACE,
            win_id,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"Memoni",
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            win_id,
            atoms._NET_WM_NAME,
            atoms.UTF8_STRING,
            b"Memoni",
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            win_id,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"memoni\0Memoni\0",
        )?;

        conn.change_property32(
            PropMode::REPLACE,
            win_id,
            atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            &[atoms._NET_WM_STATE_ABOVE],
        )?;
        conn.change_property32(
            PropMode::REPLACE,
            win_id,
            atoms._NET_WM_PID,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            win_id,
            atoms.WM_CLIENT_MACHINE,
            AtomEnum::STRING,
            get_hostname().to_string_lossy().as_bytes(),
        )?;
        conn.flush()?;

        Ok(X11Window {
            conn,
            screen,
            screen_num,
            atoms,
            win_id,
            width,
            height,
            background_color,
        })
    }

    pub fn map_window(&self) -> Result<()> {
        self.conn.map_window(self.win_id)?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn grab_input(&self) -> Result<()> {
        let grab_keyboard = self.conn.grab_keyboard(
            true,
            self.screen.root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )?;
        grab_keyboard.reply()?;

        let grab_pointer = self.conn.grab_pointer(
            true,
            self.screen.root,
            EventMask::BUTTON_RELEASE,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            self.screen.root,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )?;
        grab_pointer.reply()?;

        Ok(())
    }

    pub fn ungrab_input(&self) -> Result<()> {
        let ungrab_keyboard = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME)?;
        ungrab_keyboard.check()?;

        let ungrab_pointer = self.conn.ungrab_pointer(x11rb::CURRENT_TIME)?;
        ungrab_pointer.check()?;

        Ok(())
    }
}

fn calculate_window_pos(
    conn: &XCBConnection,
    screen: &Screen,
    atoms: &Atoms,
    width: u16,
    height: u16,
) -> Result<(i16, i16)> {
    let spacing = 10;
    let pointer = conn.query_pointer(screen.root)?.reply()?;
    let px = pointer.root_x as i32;
    let py = pointer.root_y as i32;

    let width = width as i32;
    let height = height as i32;

    let monitors = conn.randr_get_monitors(screen.root, true)?.reply()?;
    let pointer_monitor = monitors.monitors.iter().find(|m| {
        px >= m.x as i32
            && px < m.x as i32 + m.width as i32
            && py >= m.y as i32
            && py < m.y as i32 + m.height as i32
    });

    let desktop_viewport = get_current_desktop_viewport(conn, screen, atoms)?;
    let focused_monitor = desktop_viewport.and_then(|dv| {
        monitors.monitors.iter().find(|m| {
            (dv.x as i64) >= m.x as i64
                && (dv.x as i64) < m.x as i64 + m.width as i64
                && (dv.y as i64) >= m.y as i64
                && (dv.y as i64) < m.y as i64 + m.height as i64
        })
    });

    // pointer is in non-focused monitor, display the window in the middle of the focused monitor
    if let Some(fm) = focused_monitor
        && pointer_monitor
            .as_ref()
            .map(|pm| fm.name != pm.name)
            .unwrap_or(true)
    {
        let x = (fm.width as i32 - fm.x as i32 - width) / 2;
        let y = (fm.height as i32 - fm.y as i32 - height) / 2;
        return Ok((
            x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        ));
    }

    // place the window at the pointer position and try to keep it in the same monitor
    let (mx, my, mw, mh) = pointer_monitor
        .map(|pm| (pm.x as i32, pm.y as i32, pm.width as i32, pm.height as i32))
        .unwrap_or((0, 0, 0, 0));

    let place_right = px + width + spacing <= mx + mw - spacing;
    let x = if place_right {
        px + spacing
    } else {
        cmp::max(px - width - spacing, spacing)
    };

    let place_below = py + height + spacing <= my + mh - spacing;
    let y = if place_below {
        py + spacing
    } else {
        cmp::max(py - height - spacing, spacing)
    };

    Ok((
        x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
    ))
}

fn get_current_desktop_viewport(
    conn: &XCBConnection,
    screen: &Screen,
    atoms: &Atoms,
) -> Result<Option<Viewport>> {
    let reply = conn
        .get_property(
            false,
            screen.root,
            atoms._NET_CURRENT_DESKTOP,
            AtomEnum::CARDINAL,
            0,
            1,
        )?
        .reply()?;

    if reply.format == 32 && !reply.value.is_empty() {
        let current_desktop = u32::from_ne_bytes(reply.value[0..4].try_into()?) as usize;
        let mut desktop_viewports = get_desktop_viewports(conn, screen, atoms)?;
        if current_desktop < desktop_viewports.len() {
            return Ok(Some(desktop_viewports.swap_remove(current_desktop)));
        }
    }

    Ok(None)
}

fn get_desktop_viewports(
    conn: &XCBConnection,
    screen: &Screen,
    atoms: &Atoms,
) -> Result<Vec<Viewport>> {
    let reply = conn
        .get_property(
            false,
            screen.root,
            atoms._NET_DESKTOP_VIEWPORT,
            AtomEnum::CARDINAL,
            0,
            u32::MAX,
        )?
        .reply()?;

    if reply.format != 32 {
        return Ok(vec![]);
    }

    let mut values = Vec::new();
    for chunk in reply.value.chunks_exact(4) {
        values.push(u32::from_ne_bytes(chunk.try_into()?));
    }

    let viewports = values
        .chunks_exact(2)
        .map(|pair| Viewport {
            x: pair[0],
            y: pair[1],
        })
        .collect::<Vec<_>>();

    Ok(viewports)
}

fn get_hostname() -> OsString {
    OsString::from_vec(rustix::system::uname().nodename().to_bytes().to_vec())
}
