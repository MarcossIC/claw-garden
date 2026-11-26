//! The real terminal the model picker runs over: its keys read as they are
//! pressed, with no Enter and no echo, and its frames drawn on the terminal's
//! alternate screen, so that what was there comes back when the picker ends.
//!
//! A test has no terminal, so nothing here that touches one is reached by a
//! test. What can be decided without one is pure and tested: the modes a
//! console or a tty is put in, and the bytes that paint a frame.

use std::io::{Read, Write};

use super::picker::{decode, Key, Line, Terminal, Tone};

/// The alternate screen, with the cursor hidden.
const ENTER: &str = "\x1b[?1049h\x1b[?25l";
/// The cursor back, and the screen that was there.
const LEAVE: &str = "\x1b[?25h\x1b[?1049l";

/// The size taken for a terminal that does not tell its own.
const DEFAULT_SIZE: (usize, usize) = (24, 80);

/// The bytes that draw `frame` from the top of a cleared screen. Its tones
/// are intensities and reverse video, never a color, so they read on any theme.
pub fn paint(frame: &[Line]) -> String {
    let mut out = String::from("\x1b[H\x1b[2J");
    for (i, line) in frame.iter().enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        match line.tone {
            Tone::Dim => out.push_str("\x1b[2m"),
            Tone::Plain => {}
            Tone::Bright => out.push_str("\x1b[1;7m"),
        }
        out.push_str(&line.text);
        if line.tone != Tone::Plain {
            out.push_str("\x1b[0m");
        }
    }
    out
}

/// A terminal read key by key, until it is dropped.
pub struct RawTerminal {
    saved: platform::Saved,
}

impl RawTerminal {
    /// The terminal of this process, read key by key and showing its
    /// alternate screen; `None`, with nothing changed, when the input or the
    /// output is not a terminal that can be.
    pub fn enter() -> Option<Self> {
        let saved = platform::raw()?;
        let terminal = Self { saved };
        let mut out = std::io::stdout();
        // A failed write drops `terminal`, which puts the modes back.
        out.write_all(ENTER.as_bytes()).and_then(|()| out.flush()).ok()?;
        Some(terminal)
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        // Best effort: there is nowhere to report a terminal that went away.
        let _ = out.write_all(LEAVE.as_bytes()).and_then(|()| out.flush());
        platform::restore(&self.saved);
    }
}

impl Terminal for RawTerminal {
    fn size(&self) -> (usize, usize) {
        platform::size().unwrap_or(DEFAULT_SIZE)
    }

    fn read(&mut self) -> std::io::Result<Vec<Key>> {
        let mut bytes = [0u8; 1024];
        let read = match std::io::stdin().lock().read(&mut bytes) {
            // A signal cut the wait short, a resize among them: no key, and
            // the frame is drawn again at the size the terminal has now.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => return Ok(Vec::new()),
            read => read?,
        };
        // A closed input has no key left to wait for.
        if read == 0 {
            return Ok(vec![Key::Quit]);
        }
        Ok(decode(&bytes[..read]))
    }

    fn draw(&mut self, frame: &[Line]) -> std::io::Result<()> {
        let mut out = std::io::stdout().lock();
        out.write_all(paint(frame).as_bytes())?;
        out.flush()
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::io::{AsRawHandle, RawHandle};

    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetConsoleScreenBufferInfo, SetConsoleMode, CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT,
        ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
        ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    };

    /// The console's handles and the modes they had.
    pub struct Saved {
        input: RawHandle,
        input_mode: CONSOLE_MODE,
        output: RawHandle,
        output_mode: CONSOLE_MODE,
    }

    /// `mode` for an input read key by key: no line to wait for, no echo, and
    /// Ctrl-C as a key, so that leaving always passes through the restore.
    /// With virtual terminal input the arrows arrive as the sequences a Unix
    /// terminal sends, and one decoder reads both.
    pub fn raw_input(mode: CONSOLE_MODE) -> CONSOLE_MODE {
        (mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT)) | ENABLE_VIRTUAL_TERMINAL_INPUT
    }

    /// `mode` for an output that obeys the sequences of a frame.
    pub fn vt_output(mode: CONSOLE_MODE) -> CONSOLE_MODE {
        mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING
    }

    pub fn raw() -> Option<Saved> {
        let (input, output) = (std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle());
        let (mut input_mode, mut output_mode) = (0, 0);
        // SAFETY: both handles are this process's standard ones, alive for
        // the whole call, and each mode is a valid out-pointer. A handle that
        // is not a console makes its call fail, which is the answer wanted.
        let is_console = unsafe { GetConsoleMode(input, &mut input_mode) != 0 && GetConsoleMode(output, &mut output_mode) != 0 };
        if !is_console {
            return None;
        }
        let saved = Saved { input, input_mode, output, output_mode };
        // SAFETY: as above; the calls only read the handle and the mode.
        let set = unsafe { SetConsoleMode(input, raw_input(input_mode)) != 0 && SetConsoleMode(output, vt_output(output_mode)) != 0 };
        if !set {
            // A console from before virtual terminal sequences refuses the mode.
            restore(&saved);
            return None;
        }
        Some(saved)
    }

    pub fn restore(saved: &Saved) {
        // SAFETY: the handles are the standard ones `raw` read, still open.
        // A failure leaves the console as it is, and nothing can be done.
        unsafe {
            SetConsoleMode(saved.input, saved.input_mode);
            SetConsoleMode(saved.output, saved.output_mode);
        }
    }

    /// The rows and columns of the console's window, not of its buffer.
    pub fn size() -> Option<(usize, usize)> {
        let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
        // SAFETY: the handle is this process's standard output and `info` is
        // a valid out-pointer for the call.
        if unsafe { GetConsoleScreenBufferInfo(std::io::stdout().as_raw_handle(), &mut info) } == 0 {
            return None;
        }
        let window = info.srWindow;
        let rows = usize::try_from(i32::from(window.Bottom) - i32::from(window.Top) + 1).ok()?;
        let cols = usize::try_from(i32::from(window.Right) - i32::from(window.Left) + 1).ok()?;
        (rows > 0 && cols > 0).then_some((rows, cols))
    }
}

