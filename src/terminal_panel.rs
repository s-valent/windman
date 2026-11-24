use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use ghostty_vt::fmt::{Formatter, FormatterOptions};
use ghostty_vt::render::{CellIterator, RowIterator};
use ghostty_vt::style::RgbColor;
use ghostty_vt::{Event, RenderState, Terminal, TerminalOptions};
use portable_pty::{Child, MasterPty, PtySize};

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

pub struct Run {
    pub id: u64,
    pub title: String,
    pub terminal: Terminal,
    receiver: Receiver<Vec<u8>>,
    pub child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub done: bool,
    pub exit_code: Option<u32>,
    has_output: bool,
    output_closed: bool,
}

impl Run {
    pub fn finished_without_output(&self) -> bool {
        self.done && !self.has_output
    }
}

#[derive(Default)]
pub struct Runs(pub Arc<Mutex<Vec<Run>>>);

impl gpui::Global for Runs {}

#[derive(Default)]
pub struct TabSelection(pub Option<u64>);

impl gpui::Global for TabSelection {}

impl TabSelection {
    pub fn restore(&self, runs: &[Run]) -> Option<usize> {
        self.0
            .and_then(|id| runs.iter().position(|run| run.id == id))
            .or_else(|| {
                runs.iter()
                    .enumerate()
                    .max_by_key(|(_, run)| run.id)
                    .map(|(index, _)| index)
            })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    fn from_rgb(color: RgbColor) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }

    pub fn hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CellStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

pub type CellRun = (String, CellStyle);

pub struct CellShape {
    pub x: u16,
    pub glyph: char,
    pub fg: Color,
    pub bg: Color,
}

pub type CellRow = (Vec<CellRun>, Vec<CellShape>);

pub fn spawn_command(
    command: &str,
    foreground: u32,
    background: u32,
    cols: u16,
    rows: u16,
    cx: &mut gpui::App,
) -> Result<(), String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let mut builder = portable_pty::CommandBuilder::new(shell);
    builder.arg("-l");
    builder.arg("-i");
    builder.arg("-c");
    builder.arg(format!(
        "{command}; status=$?; wait; exec /bin/sh -c \"exit $status\""
    ));
    let run = spawn_pty(builder, command, foreground, background, cols, rows)?;
    let mut runs = cx.global::<Runs>().0.lock().unwrap();
    runs.retain(|run| !run.done);
    runs.push(run);
    Ok(())
}

fn spawn_pty(
    builder: portable_pty::CommandBuilder,
    command: &str,
    foreground: u32,
    background: u32,
    cols: u16,
    rows: u16,
) -> Result<Run, String> {
    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| error.to_string())?;
    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| error.to_string())?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| error.to_string())?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| error.to_string())?;

    let mut terminal = Terminal::new(TerminalOptions {
        cols,
        rows,
        ..Default::default()
    })
    .map_err(|error| error.to_string())?;
    terminal
        .set_default_fg_color(Some(rgb(foreground)))
        .map_err(|error| error.to_string())?;
    terminal
        .set_default_bg_color(Some(rgb(background)))
        .map_err(|error| error.to_string())?;

    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if sender.send(buffer[..read].to_vec()).is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(Run {
        id: NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed),
        title: command.to_string(),
        terminal,
        receiver,
        child: Arc::new(Mutex::new(child)),
        master: Arc::new(Mutex::new(pair.master)),
        writer: Arc::new(Mutex::new(writer)),
        done: false,
        exit_code: None,
        has_output: false,
        output_closed: false,
    })
}

fn rgb(color: u32) -> RgbColor {
    RgbColor {
        r: (color >> 16) as u8,
        g: (color >> 8) as u8,
        b: color as u8,
    }
}

pub struct Pumped {
    pub changed: bool,
    pub clipboard: Vec<String>,
}

