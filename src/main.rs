#[cfg(target_os = "macos")]
mod calc;
mod macos;
mod settings;
mod terminal_panel;
mod text_input;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use gpui::{
    App, Application, Bounds, Context, Entity, Focusable, KeyDownEvent, MouseButton, Render,
    Subscription, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    actions, canvas, div, img, point, prelude::*, px, rgb, rgba, size, uniform_list,
};
use terminal_panel::{Runs, TabSelection, terminate};
use text_input::{TextInput, TextInputEvent};

actions!(windman, [Quit, OpenSettings, NextTab, PreviousTab]);

const TERMINAL_LEFT: f32 = 14.;
const TERMINAL_TOP: f32 = 96.;

static CMD_USED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct HistoryEntry {
    kind: String,
    value: String,
    used: u64,
}

#[derive(Clone)]
struct AppEntry {
    name: String,
    path: PathBuf,
    icon: Option<PathBuf>,
}

#[derive(Clone)]
struct Entry {
    title: String,
    subtitle: String,
    preview: String,
    highlight: Vec<usize>,
    kind: EntryKind,
}

#[derive(Clone)]
enum EntryKind {
    App(usize),
    Command(String),
    Copy(String),
    Settings,
}

#[derive(Clone)]
struct DraggedTab {
    id: u64,
    title: String,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(22.))
            .max_w(px(240.))
            .px_2()
            .flex()
            .items_center()
            .rounded_sm()
            .border_1()
            .border_color(rgba(0xffffff60))
            .bg(rgba(0x000000a0))
            .text_color(rgb(PaletteTheme::default().foreground))
            .text_size(px(10.))
            .child(div().truncate().child(self.title.clone()))
    }
}

pub struct Windman {
    input: Entity<TextInput>,
    apps: Vec<AppEntry>,
    entries: Vec<Entry>,
    selected: usize,
    query: String,
    active_run: Option<usize>,
    terminal_mode: bool,
    cmd_down: bool,
    mouse_down: bool,
    scroll: f32,
    terminal_focus: gpui::FocusHandle,
    list_scroll: gpui::UniformListScrollHandle,
    tab_scroll: gpui::ScrollHandle,
    font_family: gpui::SharedString,
    font_size: f32,
    cell_width: f32,
    line_height: f32,
    terminal_cols: u16,
    terminal_rows: u16,
    history: Vec<HistoryEntry>,
    error: Option<String>,
    icon_jobs: std::collections::VecDeque<(usize, PathBuf)>,
    _subscriptions: Vec<Subscription>,
}