#[cfg(unix)]
mod platform {
    use std::io::IsTerminal;
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    /// The tty's settings as they were.
    pub struct Saved(libc::termios);

    /// `termios` for an input read key by key: no line to wait for, no echo,
    /// and Ctrl-C as a key, so that leaving always passes through the
    /// restore. A read returns as soon as one byte is there.
    pub fn raw_termios(mut termios: libc::termios) -> libc::termios {
        termios.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        termios.c_cc[libc::VMIN] = 1;
        termios.c_cc[libc::VTIME] = 0;
        termios
    }

    pub fn raw() -> Option<Saved> {
        // The frames go to the output, so it has to be a terminal as well.
        if !std::io::stdout().is_terminal() {
            return None;
        }
        let input = std::io::stdin().as_raw_fd();
        let mut termios = MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `input` is this process's standard input and `termios` is a
        // valid out-pointer. An input that is not a tty makes the call fail.
        if unsafe { libc::tcgetattr(input, termios.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: tcgetattr returned 0, so it filled `termios`.
        let saved = unsafe { termios.assume_init() };
        // SAFETY: the pointer is to a termios that lives through the call.
        if unsafe { libc::tcsetattr(input, libc::TCSAFLUSH, &raw_termios(saved)) } != 0 {
            return None;
        }
        Some(Saved(saved))
    }

    pub fn restore(saved: &Saved) {
        // SAFETY: the pointer is to the termios `raw` read. A failure leaves
        // the tty as it is, and nothing can be done.
        unsafe { libc::tcsetattr(std::io::stdin().as_raw_fd(), libc::TCSAFLUSH, &saved.0) };
    }

    pub fn size() -> Option<(usize, usize)> {
        let mut size = libc::winsize { ws_row: 0, ws_col: 0, ws_xpixel: 0, ws_ypixel: 0 };
        // SAFETY: TIOCGWINSZ writes one `winsize` through the pointer, which
        // is valid for the call.
        if unsafe { libc::ioctl(std::io::stdout().as_raw_fd(), libc::TIOCGWINSZ, &raw mut size) } != 0 {
            return None;
        }
        (size.ws_row > 0 && size.ws_col > 0).then_some((usize::from(size.ws_row), usize::from(size.ws_col)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_painted_from_the_top_with_its_tones() {
        let line = |tone, text: &str| Line { text: text.into(), tone };
        let painted = paint(&[line(Tone::Plain, "title"), line(Tone::Plain, ""), line(Tone::Bright, "› on"), line(Tone::Dim, "  off")]);
        assert_eq!(painted, "\x1b[H\x1b[2Jtitle\r\n\r\n\x1b[1;7m› on\x1b[0m\r\n\x1b[2m  off\x1b[0m");
        assert!(!painted.ends_with('\n'), "a frame as tall as the terminal does not scroll it");
        assert_eq!(paint(&[]), "\x1b[H\x1b[2J");
    }

    #[cfg(windows)]
    #[test]
    fn the_console_modes_for_the_picker() {
        use windows_sys::Win32::System::Console::{
            ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT, ENABLE_QUICK_EDIT_MODE,
            ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WRAP_AT_EOL_OUTPUT,
        };
        let cooked = ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT | ENABLE_QUICK_EDIT_MODE;
        let raw = platform::raw_input(cooked);
        assert_eq!(raw & (ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT), 0, "no line, no echo, Ctrl-C as a key");
        assert_ne!(raw & ENABLE_VIRTUAL_TERMINAL_INPUT, 0, "the arrows as sequences");
        assert_ne!(raw & ENABLE_QUICK_EDIT_MODE, 0, "what it does not need stays as it was");
        assert_eq!(platform::raw_input(raw), raw);

        let output = platform::vt_output(ENABLE_WRAP_AT_EOL_OUTPUT);
        assert_eq!(output, ENABLE_WRAP_AT_EOL_OUTPUT | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
    }

    #[cfg(unix)]
    #[test]
    fn the_termios_for_the_picker() {
        // SAFETY: a termios is integers and an array of them; all zeros is one.
        let mut cooked: libc::termios = unsafe { std::mem::zeroed() };
        cooked.c_lflag = libc::ICANON | libc::ECHO | libc::ISIG | libc::ECHOE;
        cooked.c_cc[libc::VMIN] = 0;
        cooked.c_cc[libc::VTIME] = 7;
        let raw = platform::raw_termios(cooked);
        assert_eq!(raw.c_lflag & (libc::ICANON | libc::ECHO | libc::ISIG), 0, "no line, no echo, Ctrl-C as a key");
        assert_ne!(raw.c_lflag & libc::ECHOE, 0, "what it does not need stays as it was");
        assert_eq!((raw.c_cc[libc::VMIN], raw.c_cc[libc::VTIME]), (1, 0), "a read returns with one byte");
    }
}
