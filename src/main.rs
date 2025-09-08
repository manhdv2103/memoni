use anyhow::Result;
use memoni::opengl_context::OpenGLContext;
use memoni::selection::Selection;
use memoni::x11_window::X11Window;
use memoni::{input::Input, utils::is_plaintext_mime};
use mio::{net::UnixListener, unix::SourceFd};
use signal_hook::consts::TERM_SIGNALS;
use signal_hook_mio::v1_0::Signals;
use std::{
    ffi::OsStr,
    fs,
    io::{ErrorKind, Read, Write},
    os::{
        fd::{AsFd as _, AsRawFd as _},
        unix::net::UnixStream,
    },
    path::Path,
    time::Duration,
};
use x11rb::connection::Connection;
use x11rb::protocol::Event;

const SOCKET_DIR: &str = "/tmp/memoni/";
const X11_TOKEN: mio::Token = mio::Token(0);
const SIGNAL_TOKEN: mio::Token = mio::Token(1);
const MEMONI_TOKEN: mio::Token = mio::Token(2);

enum Args {
    Client(ClientArgs),
    Server(ServerArgs),
}

struct ClientArgs {}

struct ServerArgs {}

fn main() -> Result<()> {
    let args = parse_args()?;

    let socket_dir = Path::new(SOCKET_DIR);
    fs::create_dir_all(socket_dir)?;
    let socket_path = socket_dir.join("memoni.sock");

    match args {
        Args::Client(ClientArgs {}) => client(socket_path)?,
        Args::Server(ServerArgs {}) => server(socket_path)?,
    }

    Ok(())
}

fn parse_args() -> Result<Args, lexopt::Error> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_env();
    let is_server_mode = parser.try_raw_args().is_some_and(|mut raw_args| {
        let is_server_mode = raw_args.peek().is_some_and(|a| a.eq(OsStr::new("server")));
        if is_server_mode {
            raw_args.next();
        }
        is_server_mode
    });

    if is_server_mode {
        // TODO:
        // while let Some(arg) = parser.next()? {
        //     match arg {
        //         Short('h') | Long("help") => {
        //             println!("Usage: hello [-n|--number=NUM] [--shout] THING");
        //             std::process::exit(0);
        //         }
        //         _ => return Err(arg.unexpected()),
        //     }
        // }

        return Ok(Args::Server(ServerArgs {}));
    }

    // TODO:
    // while let Some(arg) = parser.next()? {
    //     match arg {
    //         // Short('h') | Long("help") => {
    //         //     println!("Usage: hello [-n|--number=NUM] [--shout] THING");
    //         //     std::process::exit(0);
    //         // }
    //         _ => return Err(arg.unexpected()),
    //     }
    // }

    Ok(Args::Client(ClientArgs {}))
}

fn client<P: AsRef<Path>>(socket_path: P) -> Result<()> {
    if !fs::exists(&socket_path)? {
        eprintln!("Error: memoni server is not running");
        std::process::exit(1);
    }

    let mut stream = UnixStream::connect(&socket_path)?;
    stream.write_all(b"show_win")?;

    Ok(())
}

fn server<P: AsRef<Path>>(socket_path: P) -> Result<()> {
    let width = 420u16;
    let height = 550u16;
    let background_color = 0x191919;

    let window = X11Window::new(width, height, background_color)?;
    let mut gl_context = unsafe { OpenGLContext::new(&window)? };
    let mut input = Input::new(&window)?;
    let mut selection = Selection::new(&window)?;
    let egui_ctx = egui::Context::default();

    let mut poll = mio::Poll::new()?;
    let mut poll_events = mio::Events::with_capacity(8);

    let conn_fd = window.conn.as_fd().as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&conn_fd), X11_TOKEN, mio::Interest::READABLE)?;

    let mut signals = Signals::new(TERM_SIGNALS)?;
    poll.registry()
        .register(&mut signals, SIGNAL_TOKEN, mio::Interest::READABLE)?;

    let mut listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(ref e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            eprintln!("Error: another server is already running");
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    };
    poll.registry()
        .register(&mut listener, MEMONI_TOKEN, mio::Interest::READABLE)?;

    let mut window_shown = false;
    'main_loop: loop {
        // non-blocking when window is visible, blocking otherwise
        let poll_timeout = if window_shown {
            Some(Duration::ZERO)
        } else {
            None
        };
        poll.poll(&mut poll_events, poll_timeout).or_else(|e| {
            if e.kind() == ErrorKind::Interrupted {
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
                    let (mut stream, _) = listener.accept()?;
                    let mut buf = [0u8; 1024];
                    match stream.read(&mut buf) {
                        Ok(0) => {
                            // client closed immediately
                        }
                        Ok(n) => {
                            let command = String::from_utf8_lossy(&buf[..n]);
                            match command.as_ref() {
                                "show_win" => {
                                    window.show_window()?;
                                    window.grab_input()?;
                                    window.enable_events()?;
                                    window_shown = true;
                                }
                                _ => {
                                    eprintln!("Warning: unknown client command: {command}");
                                }
                            }
                        }
                        Err(e) => eprintln!("Warning: failed to read client command: {e}"),
                    }
                }
                _ => unreachable!(),
            }
        }

        while let Some(event) = window.conn.poll_for_event()? {
            if let Event::Error(_) = event {
                continue;
            }

            if let Event::DestroyNotify(_) = event {
                break 'main_loop;
            }

            input.handle_event(&event);
            selection.handle_event(&event)?;
        }

        let mut selection_items = vec![];
        for item in &selection.items {
            if let Some((_, value)) = item.data.iter().find(|(k, _)| is_plaintext_mime(k)) {
                selection_items.push(str::from_utf8(value)?);
            }
        }

        if window_shown {
            let mut will_quit = false;
            let mut will_hide_window = false;
            let full_output = egui_ctx.run(input.egui_input.take(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading("Hello World!");
                    if ui.button("Quit").clicked() {
                        will_quit = true;
                    }

                    if ui.button("Hide").clicked() {
                        will_hide_window = true;
                    }

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, item) in selection_items.iter().enumerate() {
                            ui.label(format!("{}: {}", i, item));
                        }
                    });
                });
            });

            if will_quit {
                break;
            }

            if will_hide_window {
                window.hide_window()?;
                window.ungrab_input()?;
                window.disable_events()?;
                window_shown = false;
            }

            gl_context.render(&egui_ctx, full_output)?;
        }
    }

    window.ungrab_input()?;
    fs::remove_file(&socket_path)?;

    Ok(())
}