impl Windman {
    fn new(
        apps: Vec<AppEntry>,
        icon_jobs: Vec<(usize, PathBuf)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new("Search apps or type a command…", window, cx));
        input.read(cx).focus_handle(cx).focus(window);
        let subscriptions = vec![
            cx.observe_window_activation(window, |view, window, cx| {
                if !window.is_window_active() {
                    view.cmd_down = false;
                    view.mouse_down = false;
                }
                cx.notify();
            }),
            cx.subscribe(&input, |view, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed => {
                    view.query = view.input.read(cx).content();
                    let active_run = if view.query.is_empty() {
                        remembered_run(cx)
                    } else {
                        None
                    };
                    view.select_run(active_run, cx);
                    view.refilter(cx);
                    view.list_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                }
                TextInputEvent::Navigate(forward) => {
                    view.active_run = None;
                    if *forward {
                        if !view.entries.is_empty() {
                            view.selected = (view.selected + 1).min(view.entries.len() - 1);
                        }
                        view.show_preview(cx);
                    } else if view.selected == 0 {
                        view.restore_query(cx);
                    } else {
                        view.selected = view.selected.saturating_sub(1);
                        view.show_preview(cx);
                    }
                    view.list_scroll.scroll_to_item(
                        view.selected,
                        if *forward {
                            gpui::ScrollStrategy::Bottom
                        } else {
                            gpui::ScrollStrategy::Top
                        },
                    );
                    cx.notify();
                }
                TextInputEvent::Submitted { reveal } => view.submit(*reveal, cx),
                TextInputEvent::Dismissed => {
                    view.query.clear();
                    view.input.update(cx, |input, cx| input.set_content("", cx));
                    view.refilter(cx);
                }
            }),
        ];
        calc::ensure_rates();
        let settings = settings::Settings::load();
        let font_family: gpui::SharedString = settings.terminal_font_family.into();
        let font_size = settings.terminal_font_size;
        let (pane_width, pane_height) = terminal_pane_size(window);
        let (cell_width, cell_height) =
            measure_font_metrics(window, font_family.clone(), font_size);
        let metrics = terminal_metrics(cell_width, cell_height, pane_width, pane_height);
        let mut view = Self {
            input,
            apps,
            entries: Vec::new(),
            selected: 0,
            query: String::new(),
            active_run: None,
            terminal_mode: false,
            cmd_down: false,
            mouse_down: false,
            scroll: 0.,
            terminal_focus: cx.focus_handle(),
            list_scroll: gpui::UniformListScrollHandle::new(),
            tab_scroll: gpui::ScrollHandle::new(),
            font_family,
            font_size,
            cell_width: metrics.cell_width,
            line_height: metrics.line_height,
            terminal_cols: metrics.cols,
            terminal_rows: metrics.rows,
            history: load_history(),
            error: None,
            icon_jobs: icon_jobs.into(),
            _subscriptions: subscriptions,
        };
        view.select_run(remembered_run(cx), cx);
        view.refilter(cx);
        cx.spawn(async move |view, cx| {
            let mut last_len = 0;
            let mut last_rates = calc::version();
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let polled = view.update(cx, |view, cx| {
                    let mut generated = false;
                    for _ in 0..4 {
                        let Some((index, output)) = view.icon_jobs.pop_front() else {
                            break;
                        };
                        let Some(app) = view.apps.get_mut(index) else {
                            continue;
                        };
                        if generate_icon(&app.path, &output) {
                            app.icon = Some(output);
                            generated = true;
                        }
                    }
                    if generated {
                        cx.notify();
                    }
                    let runs = cx.global::<Runs>().0.clone();
                    let mut runs = runs.lock().unwrap();
                    let pumped = terminal_panel::pump(&mut runs);
                    let stopped = view.terminal_mode
                        && view
                            .active_run
                            .is_some_and(|active| runs.get(active).is_some_and(|run| run.done));
                    let close_after_exit = stopped
                        && view.query.is_empty()
                        && view.active_run.is_some_and(|active| {
                            runs.get(active)
                                .is_some_and(|run| run.finished_without_output())
                        });
                    let active_run = terminal_panel::prune_finished(&mut runs, view.active_run);
                    let len = runs.len();
                    drop(runs);
                    view.select_run(active_run, cx);
                    for text in pumped.clipboard {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    }
                    if stopped {
                        view.terminal_mode = false;
                        let focus = view.input.read(cx).focus_handle(cx);
                        cx.defer(move |cx| {
                            if let Some(window) = cx.windows().first().copied() {
                                let _ = window.update(cx, |_, window, _| {
                                    if close_after_exit {
                                        save_window_frame(window.bounds());
                                        window.remove_window();
                                    } else {
                                        focus.focus(window);
                                    }
                                });
                            }
                        });
                    }
                    if calc::version() != last_rates {
                        last_rates = calc::version();
                        view.refilter(cx);
                    }
                    if pumped.changed || stopped || len != last_len {
                        last_len = len;
                        cx.notify();
                    }
                });
                if polled.is_err() {
                    return;
                }
            }
        })
        .detach();
        view
    }

    fn run_command(&mut self, command: String, cx: &mut Context<Self>) {
        let theme = PaletteTheme::default();
        if let Err(error) = terminal_panel::spawn_command(
            &command,
            theme.foreground,
            theme.background,
            self.terminal_cols,
            self.terminal_rows,
            cx,
        ) {
            self.error = Some(format!("Could not run command: {error}"));
            return;
        }
        self.record_history("command", command);
        self.input
            .update(cx, |input, cx| input.set_content_quiet("", cx));
        self.query.clear();
        let active_run = cx.global::<Runs>().0.lock().unwrap().len().checked_sub(1);
        self.select_run(active_run, cx);
        self.terminal_mode = true;
        self.focus_after_submit(cx);
        self.refilter(cx);
    }

    fn show_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(preview) = self
            .entries
            .get(self.selected)
            .map(|entry| entry.preview.clone())
        {
            self.input
                .update(cx, |input, cx| input.set_preview(preview, cx));
        }
    }

    fn restore_query(&mut self, cx: &mut Context<Self>) {
        let query = self.query.clone();
        self.input
            .update(cx, |input, cx| input.set_preview(query, cx));
    }

    fn cell_width(&self) -> f32 {
        self.cell_width
    }

    fn line_height(&self) -> f32 {
        self.line_height
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.clone();
        let trimmed = query.trim();
        let command_only = trimmed.starts_with('>');
        let search = trimmed.trim_start_matches('>').trim().to_lowercase();
        let mut matches = Vec::new();
        let mut history_apps = Vec::new();
        let mut app_matches = Vec::new();
        let mut seen_apps = std::collections::BTreeSet::new();
        let mut seen_commands = std::collections::BTreeSet::new();
        let mut history_commands = Vec::new();
        let mut latest_match = None;
        if !command_only {
            for history in &self.history {
                if history.kind == "app"
                    && let Some(index) = self
                        .apps
                        .iter()
                        .position(|app| app.path.to_string_lossy() == history.value)
                {
                    let app = &self.apps[index];
                    let haystack = app.name.to_lowercase();
                    if search.is_empty() || fuzzy_score(&haystack, &search).is_some() {
                        seen_apps.insert(index);
                        latest_match.get_or_insert(false);
                        history_apps.push(Entry {
                            title: app.name.clone(),
                            subtitle: format!("Recent · {}", app.path.display()),
                            preview: app.name.clone(),
                            highlight: fuzzy_match(&app.name, &search)
                                .map(|(_, indices)| indices)
                                .unwrap_or_default(),
                            kind: EntryKind::App(index),
                        });
                    }
                } else if history.kind == "command"
                    && (search.is_empty()
                        || fuzzy_score(&history.value.to_lowercase(), &search).is_some())
                    && seen_commands.insert(history.value.clone())
                {
                    latest_match.get_or_insert(true);
                    history_commands.push(history.value.clone());
                }
            }
            let mut scored = self
                .apps
                .iter()
                .enumerate()
                .filter(|(index, _)| !seen_apps.contains(index))
                .filter_map(|(index, app)| {
                    fuzzy_score(&app.name.to_lowercase(), &search).map(|score| (score, index))
                })
                .collect::<Vec<_>>();
            scored.sort_by_key(|(score, index)| (*score, *index));
            app_matches.extend(scored.into_iter().take(48).map(|(_, index)| {
                let app = &self.apps[index];
                Entry {
                    title: app.name.clone(),
                    subtitle: app.path.display().to_string(),
                    preview: app.name.clone(),
                    highlight: fuzzy_match(&app.name, &search)
                        .map(|(_, indices)| indices)
                        .unwrap_or_default(),
                    kind: EntryKind::App(index),
                }
            }));
        }
        let command_search = trimmed.trim_start_matches('>').trim();
        let exact_command = (!command_search.is_empty()).then(|| Entry {
            title: format!("Run “{command_search}”"),
            subtitle: "Shell command".into(),
            preview: command_search.to_string(),
            highlight: command_highlight(command_search, &search),
            kind: EntryKind::Command(command_search.to_string()),
        });
        let history_command_entries = history_commands
            .into_iter()
            .filter(|command| command != command_search)
            .map(|command| Entry {
                title: format!("Run “{command}”"),
                subtitle: "Recent command".into(),
                preview: command.clone(),
                highlight: command_highlight(&command, &search),
                kind: EntryKind::Command(command),
            })
            .collect::<Vec<_>>();
        let commands_first = !command_search.is_empty() && latest_match == Some(true);
        history_apps.extend(app_matches);
        if let Some(exact) = exact_command {
            if commands_first {
                matches.push(exact);
                matches.extend(history_command_entries);
                matches.extend(history_apps);
            } else {
                matches.extend(history_apps);
                matches.push(exact);
                matches.extend(history_command_entries);
            }
        } else {
            matches.extend(history_apps);
            matches.extend(history_command_entries);
        }
        let mut preferred = 0usize;
        let mut inserted = 0;
        if !command_only && matches!(search.as_str(), "config" | "settings" | "preferences") {
            matches.insert(
                0,
                Entry {
                    title: "Windman settings".into(),
                    subtitle: settings::path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    preview: "config".into(),
                    highlight: Vec::new(),
                    kind: EntryKind::Settings,
                },
            );
            inserted += 1;
        }
        if !command_only
            && let Some(suggestion) = calc::convert(trimmed).or_else(|| calc::calculate(trimmed))
        {
            matches.insert(
                0,
                Entry {
                    title: suggestion.title,
                    subtitle: suggestion.subtitle,
                    preview: trimmed.to_string(),
                    highlight: Vec::new(),
                    kind: EntryKind::Copy(suggestion.copy),
                },
            );
            inserted += 1;
        }
        if inserted > 0 {
            preferred = 0;
        }
        self.entries = matches;
        self.selected = preferred.min(self.entries.len().saturating_sub(1));
        self.error = None;
        cx.notify();
    }

    fn modifiers_changed(
        &mut self,
        event: &gpui::ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = event.modifiers.platform;
        if command && !self.cmd_down {
            self.cmd_down = true;
            CMD_USED.store(false, std::sync::atomic::Ordering::Relaxed);
        } else if !command && self.cmd_down {
            self.cmd_down = false;
            if !CMD_USED.swap(false, std::sync::atomic::Ordering::Relaxed) {
                self.toggle_terminal_focus(window, cx);
            }
        }
    }

    fn toggle_terminal_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_terminal_focus(!self.terminal_mode, window, cx);
    }

    fn set_terminal_focus(&mut self, terminal: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal_mode = terminal
            && self.active_run.is_some_and(|index| {
                cx.global::<Runs>()
                    .0
                    .lock()
                    .unwrap()
                    .get(index)
                    .is_some_and(|run| !run.done)
            });
        self.mouse_down = false;
        if self.terminal_mode {
            self.terminal_focus.focus(window);
        } else {
            self.input.read(cx).focus_handle(cx).focus(window);
        }
        cx.notify();
    }

    fn select_run(&mut self, active_run: Option<usize>, cx: &mut Context<Self>) {
        let selection_changed = self.active_run != active_run;
        self.active_run = active_run;
        let id = active_run.and_then(|index| {
            cx.global::<Runs>()
                .0
                .lock()
                .unwrap()
                .get(index)
                .map(|run| run.id)
        });
        if let Some(id) = id {
            let remembered_changed = cx.global::<TabSelection>().0 != Some(id);
            if remembered_changed {
                cx.set_global(TabSelection(Some(id)));
            }
            if (selection_changed || remembered_changed)
                && let Some(index) = active_run
            {
                self.tab_scroll.scroll_to_item(index);
            }
        }
    }

    fn cycle_tab(&mut self, backward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = cx.global::<Runs>().0.lock().unwrap().len();
        self.select_run(adjacent_tab(self.active_run, count, backward), cx);
        self.set_terminal_focus(false, window, cx);
    }

    fn reorder_tabs(&mut self, source: u64, target: u64, cx: &mut Context<Self>) {
        let runs = cx.global::<Runs>().0.clone();
        let mut runs = runs.lock().unwrap();
        let active_id = self
            .active_run
            .and_then(|index| runs.get(index))
            .map(|run| run.id);
        if terminal_panel::reorder_tabs(&mut runs, source, target) {
            self.active_run = active_id.and_then(|id| runs.iter().position(|run| run.id == id));
            cx.notify();
        }
    }

    fn terminal_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        use std::io::Write as _;
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/wm-key.log")
        {
            let _ = writeln!(
                log,
                "key={:?} char={:?} mode={}",
                event.keystroke.key, event.keystroke.key_char, self.terminal_mode
            );
        }
        if !self.terminal_mode {
            return;
        }
        if event.keystroke.modifiers.platform {
            if let Some(bytes) = encode_command_key(&event.keystroke) {
                self.write_terminal(&bytes, cx);
            }
            return;
        }
        let application_cursor = self
            .active_run
            .and_then(|index| {
                let runs = cx.global::<Runs>().0.clone();
                let runs = runs.lock().unwrap();
                runs.get(index)?
                    .terminal
                    .mode(ghostty_vt::terminal::Mode::DECCKM)
                    .ok()
            })
            .unwrap_or(false);
        if let Some(bytes) = encode_key(&event.keystroke, application_cursor) {
            self.write_terminal(&bytes, cx);
        }
    }

    fn focus_after_submit(&mut self, cx: &mut Context<Self>) {
        let active = self.active_run;
        let terminal_focus = self.terminal_focus.clone();
        let input_focus = self.input.read(cx).focus_handle(cx);
        cx.defer(move |cx| {
            let Some(window) = cx.windows().first().copied() else {
                return;
            };
            let running = active.is_some_and(|index| {
                let runs = cx.global::<Runs>().0.clone();
                let runs = runs.lock().unwrap();
                runs.get(index).is_some_and(|run| !run.done)
            });
            let focus = if running { terminal_focus } else { input_focus };
            let _ = window.update(cx, |_, window, _| focus.focus(window));
        });
    }

    fn write_terminal(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let Some(index) = self.active_run else {
            return;
        };
        let runs = cx.global::<Runs>().0.clone();
        let runs = runs.lock().unwrap();
        if let Some(run) = runs.get(index)
            && let Ok(mut writer) = run.writer.lock()
        {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    fn terminal_mouse(
        &mut self,
        action: ghostty_vt::mouse::Action,
        button: Option<ghostty_vt::mouse::Button>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.terminal_mode {
            return;
        }
        let Some(index) = self.active_run else {
            return;
        };
        let x = (position.x - px(TERMINAL_LEFT)) / px(self.cell_width());
        let y = (position.y - px(TERMINAL_TOP)) / px(self.line_height());
        let runs = cx.global::<Runs>().0.clone();
        let runs = runs.lock().unwrap();
        let Some(bytes) = runs.get(index).and_then(|run| {
            terminal_panel::mouse_bytes(&run.terminal, action, button, x.max(0.), y.max(0.))
        }) else {
            return;
        };
        drop(runs);
        self.write_terminal(&bytes, cx);
    }

    fn terminal_scroll(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = match event.delta {
            gpui::ScrollDelta::Lines(delta) => delta.y * 3.,
            gpui::ScrollDelta::Pixels(delta) => delta.y / px(self.line_height()),
        };
        self.scroll += delta;
        let lines = self.scroll.trunc() as isize;
        self.scroll -= lines as f32;
        if lines == 0 {
            return;
        }
        let Some(index) = self.active_run else {
            return;
        };
        let x = ((event.position.x - px(TERMINAL_LEFT)) / px(self.cell_width())).max(0.);
        let y = ((event.position.y - px(TERMINAL_TOP)) / px(self.line_height())).max(0.);
        let runs = cx.global::<Runs>().0.clone();
        let mut runs = runs.lock().unwrap();
        let Some(run) = runs.get_mut(index) else {
            return;
        };
        if run.terminal.is_mouse_tracking().unwrap_or(false) {
            let button = if lines > 0 {
                ghostty_vt::mouse::Button::Four
            } else {
                ghostty_vt::mouse::Button::Five
            };
            let Some(bytes) = terminal_panel::mouse_bytes(
                &run.terminal,
                ghostty_vt::mouse::Action::Press,
                Some(button),
                x,
                y,
            ) else {
                return;
            };
            if let Ok(mut writer) = run.writer.lock() {
                let _ = writer.write_all(&bytes);
                let _ = writer.flush();
            }
        } else {
            run.terminal
                .scroll_viewport(ghostty_vt::terminal::ScrollViewport::Delta(-lines));
            cx.notify();
        }
    }

    fn submit(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };
        match entry.kind.clone() {
            EntryKind::App(index) => {
                let path = self.apps[index].path.clone();
                let mut command = Command::new("open");
                if reveal {
                    command.arg("-R");
                }
                let launched = command.arg(&path).spawn();
                if let Err(error) = launched {
                    self.error = Some(format!(
                        "Could not {} {}: {error}",
                        if reveal { "reveal" } else { "open" },
                        path.display()
                    ));
                } else {
                    self.record_history("app", path.to_string_lossy().to_string());
                    close_launcher(cx);
                }
            }
            EntryKind::Copy(value) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(value));
                close_launcher(cx);
            }
            EntryKind::Settings => open_settings(cx),
            EntryKind::Command(command) => self.run_command(command, cx),
        }
    }

    fn record_history(&mut self, kind: &str, value: String) {
        self.history
            .retain(|entry| !(entry.kind == kind && entry.value == value));
        self.history.insert(
            0,
            HistoryEntry {
                kind: kind.into(),
                value,
                used: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_secs()),
            },
        );
        self.history.truncate(100);
        save_history(&self.history);
    }
}