pub fn pump(runs: &mut [Run]) -> Pumped {
    let mut changed = false;
    let mut clipboard = Vec::new();
    for run in runs {
        loop {
            match run.receiver.try_recv() {
                Ok(bytes) => {
                    run.terminal.vt_write(&bytes);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    run.output_closed = true;
                    break;
                }
            }
        }
        let mut replies = Vec::new();
        for event in run.terminal.events() {
            match event {
                Event::PtyWrite(bytes) => replies.push(bytes),
                Event::ClipboardWrite(write) => {
                    if let Some(text) = clipboard_text(&write) {
                        clipboard.push(text);
                    }
                }
                _ => {}
            }
        }
        if !replies.is_empty()
            && let Ok(mut writer) = run.writer.lock()
        {
            for bytes in replies {
                let _ = writer.write_all(&bytes);
            }
            let _ = writer.flush();
        }
        if run.exit_code.is_none()
            && let Ok(mut child) = run.child.lock()
            && let Ok(Some(status)) = child.try_wait()
        {
            run.exit_code = Some(status.exit_code());
        }
        if !run.done && run.exit_code.is_some() && run.output_closed {
            run.has_output =
                match Formatter::new(&run.terminal, FormatterOptions::new().with_trim(true))
                    .and_then(|mut formatter| formatter.format_alloc())
                {
                    Ok(text) => String::from_utf8_lossy(&text)
                        .chars()
                        .any(|character| !character.is_whitespace()),
                    Err(error) => {
                        eprintln!("could not check terminal output: {error}");
                        true
                    }
                };
            run.done = true;
            changed = true;
        }
    }
    Pumped { changed, clipboard }
}

pub fn prune_finished(runs: &mut Vec<Run>, active: Option<usize>) -> Option<usize> {
    let keep_finished = runs
        .iter()
        .filter(|run| run.done && run.has_output)
        .max_by_key(|run| run.id)
        .map(|run| run.id);
    let mut previous_index = 0;
    let mut retained_index = 0;
    let mut selection = None;
    runs.retain(|run| {
        let keep = !run.done || Some(run.id) == keep_finished;
        if keep {
            if active == Some(previous_index) {
                selection = Some(retained_index);
            }
            retained_index += 1;
        }
        previous_index += 1;
        keep
    });
    selection.or_else(|| active.and_then(|_| runs.len().checked_sub(1)))
}

pub fn reorder_tabs(runs: &mut [Run], source: u64, target: u64) -> bool {
    let Some(source) = runs.iter().position(|run| run.id == source) else {
        return false;
    };
    let Some(target) = runs.iter().position(|run| run.id == target) else {
        return false;
    };
    match source.cmp(&target) {
        std::cmp::Ordering::Less => runs[source..=target].rotate_left(1),
        std::cmp::Ordering::Greater => runs[target..=source].rotate_right(1),
        std::cmp::Ordering::Equal => return false,
    }
    true
}

fn clipboard_text(write: &ghostty_vt::terminal::ClipboardWrite) -> Option<String> {
    let content = write
        .contents
        .iter()
        .find(|content| {
            content.mime.is_empty()
                || content.mime.starts_with("text/")
                || content.mime == "UTF8_STRING"
                || content.mime == "STRING"
        })
        .or_else(|| write.contents.first())?;
    Some(String::from_utf8_lossy(&content.data).into_owned())
}

pub fn resize(run: &mut Run, cols: u16, rows: u16, cell_width: u32, cell_height: u32) {
    let _ = run.terminal.resize(cols, rows, cell_width, cell_height);
    if let Ok(master) = run.master.lock() {
        let _ = master.resize(PtySize {
            rows,
            cols,
            pixel_width: cell_width.min(u16::MAX as u32) as u16,
            pixel_height: cell_height.min(u16::MAX as u32) as u16,
        });
    }
}

