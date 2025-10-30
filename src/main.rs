use anyhow::{Result, bail};
use memoni::config::Config;
use memoni::input::Input;
use memoni::persistence::Persistence;
use memoni::selection::Selection;
use memoni::ui::{Ui, UiFlow};
use memoni::x11_key_converter::X11KeyConverter;
use memoni::x11_window::X11Window;
use memoni::{opengl_context::OpenGLContext, selection::SelectionType};
use mio::unix::SourceFd;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook_mio::v1_0::Signals;
use std::cell::RefCell;
use std::mem;
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;
use std::{
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    os::fd::{AsFd as _, AsRawFd as _},
    path::Path,
    time::Duration,
};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::Mapping;
use x11rb::xcb_ffi::XCBConnection;

const SOCKET_DIR: &str = "/tmp/memoni/";
const X11_TOKEN: mio::Token = mio::Token(0);
const SIGNAL_TOKEN: mio::Token = mio::Token(1);
const MEMONI_TOKEN: mio::Token = mio::Token(2);

enum Args {
    Client(ClientArgs),
    Server(ServerArgs),
}

struct ClientArgs {
    selection: SelectionType,
}

struct ServerArgs {
    selection: SelectionType,
}

fn main() -> Result<()> {
    let args = parse_args()?;

    let socket_dir = Path::new(SOCKET_DIR);
    fs::create_dir_all(socket_dir)?;

    match args {
        Args::Client(args) => client(args, socket_dir)?,
        Args::Server(args) => server(args, socket_dir)?,
    }

    Ok(())
}

fn parse_args() -> Result<Args> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_env();
    let is_server_mode = parser.try_raw_args().is_some_and(|mut raw_args| {
        let is_server_mode = raw_args.peek().is_some_and(|a| a.eq(OsStr::new("server")));
        if is_server_mode {
            raw_args.next();
        }
        is_server_mode
    });

    let mut selection_type = SelectionType::CLIPBOARD;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('s') | Long("selection") => {
                let selection_str: String = parser.value()?.parse()?;
                selection_type = match selection_str.as_str() {
                    "PRIMARY" => SelectionType::PRIMARY,
                    "CLIPBOARD" => SelectionType::CLIPBOARD,
                    _ => bail!("invalid selection type \"{selection_str}\""),
                };
            }
            Short('h') | Long("help") => {
                if is_server_mode {
                    println!(
                        "\
Start memoni server.

USAGE:
  memoni server [OPTIONS]

OPTIONS:
  -s, --selection TYPE    Sets selection type [possible values: CLIPBOARD, PRIMARY] [default: CLIPBOARD]
  -h, --help              Prints help information"
                    );
                } else {
                    println!(
                        "\
Show memoni window if memoni server is running.
To run in server mode, use: memoni server [OPTIONS]

USAGE:
  memoni [OPTIONS]

OPTIONS:
  -s, --selection TYPE    Sets selection type [possible values: CLIPBOARD, PRIMARY] [default: CLIPBOARD]
  -h, --help              Prints help information"
                    );
                }

                std::process::exit(0);
            }
            _ => return Err(arg.unexpected().into()),
        }
    }

    Ok(if is_server_mode {
        Args::Server(ServerArgs {
            selection: selection_type,
        })
    } else {
        Args::Client(ClientArgs {
            selection: selection_type,
        })
    })
}

fn client(args: ClientArgs, socket_dir: &Path) -> Result<()> {
    let socket_path = socket_dir.join(format!("{}.sock", args.selection));
    if !fs::exists(&socket_path)? {
        eprintln!(
            "Error: memoni server for selection '{}' is not running",
            args.selection
        );
        std::process::exit(1);
    }

    let mut stream = UnixStream::connect(&socket_path)?;
    stream.write_all(b"show_win")?;

    Ok(())
}