impl Render for Windman {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = PaletteTheme::default();
        let window_active = window.is_window_active();
        let selection_background = if window_active {
            theme.selection_background
        } else {
            theme.inactive_selection_background
        };
        let (pane_width, pane_height) = terminal_pane_size(window);
        let metrics = terminal_metrics(self.cell_width, self.line_height, pane_width, pane_height);
        if metrics.cols != self.terminal_cols || metrics.rows != self.terminal_rows {
            self.terminal_cols = metrics.cols;
            self.terminal_rows = metrics.rows;
            self.line_height = metrics.line_height;
            let runs = cx.global::<Runs>().0.clone();
            let mut runs = runs.lock().unwrap();
            for run in runs.iter_mut() {
                terminal_panel::resize(
                    run,
                    metrics.cols,
                    metrics.rows,
                    self.cell_width.ceil() as u32,
                    metrics.line_height.ceil() as u32,
                );
            }
        } else {
            self.line_height = metrics.line_height;
        }
        div()
            .size_full()
            .on_modifiers_changed(cx.listener(|view, event, window, context| {
                view.modifiers_changed(event, window, context)
            }))
            .on_action(cx.listener(|_, _: &OpenSettings, _, context| {
                open_settings(context);
            }))
            .on_action(cx.listener(|view, _: &NextTab, window, cx| {
                view.cycle_tab(false, window, cx);
            }))
            .on_action(cx.listener(|view, _: &PreviousTab, window, cx| {
                view.cycle_tab(true, window, cx);
            }))
            .on_action(cx.listener(|palette, _: &text_input::Paste, _, context| {
                if !palette.terminal_mode {
                    return;
                }
                let Some(text) = context.read_from_clipboard().and_then(|item| item.text()) else {
                    return;
                };
                let bracketed = palette
                    .active_run
                    .and_then(|index| {
                        let runs = context.global::<Runs>().0.clone();
                        let runs = runs.lock().unwrap();
                        runs.get(index)?
                            .terminal
                            .mode(ghostty_vt::terminal::Mode::BRACKETED_PASTE)
                            .ok()
                    })
                    .unwrap_or(false);
                let bytes = if bracketed {
                    format!("\x1b[200~{text}\x1b[201~")
                } else {
                    text
                };
                palette.write_terminal(bytes.as_bytes(), context);
            }))
            .on_action(
                cx.listener(|palette, _: &text_input::Dismiss, window, context| {
                    context.stop_propagation();
                    if palette.terminal_mode {
                        palette.write_terminal(b"\x1b", context);
                        return;
                    }
                    window.remove_window();
                }),
            )
            .flex()
            .flex_col()
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .border_color(rgba(
                (theme.muted << 8) | if window_active { 0x65 } else { 0x35 },
            ))
            .bg(rgba(if window_active {
                0x00000040
            } else {
                0x00000060
            }))
            .text_color(rgb(theme.foreground))
            .child(
                div()
                    .relative()
                    .h(px(54.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgba((theme.muted << 8) | 0x35))
                    .child(search_icon(theme.foreground))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(24.))
                            .child(self.input.clone()),
                    )
                    .on_any_mouse_down(cx.listener(|view, _, window, context| {
                        view.set_terminal_focus(false, window, context);
                    })),
            )
            .child(
                div()
                    .id("command-tabs")
                    .w_full()
                    .min_w_0()
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .border_b_1()
                    .border_color(rgba((theme.muted << 8) | 0x35))
                    .children({
                        let runs = cx.global::<Runs>().0.clone();
                        let runs = runs.lock().unwrap();
                        runs.iter()
                            .enumerate()
                            .map(|(index, run)| {
                                let active = self.active_run == Some(index);
                                let run_id = run.id;
                                div()
                                    .id(("run-tab", run_id))
                                    .flex_shrink_0()
                                    .whitespace_nowrap()
                                    .h(px(22.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(rgba(0x00000000))
                                    .bg(if active {
                                        selection_background
                                    } else {
                                        theme.tab_background
                                    })
                                    .when(window_active && !active, |tab| {
                                        tab.hover(|tab| tab.bg(theme.hover_background))
                                    })
                                    .text_size(px(10.))
                                    .cursor_pointer()
                                    .on_drag(
                                        DraggedTab {
                                            id: run_id,
                                            title: run.title.clone(),
                                        },
                                        |tab, _, window, cx| {
                                            window.defer(cx, |window, cx| {
                                                cx.set_active_drag_cursor_style(
                                                    gpui::CursorStyle::ClosedHand,
                                                    window,
                                                );
                                            });
                                            cx.new(|_| tab.clone())
                                        },
                                    )
                                    .drag_over::<DraggedTab>(move |style, tab, _, _| {
                                        if tab.id == run_id {
                                            style
                                        } else {
                                            style.border_color(rgba(0xffffff80))
                                        }
                                    })
                                    .on_drop(cx.listener(move |view, tab: &DraggedTab, _, cx| {
                                        view.reorder_tabs(tab.id, run_id, cx);
                                    }))
                                    .children((!run.done).then(|| {
                                        div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(0x8f4444))
                                    }))
                                    .child(
                                        div().max_w(px(220.)).truncate().child(run.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .id(("close-run", run_id))
                                            .px_1()
                                            .hover(|close| close.text_color(rgb(theme.foreground)))
                                            .child("×")
                                            .on_click(cx.listener(move |view, _, window, cx| {
                                                cx.stop_propagation();
                                                let runs = cx.global::<Runs>().0.clone();
                                                if let Some(run) = runs.lock().unwrap().get(index) {
                                                    terminate(run);
                                                }
                                                runs.lock().unwrap().remove(index);
                                                view.active_run = None;
                                                view.set_terminal_focus(false, window, cx);
                                            })),
                                    )
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.select_run(Some(index), cx);
                                        view.tab_scroll.scroll_to_item(index);
                                        view.set_terminal_focus(false, window, cx);
                                    }))
                            })
                            .collect::<Vec<_>>()
                    }),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .px_4()
                    .py_1()
                    .text_size(px(11.))
                    .text_color(rgb(theme.error))
                    .child(error)
            }))
            .child(match self.active_run {
                Some(index) => {
                    let runs = cx.global::<Runs>().0.clone();
                    let runs = runs.lock().unwrap();
                    let running = runs.get(index).is_some_and(|run| !run.done);
                    let rows = runs
                        .get(index)
                        .map(|run| {
                            terminal_panel::render_rows(
                                &run.terminal,
                                window_active && self.terminal_mode,
                            )
                        })
                        .unwrap_or_default();
                    let empty = rows.is_empty();
                    div()
                        .id("terminal-output")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p_3()
                        .when(running, |output| output.track_focus(&self.terminal_focus))
                        .on_key_down(
                            cx.listener(|view, event, _, context| {
                                view.terminal_key(event, context)
                            }),
                        )
                        .on_any_mouse_down(cx.listener(
                            move |view, event: &gpui::MouseDownEvent, window, context| {
                                window.prevent_default();
                                view.set_terminal_focus(true, window, context);
                                view.mouse_down =
                                    view.terminal_mode && event.button == MouseButton::Left;
                                if view.mouse_down {
                                    view.terminal_mouse(
                                        ghostty_vt::mouse::Action::Press,
                                        Some(ghostty_vt::mouse::Button::Left),
                                        event.position,
                                        context,
                                    );
                                }
                            },
                        ))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|view, event: &gpui::MouseUpEvent, _, context| {
                                view.mouse_down = false;
                                view.terminal_mouse(
                                    ghostty_vt::mouse::Action::Release,
                                    Some(ghostty_vt::mouse::Button::Left),
                                    event.position,
                                    context,
                                );
                            }),
                        )
                        .on_mouse_move(cx.listener(
                            |view, event: &gpui::MouseMoveEvent, _, context| {
                                if view.mouse_down {
                                    view.terminal_mouse(
                                        ghostty_vt::mouse::Action::Motion,
                                        Some(ghostty_vt::mouse::Button::Left),
                                        event.position,
                                        context,
                                    );
                                }
                            },
                        ))
                        .on_scroll_wheel(cx.listener(
                            |view, event: &gpui::ScrollWheelEvent, _, context| {
                                view.terminal_scroll(event, context)
                            },
                        ))
                        .border_1()
                        .border_color(if window_active && self.terminal_mode {
                            rgba((theme.muted << 8) | 0x80)
                        } else {
                            rgba(0x00000000)
                        })
                        .font_family(self.font_family.clone())
                        .text_size(px(self.font_size))
                        .children(empty.then(|| {
                            div()
                                .text_color(rgb(theme.secondary_foreground))
                                .child("Waiting for output…")
                        }))
                        .children(rows.into_iter().map(|(row, shapes)| {
                            let (text, runs) =
                                terminal_row(row, theme.background, self.font_family.clone());
                            div()
                                .relative()
                                .h(px(self.line_height()))
                                .line_height(px(self.line_height()))
                                .whitespace_nowrap()
                                .child(gpui::StyledText::new(text).with_runs(runs))
                                .children(shapes.into_iter().flat_map(|shape| {
                                    terminal_shape(
                                        shape,
                                        self.cell_width(),
                                        self.line_height(),
                                        theme.background,
                                    )
                                }))
                        }))
                        .into_any_element()
                }
                None => uniform_list(
                    "launcher-results",
                    self.entries.len(),
                    cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let entry = &view.entries[index];
                                let theme = PaletteTheme::default();
                                let selected = index == view.selected;
                                div()
                                    .id(index)
                                    .w_full()
                                    .h(px(42.))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .when(selected, |row| row.bg(selection_background))
                                    .when(window_active && !selected, |row| {
                                        row.hover(|row| row.bg(theme.hover_background))
                                    })
                                    .cursor_pointer()
                                    .child(match &entry.kind {
                                        EntryKind::App(index) => {
                                            view.apps[*index].icon.as_ref().map_or_else(
                                                || div().w(px(36.)).h(px(36.)).into_any_element(),
                                                |icon| {
                                                    img(icon.clone())
                                                        .w(px(36.))
                                                        .h(px(36.))
                                                        .into_any_element()
                                                },
                                            )
                                        }
                                        EntryKind::Command(_) => div()
                                            .w(px(36.))
                                            .h(px(36.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(16.))
                                            .text_color(rgb(theme.secondary_foreground))
                                            .child(">_")
                                            .into_any_element(),
                                        EntryKind::Copy(_) => div()
                                            .w(px(36.))
                                            .h(px(36.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(22.))
                                            .text_color(rgb(theme.secondary_foreground))
                                            .child("=")
                                            .into_any_element(),
                                        EntryKind::Settings => div()
                                            .w(px(36.))
                                            .h(px(36.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(22.))
                                            .text_color(rgb(theme.secondary_foreground))
                                            .child("⚙")
                                            .into_any_element(),
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(highlighted_title(
                                                &entry.title,
                                                &entry.highlight,
                                                13.,
                                            ))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(11.))
                                                    .text_color(rgb(theme.secondary_foreground))
                                                    .child(entry.subtitle.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(rgba((theme.muted << 8) | 0x45))
                                            .text_size(px(11.))
                                            .text_color(rgb(theme.secondary_foreground))
                                            .child(match &entry.kind {
                                                EntryKind::App(_) => "OPEN",
                                                EntryKind::Command(_) => "RUN",
                                                EntryKind::Copy(_) => "COPY",
                                                EntryKind::Settings => "EDIT",
                                            }),
                                    )
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.selected = index;
                                        view.submit(false, cx);
                                    }))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.list_scroll.clone())
                .w_full()
                .flex_1()
                .min_h_0()
                .into_any_element(),
            })
            .children(self.entries.is_empty().then(|| {
                div()
                    .h(px(60.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(theme.secondary_foreground))
                    .child(if self.query.trim().is_empty() {
                        "Type to search apps"
                    } else {
                        "No matching applications"
                    })
            }))
            .child(
                div()
                    .h(px(30.))
                    .px_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(rgba((theme.muted << 8) | 0x35))
                    .text_size(px(11.))
                    .text_color(rgb(theme.secondary_foreground))
                    .child(format!("{} results", self.entries.len()))
                    .child("↑ ↓ navigate    ↵ open / run    ⌘, settings    esc dismiss"),
            )
    }
}

