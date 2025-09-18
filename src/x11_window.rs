extern crate x11rb;

use std::cell::Cell;
use std::os::unix::ffi::OsStringExt as _;
use std::{cmp, ffi::OsString};
use std::{thread, time};

use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, *};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;

use crate::config::{Config, Dimensions, LayoutConfig};

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

pub struct X11Window<'a> {
    pub conn: XCBConnection,
    pub screen: Screen,
    pub screen_num: usize,
    pub atoms: Atoms,
    pub win_id: u32,
    pub dimensions: Dimensions,
    pub win_opened_pointer_pos: Cell<(i16, i16)>,
    pub always_follows_pointer: bool,
    config: &'a Config,
    win_event_mask: EventMask,
    win_pos: Cell<(i16, i16)>,
}

impl<'a> X11Window<'a> {
    pub fn new(config: &'a Config, always_follows_pointer: bool) -> Result<Self> {
        let (conn, screen_num) = XCBConnection::connect(None)?;
        let setup = conn.setup();
        let screen = setup.roots[screen_num].to_owned();
        let win_id = conn.generate_id()?;

        let atoms = Atoms::new(&conn)?.reply()?;

        let win_event_mask = EventMask::EXPOSURE
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION;
        let win_aux = CreateWindowAux::new()
            .event_mask(win_event_mask)
            .background_pixel(*config.theme.background)
            .win_gravity(Gravity::NORTH_WEST)
            .override_redirect(1);

        conn.create_window(
            screen.root_depth,
            win_id,
            screen.root,
            0,
            0,
            config.layout.window_dimensions.width,
            config.layout.window_dimensions.height,
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
            dimensions: config.layout.window_dimensions,
            config,
            win_event_mask,
            always_follows_pointer,
            win_pos: Cell::new((0, 0)),
            win_opened_pointer_pos: Cell::new((0, 0)),
        })
    }

    pub fn show_window(&self) -> Result<()> {
        let pointer = self.conn.query_pointer(self.screen.root)?.reply()?;
        self.win_opened_pointer_pos
            .set((pointer.root_x, pointer.root_y));

        let (x, y) = self.calculate_window_pos()?;
        self.conn.configure_window(
            self.win_id,
            &ConfigureWindowAux::new().x(x as i32).y(y as i32),
        )?;
        self.win_pos.set((x, y));

        self.conn.map_window(self.win_id)?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn hide_window(&self) -> Result<()> {
        self.conn.unmap_window(self.win_id)?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn grab_input(&self) -> Result<()> {
        let mut grab_keyboard_success = false;
        // Have to repeatedly retry because if memoni is triggered from a window manager (e.g. i3)
        // keymap, the WM is probably still grabbing the keyboard and not ungrabbing immediately
        for _ in 0..100 {
            let grab_keyboard = self.conn.grab_keyboard(
                true,
                self.screen.root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?;
            if grab_keyboard.reply()?.status == GrabStatus::SUCCESS {
                grab_keyboard_success = true;
                break;
            }
            thread::sleep(time::Duration::from_millis(10));
        }
        if !grab_keyboard_success {
            eprintln!("Warning: failed to grab keyboard");
        }

        let grab_pointer = self.conn.grab_pointer(
            true,
            self.screen.root,
            EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION | EventMask::POINTER_MOTION,
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

    pub fn enable_events(&self) -> Result<()> {
        self.conn.change_window_attributes(
            self.win_id,
            &ChangeWindowAttributesAux::new().event_mask(self.win_event_mask),
        )?;

        Ok(())
    }

    pub fn disable_events(&self) -> Result<()> {
        self.conn.change_window_attributes(
            self.win_id,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
        )?;

        Ok(())
    }

    pub fn get_current_win_pos(&self) -> (i16, i16) {
        self.win_pos.get()
    }

    pub fn get_win_opened_pointer_pos(&self) -> (i16, i16) {
        self.win_opened_pointer_pos.get()
    }

    fn calculate_window_pos(&self) -> Result<(i16, i16)> {
        let X11Window {
            conn,
            screen,
            atoms,
            win_opened_pointer_pos,
            always_follows_pointer,
            config,
            ..
        } = self;
        let LayoutConfig {
            window_dimensions: Dimensions { width, height },
            pointer_gap: spacing,
            ..
        } = config.layout;
        let pointer_pos = win_opened_pointer_pos.get();

        let px = pointer_pos.0 as i32;
        let py = pointer_pos.1 as i32;

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
        if !always_follows_pointer
            && let Some(fm) = focused_monitor
            && pointer_monitor
                .as_ref()
                .map(|pm| fm.name != pm.name)
                .unwrap_or(true)
        {
            let x = (fm.width as i32 - width) / 2 + fm.x as i32;
            let y = (fm.height as i32 - height) / 2 + fm.y as i32;
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
