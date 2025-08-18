mod input;
mod opengl_context;
mod utils;
mod x11_window;

use std::sync::mpsc::channel;

use anyhow::Result;
use input::Input;
use opengl_context::OpenGLContext;
use x11_window::X11Window;
use x11rb::connection::Connection;
use x11rb::protocol::Event;

fn main() -> Result<()> {
    let width = 420u16;
    let height = 550u16;
    let background_color = 0x191919;

    let window = X11Window::new(width, height, background_color)?;
    let mut gl_context = unsafe { OpenGLContext::new(&window)? };
    let mut input = Input::new(&window)?;
    let egui_ctx = egui::Context::default();

    let (ctrlc_tx, ctrlc_rx) = channel();
    ctrlc::set_handler(move || {
        ctrlc_tx
            .send(())
            .expect("Could not send signal on channel.")
    })?;

    window.map_window()?;
    window.grab_input()?;

    loop {
        while let Ok(Some(event)) = window.conn.poll_for_event() {
            if let Event::DestroyNotify(_) = event {
                return Ok(());
            }

            input.handle_event(event);
        }

        let mut quit = false;
        let full_output = egui_ctx.run(input.egui_input.take(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("Hello World!");
                if ui.button("Quit").clicked() {
                    quit = true;
                }
            });
        });

        if quit || ctrlc_rx.try_recv().is_ok() {
            window.ungrab_input()?;
            return Ok(());
        }

        gl_context.render(&egui_ctx, full_output)?;
    }
}