impl Focusable for Windman {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

struct PaletteTheme {
    background: u32,
    selection_background: gpui::Rgba,
    inactive_selection_background: gpui::Rgba,
    hover_background: gpui::Rgba,
    tab_background: gpui::Rgba,
    foreground: u32,
    secondary_foreground: u32,
    muted: u32,
    error: u32,
}

impl Default for PaletteTheme {
    fn default() -> Self {
        Self {
            background: 0x222222,
            selection_background: rgba(0x00000040),
            inactive_selection_background: rgba(0x00000020),
            hover_background: rgba(0x00000028),
            tab_background: rgba(0x00000018),
            foreground: 0xdbd7ca,
            secondary_foreground: 0xc8c8c8,
            muted: 0x85847f,
            error: 0xcb7676,
        }
    }
}

fn search_icon(color: u32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let position = |x, y| point(bounds.left() + px(x), bounds.top() + px(y));
            let mut lens = gpui::PathBuilder::stroke(px(1.5));
            let radius = point(px(4.5), px(4.5));
            lens.move_to(position(11., 6.5));
            lens.arc_to(radius, px(0.), false, true, position(2., 6.5));
            lens.arc_to(radius, px(0.), false, true, position(11., 6.5));
            lens.close();
            if let Ok(path) = lens.build() {
                window.paint_path(path, rgb(color));
            }

            let mut handle = gpui::PathBuilder::fill();
            let cap_radius = point(px(0.75), px(0.75));
            handle.move_to(position(9.22, 10.28));
            handle.line_to(position(12.97, 14.03));
            handle.arc_to(cap_radius, px(0.), false, false, position(14.03, 12.97));
            handle.line_to(position(10.28, 9.22));
            handle.arc_to(cap_radius, px(0.), false, false, position(9.22, 10.28));
            handle.close();
            if let Ok(path) = handle.build() {
                window.paint_path(path, rgb(color));
            }
        },
    )
    .size(px(16.))
    .flex_shrink_0()
}