fn server(args: ServerArgs, socket_dir: &Path) -> Result<()> {
    let socket_path = socket_dir.join(format!("{}.sock", args.selection));
    let config = Config::load(args.selection)?;

    let window = X11Window::new(
        &config,
        args.selection,
        args.selection == SelectionType::PRIMARY,
    )?;
    let mut gl_context = unsafe { OpenGLContext::new(&window, &config)? };
    let key_converter = Rc::new(RefCell::new(X11KeyConverter::new(&window.conn)?));
    let mut input = Input::new(&window, key_converter.clone())?;
    let persistence = Persistence::new(args.selection)?;
    let mut selection = Selection::new(
        persistence.load_selection_items()?,
        &window,
        key_converter.clone(),
        args.selection,
        &config,
        // XFixes sends a SelectionNotify for each change while the user drags the mouse to adjust selection.
        // Debounce to merge consecutive items with similar text.
        args.selection == SelectionType::PRIMARY,
    )?;

    let mut ui = Ui::new(&config)?;
    for item in &selection.items {
        ui.build_button_widget(item)?;
    }

    let mut signals = Signals::new(TERM_SIGNALS)?;
    let (mut poll, socket_listener) = match create_poll(&window.conn, &socket_path, &mut signals) {
        Ok(res) => res,
        Err(err) => {
            if let Some(io_err) = err.downcast_ref::<io::Error>()
                && io_err.kind() == io::ErrorKind::AddrInUse
            {
                eprintln!(
                    "Error: another server for selection '{}' is already running",
                    args.selection
                );
                std::process::exit(1);
            } else {
                return Err(err);
            }
        }
    };
    let mut poll_events = mio::Events::with_capacity(8);

    let main_loop_result = (|| -> Result<()> {
        let mut window_shown = false;
        let mut pointer_button_press_count = 0;
        'main_loop: loop {
            let mut will_show_window = false;
            let mut will_hide_window = false;
            let mut paste_item_id = None;

            // non-blocking when window is visible, blocking otherwise
            let poll_timeout = if window_shown {
                Some(Duration::ZERO)
            } else {
                None
            };
            poll.poll(&mut poll_events, poll_timeout).or_else(|e| {
                if e.kind() == io::ErrorKind::Interrupted {
                    // We get interrupt when a signal happens inside poll. That's non-fatal, just
                    // retry.
                    poll_events.clear();
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
            for event in &poll_events {
                match event.token() {
                    X11_TOKEN => {} // handled below
                    SIGNAL_TOKEN => break 'main_loop,
                    MEMONI_TOKEN => {
                        let (mut stream, _) = socket_listener.accept()?;
                        let mut buf = [0u8; 1024];
                        match stream.read(&mut buf) {
                            Ok(0) => {
                                // client closed immediately
                            }
                            Ok(n) => {
                                let command = String::from_utf8_lossy(&buf[..n]);
                                match command.as_ref() {
                                    "show_win" => {
                                        will_show_window = true;
                                    }
                                    _ => eprintln!("Warning: unknown client command: {command}"),
                                }
                            }
                            Err(e) => eprintln!("Warning: failed to read client command: {e}"),
                        }
                    }
                    _ => unreachable!(),
                }
            }

            while let Some(event) = window.conn.poll_for_event()? {
                if let Event::Error(err) = event {
                    eprintln!("Warning: X11 error {err:?}");
                    continue;
                }

                if let Event::DestroyNotify(_) = event {
                    will_hide_window = true;
                    continue;
                }

                if let Event::MappingNotify(ev) = event
                    && (ev.request == Mapping::KEYBOARD || ev.request == Mapping::MODIFIER)
                {
                    let _ = mem::replace(
                        &mut *key_converter.borrow_mut(),
                        X11KeyConverter::new(&window.conn)?,
                    );
                }

                if let Event::ButtonPress(_) = event {
                    pointer_button_press_count += 1;
                }

                // When clicking outside of the window, only a release event is sent
                if let Event::ButtonRelease(ev) = event {
                    if pointer_button_press_count == 0 {
                        if ev.event != window.win_id {
                            will_hide_window = true;
                        }
                    } else {
                        pointer_button_press_count -= 1;
                    }
                }

                input.handle_event(&event);
                if let Some((new_selection_item, removed_selection_items)) =
                    selection.handle_event(&event)?
                {
                    ui.remove_button_widgets(removed_selection_items);
                    if let Some(new_item) = new_selection_item {
                        ui.build_button_widget(new_item)?;
                    }

                    persistence.save_selection_items(&selection.items)?;
                }

                for input_event in &input.egui_input.events {
                    if let egui::Event::Key { key, .. } = input_event
                        && *key == egui::Key::Escape
                    {
                        will_hide_window = true;
                    }
                }
            }

            if will_show_window {
                window.update_window_pos()?;
                input.update_pointer_pos()?;
                ui.reset();
            }

            if window_shown || will_show_window {
                let ui_flow = if window.is_win_placed_above_pointer() {
                    UiFlow::BottomToTop
                } else {
                    UiFlow::TopToBottom
                };
                let full_output = ui.run(
                    input.egui_input.take(),
                    &selection.items,
                    ui_flow,
                    |selected| {
                        will_hide_window = true;
                        paste_item_id = Some(selected.id);
                    },
                )?;
                gl_context.render(&ui.egui_ctx, full_output)?;
            }

            if will_show_window {
                window.show_window()?;
                window.grab_input()?;
                window.enable_events()?;
                window_shown = true;
            }

            if will_hide_window {
                window.hide_window()?;
                window.ungrab_input()?;
                window.disable_events()?;
                window_shown = false;
            }

            if let Some(id) = paste_item_id {
                selection.paste(id, window.win_opened_pointer_pos.get())?;
            }
        }
        Ok(())
    })();

    window.ungrab_input()?;
    fs::remove_file(&socket_path)?;

    main_loop_result
}

fn create_poll<P: AsRef<Path>>(
    conn: &XCBConnection,
    socket_path: P,
    signals: &mut Signals,
) -> Result<(mio::Poll, UnixListener)> {
    let poll = mio::Poll::new()?;

    let conn_fd = conn.as_fd().as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&conn_fd), X11_TOKEN, mio::Interest::READABLE)?;

    poll.registry()
        .register(signals, SIGNAL_TOKEN, mio::Interest::READABLE)?;

    // socket file exists but isn't in use
    if fs::exists(&socket_path)?
        && let Err(err) = UnixStream::connect(&socket_path)
        && err.kind() == io::ErrorKind::ConnectionRefused
    {
        fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    listener.set_nonblocking(true)?;
    poll.registry().register(
        &mut SourceFd(&listener.as_raw_fd()),
        MEMONI_TOKEN,
        mio::Interest::READABLE,
    )?;

    Ok((poll, listener))
}
