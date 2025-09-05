use anyhow::Result;
use memoni::opengl_context::OpenGLContext;
use memoni::selection::Selection;
use memoni::x11_window::X11Window;
use memoni::{input::Input, utils::is_plaintext_mime};
use mio::unix::SourceFd;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook_mio::v1_0::Signals;
use std::{
    io::ErrorKind,
    os::fd::{AsFd as _, AsRawFd as _},
};
use x11rb::connection::Connection;
use x11rb::protocol::Event;

const X11_TOKEN: mio::Token = mio::Token(0);
const SIGNAL_TOKEN: mio::Token = mio::Token(1);

fn main() -> Result<()> {
    let width = 420u16;
    let height = 550u16;
    let background_color = 0x191919;

    let window = X11Window::new(width, height, background_color)?;
    let mut gl_context = unsafe { OpenGLContext::new(&window)? };
    let mut input = Input::new(&window)?;
    let mut selection = Selection::new(&window)?;
    let egui_ctx = egui::Context::default();

    window.map_window()?;
    window.grab_input()?;

    let mut poll = mio::Poll::new()?;
    let mut poll_events = mio::Events::with_capacity(8);

    let conn_fd = window.conn.as_fd().as_raw_fd();
    poll.registry()
        .register(&mut SourceFd(&conn_fd), X11_TOKEN, mio::Interest::READABLE)?;

    let mut signals = Signals::new(TERM_SIGNALS)?;
    poll.registry()
        .register(&mut signals, SIGNAL_TOKEN, mio::Interest::READABLE)?;

    let mut show_window = true;
    'main_loop: loop {
        if !show_window {
            poll.poll(&mut poll_events, None).or_else(|e| {
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
                    _ => unreachable!(),
                }
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

        if show_window {
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
                window.ungrab_input()?;
                window.unmap_window()?;
                window.disable_events()?;
                show_window = false;
            }

            gl_context.render(&egui_ctx, full_output)?;
        }
    }

    window.ungrab_input()?;

    Ok(())
}