fn scan_apps() -> (Vec<AppEntry>, Vec<(usize, PathBuf)>) {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    let mut apps = Vec::new();
    for root in roots {
        scan_root(&root, &mut apps);
    }
    apps.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    apps.dedup_by(|left, right| left.name.eq_ignore_ascii_case(&right.name));
    apps.truncate(4096);

    let cache = icon_cache_dir();
    if let Some(cache) = &cache {
        let _ = std::fs::create_dir_all(cache);
    }
    let mut jobs = Vec::new();
    for (index, app) in apps.iter_mut().enumerate() {
        let Some(cache) = cache.as_deref() else {
            continue;
        };
        let output = cache_icon_path(&app.path, cache);
        let source = newest_modified(&app.path);
        if output.is_file() && icon_is_usable(&output) {
            let png = std::fs::metadata(&output)
                .ok()
                .and_then(|meta| meta.modified().ok());
            if source.zip(png).is_none_or(|(source, png)| source <= png) {
                app.icon = Some(output);
            } else {
                let _ = generate_icon(&app.path, &output);
                app.icon = Some(output);
            }
            continue;
        }
        jobs.push((index, output));
    }
    (apps, jobs)
}

fn scan_root(root: &Path, apps: &mut Vec<AppEntry>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    entries.take(4096).flatten().for_each(|entry| {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "app") {
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
                apps.push(AppEntry {
                    name: name.to_string(),
                    path,
                    icon: None,
                });
            }
        } else if !entry.file_name().to_string_lossy().starts_with('.')
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            scan_root(&path, apps);
        }
    });
}

fn icon_cache_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches/windman/icons"))
}

fn icon_stem(path: &Path) -> String {
    path.file_stem()
        .map(|name| {
            name.to_string_lossy()
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                        character
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .unwrap_or_else(|| "app".into())
}

fn path_hash(path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

fn cache_icon_path(path: &Path, cache: &Path) -> PathBuf {
    cache.join(format!("{}-{:x}.png", icon_stem(path), path_hash(path)))
}

fn newest_modified(path: &Path) -> Option<std::time::SystemTime> {
    [
        path.join("Contents/Info.plist"),
        path.join("Contents/Resources"),
        path.to_path_buf(),
    ]
    .iter()
    .filter_map(|candidate| {
        std::fs::metadata(candidate)
            .and_then(|meta| meta.modified())
            .ok()
    })
    .max()
}

fn icon_is_usable(path: &Path) -> bool {
    png_size(path).is_some_and(|(width, height)| width >= 96 && height >= 96)
}

fn png_size(path: &Path) -> Option<(u32, u32)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 24 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(data[20..24].try_into().ok()?);
    Some((width, height))
}

fn generate_icon(path: &Path, output: &Path) -> bool {
    #[cfg(target_os = "macos")]
    if macos::app_icon(path, output) {
        return true;
    }
    let resources = path.join("Contents/Resources");
    let info = plist::Value::from_file(path.join("Contents/Info.plist")).ok();
    let named = info
        .as_ref()
        .and_then(plist::Value::as_dictionary)
        .and_then(|info| {
            info.get("CFBundleIconFile")
                .or_else(|| info.get("CFBundleIconName"))
                .and_then(plist::Value::as_string)
        });
    let icon = named
        .map(|name| {
            let mut icon = resources.join(name);
            if icon.extension().is_none() {
                icon.set_extension("icns");
            }
            icon
        })
        .filter(|icon| icon.is_file())
        .or_else(|| {
            std::fs::read_dir(&resources)
                .ok()?
                .take(512)
                .flatten()
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "icns")
                })
        });
    let Some(icon) = icon else {
        return false;
    };
    let status = Command::new("sips")
        .arg("-z")
        .arg("128")
        .arg("128")
        .arg("-s")
        .arg("format")
        .arg("png")
        .arg(icon)
        .arg("--out")
        .arg(output)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok();
    status.is_some_and(|status| status.success()) && output.is_file()
}

fn highlighted_title(title: &str, highlights: &[usize], size: f32) -> gpui::AnyElement {
    let highlighted = highlights
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let text = gpui::StyledText::new(title.to_string()).with_highlights(
        title
            .char_indices()
            .filter(|(index, _)| highlighted.contains(index))
            .map(|(index, character)| {
                (
                    index..index + character.len_utf8(),
                    gpui::HighlightStyle {
                        font_weight: Some(gpui::FontWeight::BOLD),
                        ..Default::default()
                    },
                )
            }),
    );
    div()
        .w_full()
        .truncate()
        .text_size(px(size))
        .child(text)
        .into_any_element()
}

fn fuzzy_match(text: &str, query: &str) -> Option<(usize, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let mut remaining = text.char_indices();
    let mut indices = Vec::new();
    let mut score = 0;
    for wanted in query.chars().filter(|character| !character.is_whitespace()) {
        let (index, _) =
            remaining.find(|(_, character)| character.to_lowercase().eq(wanted.to_lowercase()))?;
        score += index;
        indices.push(index);
    }
    Some((score, indices))
}

fn fuzzy_score(text: &str, query: &str) -> Option<usize> {
    fuzzy_match(text, query).map(|(score, _)| score)
}

fn command_highlight(command: &str, query: &str) -> Vec<usize> {
    let offset = "Run “".len();
    fuzzy_match(command, query)
        .map(|(_, indices)| indices.into_iter().map(|index| index + offset).collect())
        .unwrap_or_default()
}

pub(crate) fn data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/windman"))
    } else {
        Some(PathBuf::from(std::env::var_os("XDG_CONFIG_HOME")?).join("windman"))
    }
}

fn load_history() -> Vec<HistoryEntry> {
    let Some(path) = data_dir().map(|directory| directory.join("history.json")) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

fn save_history(history: &[HistoryEntry]) {
    let Some(directory) = data_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&directory);
    if let Ok(data) = serde_json::to_string(history) {
        let _ = std::fs::write(directory.join("history.json"), data);
    }
}

fn window_state_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/windman")
    } else {
        PathBuf::from(std::env::var_os("XDG_CONFIG_HOME")?).join("windman")
    };
    Some(base.join("window.json"))
}

fn load_window_frame() -> Option<(gpui::Point<gpui::Pixels>, gpui::Size<gpui::Pixels>)> {
    let data = std::fs::read_to_string(window_state_path()?).ok()?;
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    Some((
        point(
            px(value.get("x")?.as_f64()? as f32),
            px(value.get("y")?.as_f64()? as f32),
        ),
        size(
            px(value
                .get("w")
                .and_then(|value| value.as_f64())
                .unwrap_or(640.) as f32),
            px(value
                .get("h")
                .and_then(|value| value.as_f64())
                .unwrap_or(440.) as f32),
        ),
    ))
}

fn save_window_frame(frame: Bounds<gpui::Pixels>) {
    let Some(path) = window_state_path() else {
        return;
    };
    if let Some(directory) = path.parent() {
        let _ = std::fs::create_dir_all(directory);
    }
    let data = format!(
        "{{\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}",
        f32::from(frame.origin.x),
        f32::from(frame.origin.y),
        f32::from(frame.size.width),
        f32::from(frame.size.height)
    );
    let _ = std::fs::write(path, data);
}

fn panel_options(bounds: Bounds<gpui::Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui::TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(-100.), px(-100.))),
        }),
        kind: WindowKind::PopUp,
        is_movable: true,
        is_resizable: true,
        window_min_size: Some(size(px(420.), px(320.))),
        is_minimizable: false,
        window_background: if cfg!(target_os = "macos") {
            WindowBackgroundAppearance::Transparent
        } else {
            WindowBackgroundAppearance::Blurred
        },
        ..Default::default()
    }
}

