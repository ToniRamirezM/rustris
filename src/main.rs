mod cartridge;
mod ppu;
mod mmu;
mod cpu;
mod gb;

use gb::GB;
use cartridge::Cartridge;

use sdl2::pixels::PixelFormatEnum;
use sdl2::event::Event;
use sdl2::keyboard::Scancode;

use std::time::{Duration, Instant};
use std::hint::spin_loop as cpu_relax;

/// Maps SDL scancodes to Game Boy input bitmasks.
const INPUT_MASKS: [(Scancode, u8); 8] = [
    (Scancode::Right,  gb::BTN_RIGHT),
    (Scancode::Left,   gb::BTN_LEFT),
    (Scancode::Up,     gb::BTN_UP),
    (Scancode::Down,   gb::BTN_DOWN),
    (Scancode::X,      gb::BTN_A),
    (Scancode::Z,      gb::BTN_B),
    (Scancode::Space,  gb::BTN_SELECT),
    (Scancode::Return, gb::BTN_START),
];

/// Frame period:
/// - Real DMG cadence: 59.7275 FPS → 16_742_706 ns per frame.
const GB_FRAME_NS: u64 = 16_742_706;    // ~59.7275 FPS (Game Boy)

fn main() {
    let rom_path = "tetris.gb";
    let cartridge = match Cartridge::from_file(rom_path) {
        Ok(cart) => cart,
        Err(e) => {
            eprintln!("Error loading ROM: {}", e);
            return;
        }
    };

    emulate(GB::new(cartridge));
}

/// SDL front-end:
/// - Creates a window and a streaming RGB24 texture.
/// - Locks the texture each frame and lets the PPU render directly into it (no extra copy).
/// - Handles keyboard input and palette toggle.
/// - Presents frames and enforces a precise frame rate using a high-resolution limiter
///   (sleep for the coarse part, busy-wait for the last ~0.5 ms).
fn emulate(mut gb: GB) {
    let sdl_context = sdl2::init().unwrap();
    let video_subsystem = sdl_context.video().unwrap();

    let window = video_subsystem
        .window(
            "RUSTЯIS",
            (ppu::SCREEN_WIDTH as u32) * 4,
            (ppu::SCREEN_HEIGHT as u32) * 4,
        )
        .position_centered()
        .build()
        .unwrap();

    // IMPORTANT: no present_vsync(); the manual limiter below drives cadence.
    let mut canvas = window.into_canvas().build().unwrap();

    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(
            PixelFormatEnum::RGB24,
            ppu::SCREEN_WIDTH as u32,
            ppu::SCREEN_HEIGHT as u32,
        )
        .unwrap();

    let mut event_pump = sdl_context.event_pump().unwrap();

    // Precise frame limiter state
    let frame_period = Duration::from_nanos(GB_FRAME_NS);
    let mut next_deadline = Instant::now() + frame_period;

    'running: loop {
        // --- Event handling ---
        for event in event_pump.poll_iter() {
            match event {
                Event::KeyDown { scancode: Some(Scancode::Escape), repeat: false, .. } |
                Event::Quit { .. } => break 'running,

                Event::KeyDown { scancode: Some(Scancode::P), repeat: false, .. } => {
                    gb.toggle_palette();
                }

                Event::KeyDown { scancode: Some(sc), repeat: false, .. } => {
                    if let Some(mask) = INPUT_MASKS.iter().find(|(s, _)| *s == sc).map(|(_, m)| *m) {
                        gb.input_press(mask);
                    }
                }

                Event::KeyUp { scancode: Some(sc), .. } => {
                    if let Some(mask) = INPUT_MASKS.iter().find(|(s, _)| *s == sc).map(|(_, m)| *m) {
                        gb.input_release(mask);
                    }
                }

                Event::Window { win_event: sdl2::event::WindowEvent::FocusLost, .. } => {
                    gb.input_release(
                        gb::BTN_RIGHT | gb::BTN_LEFT | gb::BTN_UP | gb::BTN_DOWN |
                        gb::BTN_A | gb::BTN_B | gb::BTN_SELECT | gb::BTN_START
                    );
                }

                _ => {}
            }
        }

        // Lock the streaming texture and let the emulator render directly into its buffer
        texture.with_lock(None, |buf: &mut [u8], pitch: usize| {
            // Run until a full frame is produced
            while !gb.step(buf, pitch) {}
        }).unwrap();

        canvas.copy(&texture, None, None).unwrap();
        canvas.present();

        // --- Precise frame limiter (sleep + spin to reach exact deadline) ---
        let now = Instant::now();
        if next_deadline > now {
            // Sleep the coarse chunk, leaving a small margin (~0.5 ms) to fine-tune with spinning
            let remain = next_deadline - now;
            if remain > Duration::from_micros(500) {
                std::thread::sleep(remain - Duration::from_micros(500));
            }
            // Busy-wait until the precise deadline
            while Instant::now() < next_deadline {
                cpu_relax();
            }
        } else {
            // We're late; resync to avoid drift accumulation
            next_deadline = Instant::now();
        }
        // Schedule the next frame deadline
        next_deadline += frame_period;
        // -------------------------------------------------------------------
    }
}