pub fn render_rows(terminal: &Terminal, show_cursor: bool) -> Vec<CellRow> {
    match RenderState::new().and_then(|mut state| {
        let snapshot = state.update(terminal)?;
        let colors = snapshot.colors()?;
        let cursor = snapshot.cursor()?;
        let mut rows = RowIterator::new()?;
        let mut cells = CellIterator::new()?;
        let mut output = Vec::new();
        let mut row_iter = rows.update(&snapshot)?;
        while let Some(row) = row_iter.next() {
            let mut line = Vec::new();
            let mut shapes = Vec::new();
            let mut column = 0u16;
            let mut cell_iter = cells.update(row)?;
            while let Some(cell) = cell_iter.next() {
                let raw = cell.raw_cell()?;
                let wide = raw.wide()?;
                if matches!(
                    wide,
                    ghostty_vt::screen::CellWide::SpacerTail
                        | ghostty_vt::screen::CellWide::SpacerHead
                ) {
                    continue;
                }
                let graphemes = cell.graphemes()?;
                let text = if graphemes.is_empty() {
                    " ".to_string()
                } else {
                    graphemes.iter().collect::<String>()
                };
                let style = cell.style()?;
                let mut fg = cell
                    .fg_color()?
                    .map(Color::from_rgb)
                    .unwrap_or_else(|| Color::from_rgb(colors.foreground));
                let mut bg = cell
                    .bg_color()?
                    .map(Color::from_rgb)
                    .unwrap_or_else(|| Color::from_rgb(colors.background));
                if style.inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let cursor_here = show_cursor
                    && cursor.visible
                    && cursor.viewport.map(|cursor| (cursor.x, cursor.y))
                        == Some((column, output.len() as u16));
                if cursor_here {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let special = graphemes.len() == 1 && is_box_or_block(graphemes[0]);
                if special {
                    shapes.push(CellShape {
                        x: column,
                        glyph: graphemes[0],
                        fg,
                        bg,
                    });
                }
                line.push((
                    if special { " ".to_string() } else { text },
                    CellStyle {
                        fg,
                        bg,
                        bold: style.bold,
                        italic: style.italic,
                        underline: style.underline != ghostty_vt::style::Underline::None,
                    },
                ));
                column = column.saturating_add(if wide == ghostty_vt::screen::CellWide::Wide {
                    2
                } else {
                    1
                });
            }
            output.push((group_cells(line), shapes));
        }
        Ok::<_, ghostty_vt::Error>(output)
    }) {
        Ok(rows) => rows,
        Err(_) => Vec::new(),
    }
}

fn is_box_or_block(glyph: char) -> bool {
    matches!(glyph, '\u{2500}'..='\u{259f}')
}
fn group_cells(cells: Vec<CellRun>) -> Vec<CellRun> {
    let mut runs: Vec<CellRun> = Vec::new();
    for (text, style) in cells {
        match runs.last_mut() {
            Some((previous, previous_style)) if *previous_style == style => {
                previous.push_str(&text);
            }
            _ => runs.push((text, style)),
        }
    }
    runs
}

pub fn mouse_bytes(
    terminal: &Terminal,
    action: ghostty_vt::mouse::Action,
    button: Option<ghostty_vt::mouse::Button>,
    x: f32,
    y: f32,
) -> Option<Vec<u8>> {
    if !terminal.is_mouse_tracking().ok()? {
        return None;
    }
    let mut event = ghostty_vt::mouse::Event::new().ok()?;
    event
        .set_action(action)
        .set_button(button)
        .set_position(ghostty_vt::mouse::Position { x, y });
    let mut encoder = ghostty_vt::mouse::Encoder::new().ok()?;
    encoder.set_options_from_terminal(terminal);
    encoder.set_size(ghostty_vt::mouse::EncoderSize {
        screen_width: terminal.cols().ok()? as u32,
        screen_height: terminal.rows().ok()? as u32,
        cell_width: 1,
        cell_height: 1,
        padding_top: 0,
        padding_bottom: 0,
        padding_right: 0,
        padding_left: 0,
    });
    let mut bytes = Vec::new();
    encoder.encode_to_vec(&event, &mut bytes).ok()?;
    Some(bytes)
}

pub fn terminate(run: &Run) {
    let _ = run.child.lock().map(|mut child| child.kill());
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use ghostty_vt::{Terminal, TerminalOptions};

    fn command_run(command: &str) -> super::Run {
        let mut builder = portable_pty::CommandBuilder::new("/bin/sh");
        builder.arg("-c");
        builder.arg(command);
        super::spawn_pty(builder, command, 0xffffff, 0x222222, 80, 24).unwrap()
    }

    fn finished_run(command: &str) -> super::Run {
        let mut run = command_run(command);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !run.done {
            super::pump(std::slice::from_mut(&mut run));
            assert!(
                Instant::now() < deadline,
                "command did not finish: {command}"
            );
            if !run.done {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        run
    }

    #[test]
    fn dragging_tabs_reorders_both_directions_and_preserves_selection() {
        let mut runs = vec![
            finished_run("printf same"),
            finished_run("printf same"),
            finished_run("printf newest"),
        ];
        let ids = runs.iter().map(|run| run.id).collect::<Vec<_>>();
        let selection = super::TabSelection(Some(ids[1]));
        assert!(super::reorder_tabs(&mut runs, ids[0], ids[2]));
        assert_eq!(
            runs.iter().map(|run| run.id).collect::<Vec<_>>(),
            vec![ids[1], ids[2], ids[0]]
        );
        assert_eq!(selection.restore(&runs), Some(0));
        assert!(super::reorder_tabs(&mut runs, ids[0], ids[1]));
        assert_eq!(runs.iter().map(|run| run.id).collect::<Vec<_>>(), ids);
        assert_eq!(selection.restore(&runs), Some(1));
    }

    #[test]
    fn dragging_closed_tabs_or_onto_the_same_tab_does_nothing() {
        let mut runs = vec![finished_run("printf first"), finished_run("printf second")];
        let removed_id = runs.remove(0).id;
        let remaining_id = runs[0].id;
        assert!(!super::reorder_tabs(&mut runs, removed_id, remaining_id));
        assert!(!super::reorder_tabs(&mut runs, remaining_id, removed_id));
        assert!(!super::reorder_tabs(&mut runs, remaining_id, remaining_id));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, remaining_id);
    }

    #[test]
    fn reordering_does_not_change_which_finished_output_is_kept() {
        let mut runs = vec![
            finished_run("printf first"),
            finished_run("printf second"),
            finished_run("printf newest"),
        ];
        let oldest = runs[0].id;
        let newest = runs[2].id;
        assert!(super::reorder_tabs(&mut runs, oldest, newest));
        assert_eq!(super::TabSelection::default().restore(&runs), Some(1));
        assert_eq!(super::prune_finished(&mut runs, Some(1)), Some(0));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, newest);
    }

    #[test]
    fn selected_tab_survives_other_tab_removals_and_duplicate_commands() {
        let mut runs = vec![
            finished_run("printf same"),
            finished_run("printf same"),
            finished_run("printf newest"),
        ];
        let selection = super::TabSelection(Some(runs[1].id));
        assert_eq!(selection.restore(&runs), Some(1));
        runs.remove(0);
        assert_eq!(selection.restore(&runs), Some(0));
        runs.push(finished_run("printf another"));
        assert_eq!(selection.restore(&runs), Some(0));
    }

    #[test]
    fn closed_selected_tab_falls_back_to_the_latest_remaining_tab() {
        let mut runs = vec![
            finished_run("printf first"),
            finished_run("printf second"),
            finished_run("printf third"),
        ];
        let selection = super::TabSelection(Some(runs[1].id));
        runs.remove(1);
        assert_eq!(selection.restore(&runs), Some(1));
        assert_eq!(super::TabSelection::default().restore(&runs), Some(1));
        runs.clear();
        assert_eq!(selection.restore(&runs), None);
    }

    #[test]
    fn silent_commands_close_after_success_or_failure() {
        ["true", "exit 7"].into_iter().for_each(|command| {
            let mut runs = vec![finished_run(command)];
            assert!(runs[0].finished_without_output());
            assert_eq!(super::prune_finished(&mut runs, Some(0)), None);
            assert!(runs.is_empty());
        });
    }

    #[test]
    fn invisible_terminal_output_does_not_keep_a_tab_open() {
        [
            "printf ' \\t\\n\\n'",
            "printf '\\033[?2004h\\033[?2004l\\033[0m'",
            "printf '\\033]0;window title\\007\\033[?25h'",
            "printf 'temporary\\033[2J\\033[H'",
        ]
        .into_iter()
        .for_each(|command| {
            let mut runs = vec![finished_run(command)];
            assert!(runs[0].finished_without_output());
            assert_eq!(super::prune_finished(&mut runs, Some(0)), None, "{command}");
            assert!(runs.is_empty());
        });
    }

    #[test]
    fn output_in_scrollback_keeps_the_tab_open() {
        let command = format!("printf 'earlier output{}'", "\\n".repeat(48));
        let mut runs = vec![finished_run(&command)];
        assert!(!runs[0].finished_without_output());
        assert_eq!(super::prune_finished(&mut runs, Some(0)), Some(0));
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn finished_commands_keep_stdout_and_stderr() {
        ["printf stdout", "printf stderr >&2; exit 7"]
            .into_iter()
            .for_each(|command| {
                let mut runs = vec![finished_run(command)];
                assert!(!runs[0].finished_without_output());
                assert_eq!(super::prune_finished(&mut runs, Some(0)), Some(0));
                assert_eq!(runs.len(), 1);
            });
    }

    #[test]
    fn output_arriving_after_exit_is_not_discarded() {
        let mut run = command_run("true");
        run.child.lock().unwrap().wait().unwrap();
        let (sender, receiver) = mpsc::channel();
        run.receiver = receiver;
        let mut runs = vec![run];

        super::pump(&mut runs);
        assert_eq!(runs[0].exit_code, Some(0));
        assert!(!runs[0].done);
        assert!(!runs[0].finished_without_output());
        assert_eq!(super::prune_finished(&mut runs, Some(0)), Some(0));
        assert_eq!(runs.len(), 1);

        sender.send(b"last output".to_vec()).unwrap();
        drop(sender);
        super::pump(&mut runs);
        assert!(runs[0].done);
        assert!(!runs[0].finished_without_output());
        assert_eq!(super::prune_finished(&mut runs, Some(0)), Some(0));
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn silent_completion_preserves_previous_output() {
        let mut runs = vec![finished_run("printf previous"), finished_run("true")];
        assert_eq!(super::prune_finished(&mut runs, Some(1)), Some(0));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].title, "printf previous");
    }

    #[test]
    fn closing_a_tab_preserves_the_selected_running_command() {
        let mut runs = vec![
            finished_run("true"),
            command_run("cat"),
            command_run("/bin/cat"),
        ];
        assert_eq!(super::prune_finished(&mut runs, Some(1)), Some(0));
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].title, "cat");
        runs.iter().for_each(|run| {
            super::terminate(run);
            run.child.lock().unwrap().wait().unwrap();
        });
    }

    #[test]
    fn pty_runs_a_command_and_produces_terminal_output() {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut builder = portable_pty::CommandBuilder::new("/bin/sh");
        builder.arg("-c");
        builder.arg("printf 'pty-ok\\n'");
        let mut child = pair.slave.spawn_command(builder).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = reader.read_to_end(&mut output);
            let _ = sender.send(String::from_utf8_lossy(&output).into_owned());
        });
        child.wait().unwrap();
        let output = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(output.contains("pty-ok"), "{output:?}");
    }

    #[test]
    fn block_glyphs_become_shapes() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 4,
            rows: 2,
            ..Default::default()
        })
        .unwrap();
        terminal.vt_write("█▀".as_bytes());
        let rows = super::render_rows(&terminal, false);
        assert_eq!(rows[0].1.len(), 2);
        assert_eq!(rows[0].1[0].glyph, '█');
        assert_eq!(rows[0].1[1].glyph, '▀');
        let text = rows[0]
            .0
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<String>();
        assert!(text.trim().is_empty());
    }

    #[test]
    fn osc52_text_is_extracted() {
        let write = ghostty_vt::terminal::ClipboardWrite {
            location: ghostty_vt::terminal::ClipboardLocation::Standard,
            contents: vec![ghostty_vt::terminal::ClipboardContent {
                mime: "text/plain;charset=utf-8".into(),
                data: b"osc52 works".to_vec(),
            }],
            name: String::new(),
            granted: false,
            can_remember: false,
        };
        assert_eq!(
            super::clipboard_text(&write).as_deref(),
            Some("osc52 works")
        );
    }

    #[test]
    fn mouse_events_follow_terminal_tracking_mode() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 20,
            rows: 3,
            ..Default::default()
        })
        .unwrap();
        assert!(
            super::mouse_bytes(
                &terminal,
                ghostty_vt::mouse::Action::Press,
                Some(ghostty_vt::mouse::Button::Left),
                1.,
                1.
            )
            .is_none()
        );
        terminal.vt_write(b"\x1b[?1000h");
        let bytes = super::mouse_bytes(
            &terminal,
            ghostty_vt::mouse::Action::Press,
            Some(ghostty_vt::mouse::Button::Left),
            1.,
            1.,
        )
        .unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn ghostty_parses_text_and_color() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 20,
            rows: 3,
            ..Default::default()
        })
        .unwrap();
        terminal.vt_write(b"hi \x1b[31mred\x1b[0m");
        let rows = super::render_rows(&terminal, false);
        let text = rows[0]
            .0
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<String>();
        assert_eq!(text.trim_end(), "hi red");
        assert!(rows[0].0.iter().any(|(_, style)| style.fg
            == super::Color {
                r: 0xcc,
                g: 0x66,
                b: 0x66
            }));
    }
}

#[cfg(test)]
mod running_tests {
    use portable_pty::PtySize;

    #[test]
    fn a_long_running_pty_child_is_not_reported_as_finished() {
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut builder = portable_pty::CommandBuilder::new("/bin/sh");
        builder.arg("-c");
        builder.arg("sleep 5");
        let mut child = pair.slave.spawn_command(builder).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        let _ = child.kill();
        let _ = child.wait();
    }
}