fn acquire_single_instance() -> bool {
    let Some(directory) = data_dir() else {
        return true;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return true;
    }
    let path = directory.join("instance.lock");
    for attempt in 0..3 {
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
        else {
            return true;
        };
        if fs2::FileExt::try_lock_exclusive(&file).is_ok() {
            let _ = std::fs::write(&path, std::process::id().to_string());
            std::mem::forget(file);
            return true;
        }
        let old = std::fs::read_to_string(&path)
            .ok()
            .and_then(|pid| pid.trim().parse::<u32>().ok());
        if let Some(old) = old.filter(|pid| *pid != std::process::id()) {
            eprintln!("Replacing running Windman pid {old}");
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(old.to_string())
                .status();
            std::thread::sleep(std::time::Duration::from_millis(300 * (attempt + 1)));
        } else {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    eprintln!("Windman could not replace the existing instance");
    false
}

struct TerminalMetrics {
    cols: u16,
    rows: u16,
    cell_width: f32,
    line_height: f32,
}

fn terminal_metrics(
    cell_width: f32,
    cell_height: f32,
    pane_width: f32,
    pane_height: f32,
) -> TerminalMetrics {
    let rows = (pane_height / cell_height).floor().max(4.) as u16;
    let cols = (pane_width / cell_width).floor().max(20.) as u16;
    TerminalMetrics {
        cols,
        rows,
        cell_width,
        line_height: cell_height,
    }
}

fn terminal_pane_size(window: &Window) -> (f32, f32) {
    let size = window.bounds().size;
    (
        (f32::from(size.width) - 28.).max(120.),
        (f32::from(size.height) - 140.).max(40.),
    )
}

fn measure_font_metrics(
    window: &mut Window,
    font_family: gpui::SharedString,
    font_size: f32,
) -> (f32, f32) {
    let probe = "M".repeat(100);
    let run = gpui::TextRun {
        len: probe.len(),
        font: gpui::Font {
            family: font_family,
            features: Default::default(),
            fallbacks: None,
            weight: gpui::FontWeight::NORMAL,
            style: gpui::FontStyle::Normal,
        },
        color: rgb(0xffffff).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(probe.into(), px(font_size), &[run], None);
    let width = (f32::from(line.width) / 100.).max(1.);
    let height = (f32::from(line.ascent) + f32::from(line.descent))
        .ceil()
        .max(font_size);
    (width, height)
}

fn cursor_key(suffix: &str, application: bool, modifier: u8) -> Option<Vec<u8>> {
    if application && modifier == 1 {
        return Some(format!("\x1bO{suffix}").into_bytes());
    }
    Some(
        if modifier > 1 {
            format!("\x1b[1;{modifier}{suffix}")
        } else {
            format!("\x1b[{suffix}")
        }
        .into_bytes(),
    )
}

fn encode_command_key(keystroke: &gpui::Keystroke) -> Option<Vec<u8>> {
    match keystroke.key.as_str() {
        "backspace" => Some(vec![0x15]),
        "delete" => Some(vec![0x0b]),
        "left" => Some(vec![0x01]),
        "right" => Some(vec![0x05]),
        _ => None,
    }
}

fn control_byte(key: &str) -> Option<u8> {
    match key {
        "space" | "2" | "@" => Some(0x00),
        "3" | "[" => Some(0x1b),
        "4" | "\\" => Some(0x1c),
        "5" | "]" => Some(0x1d),
        "6" | "^" => Some(0x1e),
        "7" | "-" | "_" => Some(0x1f),
        "8" | "?" => Some(0x7f),
        key if key.len() == 1 => {
            let character = key.chars().next()?.to_ascii_uppercase();
            character
                .is_ascii_uppercase()
                .then(|| (character as u8) & 0x1f)
        }
        _ => None,
    }
}

fn terminal_shape(
    shape: terminal_panel::CellShape,
    cell_width: f32,
    line_height: f32,
    default_background: u32,
) -> Vec<gpui::AnyElement> {
    let x = shape.x as f32 * cell_width;
    let fg = shape.fg.hex();
    let mut rects: Vec<(f32, f32, f32, f32, u32)> = Vec::new();
    if shape.bg.hex() != default_background {
        rects.push((x, 0., cell_width, line_height, shape.bg.hex()));
    }
    if matches!(shape.glyph, '\u{2591}'..='\u{2593}') {
        let amount = match shape.glyph {
            '\u{2591}' => 0.25,
            '\u{2592}' => 0.5,
            _ => 0.75,
        };
        rects.push((
            x,
            0.,
            cell_width,
            line_height,
            mix(shape.fg, shape.bg, amount).hex(),
        ));
    } else if let Some(parts) = block_rects(shape.glyph) {
        for (left, top, width, height) in parts {
            rects.push((
                x + left * cell_width,
                top * line_height,
                width * cell_width,
                height * line_height,
                fg,
            ));
        }
    } else if let Some((left, right, up, down)) = box_connections(shape.glyph) {
        let thickness = (cell_width * 0.16).max(1.2);
        let center_x = cell_width / 2.;
        let center_y = line_height / 2.;
        if left || right {
            let start = if left { 0. } else { center_x - thickness / 2. };
            let end = if right {
                cell_width
            } else {
                center_x + thickness / 2.
            };
            rects.push((
                x + start,
                center_y - thickness / 2.,
                end - start,
                thickness,
                fg,
            ));
        }
        if up || down {
            let start = if up { 0. } else { center_y - thickness / 2. };
            let end = if down {
                line_height
            } else {
                center_y + thickness / 2.
            };
            rects.push((
                x + center_x - thickness / 2.,
                start,
                thickness,
                end - start,
                fg,
            ));
        }
    }
    rects
        .into_iter()
        .map(|(left, top, width, height, color)| {
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .h(px(height))
                .bg(rgb(color))
                .into_any_element()
        })
        .collect()
}

fn mix(
    foreground: terminal_panel::Color,
    background: terminal_panel::Color,
    amount: f32,
) -> terminal_panel::Color {
    let blend = |fg: u8, bg: u8| (bg as f32 + (fg as f32 - bg as f32) * amount).round() as u8;
    terminal_panel::Color {
        r: blend(foreground.r, background.r),
        g: blend(foreground.g, background.g),
        b: blend(foreground.b, background.b),
    }
}

fn block_rects(glyph: char) -> Option<Vec<(f32, f32, f32, f32)>> {
    Some(match glyph {
        '\u{2580}' => vec![(0., 0., 1., 0.5)],
        '\u{2581}' => vec![(0., 0.875, 1., 0.125)],
        '\u{2582}' => vec![(0., 0.75, 1., 0.25)],
        '\u{2583}' => vec![(0., 0.625, 1., 0.375)],
        '\u{2584}' => vec![(0., 0.5, 1., 0.5)],
        '\u{2585}' => vec![(0., 0.375, 1., 0.625)],
        '\u{2586}' => vec![(0., 0.25, 1., 0.75)],
        '\u{2587}' => vec![(0., 0.125, 1., 0.875)],
        '\u{2588}' => vec![(0., 0., 1., 1.)],
        '\u{2589}' => vec![(0., 0., 0.875, 1.)],
        '\u{258A}' => vec![(0., 0., 0.75, 1.)],
        '\u{258B}' => vec![(0., 0., 0.625, 1.)],
        '\u{258C}' => vec![(0., 0., 0.5, 1.)],
        '\u{258D}' => vec![(0., 0., 0.375, 1.)],
        '\u{258E}' => vec![(0., 0., 0.25, 1.)],
        '\u{258F}' => vec![(0., 0., 0.125, 1.)],
        '\u{2590}' => vec![(0.5, 0., 0.5, 1.)],
        '\u{2594}' => vec![(0., 0., 1., 0.125)],
        '\u{2595}' => vec![(0.875, 0., 0.125, 1.)],
        '\u{2596}' => vec![(0., 0.5, 0.5, 0.5)],
        '\u{2597}' => vec![(0.5, 0.5, 0.5, 0.5)],
        '\u{2598}' => vec![(0., 0., 0.5, 0.5)],
        '\u{2599}' => vec![
            (0., 0., 0.5, 0.5),
            (0., 0.5, 0.5, 0.5),
            (0.5, 0.5, 0.5, 0.5),
        ],
        '\u{259A}' => vec![(0., 0., 0.5, 0.5), (0.5, 0.5, 0.5, 0.5)],
        '\u{259B}' => vec![(0., 0., 0.5, 0.5), (0.5, 0., 0.5, 0.5), (0., 0.5, 0.5, 0.5)],
        '\u{259C}' => vec![
            (0., 0., 0.5, 0.5),
            (0.5, 0., 0.5, 0.5),
            (0.5, 0.5, 0.5, 0.5),
        ],
        '\u{259D}' => vec![(0.5, 0., 0.5, 0.5)],
        '\u{259E}' => vec![(0.5, 0., 0.5, 0.5), (0., 0.5, 0.5, 0.5)],
        '\u{259F}' => vec![
            (0.5, 0., 0.5, 0.5),
            (0., 0.5, 0.5, 0.5),
            (0.5, 0.5, 0.5, 0.5),
        ],
        _ => return None,
    })
}

fn box_connections(glyph: char) -> Option<(bool, bool, bool, bool)> {
    Some(match glyph as u32 {
        0x2500 | 0x2501 | 0x2550 => (true, true, false, false),
        0x2502 | 0x2503 | 0x2551 => (false, false, true, true),
        0x250C..=0x250F | 0x2552..=0x2554 | 0x256D => (false, true, false, true),
        0x2510..=0x2513 | 0x2555..=0x2557 | 0x256E => (true, false, false, true),
        0x2514..=0x2517 | 0x2558..=0x255A | 0x2570 => (false, true, true, false),
        0x2518..=0x251B | 0x255B..=0x255D | 0x256F => (true, false, true, false),
        0x251C..=0x2523 | 0x255E..=0x2560 => (false, true, true, true),
        0x2524..=0x252B | 0x2561..=0x2563 => (true, false, true, true),
        0x252C..=0x2533 | 0x2564..=0x2566 => (true, true, false, true),
        0x2534..=0x253B | 0x2567..=0x2569 => (true, true, true, false),
        0x253C..=0x254B | 0x256A..=0x256C => (true, true, true, true),
        0x2574 | 0x2578 => (true, false, false, false),
        0x2575 | 0x2579 => (false, false, true, false),
        0x2576 | 0x257A => (false, true, false, false),
        0x2577 | 0x257B => (false, false, false, true),
        0x257C | 0x257E => (true, true, false, false),
        0x257D | 0x257F => (false, false, true, true),
        _ => return None,
    })
}

fn terminal_row(
    row: Vec<terminal_panel::CellRun>,
    default_background: u32,
    font_family: gpui::SharedString,
) -> (gpui::SharedString, Vec<gpui::TextRun>) {
    let mut text = String::new();
    let mut runs = Vec::new();
    for (chunk, style) in row {
        if chunk.is_empty() {
            continue;
        }
        let color = rgb(style.fg.hex());
        runs.push(gpui::TextRun {
            len: chunk.len(),
            font: gpui::Font {
                family: font_family.clone(),
                features: Default::default(),
                fallbacks: None,
                weight: if style.bold {
                    gpui::FontWeight::BOLD
                } else {
                    gpui::FontWeight::NORMAL
                },
                style: if style.italic {
                    gpui::FontStyle::Italic
                } else {
                    gpui::FontStyle::Normal
                },
            },
            color: color.into(),
            background_color: (style.bg.hex() != default_background)
                .then(|| rgb(style.bg.hex()).into()),
            underline: style.underline.then(|| gpui::UnderlineStyle {
                color: Some(color.into()),
                thickness: px(1.),
                wavy: false,
            }),
            strikethrough: None,
        });
        text.push_str(&chunk);
    }
    (text.into(), runs)
}

fn encode_key(keystroke: &gpui::Keystroke, application_cursor: bool) -> Option<Vec<u8>> {
    let modifiers = keystroke.modifiers;
    let modifier =
        1 + modifiers.shift as u8 + 2 * modifiers.alt as u8 + 4 * modifiers.control as u8;
    let csi = |suffix: &str| {
        Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{suffix}")
            } else {
                format!("\x1b[{suffix}")
            }
            .into_bytes(),
        )
    };
    let tilde = |code: u8| {
        Some(
            if modifier > 1 {
                format!("\x1b[{code};{modifier}~")
            } else {
                format!("\x1b[{code}~")
            }
            .into_bytes(),
        )
    };
    if modifiers.alt && !modifiers.control && !modifiers.shift {
        match keystroke.key.as_str() {
            "left" => return Some(b"\x1b[1;5D".to_vec()),
            "right" => return Some(b"\x1b[1;5C".to_vec()),
            _ => {}
        }
    }
    match keystroke.key.as_str() {
        "enter" if modifiers.alt => Some(b"\x1b\r".to_vec()),
        "enter" => Some(b"\r".to_vec()),
        "tab" if modifiers.shift => Some(b"\x1b[Z".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "escape" if modifiers.alt => Some(b"\x1b\x1b".to_vec()),
        "escape" => Some(b"\x1b".to_vec()),
        "backspace" if modifiers.alt => Some(b"\x1b\x7f".to_vec()),
        "backspace" => Some(b"\x7f".to_vec()),
        "delete" => tilde(3),
        "insert" => tilde(2),
        "left" => cursor_key("D", application_cursor, modifier),
        "right" => cursor_key("C", application_cursor, modifier),
        "up" => cursor_key("A", application_cursor, modifier),
        "down" => cursor_key("B", application_cursor, modifier),
        "home" => csi("H"),
        "end" => csi("F"),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => Some(b"\x1bOP".to_vec()),
        "f2" => Some(b"\x1bOQ".to_vec()),
        "f3" => Some(b"\x1bOR".to_vec()),
        "f4" => Some(b"\x1bOS".to_vec()),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ => {
            if modifiers.control {
                let byte = keystroke
                    .key_char
                    .as_ref()
                    .and_then(|text| text.chars().next())
                    .filter(char::is_ascii)
                    .map(|character| (character.to_ascii_uppercase() as u8) & 0x1f)
                    .or_else(|| control_byte(&keystroke.key))?;
                return Some(vec![byte]);
            }
            let character = keystroke.key_char.as_ref()?.chars().next()?;
            let mut bytes = Vec::new();
            if modifiers.alt {
                bytes.push(0x1b);
            }
            let mut buffer = [0u8; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            Some(bytes)
        }
    }
}

fn remembered_run(cx: &App) -> Option<usize> {
    let runs = cx.global::<Runs>().0.lock().unwrap();
    cx.global::<TabSelection>().restore(&runs)
}

fn adjacent_tab(active: Option<usize>, count: usize, backward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match active.filter(|index| *index < count) {
        Some(0) if backward => Some(count - 1),
        Some(index) if backward => Some(index - 1),
        Some(index) => Some((index + 1) % count),
        None if backward => Some(count - 1),
        None => Some(0),
    }
}

fn open_launcher(cx: &mut App) {
    let (apps, icon_jobs) = scan_apps();
    let launcher_bounds = load_window_frame().map_or_else(
        || Bounds::centered(None, size(px(640.), px(440.)), cx),
        |(origin, window_size)| Bounds::new(origin, window_size),
    );
    let launcher = cx
        .open_window(panel_options(launcher_bounds), move |window, cx| {
            cx.new(|cx| Windman::new(apps, icon_jobs, window, cx))
        })
        .unwrap();
    launcher
        .update(cx, |view, window, cx| {
            #[cfg(target_os = "macos")]
            macos::configure_window(window);
            view.input.read(cx).focus_handle(cx).focus(window);
            window.on_window_should_close(cx, |window, _| {
                save_window_frame(window.bounds());
                true
            });
            cx.activate(true);
        })
        .unwrap();
    let watched = launcher;
    cx.spawn(async move |cx| {
        let mut last = None;
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let frame = watched.update(cx, |_, window, _| window.bounds()).ok();
            if let Some(frame) = frame
                && last != Some(frame)
            {
                save_window_frame(frame);
                last = Some(frame);
            }
        }
    })
    .detach();
}

fn open_settings(cx: &mut App) {
    let _ = settings::Settings::load();
    if let Some(path) = settings::path() {
        let _ = Command::new("open").arg("-t").arg(path).spawn();
    }
    close_launcher(cx);
}

fn close_launcher(cx: &mut App) {
    cx.defer(|cx| {
        let Some(window) = cx.windows().first().copied() else {
            return;
        };
        match window.update(cx, |_, window, _| window.remove_window()) {
            Ok(()) => {}
            Err(error) => eprintln!("close: remove failed: {error}"),
        }
    });
}

fn toggle_launcher(cx: &mut App) {
    if cx.windows().is_empty() {
        open_launcher(cx);
        return;
    }
    close_launcher(cx);
}

fn main() {
    Application::new().run(|cx: &mut App| {
        #[cfg(target_os = "macos")]
        {
            macos::dark_appearance();
            macos::accessory_app();
        }
        if !acquire_single_instance() {
            std::process::exit(0);
        }
        cx.set_global(Runs::default());
        cx.set_global(TabSelection::default());
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            gpui::KeyBinding::new("cmd-q", gpui::NoAction, None),
            gpui::KeyBinding::new("cmd-,", OpenSettings, None),
            gpui::KeyBinding::new("tab", NextTab, Some("TextInput")),
            gpui::KeyBinding::new("shift-tab", PreviousTab, Some("TextInput")),
            gpui::KeyBinding::new("up", text_input::MoveUp, Some("TextInput")),
            gpui::KeyBinding::new("down", text_input::MoveDown, Some("TextInput")),
            gpui::KeyBinding::new("enter", text_input::Submit, Some("TextInput")),
            gpui::KeyBinding::new("cmd-enter", text_input::Submit, Some("TextInput")),
            gpui::KeyBinding::new("escape", text_input::Dismiss, None),
            gpui::KeyBinding::new("backspace", text_input::Backspace, Some("TextInput")),
            gpui::KeyBinding::new("delete", text_input::Delete, Some("TextInput")),
            gpui::KeyBinding::new("left", text_input::Left, Some("TextInput")),
            gpui::KeyBinding::new("right", text_input::Right, Some("TextInput")),
            gpui::KeyBinding::new("shift-left", text_input::SelectLeft, Some("TextInput")),
            gpui::KeyBinding::new("shift-right", text_input::SelectRight, Some("TextInput")),
            gpui::KeyBinding::new("alt-left", text_input::WordLeft, Some("TextInput")),
            gpui::KeyBinding::new("alt-right", text_input::WordRight, Some("TextInput")),
            gpui::KeyBinding::new(
                "alt-shift-left",
                text_input::SelectWordLeft,
                Some("TextInput"),
            ),
            gpui::KeyBinding::new(
                "alt-shift-right",
                text_input::SelectWordRight,
                Some("TextInput"),
            ),
            gpui::KeyBinding::new(
                "alt-backspace",
                text_input::DeleteWordBackward,
                Some("TextInput"),
            ),
            gpui::KeyBinding::new(
                "alt-delete",
                text_input::DeleteWordForward,
                Some("TextInput"),
            ),
            gpui::KeyBinding::new("cmd-left", text_input::Home, Some("TextInput")),
            gpui::KeyBinding::new("cmd-right", text_input::End, Some("TextInput")),
            gpui::KeyBinding::new("cmd-up", text_input::Home, Some("TextInput")),
            gpui::KeyBinding::new("cmd-down", text_input::End, Some("TextInput")),
            gpui::KeyBinding::new("cmd-shift-left", text_input::SelectHome, Some("TextInput")),
            gpui::KeyBinding::new("cmd-shift-right", text_input::SelectEnd, Some("TextInput")),
            gpui::KeyBinding::new(
                "cmd-backspace",
                text_input::DeleteToStart,
                Some("TextInput"),
            ),
            gpui::KeyBinding::new("cmd-delete", text_input::DeleteToEnd, Some("TextInput")),
            gpui::KeyBinding::new("shift-home", text_input::SelectHome, Some("TextInput")),
            gpui::KeyBinding::new("shift-end", text_input::SelectEnd, Some("TextInput")),
            gpui::KeyBinding::new("cmd-a", text_input::SelectAll, Some("TextInput")),
            gpui::KeyBinding::new("cmd-v", text_input::Paste, None),
            gpui::KeyBinding::new("cmd-c", text_input::Copy, Some("TextInput")),
            gpui::KeyBinding::new("cmd-x", text_input::Cut, Some("TextInput")),
            gpui::KeyBinding::new("home", text_input::Home, Some("TextInput")),
            gpui::KeyBinding::new("end", text_input::End, Some("TextInput")),
        ]);
        cx.observe_keystrokes(|event, _, _| {
            if event.keystroke.modifiers.platform {
                CMD_USED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .detach();
        open_launcher(cx);
        if let Ok(manager) = global_hotkey::GlobalHotKeyManager::new() {
            match manager.register(global_hotkey::hotkey::HotKey::new(
                Some(global_hotkey::hotkey::Modifiers::META),
                global_hotkey::hotkey::Code::Space,
            )) {
                Ok(()) => eprintln!("Cmd+Space registered"),
                Err(error) => eprintln!("Cmd+Space registration failed: {error}"),
            }
            std::mem::forget(manager);
        }
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                while let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state == global_hotkey::HotKeyState::Pressed {
                        let _ = cx.update(|cx| toggle_launcher(cx));
                    }
                }
            }
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_score, scan_apps};

    #[test]
    fn tab_navigation_wraps_in_both_directions() {
        assert_eq!(super::adjacent_tab(Some(0), 3, false), Some(1));
        assert_eq!(super::adjacent_tab(Some(2), 3, false), Some(0));
        assert_eq!(super::adjacent_tab(Some(2), 3, true), Some(1));
        assert_eq!(super::adjacent_tab(Some(0), 3, true), Some(2));
    }

    #[test]
    fn tab_navigation_starts_from_the_result_list() {
        assert_eq!(super::adjacent_tab(None, 3, false), Some(0));
        assert_eq!(super::adjacent_tab(None, 3, true), Some(2));
        assert_eq!(super::adjacent_tab(Some(3), 3, false), Some(0));
    }

    #[test]
    fn tab_navigation_handles_no_tabs_and_single_tabs() {
        assert_eq!(super::adjacent_tab(None, 0, false), None);
        assert_eq!(super::adjacent_tab(None, 0, true), None);
        assert_eq!(super::adjacent_tab(Some(0), 1, false), Some(0));
        assert_eq!(super::adjacent_tab(Some(0), 1, true), Some(0));
    }

    #[test]
    fn finds_installed_applications() {
        let (apps, _) = scan_apps();
        assert!(!apps.is_empty());
        assert!(
            apps.iter()
                .all(|app| app.path.extension().is_some_and(|ext| ext == "app"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finds_apps_in_system_utilities() {
        let mut apps = Vec::new();
        super::scan_root(std::path::Path::new("/System/Applications"), &mut apps);
        assert!(apps.iter().any(|app| app.name == "Digital Color Meter"));
        assert!(apps.iter().any(|app| app.name == "Terminal"));
        assert!(apps.iter().all(|app| {
            !app.path
                .components()
                .any(|part| part.as_os_str() == "Contents")
        }));
    }

    #[test]
    fn fuzzy_matching_is_case_insensitive_and_ordered() {
        assert_eq!(fuzzy_score("safari", "saf"), Some(3));
        assert!(fuzzy_score("safari", "zzz").is_none());
    }

    #[test]
    fn control_keys_are_encoded_without_key_characters() {
        let control = gpui::Modifiers {
            control: true,
            ..Default::default()
        };
        for (key, byte) in [("c", 0x03), ("d", 0x04), ("space", 0x00)] {
            let keystroke = gpui::Keystroke {
                key: key.into(),
                key_char: None,
                modifiers: control,
            };
            assert_eq!(super::encode_key(&keystroke, false), Some(vec![byte]));
        }
    }

    #[test]
    fn terminal_metrics_fill_the_pane() {
        let menlo = super::terminal_metrics(6.6, 13., 612., 300.);
        assert_eq!((menlo.cols, menlo.rows), (92, 23));
        assert_eq!(menlo.line_height, 13.);

        let agave = super::terminal_metrics(7.5, 15., 612., 300.);
        assert_eq!((agave.cols, agave.rows), (81, 20));
        assert_eq!(agave.line_height, 15.);
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        let keystroke = |key: &str| gpui::Keystroke {
            key: key.into(),
            key_char: None,
            modifiers: Default::default(),
        };
        assert_eq!(
            super::encode_key(&keystroke("left"), false),
            Some(b"\x1b[D".to_vec())
        );
        assert_eq!(
            super::encode_key(&keystroke("left"), true),
            Some(b"\x1bOD".to_vec())
        );
        assert_eq!(
            super::encode_key(&keystroke("right"), true),
            Some(b"\x1bOC".to_vec())
        );
    }

    #[test]
    fn command_and_option_editing_keys_are_encoded() {
        let keystroke = |key: &str, modifiers: gpui::Modifiers| gpui::Keystroke {
            key: key.into(),
            key_char: None,
            modifiers,
        };
        let alt = gpui::Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            super::encode_key(&keystroke("left", alt), false),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            super::encode_command_key(&keystroke("delete", Default::default())),
            Some(vec![0x0b])
        );
        assert_eq!(
            super::encode_command_key(&keystroke("backspace", Default::default())),
            Some(vec![0x15])
        );
    }

    #[test]
    fn escape_is_encoded_for_the_terminal() {
        let keystroke = gpui::Keystroke {
            key: "escape".into(),
            key_char: None,
            modifiers: Default::default(),
        };
        assert_eq!(super::encode_key(&keystroke, false), Some(vec![0x1b]));
    }

    #[test]
    fn fuzzy_matching_returns_highlight_positions() {
        assert_eq!(
            super::fuzzy_match("Safari", "saf").unwrap().1,
            vec![0, 1, 2]
        );
        assert_eq!(super::command_highlight("pwd", "pwd"), vec![7, 8, 9]);
    }
}
